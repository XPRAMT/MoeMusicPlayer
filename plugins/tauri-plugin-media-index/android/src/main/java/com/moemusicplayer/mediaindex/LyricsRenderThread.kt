package com.moemusicplayer.mediaindex

import android.os.Handler
import android.os.HandlerThread
import android.os.Process
import android.util.Log
import android.view.Choreographer
import android.view.Surface
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

private const val TAG = "MoeMediaIndex"

/**
 * The floating lyrics' render thread (ADR 0032, GL overlay): a `HandlerThread` with its own `Choreographer`, so frames
 * follow whatever refresh rate the display is currently set to (nothing here asks for a rate). Each frame it reads the
 * newest [RenderState] and [LyricsClock] snapshot, plans ([LyricsFramePlanner]), and draws only when the picture
 * changed ([GlLyricsRenderer]). It asks for the next frame only while something moves; otherwise it sleeps until a
 * state change, a clock event, a finished texture, or the time of the next line. The main thread (shared with the
 * WebView) never draws.
 *
 * Failures never crash the app: a lost context is rebuilt; a surface that went away waits for the next one; repeated
 * failures, or a GL set-up that cannot work, call [onFatal] (on the render thread) once so the overlay can fall back to
 * the Views renderer.
 */
internal class LyricsRenderThread(private val onFatal: (String) -> Unit) {
    private val thread = HandlerThread("moe-lyrics-render", Process.THREAD_PRIORITY_DISPLAY).also { it.start() }
    private val handler = Handler(thread.looper)
    private val choreographer: Choreographer by lazy(LazyThreadSafetyMode.NONE) { Choreographer.getInstance() }
    private val renderer = GlLyricsRenderer({ handler.post(it) }, ::wake)

    @Volatile
    private var state: RenderState? = null

    @Volatile
    private var released = false
    private val wakePosted = AtomicBoolean(false)

    // Render thread only.
    private var surface: Surface? = null
    private var surfaceReady = false
    private var frameScheduled = false
    private var forceRedraw = true
    private var lastVersion = -1
    private var lastW = 0
    private var lastH = 0
    private var lastComplete = false
    private var fatalReported = false
    private val planner = LyricsFramePlanner()
    private val clockModel = ClockModel()
    private val current = FrameParams()
    private val last = FrameParams()
    private val size = IntArray(2)
    private val failureTimes = LongArray(MAX_FAILURES)
    private var failureCount = 0

    private val wakeTask = Runnable {
        wakePosted.set(false)
        scheduleFrame()
    }
    private val delayedWake = Runnable { scheduleFrame() }
    private val frameCallback = Choreographer.FrameCallback { doFrame(it) }

    // ---- any thread

    fun setState(value: RenderState) {
        state = value
        wake()
    }

    /** Something changed (state, clock event, a texture landed): draw a frame soon. */
    fun wake() {
        if (released) return
        if (wakePosted.compareAndSet(false, true)) handler.post(wakeTask)
    }

    fun attach(target: Surface) {
        if (released) return
        handler.post { onAttach(target) }
    }

    /** The surface was resized or redrawn by the system: draw it again. */
    fun surfaceChanged() {
        if (released) return
        handler.post {
            forceRedraw = true
            scheduleFrame()
        }
    }

    /** Stops drawing and releases the window surface; returns when the render thread no longer uses it (or after a timeout). */
    fun detach() {
        if (released) return
        val done = CountDownLatch(1)
        handler.post {
            try {
                onDetach()
            } finally {
                done.countDown()
            }
        }
        if (!done.await(DETACH_TIMEOUT_MS, TimeUnit.MILLISECONDS)) Log.w(TAG, "lyrics gl: the render thread did not release the surface in time")
    }

    /** Frees everything and ends the thread. */
    fun release() {
        if (released) return
        released = true
        handler.post {
            try {
                onDetach()
                renderer.release()
            } catch (error: Throwable) {
                Log.w(TAG, "lyrics gl: release failed", error)
            }
        }
        thread.quitSafely()
    }

    // ---- render thread

    private fun onAttach(target: Surface) {
        if (released) return
        surface = target
        surfaceReady = false
        if (!target.isValid) return
        try {
            renderer.attachSurface(target)
        } catch (lost: GlContextLostException) {
            try {
                renderer.dropContext()
                renderer.attachSurface(target)
            } catch (error: Throwable) {
                fatal("context lost and not recoverable", error)
                return
            }
        } catch (error: Throwable) {
            fatal("EGL / GL set-up failed", error)
            return
        }
        surfaceReady = true
        forceRedraw = true
        renderer.invalidatePrefetch()
        scheduleFrame()
    }

