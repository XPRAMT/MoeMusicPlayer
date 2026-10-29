package com.moemusicplayer.mediaindex

import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.WindowManager

/**
 * The one-time carry-over of the floating lyrics values the overlay used to keep in its SharedPreferences (ADR 0032):
 * `locked`, `x_portrait` / `y_portrait`, `x_landscape` / `y_landscape` (px of the lyric area's top-left). They go to
 * settings.json through Rust ([FloatingLyricsStore.importLegacy]; a position already there is kept), converted to
 * ratios with the screen of each orientation now. Only once Rust has written them are the old keys removed; the file
 * then holds nothing but the debug flag `trace` ([LyricsTrace]) and the `migrated` marker. When Rust cannot take them
 * yet, nothing is removed and the next overlay update tries again. Runs on a thread of its own (the import writes the
 * settings file).
 */
internal object FloatingLyricsLegacyPrefs {
    private const val PREFS = "moe_floating_lyrics"
    private const val MIGRATED = "migrated"
    private const val TAG = "MoeMediaIndex"
    /** Keys that stay in the file: the debug flag and the marker. */
    private val KEEP = setOf("trace", MIGRATED)

    @Volatile private var done = false
    @Volatile private var running = false

    /** Starts the carry-over unless it is done or running; `then` runs on the main thread after values were moved. */
    fun migrateOnce(context: Context, then: () -> Unit) {
        if (done || running) return
        running = true
        val app = context.applicationContext
        Thread({
            try {
                if (migrate(app)) Handler(Looper.getMainLooper()).post(then)
            } catch (error: Throwable) {
                Log.w(TAG, "floating lyrics: carrying the old values over failed", error)
            } finally {
                running = false
            }
        }, "floating-lyrics-migrate").start()
    }

    /** True when values were carried over (the store then holds them). */
    private fun migrate(context: Context): Boolean {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (prefs.getBoolean(MIGRATED, false)) {
            done = true
            return false
        }
        fun pair(suffix: String): Pair<Int, Int>? =
            if (prefs.contains("x_$suffix") && prefs.contains("y_$suffix")) {
                Pair(prefs.getInt("x_$suffix", 0), prefs.getInt("y_$suffix", 0))
            } else {
                null
            }
        val locked = if (prefs.contains("locked")) prefs.getBoolean("locked", false) else null
        val portrait = pair(FloatingLyricsRules.orientationKey(false))
        val landscape = pair(FloatingLyricsRules.orientationKey(true))
        val moved = locked != null || portrait != null || landscape != null
        if (moved) {
            val (width, height) = screenSize(context)
            val (oldLocked, portraitRatio, landscapeRatio) =
                FloatingLyricsRules.legacyPlacement(locked, portrait, landscape, width, height)
            if (!FloatingLyricsStore.importLegacy(FloatingLyricsStore.legacyJson(oldLocked, portraitRatio, landscapeRatio))) {
                // Rust has not read the settings yet: keep everything and try again later.
                return false
            }
        }
        val edit = prefs.edit()
        for (key in prefs.all.keys) if (key !in KEEP) edit.remove(key)
        edit.putBoolean(MIGRATED, true)
        if (!edit.commit()) return false
        done = true
        if (moved) FloatingLyricsStore.reload()
        return moved
    }

    /** The whole display now (as the overlay measures it). */
    private fun screenSize(context: Context): Pair<Int, Int> {
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bounds = windowManager.maximumWindowMetrics.bounds
            return Pair(bounds.width(), bounds.height())
        }
        val metrics = android.util.DisplayMetrics()
        @Suppress("DEPRECATION") windowManager.defaultDisplay.getRealMetrics(metrics)
        return Pair(metrics.widthPixels, metrics.heightPixels)
    }
}
