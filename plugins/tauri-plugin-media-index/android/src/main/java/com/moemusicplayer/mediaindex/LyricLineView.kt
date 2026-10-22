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

    fun setLine(value: String, progress: Float) {
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
        val height = Math.ceil((metrics.bottom - metrics.top + shadowRadius).toDouble()).toInt() + paddingTop + paddingBottom
        setMeasuredDimension(MeasureSpec.getSize(widthMeasureSpec), resolveSize(height, heightMeasureSpec))
    }

    override fun onDraw(canvas: Canvas) {
        if (text.isEmpty()) return
        val available = (width - paddingLeft - paddingRight).toFloat()
        val textWidth = paint.measureText(text)
        val x = FloatingLyricsRules.lineStartX(available, textWidth, align, progress) + paddingLeft
        val metrics = paint.fontMetrics
        val baseline = (height - (metrics.bottom - metrics.top)) / 2f - metrics.top
        canvas.save()
        canvas.clipRect(0, 0, width, height)
        canvas.drawText(text, x, baseline, paint)
        canvas.restore()
    }
}