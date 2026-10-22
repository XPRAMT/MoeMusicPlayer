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

    fun setLook(sizePx: Float, color: Int, align: String, shadowRadius: Float, shadowColor: Int) {
        val resize = paint.textSize != sizePx || this.shadowRadius != shadowRadius
        paint.textSize = sizePx
        paint.color = color
        this.align = align
        this.shadowRadius = shadowRadius
        if (shadowRadius > 0f) paint.setShadowLayer(shadowRadius, 0f, 0f, shadowColor) else paint.clearShadowLayer()
        if (resize) requestLayout()
        invalidate()
    }

    // Continuous animation (ADR 0032): every line in one strip; `stripTexts` null means one line at a time.
    private var stripTexts: List<String>? = null
    private var stripCells = FloatArray(0)
    private var stripWidths = FloatArray(0)
    private var stripOffset = 0f

    /** Width of `value` in this line's font (for the strip layout). */
    fun textWidth(value: String): Float = if (value.isEmpty()) 0f else paint.measureText(value)

    /** Usable width (the alignment point is measured in it). */
    fun availableWidth(): Float = (width - paddingLeft - paddingRight).toFloat()

    /** Draw the whole strip: each text aligned in its cell (`cells` left edges, `widths`), all shifted by `offset`. */
    fun setStrip(texts: List<String>, cells: FloatArray, widths: FloatArray, offset: Float) {
        val changed = texts !== stripTexts || !cells.contentEquals(stripCells) || !widths.contentEquals(stripWidths)
        if (changed || offset != stripOffset) {
            stripTexts = texts
            stripCells = cells
            stripWidths = widths
            stripOffset = offset
            text = ""
            invalidate()
        }
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
            canvas.drawText(value, cell + FloatingLyricsRules.lineStartX(cellWidth, textWidth, FloatingLyricsRules.STRIP_ALIGN, 0f), baseline, paint)
        }
        canvas.restore()
    }
}