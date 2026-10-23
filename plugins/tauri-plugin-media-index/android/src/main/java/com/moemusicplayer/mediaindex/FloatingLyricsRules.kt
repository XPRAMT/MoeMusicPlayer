package com.moemusicplayer.mediaindex

/**
 * Pure rules of the Android floating lyrics (ADR 0032), ports of the web ones so the overlay shows the same line at the
 * same moment: `findActiveLyricIndex` (lyrics-window.js), the line duration and `getLyricScrollProgress`
 * (TaskbarLyricsView / taskbar-lyrics-settings.js) and the effect colour (`getTaskbarLyricsEffectRgb`).
 */
object FloatingLyricsRules {
    /** How long the last line of a track is assumed to stay when the track length is unknown. */
    const val LAST_LINE_FALLBACK_MS = 5_000L
    /** Terminal continuous-lyrics glide estimate per user-perceived character. */
    const val LYRICS_MS_PER_GRAPHEME = 500L

    /** Estimate the terminal glide end from the last line's graphemes, capped at track end when known. */
    fun finalLineArrivalMs(lineStartMs: Long, text: String, trackEndMs: Long?): Long {
        val iterator = java.text.BreakIterator.getCharacterInstance(java.util.Locale.ROOT)
        iterator.setText(text)
        var graphemes = 0
        iterator.first()
        while (iterator.next() != java.text.BreakIterator.DONE) graphemes++
        val estimated = lineStartMs + maxOf(1, graphemes) * LYRICS_MS_PER_GRAPHEME
        return trackEndMs?.let { minOf(estimated, it) } ?: estimated
    }

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
     * Shift of the strip (px) at lyrics time `lyricsMs` (position + offset), port of `lyricsStripCubic`: a monotone C1
     * cubic Hermite through the knots (line start, that line's anchor on the alignment point) and the terminal knot
     * (`finalArrivalMs`, the last main-text glyph centre), so the speed never jumps at a line start. Tangents are 0 at
     * the first and last knot (the strip rests before / after them) and, inside, the weighted harmonic mean of the two
     * neighbouring slopes (Fritsch-Butland; 0 when either is 0 or a neighbour has the same time), which keeps it
     * monotone and never faster than 3x the slower neighbour. Each segment reads only its four nearest knots.
     */
    fun stripOffset(
        startTimesMs: LongArray,
        cellStarts: FloatArray,
        cellWidths: FloatArray,
        align: String,
        lyricsMs: Long,
        viewWidth: Float,
        targetAlign: String = align,
        finalArrivalMs: Long? = null,
        mainTextWidths: FloatArray = cellWidths,
        lastGlyphCenters: FloatArray = FloatArray(0),
    ): Float {
        val count = minOf(startTimesMs.size, cellStarts.size)
        if (count == 0) return 0f
        val target = when (targetAlign) {
            "right" -> viewWidth.toDouble()
            "center" -> viewWidth / 2.0
            else -> 0.0
        }
        val last = count - 1
        val end = finalArrivalMs?.takeIf { it > startTimesMs[last] }
        val knots = count + if (end != null) 1 else 0
        fun time(k: Int): Double = (if (k < count) startTimesMs[k] else end!!).toDouble()
        fun value(k: Int): Double {
            if (k < count) return stripAnchor(cellStarts, cellWidths, k, align).toDouble()
            val mainWidth = maxOf(0f, mainTextWidths.getOrElse(last) { cellWidths.getOrElse(last) { 0f } })
            val glyphCenter = lastGlyphCenters.getOrElse(last) { mainWidth }.coerceIn(0f, mainWidth)
            return maxOf(
                stripAnchor(cellStarts, cellWidths, last, align).toDouble(),
                cellStarts.getOrElse(last) { 0f }.toDouble() + glyphCenter,
            )
        }
        fun tangent(k: Int): Double {
            if (k <= 0 || k >= knots - 1) return 0.0
            val h0 = time(k) - time(k - 1)
            val h1 = time(k + 1) - time(k)
            if (h0 <= 0 || h1 <= 0) return 0.0
            val d0 = (value(k) - value(k - 1)) / h0
            val d1 = (value(k + 1) - value(k)) / h1
            if (d0 <= 0 || d1 <= 0) return 0.0
            val w0 = 2 * h1 + h0
            val w1 = h1 + 2 * h0
            return (w0 + w1) / (w0 / d0 + w1 / d1)
        }
        val p = lyricsMs.toDouble()
        var index = -1
        while (index + 1 < knots && time(index + 1) <= p) index++
        if (index < 0) return (target - value(0)).toFloat()
        if (index >= knots - 1) return (target - value(knots - 1)).toFloat()
        val t0 = time(index)
        val h = time(index + 1) - t0
        val v0 = value(index)
        val m0 = tangent(index)
        val m1 = tangent(index + 1)
        val d = (value(index + 1) - v0) / h
        val c2 = (3 * d - 2 * m0 - m1) / h
        val c3 = (m0 + m1 - 2 * d) / (h * h)
        val u = p - t0
        return (target - (v0 + u * (m0 + u * (c2 + u * c3)))).toFloat()
    }

    /**
     * The effect drawn: the selected one in every mode, as on Windows. (Continuous mode used to swap the shadow for the
     * outline; lines are now rasterised once with their shadow, LineRasterCache, so the glide never blurs per frame.)
     */
    @Suppress("UNUSED_PARAMETER")
    fun effectiveTextEffect(animation: String, preference: String): String = preference

    /**
     * Shadow look matching Windows (`text-shadow: 0 0 0.6em`, D2D shadow sigma 0.3 em, 75 % alpha): the
     * `Paint.setShadowLayer` radius for a `fontPx` font. Skia turns a radius r into sigma 0.57735 r + 0.5.
     */
    fun shadowLayerRadius(fontPx: Float): Float = maxOf(1f, (fontPx * SHADOW_SIGMA_EM - 0.5f) / 0.57735f)

    /** Blur reach (3 sigma) that a rasterised line keeps around its text so the shadow is not cut. */
    fun shadowPadPx(fontPx: Float): Int = kotlin.math.ceil(3f * fontPx * SHADOW_SIGMA_EM).toInt()

    const val SHADOW_SIGMA_EM = 0.3f
    const val SHADOW_ALPHA = 0xBF

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
     * from the last report by the time since then. A difference between that and the position shown on the previous
     * frame of less than `resyncMs` is never stepped over: the shown position keeps running from where it was, a
     * little fast or slow (`slewMs` time constant), so it stays continuous with a continuous speed and never moves
     * back. A larger jump (seek, a new track, a stall) is taken as is.
     */
    fun smoothPositionMs(
        previousMs: Double?,
        previousAtMs: Long,
        reportedMs: Long,
        reportedAtMs: Long,
        nowMs: Long,
        playing: Boolean,
        resyncMs: Long = 250,
        slewMs: Double = 750.0,
    ): Double {
        if (!playing) return reportedMs.toDouble()
        val carried = (reportedMs + maxOf(0L, nowMs - reportedAtMs)).toDouble()
        if (previousMs == null) return carried
        val step = maxOf(0L, nowMs - previousAtMs).toDouble()
        val drift = carried - (previousMs + step)
        if (kotlin.math.abs(drift) >= resyncMs) return carried
        val rate = (1.0 + drift / slewMs).coerceIn(0.5, 1.5)
        return previousMs + step * rate
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
     * rest before the first / after the last line need no special case). Before the first line starts nothing is faded
     * out: every cell is 1 (the main opacity). Over the `fadeMs` before the first line's start the other cells ease
     * from 1 down to 0 (the first line stays 1), the same lead-in every later hand-over uses: a line ramps 0 to 1 over
     * the `fadeMs` before its own start and back down over the `fadeMs` before the next line's start, so the old line
     * dims while the next brightens and both finish when the next line starts (as the strip's glide ends). The last
     * line stays 1. Lines sharing a start time are never current (the later one is).
     */
    fun cellWeight(startTimesMs: LongArray, index: Int, lyricsMs: Long, fadeMs: Long = STRIP_FADE_MS): Float {
        if (index < 0 || index >= startTimesMs.size) return 0f
        val fade = maxOf(1L, fadeMs)
        // 1 until the first line's lead-in, then down to 0 as the first line starts: lifts every cell before it.
        val before = ((startTimesMs[0] - lyricsMs).toFloat() / fade).coerceIn(0f, 1f)
        val start = startTimesMs[index]
        // The first line has no hand-over to wait for: it is fully current from the start of the song.
        val rise = if (index == 0) 1f else ((lyricsMs - (start - fade)).toFloat() / fade).coerceIn(0f, 1f)
        if (index + 1 >= startTimesMs.size) return maxOf(rise, before)
        val next = startTimesMs[index + 1]
        if (next <= start) return before
        val fall = ((lyricsMs - (next - fade)).toFloat() / fade).coerceIn(0f, 1f)
        return maxOf(rise * (1f - fall), before)
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
     * Device-pixel rule (AGENTS.md): a moving position is redrawn only when it crosses into another whole pixel, so the
     * overlay draws nothing while the text has not visibly moved; once it stops (`raw` equal to the previous frame's
     * `previousRaw`) the exact position is drawn, so every target is reached exactly. `shown` is the position last drawn;
     * between redraws it lags `raw` by less than one pixel.
     */
    fun needsPixelRedraw(shown: Float, raw: Float, previousRaw: Float): Boolean =
        kotlin.math.floor(raw) != kotlin.math.floor(shown) || (raw == previousRaw && raw != shown)

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
    /**
     * Keeps the panel's lyric text area on the screen; the transparent `inset`s (the side margin left of the lyric
     * area, the margin right of the lock so it stays reachable, the room above and below the lyric area) may go past
     * the edges.
     */
    fun clampPosition(
        x: Int, y: Int, width: Int, height: Int, screenWidth: Int, screenHeight: Int,
        insetLeft: Int = 0, insetTop: Int = 0, insetRight: Int = 0, insetBottom: Int = 0,
    ): Pair<Int, Int> = Pair(
        x.coerceIn(-insetLeft, maxOf(-insetLeft, screenWidth - width + insetRight)),
        y.coerceIn(-insetTop, maxOf(-insetTop, screenHeight - height + insetBottom)),
    )

    /** Transparent room above (and below) the lyric area: the panel is text height × 1.5, the area × 1.1, centred. */
    fun textVerticalInset(panelHeight: Int): Int = maxOf(0, panelHeight * 2 / 15)

    /**
     * A width change keeps the lyric area's horizontal centre: the new left edge of a panel that was `oldWidth` wide
     * at `left` and becomes `newWidth` (`lead` = margin left of the lyric area, `trail` = lock column plus margin).
     */
    fun resizeAroundCentre(left: Int, oldWidth: Int, newWidth: Int, lead: Int, trail: Int): Int {
        val centre2 = 2 * (left + lead) + maxOf(0, oldWidth - lead - trail)
        return Math.floorDiv(centre2 - maxOf(0, newWidth - lead - trail), 2) - lead
    }

    /**
     * Edge fade of the lyric area (same as `lyricsEdgeFadeWidth`, player-lyrics-rules): a square zone on each side,
     * as wide as the area is high, at most half the area width.
     */
    fun edgeFadeWidth(areaWidth: Float, areaHeight: Float): Float =
        if (!(areaWidth > 0f) || !(areaHeight > 0f)) 0f else minOf(areaHeight, areaWidth / 2f)

    /** The edge fade's opacity (0..1) at `x` px from the area's left edge (`lyricsEdgeFadeOpacity`). */
    fun edgeFadeOpacity(x: Float, areaWidth: Float, areaHeight: Float): Float {
        val zone = edgeFadeWidth(areaWidth, areaHeight)
        if (!(zone > 0f)) return 0f
        return (minOf(x, areaWidth - x) / zone).coerceIn(0f, 1f)
    }
}
