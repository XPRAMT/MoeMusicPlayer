package com.moemusicplayer.mediaindex

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Process
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** Everything that changes how a rasterised line looks (ADR 0032): a change re-rasterises every line. */
internal data class LineLook(
    val sizePx: Float,
    val color: Int,
    val shadowRadius: Float,
    val shadowColor: Int,
    val strokeWidth: Float,
    val strokeColor: Int,
    val density: Float,
    /** Font weight of this line's typeface: lines drawn at another weight are other bitmaps. */
    val weight: Int = FloatingLyricsRules.DEFAULT_FONT_WEIGHT,
    /** The view never left the platform's default face (its weight was never changed from the default): that face, not `weight`'s. */
    val defaultFace: Boolean = false,
)

/**
 * A rasterised line for the GL renderer, cut into texture tiles (see [TilePlan]): `bitmaps[k]` holds the image columns
 * `tiles[k].srcX until srcX + width` plus the one-pixel apron on each side. The renderer owns the bitmaps from here on
 * (uploads them, then recycles them).
 */
internal class TileSet(val bitmaps: Array<Bitmap>, val tiles: List<TilePlan.Tile>, val imageW: Int, val imageH: Int, val pad: Int)

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
    // Line images taller than this are not drawn.
    private const val MAX_RASTER_PX = 4096

    /** Widest line image the GL renderer takes (a longer one is not drawn); tiles keep every bitmap small. */
    private const val MAX_TILED_PX = 32_768

    /**
     * [render] for the GL renderer: the same image (text, shadow and outline at full opacity, `pad` of room), cut into
     * tiles of at most `chunk` content columns so a line can be wider than a texture. Null if empty or absurdly large.
     * A pure function of `value`, `look` and `chunk`, so it runs on any thread.
     */
    fun renderTiles(value: String, look: LineLook, chunk: Int): TileSet? {
        if (!LyricsTrace.on) return buildTiles(value, look, chunk)
        val section = LyricsTrace.begin("moeLyrics.raster")
        try {
            return buildTiles(value, look, chunk)
        } finally {
            LyricsTrace.end(section, false)
        }
    }

    private fun buildTiles(value: String, look: LineLook, chunk: Int): TileSet? {
        if (value.isEmpty()) return null
        val started = if (LyricsTrace.on) System.nanoTime() else 0L
        val face = if (look.defaultFace) Typeface.DEFAULT else lyricTypeface(look.weight)
        val fill = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
            typeface = face
            textSize = look.sizePx
            color = look.color
            if (look.shadowRadius > 0f) setShadowLayer(look.shadowRadius, 0f, 0f, look.shadowColor)
        }
        val outline = if (look.strokeWidth > 0f) {
            Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
                typeface = face
                textSize = look.sizePx
                style = Paint.Style.STROKE
                strokeJoin = Paint.Join.ROUND
                color = look.strokeColor
                strokeWidth = look.strokeWidth
            }
        } else {
            null
        }
        val blur = if (look.shadowRadius > 0f) kotlin.math.ceil(3f * (0.57735f * look.shadowRadius + 0.5f)).toInt() else 0
        val pad = maxOf(blur, kotlin.math.ceil(look.strokeWidth).toInt()) + 1
        val metrics = fill.fontMetrics
        val w = kotlin.math.ceil(fill.measureText(value)).toInt() + 2 * pad
        val h = kotlin.math.ceil(metrics.descent - metrics.ascent).toInt() + 2 * pad
        if (w <= 0 || h <= 0 || w > MAX_TILED_PX || h > MAX_RASTER_PX) return null
        val tiles = TilePlan.of(w, chunk)
        val baseline = pad - metrics.ascent
        val bitmaps = ArrayList<Bitmap>(tiles.size)
        try {
            for (tile in tiles) {
                val bitmap = Bitmap.createBitmap(tile.width + 2 * TilePlan.APRON, h, Bitmap.Config.ARGB_8888)
                bitmaps.add(bitmap)
                val target = Canvas(bitmap)
                // Image column x lands at bitmap column x - (srcX - apron): the text is drawn shifted left by the tile's start.
                val x = (pad - (tile.srcX - TilePlan.APRON)).toFloat()
                if (outline != null) target.drawText(value, x, baseline, outline)
                target.drawText(value, x, baseline, fill)
            }
        } catch (error: Throwable) {
            bitmaps.forEach { it.recycle() }
            throw error
        }
        if (LyricsTrace.on) LyricsTrace.rasterised(w, h, System.nanoTime() - started)
        return TileSet(bitmaps.toTypedArray(), tiles, w, h, pad)
    }
}
