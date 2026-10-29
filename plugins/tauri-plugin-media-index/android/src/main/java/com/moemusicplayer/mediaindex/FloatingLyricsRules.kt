package com.moemusicplayer.mediaindex

/**
 * Pure rules of the Android floating lyrics (ADR 0032), ports of the web ones so the overlay shows the same line at the
 * same moment: `findActiveLyricIndex` (lyrics-window.js), the line duration and `getLyricScrollProgress`
 * (TaskbarLyricsView / taskbar-lyrics-settings.js) and the effect colour (`getTaskbarLyricsEffectRgb`).
 */
object FloatingLyricsRules {
    /** How long the last line of a track is assumed to stay when the track length is unknown. */
    const val LAST_LINE_FALLBACK_MS = 5_000L
    /**
     * Terminal continuous-lyrics glide estimate per grapheme cluster whose base (first) code point is East Asian Wide
     * or Fullwidth ([EastAsianWidth.isWide]: CJK ideographs, kana, Hangul, full-width forms, wide emoji) ...
     */
    const val LYRICS_WIDE_GRAPHEME_MS = 500L
    /** ... and per any other grapheme cluster (Latin, half-width katakana, ambiguous / neutral symbols, spaces). */
    const val LYRICS_NARROW_GRAPHEME_MS = 250L

    /**
     * The continuous strip's duration cap of a line without word times that is not the last line (ADR 0041 D10,
     * `player-lyrics-rules` `line_cap_ms`): per wide grapheme cluster (the same table and segmentation as
     * [terminalLineEstimateMs]) ...
     */
    const val LINE_CAP_WIDE_GRAPHEME_MS = 1_000L
    /** ... and per any other grapheme cluster (spaces included); an empty line counts as one. */
    const val LINE_CAP_NARROW_GRAPHEME_MS = 500L

    /** "No time" for the primitive (allocation-free) overloads that otherwise take a `Long?`. */
    const val NO_TIME = Long.MIN_VALUE

    /** Number of user-perceived characters (grapheme clusters) of `text`. */
    fun graphemeCount(text: String): Int {
        val iterator = java.text.BreakIterator.getCharacterInstance(java.util.Locale.ROOT)
        iterator.setText(text)
        var graphemes = 0
        iterator.first()
        while (iterator.next() != java.text.BreakIterator.DONE) graphemes++
        return graphemes
    }

    /**
     * How long the terminal line is estimated to take: [LYRICS_WIDE_GRAPHEME_MS] per wide grapheme cluster plus
     * [LYRICS_NARROW_GRAPHEME_MS] per other one; an empty line counts as one narrow grapheme. Same segmentation as
     * [graphemeCount].
     */
    fun terminalLineEstimateMs(text: String): Long =
        graphemeDurationMs(text, LYRICS_WIDE_GRAPHEME_MS, LYRICS_NARROW_GRAPHEME_MS)

    /**
     * How long the continuous strip may glide through a line without word times that is not the last line:
     * [LINE_CAP_WIDE_GRAPHEME_MS] per wide grapheme cluster plus [LINE_CAP_NARROW_GRAPHEME_MS] per other one; an empty
     * line counts as one narrow grapheme (`WordLyricsRules.KnotLine.capMs`).
     */
    fun lineCapMs(text: String): Long = graphemeDurationMs(text, LINE_CAP_WIDE_GRAPHEME_MS, LINE_CAP_NARROW_GRAPHEME_MS)

    /** `wide` per grapheme cluster whose base code point is East Asian W/F, `narrow` per other one, at least `narrow`. */
    private fun graphemeDurationMs(text: String, wide: Long, narrow: Long): Long {
        val iterator = java.text.BreakIterator.getCharacterInstance(java.util.Locale.ROOT)
        iterator.setText(text)
        var total = 0L
        var start = iterator.first()
        var end = iterator.next()
        while (end != java.text.BreakIterator.DONE) {
            total += if (EastAsianWidth.isWide(text.codePointAt(start))) wide else narrow
            start = end
            end = iterator.next()
        }
        return maxOf(narrow, total)
    }

    /** Estimate the terminal glide end from the last line's graphemes, capped at track end when known. */
    fun finalLineArrivalMs(lineStartMs: Long, text: String, trackEndMs: Long?): Long =
        finalLineArrivalFromEstimate(lineStartMs, terminalLineEstimateMs(text), trackEndMs ?: NO_TIME)