    private fun onDetach() {
        surfaceReady = false
        frameScheduled = false
        choreographer.removeFrameCallback(frameCallback)
        handler.removeCallbacks(delayedWake)
        try {
            renderer.destroySurface()
        } catch (error: Throwable) {
            Log.w(TAG, "lyrics gl: destroying the surface failed", error)
        }
        surface = null
        forceRedraw = true
    }

    private fun scheduleFrame() {
        if (frameScheduled || !surfaceReady || released) return
        frameScheduled = true
        choreographer.postFrameCallback(frameCallback)
    }

    private fun doFrame(frameTimeNanos: Long) {
        frameScheduled = false
        handler.removeCallbacks(delayedWake)
        if (!surfaceReady || released) return
        val value = state ?: return
        try {
            step(value, frameTimeNanos)
        } catch (lost: GlContextLostException) {
            Log.w(TAG, "lyrics gl: context lost, rebuilding")
            recover(lost)
        } catch (gone: GlSurfaceLostException) {
            // The surface went away under us; the next surfaceCreated attaches a new one.
            Log.w(TAG, "lyrics gl: surface lost")
            surfaceReady = false
        } catch (error: Throwable) {
            Log.w(TAG, "lyrics gl: frame failed", error)
            recover(error)
        }
    }

    private fun step(value: RenderState, frameTimeNanos: Long) {
        val tracing = LyricsTrace.on
        val started = if (tracing) System.nanoTime() else 0L
        renderer.store.setLooks(value.look, true)
        val uploaded = renderer.store.pump(UPLOAD_BUDGET_BYTES)
        val snapshot = LyricsClock.snapshot
        val position = clockModel.positionAt(snapshot, frameTimeNanos)
        planner.plan(value, snapshot, position, current)
        renderer.prefetch(value, current)
        if (!renderer.surfaceSize(size)) {
            scheduleFrame()
            return
        }
        val resized = size[0] != lastW || size[1] != lastH
        val redraw = forceRedraw || uploaded || resized || value.version != lastVersion || !lastComplete || !current.sameAs(last)
        if (redraw) {
            val complete = renderer.draw(value, current, size[0], size[1])
            val drawn = if (tracing) System.nanoTime() else 0L
            renderer.swap()
            last.set(current)
            lastVersion = value.version
            lastW = size[0]
            lastH = size[1]
            lastComplete = complete
            forceRedraw = false
            failureCount = 0
            if (tracing) LyricsTrace.glFrame(drawn - started)
        } else if (tracing) {
            LyricsTrace.glSkippedFrame()
        }
        if (current.animating || renderer.store.hasPendingWork()) {
            scheduleFrame()
        } else if (current.wakeAtLyricsMs != FloatingLyricsRules.NO_TIME && snapshot != null) {
            val speed = maxOf(0.05f, snapshot.speed)
            val delay = ((current.wakeAtLyricsMs - current.lyricsMs) / speed).toLong() - WAKE_EARLY_MS
            handler.postDelayed(delayedWake, delay.coerceIn(1L, MAX_IDLE_WAKE_MS))
        }
    }

    /** A frame failed: rebuild the context once; three failures within ten seconds give up (the overlay falls back). */
    private fun recover(error: Throwable) {
        val now = System.nanoTime()
        failureTimes[failureCount % MAX_FAILURES] = now
        failureCount++
        if (failureCount >= MAX_FAILURES) {
            val oldest = failureTimes[failureCount % MAX_FAILURES]
            if (now - oldest < FAILURE_WINDOW_NANOS) {
                fatal("repeated frame failures", error)
                return
            }
        }
        val target = surface
        surfaceReady = false
        try {
            renderer.dropContext()
        } catch (failure: Throwable) {
            Log.w(TAG, "lyrics gl: dropping the context failed", failure)
        }
        if (target != null && target.isValid) onAttach(target)
    }

    private fun fatal(reason: String, error: Throwable?) {
        surfaceReady = false
        if (error != null) Log.e(TAG, "lyrics gl: $reason", error) else Log.e(TAG, "lyrics gl: $reason")
        if (fatalReported) return
        fatalReported = true
        onFatal(reason)
    }

    private companion object {
        /** Texture upload per frame, about (bytes): more than that is spread over the next frames. */
        const val UPLOAD_BUDGET_BYTES = 1_500_000
        const val DETACH_TIMEOUT_MS = 500L
        const val WAKE_EARLY_MS = 4L
        const val MAX_IDLE_WAKE_MS = 2_000L
        const val MAX_FAILURES = 3
        const val FAILURE_WINDOW_NANOS = 10_000_000_000L
    }
}
