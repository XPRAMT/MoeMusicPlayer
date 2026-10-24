package com.moemusicplayer.mediaindex

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Shader
import android.graphics.Paint
import android.graphics.Typeface
import android.view.View
import java.text.BreakIterator
import java.util.Locale

/**
 * One line of the Android floating lyrics, drawn directly (ADR 0032). A line that fits is placed by the alignment; a
 * wider one starts at the left and slides by `progress` (0..1) to its end, at a fractional offset so the overlay can
 * move it every display frame without the jumps of `TextView.scrollTo` and without TextView's single-line layout,
 * which put short centred or right-aligned lines outside the view.
 */
internal class LyricLineView(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        typeface = Typeface.DEFAULT
    }
    private var fontWeight = FloatingLyricsRules.DEFAULT_FONT_WEIGHT
    private val outlinePaint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        typeface = Typeface.DEFAULT
        style = Paint.Style.STROKE
        strokeJoin = Paint.Join.ROUND
    }
    private var text = ""
    private var align = "center"
    private var progress = 0f
    private var shadowRadius = 0f
    private var shadowColor = 0
    private var strokeWidth = 0f
    private var strokeColor = 0
    private var color = 0

    // Continuous animation: lines other than the current one are drawn at `inactiveFactor` of the main opacity (1: no
    // dimming). `dimPaint` is the look at that opacity, `fadePaint` the one reused for the (at most two) cells that
    // are fading, so a frame allocates nothing and re-styles a paint only when its opacity really changed.
    private var inactiveFactor = 1f
    private val dimPaint = Paint(paint)
    private val fadePaint = Paint(paint)
    private var fadeAlpha = -1

    // Each line is rasterised once with its shadow / outline (ADR 0032) and only drawn at its offset while gliding.
    private val bitmapPaint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val rasterKeep = HashSet<String>()
    private val rasters = LineRasterCache<Raster>({ value, look -> rasterise(value, look) }) { /* dropped; the GC frees it after any frame using it */ }

    private class Raster(val bitmap: Bitmap, val pad: Int)

    // Karaoke (the none mode, ADR 0041): the line is also rasterised in the accent (same shadow / outline, so the cut
    // between the two images has no seam); the accent image is drawn up to the karaoke edge, the white one after it.
    private val accentPaint = Paint(paint)
    private var accent = 0
    private val accentKeep = HashSet<String>()
    private val accentRasters = LineRasterCache<Raster>({ value, look -> rasterise(value, look, accentPaint) }) { }
    private var karaokeSpans: List<Pair<Double, Double>>? = null
    private var karaokeTimes: List<Pair<Double, Double>>? = null
    private var karaokeGlide: List<WordLyricsRules.GlidePiece> = emptyList()
    private var karaokeSpansText = ""
    private var karaokeEdge = Float.NaN
    private var karaokeRawEdge = Float.NaN

    // Continuous lyrics, line- and word-timed: the static mask at this em, main and secondary opacity (em 0 = none).
    private var maskEm = 0f
    private var maskMain = 1f
    private var maskSecondary = 1f
    private val maskPaint = Paint().apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.DST_IN) }
    private var maskKey = listOf<Any>()

    fun textSizePx(): Float = paint.textSize

    /** Each word's x range `(left, right)` in `value` drawn in this line's font (`ranges` = UTF-16 `(start, end)`). */
    fun wordSpans(value: String, ranges: List<Pair<Int, Int>>): List<Pair<Double, Double>> = ranges.map { (a, b) ->
        val start = a.coerceIn(0, value.length)
        val end = b.coerceIn(start, value.length)
        paint.measureText(value, 0, start).toDouble() to paint.measureText(value, 0, end).toDouble()
    }

    fun setAccent(color: Int) {
        if (color == accent) return
        accent = color
        styleAccent()
        invalidate()
    }

    private fun styleAccent() {
        accentPaint.set(paint)
        accentPaint.color = accent
        accentRasters.setLook(LineLook(paint.textSize, accent, shadowRadius, shadowColor, strokeWidth, strokeColor, resources.displayMetrics.density, fontWeight))
    }

    /**
     * The karaoke of the shown line: `ranges` its words' UTF-16 ranges, `times` their times and `lyricsMs` now (null:
     * the whole line in the text colour). Redrawn only when the edge moves a whole device pixel.
     */
    fun setKaraoke(ranges: List<Pair<Int, Int>>?, times: List<Pair<Double, Double>>?, lyricsMs: Double) {
        if (ranges == null || times == null || text.isEmpty() || stripTexts != null) {
            if (karaokeSpans != null) {
                karaokeSpans = null
                invalidate()
            }
            return
        }
        if (karaokeSpans == null || karaokeSpansText != text) {
            karaokeSpans = wordSpans(text, ranges)
            karaokeSpansText = text
            karaokeEdge = Float.NaN
            karaokeTimes = null
        }
        // One smooth edge per line (ADR 0041 D10), built once per line; per frame only its value is read.
        if (karaokeTimes !== times) {
            karaokeGlide = WordLyricsRules.karaokeGlide(karaokeSpans!!, times)
            karaokeTimes = times
        }
        val edge = WordLyricsRules.glideValue(karaokeGlide, lyricsMs).toFloat()
        if (karaokeEdge.isNaN() || FloatingLyricsRules.needsPixelRedraw(karaokeEdge, edge, karaokeRawEdge)) {
            karaokeEdge = edge
            invalidate()
        }
        karaokeRawEdge = edge
    }

    /** True while the karaoke edge of the shown line still moves at `lyricsMs` (frames are wanted). */
    fun karaokeMoving(lyricsMs: Double): Boolean = karaokeSpans != null && WordLyricsRules.glideMoving(karaokeGlide, lyricsMs)

    /** The static mask of continuous lyrics at `em` px, the main and the secondary opacity (0..1); em 0: none. */
    fun setWordMask(em: Float, main: Float = 1f, secondary: Float = 1f) {
        if (em == maskEm && main == maskMain && secondary == maskSecondary) return
        maskEm = em
        maskMain = main
        maskSecondary = secondary
        maskKey = listOf()
        invalidate()
    }

    /** The typeface for `weight`: any weight on API 28+, bold from 600 on older systems (see `typefaceChoice`). */
    private fun typefaceFor(weight: Int): Typeface {
        val choice = FloatingLyricsRules.typefaceChoice(weight, android.os.Build.VERSION.SDK_INT)
        return if (choice.weight != null) {
            Typeface.create(Typeface.DEFAULT, choice.weight, false)
        } else {
            Typeface.create(Typeface.DEFAULT, if (choice.bold) Typeface.BOLD else Typeface.NORMAL)
        }
    }

    /** `weight` is this view's own (the original text's, or 50 lighter for the translation). A change re-measures and re-rasterises. */
    fun setLook(sizePx: Float, color: Int, align: String, shadowRadius: Float, shadowColor: Int, strokeWidth: Float, strokeColor: Int, weight: Int = FloatingLyricsRules.DEFAULT_FONT_WEIGHT) {
        val resize = paint.textSize != sizePx || this.shadowRadius != shadowRadius || this.strokeWidth != strokeWidth || fontWeight != weight
        if (fontWeight != weight || paint.typeface == null) {
            fontWeight = weight
            val face = typefaceFor(weight)
            paint.typeface = face
            outlinePaint.typeface = face
        }
        paint.textSize = sizePx
        paint.color = color
        this.color = color
        this.shadowColor = shadowColor
        this.align = align
        this.shadowRadius = shadowRadius
        this.strokeWidth = strokeWidth
        this.strokeColor = strokeColor
        outlinePaint.textSize = sizePx
        outlinePaint.color = strokeColor
        outlinePaint.strokeWidth = strokeWidth
        if (shadowRadius > 0f) paint.setShadowLayer(shadowRadius, 0f, 0f, shadowColor) else paint.clearShadowLayer()
        styleDim()
        fadeAlpha = -1
        rasters.setLook(LineLook(sizePx, color, shadowRadius, shadowColor, strokeWidth, strokeColor, resources.displayMetrics.density, weight))
        styleAccent()
        if (resize) requestLayout()
        invalidate()
    }

    /** Opacity of the lines other than the current one in the continuous strip, relative to the main opacity (1 = none). */
    fun setInactiveFactor(factor: Float) {
        val value = factor.coerceIn(0f, 1f)
        if (value == inactiveFactor) return
        inactiveFactor = value
        styleDim()
        fadeAlpha = -1
        invalidate()
    }

    private fun styleDim() {
        if (inactiveFactor < 1f) style(dimPaint, inactiveFactor)
    }

    /** `target` as `paint` at `factor` of its opacity: the text and, as the shadow colour is not opaque, the shadow too. */
    private fun style(target: Paint, factor: Float) {
        target.typeface = paint.typeface
        target.textSize = paint.textSize
        target.color = color
        target.alpha = Math.round(android.graphics.Color.alpha(color) * factor)
        if (shadowRadius > 0f) {
            val shadow = Math.round(android.graphics.Color.alpha(shadowColor) * factor)
            target.setShadowLayer(shadowRadius, 0f, 0f, (shadowColor and 0x00ffffff) or (shadow shl 24))
        } else {
            target.clearShadowLayer()
        }
    }

    /** The paint for a cell of weight `weight` (how much it is the current line, 0..1). */
    private fun paintFor(weight: Float): Paint {
        if (inactiveFactor >= 1f || weight >= 1f) return paint
        if (weight <= 0f) return dimPaint
        val factor = FloatingLyricsRules.fadedFactor(inactiveFactor, weight)
        val alpha = Math.round(255 * factor)
        if (alpha != fadeAlpha) {
            fadeAlpha = alpha
            style(fadePaint, factor)
        }
        return fadePaint
    }

    // Continuous animation (ADR 0032): every line in one strip; `stripTexts` null means one line at a time.
    private var stripTexts: List<String>? = null
    private var stripCells = FloatArray(0)
    private var stripWidths = FloatArray(0)
    private var stripOffset = 0f
    private var stripRawOffset = 0f
    private var stripStarts = LongArray(0)
    private var stripLyricsMs = 0L
    private var stripNextStart = 0

    /** Width of `value` in this line's font (for the strip layout). */
    fun textWidth(value: String): Float = if (value.isEmpty()) 0f else paint.measureText(value)

    /** Centre of the first grapheme's advance (px from the line's start): the continuous strip's anchor (ADR 0041 D10). */
    fun firstGlyphCenter(value: String): Float {
        if (value.isEmpty()) return 0f
        val iterator = BreakIterator.getCharacterInstance(Locale.ROOT)
        iterator.setText(value)
        val end = iterator.following(0).let { if (it == BreakIterator.DONE) value.length else it }
        return paint.measureText(value, 0, end) / 2f
    }

    /** X coordinate of the final grapheme's visual bounds centre, relative to the start of the complete line. */
    fun lastGlyphCenter(value: String): Float {
        if (value.isEmpty()) return 0f
        val iterator = BreakIterator.getCharacterInstance(Locale.ROOT)
        iterator.setText(value)
        val prefixEnd = iterator.preceding(value.length).coerceAtLeast(0)
        val bounds = android.graphics.Rect()
        paint.getTextBounds(value, prefixEnd, value.length, bounds)
        val prefixWidth = paint.measureText(value, 0, prefixEnd)
        return maxOf(0f, prefixWidth + (bounds.left + bounds.right) / 2f)
    }

    /** Usable width (the alignment point is measured in it). */
    fun availableWidth(): Float = (width - paddingLeft - paddingRight).toFloat()

    /**
     * Draw the whole strip: each text aligned in its cell (`cells` left edges, `widths`), all shifted by `offset`.
     * `starts` are the lines' start times and `lyricsMs` the lyrics time of this frame: the cells' fades follow from
     * them (FloatingLyricsRules.cellWeight), so redrawing is wanted when the offset moves, and while a fade is running
     * or another line became the next one (a seek) even if the strip itself stays put.
     */
    fun setStrip(texts: List<String>, cells: FloatArray, widths: FloatArray, offset: Float, starts: LongArray, lyricsMs: Long) {
        val changed = texts !== stripTexts || !cells.contentEquals(stripCells) || !widths.contentEquals(stripWidths)
        var fading = false
        var nextStart = 0
        if (inactiveFactor < 1f && maskEm <= 0f) {
            nextStart = FloatingLyricsRules.firstStartAfter(starts, lyricsMs)
            fading = nextStart != stripNextStart ||
                (lyricsMs != stripLyricsMs && nextStart < starts.size && starts[nextStart] - lyricsMs < FloatingLyricsRules.STRIP_FADE_MS)
        }
        // Moves are drawn per whole pixel (FloatingLyricsRules.needsPixelRedraw).
        val moved = FloatingLyricsRules.needsPixelRedraw(stripOffset, offset, stripRawOffset)
        stripRawOffset = offset
        if (changed || moved || fading) {
            stripTexts = texts
            stripCells = cells
            stripWidths = widths
            stripOffset = offset
            text = ""
            invalidate()
        }
        stripStarts = starts
        stripLyricsMs = lyricsMs
        stripNextStart = nextStart
    }

    private var rawProgress = 0f

    fun setLine(value: String, progress: Float) {
        if (stripTexts != null) {
            stripTexts = null
            text = ""
        }
        val fraction = progress.coerceIn(0f, 1f)
        if (value != text) {
            text = value
            this.progress = fraction
            invalidate()
        } else if (fraction != this.progress) {
            // A long line slides by `progress` of its overflow: redraw only when that moves a whole pixel.
            val overflow = overflow()
            if (overflow > 0f && FloatingLyricsRules.needsPixelRedraw(overflow * this.progress, overflow * fraction, overflow * rawProgress)) {
                this.progress = fraction
                invalidate()
            }
        }
        rawProgress = fraction
    }

    /** True when the line is wider than the view and therefore slides. */
    fun overflows(): Boolean = overflow() > 0f

    private fun overflow(): Float {
        val available = width - paddingLeft - paddingRight
        return if (available > 0 && text.isNotEmpty()) paint.measureText(text) - available else 0f
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val metrics = paint.fontMetrics
        val height = FloatingLyricsRules.lineHeightPx(metrics.ascent, metrics.descent) + paddingTop + paddingBottom
        setMeasuredDimension(MeasureSpec.getSize(widthMeasureSpec), resolveSize(height, heightMeasureSpec))
    }

    override fun onDraw(canvas: Canvas) {
        stripTexts?.let { drawStrip(canvas, it); return }
        if (text.isEmpty()) return
        val available = (width - paddingLeft - paddingRight).toFloat()
        val textWidth = paint.measureText(text)
        val x = FloatingLyricsRules.lineStartX(available, textWidth, align, progress) + paddingLeft
        val metrics = paint.fontMetrics
        val baseline = (height - (metrics.descent - metrics.ascent)) / 2f - metrics.ascent
        canvas.save()
        // A sliding line is cut at the area's sides; the shadow / glow may reach past it otherwise, and above and below
        // always (the panel's transparent room; the parents do not clip).
        val sides = if (textWidth > available) 0 else width
        canvas.clipRect(-sides, -height, width + sides, 2 * height)
        rasterKeep.clear()
        rasterKeep.add(text)
        rasters.retain(rasterKeep)
        val spans = karaokeSpans
        if (spans != null && accent != 0 && !karaokeEdge.isNaN()) {
            // Complementary clips at the karaoke edge: accent before it, the text colour after it.
            val cut = x + karaokeEdge
            accentKeep.clear()
            accentKeep.add(text)
            accentRasters.retain(accentKeep)
            canvas.save()
            canvas.clipRect(-width.toFloat(), -height.toFloat(), cut, 2f * height)
            drawAccent(canvas, text, x, baseline)
            canvas.restore()
            canvas.clipRect(cut, -height.toFloat(), 2f * width, 2f * height)
        }
        drawText(canvas, text, x, baseline, 1f)
        canvas.restore()
    }

    private fun drawStrip(canvas: Canvas, texts: List<String>) {
        val metrics = paint.fontMetrics
        val baseline = (height - (metrics.descent - metrics.ascent)) / 2f - metrics.ascent
        // With the centre mask the strip is drawn into a layer the mask is applied to (DST_IN).
        if (maskEm > 0f) canvas.saveLayer(0f, -height.toFloat(), width.toFloat(), 2f * height, null) else canvas.save()
        // The strip is cut at the area's sides; above and below the shadow may reach into the panel's room.
        canvas.clipRect(0, -height, width, 2 * height)
        // Only the cells near the view (one view of margin on each side), not every line of the song.
        val cells = FloatingLyricsRules.stripWindow(stripCells, stripWidths, stripOffset + paddingLeft, width.toFloat(), width.toFloat())
        rasterKeep.clear()
        for (index in cells) texts.getOrNull(index)?.let { rasterKeep.add(it) }
        rasters.retain(rasterKeep)
        for (index in cells) {
            val value = texts.getOrNull(index) ?: continue
            if (value.isEmpty()) continue
            val cell = stripCells.getOrElse(index) { 0f } + stripOffset + paddingLeft
            val cellWidth = stripWidths.getOrElse(index) { 0f }
            if (cell > width || cell + cellWidth < 0f) continue
            val textWidth = paint.measureText(value)
            // The centre mask dims instead of the line fades.
            val weight = if (inactiveFactor < 1f && maskEm <= 0f) FloatingLyricsRules.cellWeight(stripStarts, index, stripLyricsMs) else 1f
            drawText(canvas, value, cell + FloatingLyricsRules.lineStartX(cellWidth, textWidth, FloatingLyricsRules.STRIP_ALIGN, 0f), baseline, weight)
        }
        if (maskEm > 0f) {
            // DST_IN over the row's layer: one static gradient (rebuilt only when the width, em or an opacity changes).
            val key = listOf<Any>(width, maskEm, maskMain, maskSecondary)
            if (maskKey != key) {
                maskKey = key
                val stops = WordLyricsRules.centreMaskStops(width.toDouble(), maskEm.toDouble(), maskMain.toDouble(), maskSecondary.toDouble())
                maskPaint.shader = if (stops.isEmpty()) null else LinearGradient(
                    0f, 0f, width.toFloat(), 0f,
                    IntArray(stops.size) { (Math.round(stops[it].second * 255).toInt().coerceIn(0, 255) shl 24) or 0xffffff },
                    FloatArray(stops.size) { stops[it].first.toFloat() },
                    Shader.TileMode.CLAMP,
                )
            }
            canvas.drawRect(0f, -height.toFloat(), width.toFloat(), 2f * height, maskPaint)
        }
        canvas.restore()
    }

    private fun drawText(canvas: Canvas, value: String, x: Float, baseline: Float, weight: Float) {
        val raster = rasters.get(value)
        if (raster != null) {
            val factor = if (inactiveFactor >= 1f || weight >= 1f) 1f
            else if (weight <= 0f) inactiveFactor
            else FloatingLyricsRules.fadedFactor(inactiveFactor, weight)
            // The image holds text, shadow and outline at full opacity, so its alpha fades all three together.
            bitmapPaint.alpha = Math.round(255 * factor).coerceIn(0, 255)
            val top = baseline + paint.fontMetrics.ascent - raster.pad
            canvas.drawBitmap(raster.bitmap, Math.round(x - raster.pad).toFloat(), Math.round(top).toFloat(), bitmapPaint)
            return
        }
        val fill = paintFor(weight)
        if (strokeWidth > 0f) {
            val baseAlpha = android.graphics.Color.alpha(color)
            val opacity = if (baseAlpha == 0) 0f else fill.alpha.toFloat() / baseAlpha
            outlinePaint.alpha = Math.round(android.graphics.Color.alpha(strokeColor) * opacity).coerceIn(0, 255)
            canvas.drawText(value, x, baseline, outlinePaint)
        }
        canvas.drawText(value, x, baseline, fill)
    }

    private fun drawAccent(canvas: Canvas, value: String, x: Float, baseline: Float) {
        val raster = accentRasters.get(value)
        if (raster != null) {
            bitmapPaint.alpha = 255
            val top = baseline + paint.fontMetrics.ascent - raster.pad
            canvas.drawBitmap(raster.bitmap, Math.round(x - raster.pad).toFloat(), Math.round(top).toFloat(), bitmapPaint)
            return
        }
        if (strokeWidth > 0f) {
            outlinePaint.alpha = android.graphics.Color.alpha(strokeColor)
            canvas.drawText(value, x, baseline, outlinePaint)
        }
        canvas.drawText(value, x, baseline, accentPaint)
    }

    /** `value` with its shadow / outline into an image with room for the blur; null if empty or too large for a texture. */
    private fun rasterise(value: String, look: LineLook, fill: Paint = paint): Raster? {
        if (value.isEmpty()) return null
        val blur = if (look.shadowRadius > 0f) kotlin.math.ceil(3f * (0.57735f * look.shadowRadius + 0.5f)).toInt() else 0
        val pad = maxOf(blur, kotlin.math.ceil(look.strokeWidth).toInt()) + 1
        val metrics = paint.fontMetrics
        val w = kotlin.math.ceil(paint.measureText(value)).toInt() + 2 * pad
        val h = kotlin.math.ceil(metrics.descent - metrics.ascent).toInt() + 2 * pad
        if (w <= 0 || h <= 0 || w > MAX_RASTER_PX || h > MAX_RASTER_PX) return null
        val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        val target = Canvas(bitmap)
        val baseline = pad - metrics.ascent
        if (look.strokeWidth > 0f) {
            outlinePaint.alpha = android.graphics.Color.alpha(look.strokeColor)
            target.drawText(value, pad.toFloat(), baseline, outlinePaint)
        }
        target.drawText(value, pad.toFloat(), baseline, fill)
        return Raster(bitmap, pad)
    }

    private companion object {
        // Bitmaps wider than the GPU's texture limit are not drawn; such a line falls back to direct drawing.
        const val MAX_RASTER_PX = 4096
    }
}