    /**
     * How long the terminal line's glide takes before the track-end cap (`player-lyrics-rules`
     * `terminal_line_duration_ms`): its own end minus its start when it has an end after the start, else
     * [terminalLineEstimateMs]. Pass it to [finalLineArrivalFromEstimate].
     */
    fun terminalLineDurationMs(lineStartMs: Long, ownEndMs: Long?, text: String): Long =
        if (ownEndMs != null && ownEndMs > lineStartMs) ownEndMs - lineStartMs else terminalLineEstimateMs(text)

    /** [finalLineArrivalMs] with [terminalLineEstimateMs] known already (once per strip); `trackEndMs` [NO_TIME] when unknown. */
    fun finalLineArrivalFromEstimate(lineStartMs: Long, estimateMs: Long, trackEndMs: Long): Long {
        val estimated = lineStartMs + estimateMs
        return if (trackEndMs != NO_TIME) minOf(estimated, trackEndMs) else estimated
    }

    /** Main opacity: same range and default as settings.rs `MAIN_OPACITY_RANGE_PERCENT` (1-100, 100). */
    const val DEFAULT_MAIN_OPACITY_PERCENT = 100
    fun mainOpacityPercent(value: Int): Int = value.coerceIn(1, 100)

    /** Secondary opacity: settings.rs `SECONDARY_OPACITY_RANGE_PERCENT` (1-100, 50). May exceed the main one. */
    const val DEFAULT_SECONDARY_OPACITY_PERCENT = 50
    fun secondaryOpacityPercent(value: Int): Int = value.coerceIn(1, 100)

    /**
     * What the two line views draw at (their own alpha): animation "none" shows the line at the main opacity;
     * "continuous" draws at 1 because the static mask carries the main and the secondary opacity.
     */
    fun lineViewAlpha(animation: String, mainPercent: Int): Float =
        if (animation == "continuous") 1f else mainOpacityPercent(mainPercent) / 100f

    /** Font weight of the original text: settings.rs `LYRICS_FONT_WEIGHT_RANGE` (100-1000) and default 500. */
    const val DEFAULT_FONT_WEIGHT = 500
    const val MIN_FONT_WEIGHT = 100
    const val MAX_FONT_WEIGHT = 1000
    fun fontWeight(value: Int): Int = value.coerceIn(MIN_FONT_WEIGHT, MAX_FONT_WEIGHT)

    /** Translation rows are 50 lighter than the original text, never below 1. */
    fun auxiliaryFontWeight(weight: Int): Int = maxOf(1, fontWeight(weight) - 50)

    /** `Typeface.create(family, weight, italic)` takes any weight from API 28; older systems only know normal and bold. */
    const val WEIGHTED_TYPEFACE_API = 28

    /** On API < 28: bold from 600 (CSS's "semi-bold and heavier"), otherwise normal. */
    fun legacyBold(weight: Int): Boolean = weight >= 600

    /** Which typeface a weight maps to on `sdkInt`: an exact `weight` (API 28+) or only the `bold` flag (older). */
    data class TypefaceChoice(val weight: Int?, val bold: Boolean)

    fun typefaceChoice(weight: Int, sdkInt: Int): TypefaceChoice {
        val value = maxOf(1, minOf(MAX_FONT_WEIGHT, weight))
        return if (sdkInt >= WEIGHTED_TYPEFACE_API) TypefaceChoice(value, value >= 600) else TypefaceChoice(null, legacyBold(value))
    }

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

    /** The continuous strip draws a line from its cell's LEFT edge and anchors it by its first glyph's centre (`firstGlyphCenters`, ADR 0041 D10) ... */
    const val STRIP_ALIGN = "left"

    /** ... on the lyric area's horizontal CENTRE (Rust `STRIP_TARGET`), whatever the alignment setting and panel width. */
    const val STRIP_TARGET = "center"

