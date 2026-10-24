package com.moemusicplayer.mediaindex

import android.graphics.Paint
import android.graphics.Rect
import java.text.BreakIterator
import java.util.Locale

/** [TextMetrics] on a `Paint` with the lyric typeface (the same face and size the line images are drawn with). */
internal class PaintTextMetrics(sizePx: Float, weight: Int) : TextMetrics {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        typeface = lyricTypeface(weight)
        textSize = sizePx
    }
    private val bounds = Rect()

    override fun width(text: String): Float = if (text.isEmpty()) 0f else paint.measureText(text)

    override fun firstGlyphCenter(text: String): Float {
        if (text.isEmpty()) return 0f
        val iterator = BreakIterator.getCharacterInstance(Locale.ROOT)
        iterator.setText(text)
        val end = iterator.following(0).let { if (it == BreakIterator.DONE) text.length else it }
        return paint.measureText(text, 0, end) / 2f
    }

    override fun lastGlyphCenter(text: String): Float {
        if (text.isEmpty()) return 0f
        val iterator = BreakIterator.getCharacterInstance(Locale.ROOT)
        iterator.setText(text)
        val prefixEnd = iterator.preceding(text.length).coerceAtLeast(0)
        paint.getTextBounds(text, prefixEnd, text.length, bounds)
        val prefixWidth = paint.measureText(text, 0, prefixEnd)
        return maxOf(0f, prefixWidth + (bounds.left + bounds.right) / 2f)
    }

    override fun spans(text: String, ranges: List<Pair<Int, Int>>): List<Pair<Double, Double>> = ranges.map { (a, b) ->
        val start = a.coerceIn(0, text.length)
        val end = b.coerceIn(start, text.length)
        paint.measureText(text, 0, start).toDouble() to paint.measureText(text, 0, end).toDouble()
    }
}

/** The mask gradients as parallel arrays of six stops (px from the lyric area's left edge, opacity 0..1). */
internal object LyricsMasks {
    const val STOPS = 6

    /** The "none" look: the edge fade (`edgeFadeWidth`) at `opacity`; flat `opacity` when the area is too small to fade. */
    fun edgeFade(areaWidth: Float, areaHeight: Float, opacity: Float): Pair<FloatArray, FloatArray> {
        val o = opacity.coerceIn(0f, 1f)
        val zone = FloatingLyricsRules.edgeFadeWidth(areaWidth, areaHeight)
        if (zone <= 0f) return Pair(FloatArray(STOPS) { if (it < STOPS / 2) 0f else areaWidth }, FloatArray(STOPS) { o })
        return Pair(
            floatArrayOf(0f, zone, zone, areaWidth - zone, areaWidth - zone, areaWidth),
            floatArrayOf(0f, o, o, o, o, 0f),
        )
    }

    /** The continuous look: [WordLyricsRules.centreMaskStops] at `emPx`; flat 1 when there is no em. */
    fun centre(areaWidth: Float, emPx: Float, main: Float, secondary: Float): Pair<FloatArray, FloatArray> {
        val stops = WordLyricsRules.centreMaskStops(areaWidth.toDouble(), emPx.toDouble(), main.toDouble(), secondary.toDouble())
        if (stops.size != STOPS) return Pair(FloatArray(STOPS) { if (it < STOPS / 2) 0f else areaWidth }, FloatArray(STOPS) { 1f })
        return Pair(
            FloatArray(STOPS) { (stops[it].first * areaWidth).toFloat() },
            FloatArray(STOPS) { stops[it].second.toFloat() },
        )
    }
}

/**
 * Turns the overlay's inputs into the immutable [RenderState] the render thread reads (ADR 0032, GL overlay). Pure
 * apart from the injected text measuring, so it is unit-tested on the JVM. One instance, used by one thread at a time;
 * the previous model is reused while the lyrics and the font measures are unchanged, and the raster generation
 * increases whenever any raster look changed.
 */
internal class RenderBuilder(private val metricsFor: (sizePx: Float, weight: Int) -> TextMetrics) {
    private data class ModelKey(
        val linesVersion: Int,
        val showTranslation: Boolean,
        val mainPx: Float,
        val secondPx: Float,
        val mainWeight: Int,
        val secondWeight: Int,
        val strip: Boolean,
    )

    private var key: ModelKey? = null
    private var model: RenderModel? = null
    private var generation = 0
    private var lastMain: LineLook? = null
    private var lastSecond: LineLook? = null
    private var lastAccent: LineLook? = null
    private var version = 0

    fun build(inputs: RenderInputs): RenderState {
        val look = buildLook(inputs)
        return RenderState(++version, look, buildModel(inputs))
    }

    private fun lookFor(i: RenderInputs, px: Float, weight: Int, fillArgb: Int): LineLook {
        val effect = FloatingLyricsRules.effectRgb(i.colorRgb)
        val chosen = FloatingLyricsRules.effectiveTextEffect(if (i.animation == ANIMATION_CONTINUOUS) "continuous" else "none", i.textEffect)
        val shadow = chosen == "shadow"
        val stroke = chosen == "stroke"
        return LineLook(
            sizePx = px,
            color = fillArgb,
            shadowRadius = if (shadow) FloatingLyricsRules.shadowLayerRadius(px) else 0f,
            shadowColor = if (shadow) (FloatingLyricsRules.SHADOW_ALPHA shl 24) or effect else 0,
            strokeWidth = if (stroke) maxOf(1f, px * 0.08f) else 0f,
            strokeColor = if (stroke) 0x80000000.toInt() or effect else 0,
            density = i.density,
            weight = weight,
            defaultFace = false,
        )
    }

