package com.moemusicplayer.mediaindex

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.Trace
import android.util.Log
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * Diagnostics of the floating lyrics' cost (ADR 0032), off by default and free when off: every call site checks [on]
 * (a plain boolean) first. Turn it on with the system property `debug.moe.lyrics.trace` = 1 or the
 * `moe_floating_lyrics` SharedPreferences boolean `trace`; it is read when the overlay window is created. When on:
 * `android.os.Trace` sections (`moeLyrics.render`, `moeLyrics.draw`, `moeLyrics.applyData`, `moeLyrics.raster`), and
 * every 5 s one logcat line (tag MoeMediaIndex) with the main-thread time of render() plus onDraw per frame (p50 / p99)
 * and the counters: invalidates, rasterise calls (and their total ms on the worker), and cell draws that found no
 * finished image ("fallback").
 */
internal object LyricsTrace {
    private const val TAG = "MoeMediaIndex"
    private const val REPORT_MS = 5_000L
    private const val MAX_FRAMES = 2048

    @Volatile
    @JvmField
    var on = false

    private val invalidates = AtomicInteger()
    private val rasterCalls = AtomicInteger()
    private val rasterNanos = AtomicLong()
    private val fallbacks = AtomicInteger()

    // Main thread only: the per-frame main-thread time (render + onDraws) in nanoseconds.
    private val frames = LongArray(MAX_FRAMES)
    private var frameCount = 0
    private var accumulated = 0L

    private val main = Handler(Looper.getMainLooper())
    private val reporter = object : Runnable {
        override fun run() {
            if (!on) return
            report()
            main.postDelayed(this, REPORT_MS)
        }
    }

    /** Read the switch (called when the overlay window is created) and start or stop the periodic report. */
    fun configure(context: Context) {
        val wanted = systemProperty("debug.moe.lyrics.trace") == "1" ||
            context.applicationContext.getSharedPreferences("moe_floating_lyrics", Context.MODE_PRIVATE).getBoolean("trace", false)
        main.removeCallbacks(reporter)
        on = wanted
        if (wanted) {
            reset()
            main.postDelayed(reporter, REPORT_MS)
        }
    }

    fun stop() {
        main.removeCallbacks(reporter)
        on = false
    }

    private fun systemProperty(name: String): String = try {
        Class.forName("android.os.SystemProperties").getMethod("get", String::class.java, String::class.java)
            .invoke(null, name, "") as? String ?: ""
    } catch (_: Throwable) {
        ""
    }

    /** Open a section on this thread; the return value goes to [end]. */
    fun begin(name: String): Long {
        Trace.beginSection(name)
        return System.nanoTime()
    }

    /** Close the section; `mainThread` time counts towards the frame's total. */
    fun end(startedNanos: Long, mainThread: Boolean) {
        if (mainThread) accumulated += System.nanoTime() - startedNanos
        Trace.endSection()
    }

    /** A new display frame starts: file the previous frame's total. */
    fun frameBoundary() {
        if (accumulated > 0L) {
            if (frameCount < MAX_FRAMES) frames[frameCount++] = accumulated
            accumulated = 0L
        }
    }

    fun invalidated() {
        invalidates.incrementAndGet()
    }

    fun fellBack() {
        fallbacks.incrementAndGet()
    }

    fun rasterised(width: Int, height: Int, nanos: Long) {
        rasterCalls.incrementAndGet()
        rasterNanos.addAndGet(nanos)
        Log.i(TAG, "lyrics raster ${width}x$height ${nanos / 1_000}us")
    }

    private fun reset() {
        invalidates.set(0)
        rasterCalls.set(0)
        rasterNanos.set(0)
        fallbacks.set(0)
        frameCount = 0
        accumulated = 0L
    }

    private fun report() {
        val count = frameCount
        val sorted = frames.copyOf(count).also { it.sort() }
        fun percentile(p: Double): Long = if (count == 0) 0L else sorted[minOf(count - 1, (count * p).toInt())]
        Log.i(
            TAG,
            "lyrics trace 5s: frames=$count main p50=${percentile(0.5) / 1_000}us p99=${percentile(0.99) / 1_000}us " +
                "invalidates=${invalidates.get() / 5}/s rasterCalls=${rasterCalls.get()} rasterMs=${rasterNanos.get() / 1_000_000} " +
                "fallbackDraws=${fallbacks.get()}",
        )
        reset()
    }
}
