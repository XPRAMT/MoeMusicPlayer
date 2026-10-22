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
    /** Stored lyric area width (dp); on screen it is also kept within the screen (`overlayWidth`). */
    fun widthDp(value: Int): Int = value.coerceIn(100, 8192)

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

    /** The continuous strip always anchors by the right edge (`LYRICS_STRIP_ALIGN`), whatever the alignment setting. */
    const val STRIP_ALIGN = "right"

    /**
     * Continuous animation (ADR 0032, port of lyrics-strip.js): cells of the strip, each as wide as the wider of its
     * main text and translation, `gapPx` (one U+3000 of the main line) apart. Returns the cells' left edges and widths.
     */
    fun stripLayout(mainWidths: FloatArray, translationWidths: FloatArray, gapPx: Float): Pair<FloatArray, FloatArray> {
        val starts = FloatArray(mainWidths.size)
        val widths = FloatArray(mainWidths.size)
        var x = 0f
        for (index in mainWidths.indices) {
            val width = maxOf(0f, mainWidths[index], translationWidths.getOrElse(index) { 0f })
            starts[index] = x
            widths[index] = width
            x += width + maxOf(0f, gapPx)
        }
        return Pair(starts, widths)
    }

    /** The point of a cell that sits on the view's alignment point. */
    fun stripAnchor(cellStarts: FloatArray, cellWidths: FloatArray, index: Int, align: String): Float {
        val start = cellStarts.getOrElse(index) { 0f }
        val width = cellWidths.getOrElse(index) { 0f }
        return when (align) {
            "right" -> start + width
            "center" -> start + width / 2f
            else -> start
        }
    }

    /**
     * Shift of the strip (px) at lyrics time `lyricsMs` (position + offset): the current line's anchor on the alignment
     * point, gliding linearly to the next line's anchor until that line starts; rests on the first line before it and on
     * the last one after it.
     */
    fun stripOffset(
        startTimesMs: LongArray,
        cellStarts: FloatArray,
        cellWidths: FloatArray,
        align: String,
        lyricsMs: Long,
        viewWidth: Float,
    ): Float {
        val count = minOf(startTimesMs.size, cellStarts.size)
        if (count == 0) return 0f
        val target = when (align) {
            "right" -> viewWidth
            "center" -> viewWidth / 2f
            else -> 0f
        }
        var index = -1
        while (index + 1 < count && startTimesMs[index + 1] <= lyricsMs) index++
        val anchor = when {
            index < 0 -> stripAnchor(cellStarts, cellWidths, 0, align)
            index >= count - 1 -> stripAnchor(cellStarts, cellWidths, count - 1, align)
            else -> {
                val span = startTimesMs[index + 1] - startTimesMs[index]
                val fraction = if (span > 0) ((lyricsMs - startTimesMs[index]).toFloat() / span).coerceIn(0f, 1f) else 1f
                val from = stripAnchor(cellStarts, cellWidths, index, align)
                from + (stripAnchor(cellStarts, cellWidths, index + 1, align) - from) * fraction
            }
        }
        return target - anchor
    }

    /** Suffix of the saved position keys: portrait and landscape are saved apart. */
    fun orientationKey(landscape: Boolean): String = if (landscape) "landscape" else "portrait"

    /**
     * Overlay width in px: the lyric area (the chosen width, narrowed so it and the lock gutter fit the screen) plus
     * the `gutterPx` lock gutter on its right.
     */
    fun overlayWidth(chosenPx: Int, screenWidth: Int, gutterPx: Int): Int {
        val gutter = maxOf(0, gutterPx)
        return maxOf(1, minOf(chosenPx, screenWidth - gutter)) + gutter
    }

    /** Left edge that puts the lyric area (the overlay less its lock gutter) at the centre of `available`. */
    fun lyricCentredStart(available: Int, overlayWidth: Int, gutterPx: Int, leadPx: Int = 0): Int =
        centredStart(available, maxOf(0, overlayWidth - maxOf(0, gutterPx))) - maxOf(0, leadPx)

    /** Transparent side margin (px) for the shadow / glow: a quarter of the main font, at least 1. */
    fun sideMarginPx(fontPx: Float): Int = maxOf(1, kotlin.math.ceil(fontPx * 0.25f).toInt())

    /** Room above and below a lyric area `areaPx` tall (text height × 1.1) so the panel is text height × 1.5. */
    fun panelRoomPx(areaPx: Int): Int = if (areaPx <= 0) 0 else kotlin.math.ceil(areaPx * 2.0 / 11.0).toInt()

    /** Top of a panel `heightPx` tall whose vertical centre is the status bar's (can be above the screen top). */
    fun centredOnBar(barHeightPx: Int, heightPx: Int): Int = (barHeightPx - heightPx) / 2

    /**
     * The playback position for this display frame. ExoPlayer's reported position only advances in coarse steps (it
     * is refreshed when the playback thread does work, every ~100-200 ms for audio), so while playing it is carried on
     * from the last report by the time since then. A report a little behind the carried-on value does not pull the
     * text back (no jitter); a jump (seek, a new track, a stall) of more than `resyncMs` is taken as is.
     */
    fun smoothPositionMs(
        previousMs: Long?,
        reportedMs: Long,
        reportedAtMs: Long,
        nowMs: Long,
        playing: Boolean,
        resyncMs: Long = 250,
    ): Long {
        if (!playing) return reportedMs
        val carried = reportedMs + maxOf(0L, nowMs - reportedAtMs)
        if (previousMs == null) return carried
        val drift = carried - previousMs
        return if (drift < 0 && drift > -resyncMs) previousMs else carried
    }

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