package com.moemusicplayer.mediaindex

/** Animation modes of the render thread (the settings' `none` / `continuous`). */
internal const val ANIMATION_NONE = 0
internal const val ANIMATION_CONTINUOUS = 1

/** Text measuring behind the model builder; backed by a `Paint` on Android, faked in the JVM tests. */
internal interface TextMetrics {
    fun width(text: String): Float

    /** Centre of the first grapheme's advance (px from the line's start): the strip's anchor (ADR 0041 D10). */
    fun firstGlyphCenter(text: String): Float

    /** X of the final grapheme's visual bounds centre, relative to the start of the complete line. */
    fun lastGlyphCenter(text: String): Float

    /** Each word's x range `(left, right)` in `text` (`ranges` = UTF-16 `(start, end)`). */
    fun spans(text: String, ranges: List<Pair<Int, Int>>): List<Pair<Double, Double>>
}

/**
 * The panel's geometry in px, one pure function of the font metrics, the width and whether the translation row is
 * reserved (ADR 0032): the lyric area is the main row plus, when reserved, the translation row; the surface is that
 * plus the panel room (`panelRoomPx`) above and below, with the transparent side margins and the lock gutter at the
 * sides. Everything the window layout and the renderer position is derived here, so they cannot disagree.
 */
internal class LyricsLayout(
    val marginPx: Int,
    val lockPx: Int,
    val areaWidthPx: Int,
    val mainAscent: Float,
    val mainDescent: Float,
    val secondAscent: Float,
    val secondDescent: Float,
    val reserveTranslation: Boolean,
) {
    val mainRowH: Int = maxOf(1, FloatingLyricsRules.lineHeightPx(mainAscent, mainDescent))
    val secondRowH: Int = maxOf(1, FloatingLyricsRules.lineHeightPx(secondAscent, secondDescent))
    val areaH: Int = mainRowH + if (reserveTranslation) secondRowH else 0
    val room: Int = FloatingLyricsRules.panelRoomPx(areaH)
    val surfaceW: Int = areaWidthPx + lockPx + 2 * marginPx
    val surfaceH: Int = areaH + 2 * room
    val areaLeft: Int = marginPx
    val areaTop: Int = room
    val mainRowTop: Int = room
    val secondRowTop: Int = room + mainRowH

    /** Top of a rasterised line image (text plus `pad`) in a row that starts at `rowTop`; the text is centred in the row. */
    fun imageTop(rowTop: Int, rowH: Int, ascent: Float, descent: Float, pad: Int): Float =
        rowTop + (rowH - (descent - ascent)) / 2f - pad

    override fun equals(other: Any?): Boolean = other is LyricsLayout && other.marginPx == marginPx && other.lockPx == lockPx &&
        other.areaWidthPx == areaWidthPx && other.mainAscent == mainAscent && other.mainDescent == mainDescent &&
        other.secondAscent == secondAscent && other.secondDescent == secondDescent && other.reserveTranslation == reserveTranslation

    override fun hashCode(): Int = ((marginPx * 31 + lockPx) * 31 + areaWidthPx) * 31 + if (reserveTranslation) 1 else 0
}

/** The continuous strip (ADR 0032): the lines with some text, as cells of one strip. All arrays are never changed. */
internal class StripModel(
    val starts: LongArray,
    val texts: Array<String>,
    val translations: Array<String>,
    val cells: FloatArray,
    val widths: FloatArray,
    val mainWidths: FloatArray,
    val translationWidths: FloatArray,
    val lastGlyphCenters: FloatArray,
    val firstGlyphCenters: FloatArray,
    /** Word-timed lyrics (ADR 0041 D10): the word glide's knots (lyrics ms, strip px); null for line-timed lyrics. */
    val wordKnots: WordLyricsRules.WordKnots?,
    /** Grapheme count of the last line (the terminal glide's duration estimate), counted once. */
    val lastGraphemes: Int,
)

/** What the lyrics and the font measures make: immutable, built off the render thread and read once per frame. */
internal class RenderModel(
    val trackId: String?,
    val offsetMs: Long,
    val durationMs: Long,
    val starts: LongArray,
    val texts: Array<String>,
    /** Per line; empty strings while the translation switch is off. */
    val translations: Array<String>,
    val textWidths: FloatArray,
    val translationWidths: FloatArray,
    /** Per line: the karaoke edge's glide (word-timed lines only). */
    val karaoke: Array<List<WordLyricsRules.GlidePiece>?>,
    val strip: StripModel?,
    val noteWidth: Float,
    /** The translation row is reserved for the whole track (it has a translation and the switch is on). */
    val reserveTranslation: Boolean,
    val linesVersion: Int,
) {
    companion object {
        /** The placeholder shown in move mode while no line is current. */
        const val NOTE = "♪"
    }
}

/** What the lines look like: the raster looks (a change re-rasterises every line) and the per-frame parameters. */
internal class RenderLook(
    /** Increases whenever any raster look changed: textures made under an older generation are discarded. */
    val generation: Int,
    val layout: LyricsLayout,
    val mainLook: LineLook,
    val secondLook: LineLook,
    /** The main row in the accent colour (karaoke), same shadow and outline. */
    val accentLook: LineLook,
    val animation: Int,
    val align: String,
    /** Fixed mode: no background, no placeholder. */
    val locked: Boolean,
    /** Mask stops (px from the lyric area's left edge, opacity) of the "none" look: the edge fade at the main opacity. */
    val noneMaskX: FloatArray,
    val noneMaskV: FloatArray,
    /** Mask stops of the continuous look: the static centre mask. */
    val contMaskX: FloatArray,
    val contMaskV: FloatArray,
    val backgroundArgb: Int,
    val backgroundRadiusPx: Float,
)

/** One consistent pair: published together so the renderer never mixes a model with another look's measures. */
internal class RenderState(val version: Int, val look: RenderLook, val model: RenderModel)

/** Everything the main thread hands to the builder (a snapshot of the overlay's fields). */
internal class RenderInputs(
    val linesVersion: Int,
    val trackId: String?,
    val offsetMs: Long,
    val durationMs: Long,
    val lines: List<FloatingLyricsOverlay.Line>,
    val animation: Int,
    val align: String,
    val textEffect: String,
    val colorRgb: Int,
    val accentArgb: Int,
    val mainPx: Float,
    val secondPx: Float,
    val mainWeight: Int,
    val secondWeight: Int,
    val showTranslation: Boolean,
    val mainOpacity: Float,
    val secondaryOpacity: Float,
    val locked: Boolean,
    val density: Float,
    val layout: LyricsLayout,
    val backgroundRadiusPx: Float,
)
