package com.moemusicplayer.mediaindex

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Typeface
import android.view.View

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
    private var text = ""
    private var align = "center"
    private var progress = 0f
    private var shadowRadius = 0f
    private var shadowColor = 0
    private var color = 0

    // Continuous animation: lines other than the current one are drawn at `inactiveFactor` of the main opacity (1: no
    // dimming). `dimPaint` is the look at that opacity, `fadePaint` the one reused for the (at most two) cells that
    // are fading, so a frame allocates nothing and re-styles a paint only when its opacity really changed.
    private var inactiveFactor = 1f
    private val dimPaint = Paint(paint)
    private val fadePaint = Paint(paint)
    private var fadeAlpha = -1

    fun setLook(sizePx: Float, color: Int, align: String, shadowRadius: Float, shadowColor: Int) {
        val resize = paint.textSize != sizePx || this.shadowRadius != shadowRadius
        paint.textSize = sizePx
        paint.color = color
        this.color = color
        this.shadowColor = shadowColor
        this.align = align
        this.shadowRadius = shadowRadius
        if (shadowRadius > 0f) paint.setShadowLayer(shadowRadius, 0f, 0f, shadowColor) else paint.clearShadowLayer()
        styleDim()
        fadeAlpha = -1
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
    private var stripStarts = LongArray(0)
    private var stripLyricsMs = 0L
    private var stripNextStart = 0

    /** Width of `value` in this line's font (for the strip layout). */
    fun textWidth(value: String): Float = if (value.isEmpty()) 0f else paint.measureText(value)

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
        if (inactiveFactor < 1f) {
            nextStart = FloatingLyricsRules.firstStartAfter(starts, lyricsMs)
            fading = nextStart != stripNextStart ||
                (lyricsMs != stripLyricsMs && nextStart < starts.size && starts[nextStart] - lyricsMs < FloatingLyricsRules.STRIP_FADE_MS)
        }
        if (changed || offset != stripOffset || fading) {
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
            this.progress = fraction
            if (overflow() > 0f) invalidate()
        }
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
        canvas.drawText(text, x, baseline, paint)
        canvas.restore()
    }

    private fun drawStrip(canvas: Canvas, texts: List<String>) {
        val metrics = paint.fontMetrics
        val baseline = (height - (metrics.descent - metrics.ascent)) / 2f - metrics.ascent
        canvas.save()
        // The strip is cut at the area's sides; above and below the shadow may reach into the panel's room.
        canvas.clipRect(0, -height, width, 2 * height)
        // Only the cells near the view (one view of margin on each side), not every line of the song.
        val cells = FloatingLyricsRules.stripWindow(stripCells, stripWidths, stripOffset + paddingLeft, width.toFloat(), width.toFloat())
        for (index in cells) {
            val value = texts.getOrNull(index) ?: continue
            if (value.isEmpty()) continue
            val cell = stripCells.getOrElse(index) { 0f } + stripOffset + paddingLeft
            val cellWidth = stripWidths.getOrElse(index) { 0f }
            if (cell > width || cell + cellWidth < 0f) continue
            val textWidth = paint.measureText(value)
            val weight = if (inactiveFactor < 1f) FloatingLyricsRules.cellWeight(stripStarts, index, stripLyricsMs) else 1f
            canvas.drawText(value, cell + FloatingLyricsRules.lineStartX(cellWidth, textWidth, FloatingLyricsRules.STRIP_ALIGN, 0f), baseline, paintFor(weight))
        }
        canvas.restore()
    }
}