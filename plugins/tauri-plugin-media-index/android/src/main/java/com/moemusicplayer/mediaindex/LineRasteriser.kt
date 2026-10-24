package com.moemusicplayer.mediaindex

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Process
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** A rasterised line: the image (text, shadow and outline at full opacity) and the room `pad` it keeps around the text. */
internal class Raster(val bitmap: Bitmap, val pad: Int)

/** The typeface for `weight`: any weight on API 28+, bold from 600 on older systems (see `typefaceChoice`). */
internal fun lyricTypeface(weight: Int): Typeface {
    val choice = FloatingLyricsRules.typefaceChoice(weight, android.os.Build.VERSION.SDK_INT)
    return if (choice.weight != null) {
        Typeface.create(Typeface.DEFAULT, choice.weight, false)
    } else {
        Typeface.create(Typeface.DEFAULT, if (choice.bold) Typeface.BOLD else Typeface.NORMAL)
    }
}

/**
 * Builds the line images on a background thread (ADR 0032): a single low-priority worker that sleeps when idle, so the
 * main thread (which also runs the WebView) never pays for the text shadow's blur. Not starved: the priority is one
 * step below the default, not the background group.
 */
internal object RasterWorker {
    private val executor: ThreadPoolExecutor by lazy {
        ThreadPoolExecutor(1, 1, 20, TimeUnit.SECONDS, LinkedBlockingQueue(), { task ->
            Thread({
                Process.setThreadPriority(Process.THREAD_PRIORITY_LESS_FAVORABLE)
                task.run()
            }, "moe-lyrics-raster")
        }).apply { allowCoreThreadTimeOut(true) }
    }

    fun submit(task: Runnable) {
        executor.execute(task)
    }
}

internal object LineRasteriser {
    // Bitmaps wider than the GPU's texture limit are not drawn; such a line falls back to direct drawing.
    private const val MAX_RASTER_PX = 4096

    /**
     * `value` with its shadow / outline into an image with room for the blur; null if empty or too large for a texture.
     * A pure function of `value` and `look`, so it runs on any thread; the finished bitmap is uploaded for drawing
     * here (`prepareToDraw`) so the first frame that draws it does not.
     */
    fun render(value: String, look: LineLook): Raster? {
        if (!LyricsTrace.on) return build(value, look)
        val section = LyricsTrace.begin("moeLyrics.raster")
        try {
            return build(value, look)
        } finally {
            LyricsTrace.end(section, false)
        }
    }

    private fun build(value: String, look: LineLook): Raster? {
        if (value.isEmpty()) return null
        val started = if (LyricsTrace.on) System.nanoTime() else 0L
        val face = if (look.defaultFace) Typeface.DEFAULT else lyricTypeface(look.weight)
        val fill = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
            typeface = face
            textSize = look.sizePx
            color = look.color
            if (look.shadowRadius > 0f) setShadowLayer(look.shadowRadius, 0f, 0f, look.shadowColor)
        }
        val blur = if (look.shadowRadius > 0f) kotlin.math.ceil(3f * (0.57735f * look.shadowRadius + 0.5f)).toInt() else 0
        val pad = maxOf(blur, kotlin.math.ceil(look.strokeWidth).toInt()) + 1
        val metrics = fill.fontMetrics
        val w = kotlin.math.ceil(fill.measureText(value)).toInt() + 2 * pad
        val h = kotlin.math.ceil(metrics.descent - metrics.ascent).toInt() + 2 * pad
        if (w <= 0 || h <= 0 || w > MAX_RASTER_PX || h > MAX_RASTER_PX) return null
        val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        val target = Canvas(bitmap)
        val baseline = pad - metrics.ascent
        if (look.strokeWidth > 0f) {
            val outline = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
                typeface = face
                textSize = look.sizePx
                style = Paint.Style.STROKE
                strokeJoin = Paint.Join.ROUND
                color = look.strokeColor
                strokeWidth = look.strokeWidth
            }
            target.drawText(value, pad.toFloat(), baseline, outline)
        }
        target.drawText(value, pad.toFloat(), baseline, fill)
        bitmap.prepareToDraw()
        if (LyricsTrace.on) LyricsTrace.rasterised(w, h, System.nanoTime() - started)
        return Raster(bitmap, pad)
    }
}
