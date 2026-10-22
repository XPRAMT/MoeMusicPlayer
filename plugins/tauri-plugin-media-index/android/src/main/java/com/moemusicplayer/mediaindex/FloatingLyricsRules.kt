package com.moemusicplayer.mediaindex

/**
 * Pure rules of the Android floating lyrics (ADR 0032), ports of the web ones so the overlay shows the same line at the
 * same moment: `findActiveLyricIndex` (lyrics-window.js), the line duration and `getLyricScrollProgress`
 * (TaskbarLyricsView / taskbar-lyrics-settings.js) and the effect colour (`getTaskbarLyricsEffectRgb`).
 */
object FloatingLyricsRules {
    /** How long the last line of a track is assumed to stay when the track length is unknown. */
    const val LAST_LINE_FALLBACK_MS = 5_000L

    /** Same range as settings.rs `OPACITY_RANGE_PERCENT`. */
    fun opacityPercent(value: Int): Int = value.coerceIn(1, 100)

    /** Same range as settings.rs `FloatingLyricsSettings::WIDTH_RANGE_PX` (dp here). */
    fun widthDp(value: Int): Int = value.coerceIn(200, 4000)

    /** Font px whose line (text height × 1.1, text height = `emHeight` × px) is at most `lineHeight` px. */
    fun fontForLineHeight(lineHeight: Int, emHeight: Float): Float =
        maxOf(1f, Math.floor((lineHeight / (emHeight * 1.1f)).toDouble()).toFloat())

    /**
     * Left edge in the status bar strip: centred when there is no top cutout; otherwise beside the camera, on the
     * side with more room (`gap` px away), kept on the screen.
     */
    fun statusBarX(screenWidth: Int, width: Int, cutout: Pair<Int, Int>?, gap: Int): Int {
        val maxX = maxOf(0, screenWidth - width)
        if (cutout == null) return centredStart(screenWidth, width)
        val (left, right) = cutout
        val x = if (screenWidth - right >= left) right + gap else left - gap - width
        return x.coerceIn(0, maxX)
    }

    /** Suffix of the saved position keys: portrait and landscape are saved apart. */
    fun orientationKey(landscape: Boolean): String = if (landscape) "landscape" else "portrait"

    /** Overlay width in px: the chosen width, narrowed to the screen. */
    fun overlayWidth(chosenPx: Int, screenWidth: Int): Int = maxOf(1, minOf(chosenPx, screenWidth))

    /** Height of one drawn line: the text's own height (ascent to descent) with a 10% buffer. */
    fun lineHeightPx(ascent: Float, descent: Float): Int = Math.ceil(((descent - ascent) * 1.1f).toDouble()).toInt()

    /** Window alpha from the opacity setting, capped by the system's touch-through limit when there is one. */
    fun windowAlpha(opacityPercent: Int, systemLimit: Float?): Float {
        val wanted = opacityPercent(opacityPercent) / 100f
        return if (systemLimit == null) wanted else minOf(wanted, systemLimit)
    }

    /**
     * Left edge of a line `textWidth` wide in `available`: by the alignment when it fits, otherwise starting at the left
     * and sliding left by `progress` of the overflow (the web marquee rule).
     */
    fun lineStartX(available: Float, textWidth: Float, align: String, progress: Float): Float {
        val overflow = textWidth - available
        if (overflow > 0f) return -overflow * progress.coerceIn(0f, 1f)
        return when (align) {
            "left" -> 0f
            "right" -> available - textWidth
            else -> (available - textWidth) / 2f
        }
    }

    /** Start coordinate that centres `size` in `available` (never negative). */
    fun centredStart(available: Int, size: Int): Int = maxOf(0, (available - size) / 2)

    /** Index of the last line starting at or before `positionMs + offsetMs`; -1 before the first. */
    fun activeIndex(starts: LongArray, positionMs: Long, offsetMs: Long): Int {
        if (starts.isEmpty()) return -1
        val target = positionMs + offsetMs
        var low = 0
        var high = starts.size
        while (low < high) {
            val middle = low + (high - low) / 2
            if (starts[middle] <= target) low = middle + 1 else high = middle
        }
        return low - 1
    }

    /** From the line's start to the next line's (the end of the track for the last one); at least one second. */
    fun lineDurationMs(starts: LongArray, index: Int, durationMs: Long?, offsetMs: Long): Long {
        val start = starts[index]
        val next = if (index + 1 < starts.size) starts[index + 1]
        else if (durationMs != null && durationMs > 0) durationMs - offsetMs
        else start + LAST_LINE_FALLBACK_MS
        return maxOf(1_000L, next - start)
    }

    /** 0..1: a line wider than the overlay scrolls over 90% of its time and rests for the last tenth. */
    fun scrollProgress(elapsedMs: Long, durationMs: Long): Float {
        if (durationMs <= 0) return 0f
        return (elapsedMs.toFloat() / (durationMs * 0.9f)).coerceIn(0f, 1f)
    }

    /** Black behind light text, white behind dark text (Rec. 601 brightness), as 0xRRGGBB. */
    fun effectRgb(textRgb: Int): Int {
        val red = (textRgb shr 16) and 0xff
        val green = (textRgb shr 8) and 0xff
        val blue = textRgb and 0xff
        val brightness = 0.299 * red + 0.587 * green + 0.114 * blue
        return if (brightness >= 128) 0x000000 else 0xffffff
    }

    /** `#rrggbb` to 0xRRGGBB, or null. */
    fun parseHexColor(value: String?): Int? {
        if (value == null || value.length != 7 || value[0] != '#') return null
        return value.substring(1).toIntOrNull(16)
    }

    /**
     * Keep the overlay fully on the whole display, status bar area included (sizes in px; a view larger than the
     * screen sticks to the top-left).
     */
    fun clampPosition(x: Int, y: Int, width: Int, height: Int, screenWidth: Int, screenHeight: Int): Pair<Int, Int> =
        Pair(x.coerceIn(0, maxOf(0, screenWidth - width)), y.coerceIn(0, maxOf(0, screenHeight - height)))
}