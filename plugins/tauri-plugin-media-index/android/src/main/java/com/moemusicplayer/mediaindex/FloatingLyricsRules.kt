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

    /** The continuous strip anchors a line by its LEFT edge (`LYRICS_STRIP_ALIGN`) ... */
    const val STRIP_ALIGN = "left"

    /** ... on the lyric area's horizontal CENTRE (`LYRICS_STRIP_TARGET`), whatever the alignment setting and panel width. */
    const val STRIP_TARGET = "center"

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

    /**
     * The cells to draw (port of lyricsStripWindow): those reaching into the view widened by `marginPx` on both sides
     * at shift `offset`, found by binary search on the sorted cells, so a frame never walks the whole song.
     */
    fun stripWindow(cellStarts: FloatArray, cellWidths: FloatArray, offset: Float, viewWidth: Float, marginPx: Float): IntRange {
        val left = -offset - maxOf(0f, marginPx)
        val right = -offset + maxOf(0f, viewWidth) + maxOf(0f, marginPx)
        var lo = 0
        var hi = cellStarts.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (cellStarts[mid] + cellWidths.getOrElse(mid) { 0f } < left) lo = mid + 1 else hi = mid
        }
        var last = lo - 1
        while (last + 1 < cellStarts.size && cellStarts[last + 1] <= right) last++
        return lo..last
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
        targetAlign: String = align,
    ): Float {
        val count = minOf(startTimesMs.size, cellStarts.size)
        if (count == 0) return 0f
        val target = when (targetAlign) {
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

    /** Same range as `lyricsPreferences.inactiveOpacityPercent` (lyrics-preferences.js); 50 is its default. */
    const val DEFAULT_INACTIVE_OPACITY_PERCENT = 50
    fun inactiveOpacityPercent(value: Int): Int = value.coerceIn(10, 100)

    /** The continuous strip fades a line over this long, ending exactly when the next line starts. */
    const val STRIP_FADE_MS = 200L

    /**
     * How much strip cell `index` is "the current line", 0..1, from the lyrics time alone (no timer, so a seek or the
     * rest before the first / after the last line need no special case). A line ramps 0 to 1 over the `fadeMs` before
     * its own start and ramps back down over the `fadeMs` before the next line's start, so the old line dims while
     * the next brightens and both finish when the next line starts (as the strip's glide ends). The last line stays 1.
     * Lines sharing a start time are never current (the later one is).
     */
    fun cellWeight(startTimesMs: LongArray, index: Int, lyricsMs: Long, fadeMs: Long = STRIP_FADE_MS): Float {
        if (index < 0 || index >= startTimesMs.size) return 0f
        val fade = maxOf(1L, fadeMs)
        val start = startTimesMs[index]
        val rise = ((lyricsMs - (start - fade)).toFloat() / fade).coerceIn(0f, 1f)
        if (index + 1 >= startTimesMs.size) return rise
        val next = startTimesMs[index + 1]
        if (next <= start) return 0f
        val fall = ((lyricsMs - (next - fade)).toFloat() / fade).coerceIn(0f, 1f)
        return rise * (1f - fall)
    }

    /** Index of the first line starting after `lyricsMs` (the size when none does); lines are sorted. */
    fun firstStartAfter(startTimesMs: LongArray, lyricsMs: Long): Int {
        var low = 0
        var high = startTimesMs.size
        while (low < high) {
            val middle = (low + high) ushr 1
            if (startTimesMs[middle] <= lyricsMs) low = middle + 1 else high = middle
        }
        return low
    }

    /** True while some line is within its `fadeMs` lead-in, i.e. the cells' fades change from frame to frame. */
    fun fadeActive(startTimesMs: LongArray, lyricsMs: Long, fadeMs: Long = STRIP_FADE_MS): Boolean {
        val next = firstStartAfter(startTimesMs, lyricsMs)
        return next < startTimesMs.size && startTimesMs[next] - lyricsMs < fadeMs
    }

    /**
     * Opacity of a cell relative to the main opacity (which dims the whole overlay): 1 for the current line, the
     * inactive opacity for the others, in between while fading. Only the continuous animation fades; "none" keeps 1.
     */
    fun inactiveFactor(animation: String, inactivePercent: Int, weight: Float): Float {
        if (animation != "continuous") return 1f
        return fadedFactor(inactiveOpacityPercent(inactivePercent) / 100f, weight)
    }

    /** `inactive` (the factor of a line that is not current) lifted toward 1 by `weight` (0..1). */
    fun fadedFactor(inactive: Float, weight: Float): Float = inactive + (1f - inactive) * weight.coerceIn(0f, 1f)

    /** What reaches the screen for a cell: main opacity times its factor (80% main, 50% inactive: 0.4). */
    fun cellOpacity(animation: String, opacityPercent: Int, inactivePercent: Int, weight: Float): Float =
        opacityPercent(opacityPercent) / 100f * inactiveFactor(animation, inactivePercent, weight)

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