    /**
     * Continuous animation (ADR 0032, same rules as `player-lyrics-rules`): cells of the strip, each as wide as the wider of its
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
        val packed = stripWindowPacked(cellStarts, cellWidths, offset, viewWidth, marginPx)
        return windowFirst(packed)..windowLast(packed)
    }

    /**
     * [stripWindow] without an allocation: the first cell in the high 32 bits and the last cell plus one in the low 32
     * (an empty window has both equal); read it with [windowFirst] / [windowLast].
     */
    fun stripWindowPacked(cellStarts: FloatArray, cellWidths: FloatArray, offset: Float, viewWidth: Float, marginPx: Float): Long {
        val left = -offset - maxOf(0f, marginPx)
        val right = -offset + maxOf(0f, viewWidth) + maxOf(0f, marginPx)
        var lo = 0
        var hi = cellStarts.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (cellStarts[mid] + (if (mid < cellWidths.size) cellWidths[mid] else 0f) < left) lo = mid + 1 else hi = mid
        }
        var last = lo - 1
        while (last + 1 < cellStarts.size && cellStarts[last + 1] <= right) last++
        return (lo.toLong() shl 32) or ((last + 1).toLong() and 0xffffffffL)
    }

    fun windowFirst(packed: Long): Int = (packed ushr 32).toInt()

    /** Last cell of a [stripWindowPacked] window (first - 1 when empty). */
    fun windowLast(packed: Long): Int = (packed and 0xffffffffL).toInt() - 1

    /** The point of a cell that sits on the view's alignment point. */
    fun stripAnchor(cellStarts: FloatArray, cellWidths: FloatArray, index: Int, align: String, anchors: FloatArray = FloatArray(0)): Float {
        val start = cellStarts.getOrElse(index) { 0f }
        val width = cellWidths.getOrElse(index) { 0f }
        return when (align) {
            "right" -> start + width
            "center" -> start + width / 2f
            else -> start + anchors.getOrElse(index) { 0f }.coerceIn(0f, maxOf(0f, width))
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
        firstGlyphCenters: FloatArray = FloatArray(0),
    ): Float {
        val knots = stripKnots(
            startTimesMs, cellStarts, cellWidths, align, finalArrivalMs ?: NO_TIME, mainTextWidths, lastGlyphCenters, firstGlyphCenters,
        ) ?: return 0f
        return stripKnotsOffset(knots, lyricsMs, stripTarget(viewWidth, targetAlign))
    }

    /** The strip glide's knots (time ms, anchor px) built once per strip; the per-frame [stripKnotsOffset] only reads them. */
    class StripKnots(val times: DoubleArray, val values: DoubleArray, val endMs: Long)

    /**
     * The knots of [stripOffset]: one per line start (its anchor) and, when `finalArrivalMs` ([NO_TIME]: none) is after
     * the last start, the terminal knot (the last main-text glyph centre). Null when there are no lines.
     */
    fun stripKnots(
        startTimesMs: LongArray,
        cellStarts: FloatArray,
        cellWidths: FloatArray,
        align: String,
        finalArrivalMs: Long,
        mainTextWidths: FloatArray = cellWidths,
        lastGlyphCenters: FloatArray = FloatArray(0),
        firstGlyphCenters: FloatArray = FloatArray(0),
    ): StripKnots? {
        val count = minOf(startTimesMs.size, cellStarts.size)
        if (count == 0) return null
        val last = count - 1
        val hasEnd = finalArrivalMs != NO_TIME && finalArrivalMs > startTimesMs[last]
        val knots = count + if (hasEnd) 1 else 0
        val times = DoubleArray(knots)
        val values = DoubleArray(knots)
        for (k in 0 until count) {
            times[k] = startTimesMs[k].toDouble()
            values[k] = stripAnchor(cellStarts, cellWidths, k, align, firstGlyphCenters).toDouble()
        }
        if (hasEnd) {
            val mainWidth = maxOf(0f, mainTextWidths.getOrElse(last) { cellWidths.getOrElse(last) { 0f } })
            val glyphCenter = lastGlyphCenters.getOrElse(last) { mainWidth }.coerceIn(0f, mainWidth)
            times[count] = finalArrivalMs.toDouble()
            values[count] = maxOf(
                stripAnchor(cellStarts, cellWidths, last, align, firstGlyphCenters).toDouble(),
                cellStarts.getOrElse(last) { 0f }.toDouble() + glyphCenter,
            )
        }
        return StripKnots(times, values, if (hasEnd) finalArrivalMs else NO_TIME)
    }

    /** The alignment point inside a view `viewWidth` wide. */
    fun stripTarget(viewWidth: Float, targetAlign: String): Double = when (targetAlign) {
        "right" -> viewWidth.toDouble()
        "center" -> viewWidth / 2.0
        else -> 0.0
    }

    /** [stripOffset] from prepared knots: a binary search for the segment and no allocation. */
    fun stripKnotsOffset(knots: StripKnots, lyricsMs: Long, target: Double): Float =
        stripKnotsOffset(knots, lyricsMs.toDouble(), target)

    /** [stripKnotsOffset] at a fractional lyrics time (the GL renderer's clock is not a whole millisecond). */
    fun stripKnotsOffset(knots: StripKnots, lyricsMs: Double, target: Double): Float {
        val times = knots.times
        val values = knots.values
        val count = times.size
        val p = lyricsMs
        // The last knot at or before p (-1 before the first).
        var lo = 0
        var hi = count
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (times[mid] <= p) lo = mid + 1 else hi = mid
        }
        val index = lo - 1
        if (index < 0) return (target - values[0]).toFloat()
        if (index >= count - 1) return (target - values[count - 1]).toFloat()
        val t0 = times[index]
        val h = times[index + 1] - t0
        val v0 = values[index]
        val m0 = knotTangent(times, values, index)
        val m1 = knotTangent(times, values, index + 1)
        val d = (values[index + 1] - v0) / h
        val c2 = (3 * d - 2 * m0 - m1) / h
        val c3 = (m0 + m1 - 2 * d) / (h * h)
        val u = p - t0
        return (target - (v0 + u * (m0 + u * (c2 + u * c3)))).toFloat()
    }

    /** Fritsch-Butland tangent at knot `k`: 0 at the ends and where a neighbour has no time or no rise. */
    private fun knotTangent(times: DoubleArray, values: DoubleArray, k: Int): Double {
        if (k <= 0 || k >= times.size - 1) return 0.0
        val h0 = times[k] - times[k - 1]
        val h1 = times[k + 1] - times[k]
        if (h0 <= 0 || h1 <= 0) return 0.0
        val d0 = (values[k] - values[k - 1]) / h0
        val d1 = (values[k + 1] - values[k]) / h1
        if (d0 <= 0 || d1 <= 0) return 0.0
        val w0 = 2 * h1 + h0
        val w1 = h1 + 2 * h0
        return (w0 + w1) / (w0 / d0 + w1 / d1)
    }

    /**
     * The effect drawn: the selected one in every mode, as on Windows. (Continuous mode used to swap the shadow for the
     * outline; lines are rasterised once with their shadow, so the glide never blurs per frame.)
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

    /** Room for `areaPx` of lyric area (text height × 1.1): the panel is text height × 1.5 when the area is one row. */
    fun panelRoomPx(areaPx: Int): Int = if (areaPx <= 0) 0 else kotlin.math.ceil(areaPx * 2.0 / 11.0).toInt()

    /**
     * Transparent room above the lyric area: from the MAIN row alone, so showing or hiding the translation row never
     * moves the first row (ADR 0032). Only the room below follows the whole area ([panelRoomPx] of its height).
     */
    fun panelTopRoomPx(mainRowPx: Int): Int = panelRoomPx(mainRowPx)

    /** The saved position is the lyric area's top-left; the window's top-left is that less the margin and the top room. */
    fun windowOrigin(areaX: Int, areaY: Int, marginPx: Int, topRoomPx: Int): Pair<Int, Int> =
        Pair(areaX - marginPx, areaY - topRoomPx)

    /** The inverse of [windowOrigin]: the lyric area's top-left of a window at (`windowX`, `windowY`). */
    fun areaOrigin(windowX: Int, windowY: Int, marginPx: Int, topRoomPx: Int): Pair<Int, Int> =
        Pair(windowX + marginPx, windowY + topRoomPx)

    /** Top of the lock button (px from the window's top) so it stays centred on the lyric area. */
    fun lockTop(topRoomPx: Int, areaPx: Int, lockPx: Int): Int = topRoomPx + (areaPx - lockPx) / 2

    /**
     * Keeps a window's lyric area on the screen (window coordinates in and out): only the area must stay on it; the
     * side margins, the margin right of the lock and the rooms above and below may go past the edges. `areaPx` is the
     * area's current height (main row plus the translation row when shown).
     */
    fun clampWindowToArea(
        x: Int, y: Int, width: Int, areaPx: Int, topRoomPx: Int, marginPx: Int, screenWidth: Int, screenHeight: Int,
    ): Pair<Int, Int> {
        // The kept box (the lyric area and the lock column) through the shared rule, back to window coordinates.
        val (keptX, keptY) = clampKeptBox(
            x + marginPx, y + topRoomPx, width - 2 * marginPx, areaPx, 0, 0, screenWidth, screenHeight,
        )
        return Pair(keptX - marginPx, keptY - topRoomPx)
    }

    // ---- Placement (ADR 0032; player-lyrics-rules `placement`, shared vectors placementRatio / Px / Clamp / Restore).
    // The saved position is the lyric area's top-left as a ratio of the screen in that orientation:
    // x = (areaX - screenLeft) / screenWidth, y likewise. It is always clamped after being put back in pixels.

    /** The ratio saved for `px` on an axis starting at `origin`, `extent` px long; null for an empty axis. */
    fun positionRatio(px: Int, origin: Int, extent: Int): Double? =
        if (extent > 0) (px.toDouble() - origin.toDouble()) / extent.toDouble() else null

    /** The pixel a saved `ratio` stands for: origin + floor(ratio * extent + 0.5); origin for a bad ratio or axis. */
    fun positionPx(ratio: Double, origin: Int, extent: Int): Int {
        if (!ratio.isFinite() || extent <= 0) return origin
        val offset = kotlin.math.floor(ratio * extent.toDouble() + 0.5)
        return (origin.toDouble() + offset).coerceIn(Int.MIN_VALUE.toDouble(), Int.MAX_VALUE.toDouble()).toInt()
    }

    /**
     * Keeps the box that must stay visible (the lyric area plus the lock column, `width` x `height` at `x`, `y`)
     * inside left..right, top..bottom; a box larger than the bounds sits at the start (`clamp_kept_box`).
     */
    fun clampKeptBox(x: Int, y: Int, width: Int, height: Int, left: Int, top: Int, right: Int, bottom: Int): Pair<Int, Int> {
        fun axis(value: Int, low: Int, high: Int, size: Int): Int {
            val max = maxOf(low.toLong(), high.toLong() - maxOf(0, size).toLong()).toInt()
            return value.coerceIn(low, max)
        }
        return Pair(axis(x, left, right, width), axis(y, top, bottom, height))
    }

    /** A saved ratio put back on a `screenWidth` x `screenHeight` screen and kept on it (`restore_area_origin`). */
    fun restoreAreaOrigin(ratioX: Double, ratioY: Double, screenWidth: Int, screenHeight: Int, keptWidth: Int, keptHeight: Int): Pair<Int, Int> =
        clampKeptBox(
            positionPx(ratioX, 0, screenWidth), positionPx(ratioY, 0, screenHeight), keptWidth, keptHeight,
            0, 0, screenWidth, screenHeight,
        )

    /** A saved position (screen ratio) of one orientation. */
    data class Ratio(val x: Double, val y: Double) {
        /** Same range as settings.rs `FloatingLyricsPosition::RATIO_RANGE`. */
        fun isValid(): Boolean = listOf(x, y).all { it.isFinite() && it >= -1.0 && it <= 2.0 }
    }

    /** What settings.json keeps for the overlay: `locked`, `position` (portrait) and `landscapePosition`. */
    data class SavedPlacement(val locked: Boolean = false, val portrait: Ratio? = null, val landscape: Ratio? = null) {
        /** The position of the orientation shown. */
        fun position(landscape: Boolean): Ratio? = if (landscape) this.landscape else portrait

        fun withPosition(landscape: Boolean, ratio: Ratio?): SavedPlacement =
            if (landscape) copy(landscape = ratio) else copy(portrait = ratio)
    }

    /** The settings key of an orientation's position. */
    fun positionKey(landscape: Boolean): String = if (landscape) "landscapePosition" else "position"

    /** The ratio of a lyric area at (`areaX`, `areaY`) on a `screenWidth` x `screenHeight` screen; null on an empty screen. */
    fun ratioOf(areaX: Int, areaY: Int, screenWidth: Int, screenHeight: Int): Ratio? {
        val x = positionRatio(areaX, 0, screenWidth) ?: return null
        val y = positionRatio(areaY, 0, screenHeight) ?: return null
        return Ratio(x, y)
    }

    /**
     * The old SharedPreferences values (`locked`, `x_/y_portrait`, `x_/y_landscape`: px of the lyric area's top-left)
     * as ratios, each orientation with the screen of that orientation now (`screenWidth` x `screenHeight` is the
     * current one; portrait is the narrow side across, landscape the wide one). A missing or half pair is no position.
     */
    fun legacyPlacement(
        locked: Boolean?, portrait: Pair<Int, Int>?, landscape: Pair<Int, Int>?, screenWidth: Int, screenHeight: Int,
    ): Triple<Boolean?, Ratio?, Ratio?> {
        val narrow = minOf(screenWidth, screenHeight)
        val wide = maxOf(screenWidth, screenHeight)
        return Triple(
            locked,
            portrait?.let { ratioOf(it.first, it.second, narrow, wide) },
            landscape?.let { ratioOf(it.first, it.second, wide, narrow) },
        )
    }

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
    ): Double = smoothPosition(
        previousMs ?: 0.0, previousMs != null, previousAtMs.toDouble(), reportedMs, reportedAtMs.toDouble(), nowMs.toDouble(),
        playing, resyncMs, slewMs,
    )

    /**
     * [smoothPositionMs] on a fractional millisecond clock (the display frame's time is not a whole millisecond) and
     * without boxing: `hasPrevious` false = no previous position. Whole-millisecond inputs give the same result.
     */
    fun smoothPosition(
        previousMs: Double,
        hasPrevious: Boolean,
        previousAtMs: Double,
        reportedMs: Long,
        reportedAtMs: Double,
        nowMs: Double,
        playing: Boolean,
        resyncMs: Long = 250,
        slewMs: Double = 750.0,
        speed: Double = 1.0,
    ): Double {
        if (!playing) return reportedMs.toDouble()
        val carried = reportedMs + maxOf(0.0, nowMs - reportedAtMs) * speed
        if (!hasPrevious) return carried
        val step = maxOf(0.0, nowMs - previousAtMs) * speed
        val drift = carried - (previousMs + step)
        if (kotlin.math.abs(drift) >= resyncMs) return carried
        val rate = (1.0 + drift / slewMs).coerceIn(0.5, 1.5)
        return previousMs + step * rate
    }

    /** Height of one drawn line: the text's own height (ascent to descent) with a 10% buffer. */
    fun lineHeightPx(ascent: Float, descent: Float): Int = Math.ceil(((descent - ascent) * 1.1f).toDouble()).toInt()

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

    /** `inactive` (the factor of a line that is not current) lifted toward 1 by `weight` (0..1). */
    fun fadedFactor(inactive: Float, weight: Float): Float = inactive + (1f - inactive) * weight.coerceIn(0f, 1f)

    /**
     * The overlay window's alpha: 1, except that a locked (not touchable) overlay on Android 12+ may not exceed the
     * system's maximum obscuring opacity for touches (`systemLimit`, normally 0.8). The drawn values (main and
     * secondary opacity) are not compensated, so the locked brightness is capped by that limit.
     */
    fun windowAlpha(systemLimit: Float?): Float = if (systemLimit == null) 1f else minOf(1f, systemLimit)

    /**
     * Device-pixel rule (AGENTS.md animation rules): a moving position is redrawn only when it crosses into another whole pixel, so the
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
    fun lineDurationMs(starts: LongArray, index: Int, durationMs: Long?, offsetMs: Long): Long =
        lineDurationMs(starts, index, durationMs ?: NO_TIME, offsetMs)

    /** [lineDurationMs] without boxing: `durationMs` [NO_TIME] (or not positive) when the track length is unknown. */
    fun lineDurationMs(starts: LongArray, index: Int, durationMs: Long, offsetMs: Long): Long {
        val start = starts[index]
        val next = if (index + 1 < starts.size) starts[index + 1]
        else if (durationMs != NO_TIME && durationMs > 0) durationMs - offsetMs
        else start + LAST_LINE_FALLBACK_MS
        return maxOf(1_000L, next - start)
    }

    /** 0..1: a line wider than the overlay scrolls over 90% of its time and rests for the last tenth. */
    fun scrollProgress(elapsedMs: Long, durationMs: Long): Float {
        if (durationMs <= 0) return 0f
        return (elapsedMs.toFloat() / (durationMs * 0.9f)).coerceIn(0f, 1f)
    }

    /** [scrollProgress] at a fractional elapsed time. */
    fun scrollProgress(elapsedMs: Double, durationMs: Long): Float {
        if (durationMs <= 0) return 0f
        return (elapsedMs / (durationMs * 0.9)).toFloat().coerceIn(0f, 1f)
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
