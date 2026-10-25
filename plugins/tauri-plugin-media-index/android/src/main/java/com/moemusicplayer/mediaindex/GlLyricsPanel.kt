package com.moemusicplayer.mediaindex

import android.content.Context
import android.graphics.PixelFormat
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.util.Log
import android.view.SurfaceHolder
import android.view.SurfaceView
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

private const val TAG = "MoeMediaIndex"

/**
 * The GL renderer's part of the overlay window (ADR 0032): a `SurfaceView` (translucent, default z-order, so the
 * overlay's real lock button stays above it), the render thread that draws into it, and the builder thread that turns
 * the overlay's [RenderInputs] into the immutable [RenderState] the render thread reads. The main thread only creates
 * and forwards: it never plans, rasterises or draws. [onFatal] runs on the main thread, once, when GL cannot work.
 */
internal class GlLyricsPanel(context: Context, private val onFatal: (String) -> Unit) {
    val surfaceView = SurfaceView(context)

    private val mainHandler = Handler(Looper.getMainLooper())
    private val renderThread = LyricsRenderThread { reason -> mainHandler.post { if (!destroyed) onFatal(reason) } }
    private val builder = RenderBuilder { size, weight -> PaintTextMetrics(size, weight) }
    private val pending = AtomicReference<RenderInputs?>()
    private val scheduled = AtomicBoolean(false)
    private val executor: ThreadPoolExecutor by lazy {
        ThreadPoolExecutor(1, 1, 20, TimeUnit.SECONDS, LinkedBlockingQueue(), { task ->
            Thread({
                Process.setThreadPriority(Process.THREAD_PRIORITY_LESS_FAVORABLE)
                task.run()
            }, "moe-lyrics-build")
        }).apply { allowCoreThreadTimeOut(true) }
    }

    @Volatile
    private var destroyed = false
    private val wakeHook: () -> Unit = { renderThread.wake() }

    init {
        surfaceView.holder.setFormat(PixelFormat.TRANSLUCENT)
        surfaceView.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                if (!destroyed) renderThread.attach(holder.surface)
            }

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                if (!destroyed) renderThread.surfaceChanged()
            }

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                // The surface must not be used once this returns: wait for the render thread to let go of it.
                if (!destroyed) renderThread.detach()
            }
        })
        LyricsClock.demand.incrementAndGet()
        LyricsClock.onWake = wakeHook
    }

    /** Hand over the latest inputs (the newest wins); the model and look are built off the main thread. */
    fun submit(inputs: RenderInputs) {
        if (destroyed) return
        pending.set(inputs)
        if (scheduled.compareAndSet(false, true)) runBuild()
    }

    private fun buildPending() {
        while (true) {
            val inputs = pending.getAndSet(null) ?: break
            try {
                renderThread.setState(builder.build(inputs))
            } catch (error: Throwable) {
                Log.w(TAG, "lyrics gl: building the render state failed", error)
            }
        }
        scheduled.set(false)
        if (pending.get() != null && scheduled.compareAndSet(false, true)) runBuild()
    }

    private fun runBuild() {
        try {
            executor.execute(::buildPending)
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            scheduled.set(false)
        }
    }

    /** 「暫停時隱藏」: no frames while the window is faded out (see [LyricsRenderThread.setSuspended]). */
    fun setSuspended(value: Boolean) {
        if (!destroyed) renderThread.setSuspended(value)
    }

    /** Stop everything: the render thread (and its EGL objects) end, the clock no longer needs to be polled. */
    fun destroy() {
        if (destroyed) return
        destroyed = true
        if (LyricsClock.onWake === wakeHook) LyricsClock.onWake = null
        LyricsClock.demand.decrementAndGet()
        pending.set(null)
        // Let go of the surface before the window (and with it the surface) is removed.
        renderThread.detach()
        renderThread.release()
        executor.shutdown()
    }
}