    private fun buildLook(i: RenderInputs): RenderLook {
        val fill = 0xff000000.toInt() or i.colorRgb
        val main = lookFor(i, i.mainPx, i.mainWeight, fill)
        val second = lookFor(i, i.secondPx, i.secondWeight, fill)
        val accent = lookFor(i, i.mainPx, i.mainWeight, i.accentArgb)
        if (main != lastMain || second != lastSecond || accent != lastAccent) generation++
        lastMain = main
        lastSecond = second
        lastAccent = accent
        val layout = i.layout
        val areaW = layout.areaWidthPx.toFloat()
        // Animation "none" draws the line at the main opacity; the continuous mode's own mask carries both opacities, and
        // the single-line fallback of that mode draws at 1 (FloatingLyricsRules.lineViewAlpha).
        val noneOpacity = if (i.animation == ANIMATION_CONTINUOUS) 1f else i.mainOpacity
        val (nx, nv) = LyricsMasks.edgeFade(areaW, layout.areaH.toFloat(), noneOpacity)
        val (cx, cv) = LyricsMasks.centre(areaW, i.mainPx, i.mainOpacity, i.secondaryOpacity)
        return RenderLook(
            generation, layout, main, second, accent, i.animation, i.align, i.locked,
            nx, nv, cx, cv, MOVE_BACKGROUND_ARGB, i.backgroundRadiusPx,
        )
    }

    private fun buildModel(i: RenderInputs): RenderModel {
        val wantStrip = i.animation == ANIMATION_CONTINUOUS
        val wanted = ModelKey(i.linesVersion, i.showTranslation, i.mainPx, i.secondPx, i.mainWeight, i.secondWeight, wantStrip)
        val cached = model
        if (cached != null && wanted == key) return cached
        val main = metricsFor(i.mainPx, i.mainWeight)
        val second = metricsFor(i.secondPx, i.secondWeight)
        val lines = i.lines
        val n = lines.size
        val texts = Array(n) { lines[it].trimmedText }
        val translations = Array(n) { if (i.showTranslation) lines[it].trimmedTranslation else "" }
        val widths = FloatArray(n) { main.width(texts[it]) }
        val translationWidths = FloatArray(n) { second.width(translations[it]) }
        val karaoke = Array<List<WordLyricsRules.GlidePiece>?>(n) { index ->
            val line = lines[index]
            val times = line.wordTimes
            val ranges = line.wordRanges
            if (times != null && ranges != null && texts[index].isNotEmpty()) {
                WordLyricsRules.karaokeGlide(main.spans(texts[index], ranges), times)
            } else {
                null
            }
        }
        val starts = LongArray(n) { lines[it].startMs }
        val built = RenderModel(
            i.trackId, i.offsetMs, i.durationMs, starts, texts, translations, widths, translationWidths, karaoke,
            if (wantStrip) buildStrip(i, main, second) else null,
            main.width(RenderModel.NOTE),
            i.showTranslation && translations.any { it.isNotEmpty() },
            i.linesVersion,
        )
        key = wanted
        model = built
        return built
    }

    /** The strip of the lines with some text (main or translation); port of the Views path's `stripFor`. */
    private fun buildStrip(i: RenderInputs, main: TextMetrics, second: TextMetrics): StripModel? {
        val kept = i.lines.filter { it.trimmedText.isNotEmpty() || (i.showTranslation && it.trimmedTranslation.isNotEmpty()) }
        if (kept.isEmpty()) return null
        val texts = Array(kept.size) { kept[it].trimmedText }
        val translations = Array(kept.size) { if (i.showTranslation) kept[it].trimmedTranslation else "" }
        val mainWidths = FloatArray(kept.size) { main.width(texts[it]) }
        val translationWidths = FloatArray(kept.size) { second.width(translations[it]) }
        val lastCenters = FloatArray(kept.size) { main.lastGlyphCenter(texts[it]) }
        val firstCenters = FloatArray(kept.size) { main.firstGlyphCenter(texts[it]) }
        val (cells, widths) = FloatingLyricsRules.stripLayout(mainWidths, translationWidths, main.width("　"))
        val wordKnots = if (kept.any { it.wordRanges != null }) {
            WordLyricsRules.WordKnots.of(
                WordLyricsRules.wordStripKnots(kept.mapIndexed { index, line ->
                    val times = line.wordTimes
                    val end = if (line.wordRanges != null && times != null) times.lastOrNull()?.second else null
                    WordLyricsRules.KnotLine(
                        line.startMs.toDouble(), cells[index].toDouble(), firstCenters[index].toDouble(), end, lastCenters[index].toDouble(),
                    )
                }),
            )
        } else {
            null
        }
        return StripModel(
            LongArray(kept.size) { kept[it].startMs }, texts, translations, cells, widths, mainWidths, translationWidths,
            lastCenters, firstCenters, wordKnots, FloatingLyricsRules.graphemeCount(texts.last()),
        )
    }

    companion object {
        /** The move-mode background behind the lyric area (the Views path's 0x47000000). */
        const val MOVE_BACKGROUND_ARGB = 0x47000000
    }
}
