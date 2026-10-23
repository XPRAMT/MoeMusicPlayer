package com.moemusicplayer.mediaindex

import android.content.Context
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Shader
import android.widget.LinearLayout

/**
 * The floating lyrics' lyric area (ADR 0032): its lines, with the left and right edge fade (square zones as wide as
 * the area is high, `FloatingLyricsRules.edgeFadeWidth`) applied as a horizontal alpha mask over everything the lines
 * draw, shadow and outline included. The background (move-mode highlight) is drawn by `draw` before `dispatchDraw`, so
 * it is not faded. The gradient is rebuilt only when the size changes; the lines' per-frame redraw is unchanged.
 */
internal class EdgeFadeLayout(context: Context) : LinearLayout(context) {
    private val maskPaint = Paint().apply { xfermode = PorterDuffXfermode(PorterDuff.Mode.DST_IN) }
    private var maskSize = 0L

    private fun updateMask(w: Int, h: Int): Boolean {
        val zone = FloatingLyricsRules.edgeFadeWidth(w.toFloat(), h.toFloat())
        if (zone <= 0f) return false
        val key = (w.toLong() shl 32) or h.toLong()
        if (key != maskSize) {
            val edge = zone / w
            maskPaint.shader = LinearGradient(
                0f, 0f, w.toFloat(), 0f,
                intArrayOf(0, -0x1000000, -0x1000000, 0),
                floatArrayOf(0f, edge, 1f - edge, 1f),
                Shader.TileMode.CLAMP,
            )
            maskSize = key
        }
        return true
    }

    override fun dispatchDraw(canvas: Canvas) {
        if (!updateMask(width, height)) {
            super.dispatchDraw(canvas)
            return
        }
        // The shadow / glow reaches into the panel's room above and below; the layer covers it too.
        val top = -height.toFloat()
        val bottom = 2f * height
        val layer = canvas.saveLayer(0f, top, width.toFloat(), bottom, null)
        super.dispatchDraw(canvas)
        canvas.drawRect(0f, top, width.toFloat(), bottom, maskPaint)
        canvas.restoreToCount(layer)
    }
}
