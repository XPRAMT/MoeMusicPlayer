package com.moemusicplayer.mediaindex

import android.content.Context
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.Shader
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
import android.view.View
import java.text.BreakIterator
import java.util.Locale

/**
 * One line of the Android floating lyrics, drawn directly (ADR 0032). A line that fits is placed by the alignment; a
 * wider one starts at the left and slides by `progress` (0..1) to its end, at a fractional offset so the overlay can
 * move it every display frame without the jumps of `TextView.scrollTo` and without TextView's single-line layout,
 * which put short centred or right-aligned lines outside the view.
 *
 * Each line is an image made on a background thread ([LineRasterCache]); `onDraw` only ever draws finished images and
 * never rasterises. The overlay asks for the images ahead of time (the strip window with a view of room on both sides;
 * the neighbouring lines in the none mode, [prefetchLines]). A cell whose image is still on its way is not drawn that
 * frame (a request is repeated and the view redraws the moment it arrives; counted as a fallback in [LyricsTrace]);
 * only a line that cannot be an image at all (too large for a texture) is drawn directly.
 */
internal class LyricLineView(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        typeface = Typeface.DEFAULT
    }
    private var fontWeight = FloatingLyricsRules.DEFAULT_FONT_WEIGHT
    /** False until the weight first changes: until then the paint keeps the platform's default face, which the images must use too. */
    private var customFace = false
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

    /** Bumped when the text size or the typeface changes: what the measured widths and the font metrics depend on. */
    var measureVersion = 0
        private set
    private val fontMetricsBuffer = Paint.FontMetrics()
    private var metricsVersion = -1
    private var ascent = 0f
    private var descent = 0f
    private var shownWidthText: String? = null
    private var shownWidthVersion = -1
    private var shownWidth = 0f

    // Continuous animation: lines other than the current one are drawn at `inactiveFactor` of the main opacity (1: no
    // dimming). `dimPaint` is the look at that opacity, `fadePaint` the one reused for the (at most two) cells that
    // are fading, so a frame allocates nothing and re-styles a paint only when its opacity really changed.
    private var inactiveFactor = 1f
    private val dimPaint = Paint(paint)
    private val fadePaint = Paint(paint)
    private var fadeAlpha = -1

    private val mainHandler = Handler(Looper.getMainLooper())

    // Each line is rasterised once (in the background) with its shadow / outline (ADR 0032) and only drawn at its offset
    // while gliding. The images are immutable, so they are shared with the draw thread without copying.
    private val bitmapPaint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val rasterKeep = HashSet<String>()
    private val rasters = newCache()

    // Karaoke (the none mode, ADR 0041): the line is also rasterised in the accent (same shadow / outline, so the cut
    // between the two images has no seam); the accent image is drawn up to the karaoke edge, the white one after it.
    private val accentPaint = Paint(paint)
    private var accent = 0
    private val accentKeep = HashSet<String>()
    private val accentRasters = newCache()
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
    private var maskBuilt = false
    private var maskBuiltWidth = 0
    private var maskBuiltEm = 0f
    private var maskBuiltMain = 0f
    private var maskBuiltSecondary = 0f

    private fun newCache() = LineRasterCache<Raster>(
        create = { value, look -> LineRasteriser.render(value, look) },
        release = { /* dropped; the GC frees it after any frame using it */ },
        submit = RasterWorker::submit,
        deliver = { mainHandler.post(it) },
        onReady = { inval() },
    )

    private fun inval() {
        if (LyricsTrace.on) LyricsTrace.invalidated()
        invalidate()
    }

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
        inval()
    }

    private fun lookFor(fill: Int): LineLook = LineLook(
        paint.textSize, fill, shadowRadius, shadowColor, strokeWidth, strokeColor, resources.displayMetrics.density, fontWeight, !customFace,
    )

    private fun styleAccent() {
        accentPaint.set(paint)
        accentPaint.color = accent
        accentRasters.setLook(lookFor(accent))
    }

    /**
     * The karaoke of the shown line: `ranges` its words' UTF-16 ranges, `times` their times and `lyricsMs` now (null:
     * the whole line in the text colour). Redrawn only when the edge moves a whole device pixel.
     */
    fun setKaraoke(ranges: List<Pair<Int, Int>>?, times: List<Pair<Double, Double>>?, lyricsMs: Double) {
        if (ranges == null || times == null || text.isEmpty() || stripTexts != null) {
            if (karaokeSpans != null) {
                karaokeSpans = null
                inval()
            }
            return
        }
        if (karaokeSpans == null || karaokeSpansText != text) {
            karaokeSpans = wordSpans(text, ranges)
            karaokeSpansText = text
            karaokeEdge = Float.NaN
            karaokeTimes = null
        }
        accentRasters.request(text)
        // One smooth edge per line (ADR 0041 D10), built once per line; per frame only its value is read.
        if (karaokeTimes !== times) {
            karaokeGlide = WordLyricsRules.karaokeGlide(karaokeSpans!!, times)
            karaokeTimes = times
        }
        val edge = WordLyricsRules.glideValue(karaokeGlide, lyricsMs).toFloat()
        if (karaokeEdge.isNaN() || FloatingLyricsRules.needsPixelRedraw(karaokeEdge, edge, karaokeRawEdge)) {
            karaokeEdge = edge
            inval()
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
        maskBuilt = false
        inval()
    }

    /** The typeface for `weight`: any weight on API 28+, bold from 600 on older systems (see `typefaceChoice`). */
    private fun typefaceFor(weight: Int): Typeface = lyricTypeface(weight)

    /** `weight` is this view's own (the original text's, or 50 lighter for the translation). A change re-measures and re-rasterises. */
    fun setLook(sizePx: Float, color: Int, align: String, shadowRadius: Float, shadowColor: Int, strokeWidth: Float, strokeColor: Int, weight: Int = FloatingLyricsRules.DEFAULT_FONT_WEIGHT) {
        val resize = paint.textSize != sizePx || this.shadowRadius != shadowRadius || this.strokeWidth != strokeWidth || fontWeight != weight
        var measuresChanged = paint.textSize != sizePx
        if (fontWeight != weight || paint.typeface == null) {
            fontWeight = weight
            customFace = true
            measuresChanged = true
            val face = typefaceFor(weight)
            paint.typeface = face
            outlinePaint.typeface = face
        }
        if (measuresChanged) measureVersion++
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
        rasters.setLook(lookFor(color))
        styleAccent()
        if (resize) requestLayout()
        inval()
    }

    /** Opacity of the lines other than the current one in the continuous strip, relative to the main opacity (1 = none). */
    fun setInactiveFactor(factor: Float) {
        val value = factor.coerceIn(0f, 1f)
        if (value == inactiveFactor) return
        inactiveFactor = value
        styleDim()
        fadeAlpha = -1
        inval()
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

    private fun ensureMetrics() {
        if (metricsVersion == measureVersion) return
        paint.getFontMetrics(fontMetricsBuffer)
        ascent = fontMetricsBuffer.ascent
        descent = fontMetricsBuffer.descent
        metricsVersion = measureVersion
    }

    /** Width of the shown line: measured once per text and font, not every frame. */
    private fun shownTextWidth(): Float {
        if (shownWidthVersion != measureVersion || shownWidthText != text) {
            shownWidth = paint.measureText(text)
            shownWidthText = text
            shownWidthVersion = measureVersion
        }
        return shownWidth
    }

    // Continuous animation (ADR 0032): every line in one strip; `stripTexts` null means one line at a time.
    private var stripTexts: List<String>? = null
    private var stripCells = FloatArray(0)
    private var stripWidths = FloatArray(0)
    private var stripTextWidths = FloatArray(0)
    private var stripOffset = 0f
    private var stripRawOffset = 0f
    private var stripStarts = LongArray(0)
    private var stripLyricsMs = 0L
    private var stripNextStart = 0
    // What the strip's images were last asked for: the window, the strip and the look they were asked under.
    private var prefetchPacked = -1L
    private var prefetchTexts: List<String>? = null
    private var prefetchGeneration = -1
    private var prefetchWidth = -1

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
     * Draw the whole strip: each text aligned in its cell (`cells` left edges, `widths`), all shifted by `offset`
     * (`textWidths` are the texts' own widths). `starts` are the lines' start times and `lyricsMs` the lyrics time of
     * this frame: the cells' fades follow from them (FloatingLyricsRules.cellWeight), so redrawing is wanted when the
     * offset moves, and while a fade is running or another line became the next one (a seek) even if the strip itself
     * stays put. The arrays belong to the strip and are replaced, never changed, when it is rebuilt.
     */
    fun setStrip(texts: List<String>, cells: FloatArray, widths: FloatArray, textWidths: FloatArray, offset: Float, starts: LongArray, lyricsMs: Long) {
        val changed = texts !== stripTexts || cells !== stripCells || widths !== stripWidths
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
            stripTextWidths = textWidths
            stripOffset = offset
            text = ""
            inval()
        }
        stripStarts = starts
        stripLyricsMs = lyricsMs
        stripNextStart = nextStart
        prefetchStrip(texts, cells, widths, offset)
    }

    /**
     * Ask for the images of the cells in (and a view either side of) the drawn window, nearest the view's centre first,
     * and drop the ones beyond. Only runs when the window, the strip, the width or the look changed (about once per
     * line change), never every frame.
     */
    private fun prefetchStrip(texts: List<String>, cells: FloatArray, widths: FloatArray, offset: Float) {
        val viewWidth = width
        if (viewWidth <= 0) return
        val packed = FloatingLyricsRules.stripWindowPacked(cells, widths, offset + paddingLeft, viewWidth.toFloat(), (PREFETCH_VIEWS * viewWidth).toFloat())
        if (packed == prefetchPacked && texts === prefetchTexts && rasters.generation == prefetchGeneration && viewWidth == prefetchWidth) return
        prefetchPacked = packed
        prefetchTexts = texts
        prefetchGeneration = rasters.generation
        prefetchWidth = viewWidth
        val first = FloatingLyricsRules.windowFirst(packed)
        val last = FloatingLyricsRules.windowLast(packed)
        rasterKeep.clear()
        for (index in first..last) texts.getOrNull(index)?.let { if (it.isNotEmpty()) rasterKeep.add(it) }
        rasters.retain(rasterKeep)
        if (first > last) return
        val visible = FloatingLyricsRules.stripWindowPacked(cells, widths, offset + paddingLeft, viewWidth.toFloat(), 0f)
        val centre = if (FloatingLyricsRules.windowFirst(visible) <= FloatingLyricsRules.windowLast(visible)) {
            (FloatingLyricsRules.windowFirst(visible) + FloatingLyricsRules.windowLast(visible)) / 2
        } else {
            first
        }.coerceIn(first, last)
        val reach = maxOf(centre - first, last - centre)
        for (distance in 0..reach) {
            texts.getOrNull(centre - distance)?.let { rasters.request(it) }
            if (distance > 0) texts.getOrNull(centre + distance)?.let { rasters.request(it) }
        }
    }

    /**
     * The none mode: keep and prepare the images of the lines around the shown one (`texts` in the wanted order,
     * `accentTexts` for the karaoke copies) so a line change finds its image ready. The shown line is always kept.
     */
    fun prefetchLines(texts: List<String>, accentTexts: List<String>) {
        rasterKeep.clear()
        rasterKeep.addAll(texts)
        if (text.isNotEmpty()) rasterKeep.add(text)
        rasters.retain(rasterKeep)
        for (value in texts) rasters.request(value)
        accentKeep.clear()
        accentKeep.addAll(accentTexts)
        if (text.isNotEmpty()) accentKeep.add(text)
        accentRasters.retain(accentKeep)
        for (value in accentTexts) accentRasters.request(value)
    }

    /** Changes whenever the look (so every image) was replaced: the overlay prepares its images again then. */
    fun rasterEpoch(): Int = rasters.generation * 31 + accentRasters.generation

    private var rawProgress = 0f

    fun setLine(value: String, progress: Float) {
        if (stripTexts != null) {
            stripTexts = null
            prefetchTexts = null
            text = ""
        }
        val fraction = progress.coerceIn(0f, 1f)
        if (value != text) {
            text = value
            this.progress = fraction
            rasters.request(value)
            inval()
        } else if (fraction != this.progress) {
            // A long line slides by `progress` of its overflow: redraw only when that moves a whole pixel.
            val overflow = overflow()
            if (overflow > 0f && FloatingLyricsRules.needsPixelRedraw(overflow * this.progress, overflow * fraction, overflow * rawProgress)) {
                this.progress = fraction
                inval()
            }
        }
        rawProgress = fraction
    }

    /** True when the line is wider than the view and therefore slides. */
    fun overflows(): Boolean = overflow() > 0f

    private fun overflow(): Float {
        val available = width - paddingLeft - paddingRight
        return if (available > 0 && text.isNotEmpty()) shownTextWidth() - available else 0f
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        ensureMetrics()
        val height = FloatingLyricsRules.lineHeightPx(ascent, descent) + paddingTop + paddingBottom
        setMeasuredDimension(MeasureSpec.getSize(widthMeasureSpec), resolveSize(height, heightMeasureSpec))
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        prefetchTexts = null
    }

    override fun onDraw(canvas: Canvas) {
        if (!LyricsTrace.on) {
            drawContent(canvas)
            return
        }
        val section = LyricsTrace.begin("moeLyrics.draw")
        try {
            drawContent(canvas)
        } finally {
            LyricsTrace.end(section, true)
        }
    }

    private fun drawContent(canvas: Canvas) {
        stripTexts?.let { drawStrip(canvas, it); return }
        if (text.isEmpty()) return
        ensureMetrics()
        val available = (width - paddingLeft - paddingRight).toFloat()
        val textWidth = shownTextWidth()
        val x = FloatingLyricsRules.lineStartX(available, textWidth, align, progress) + paddingLeft
        val baseline = (height - (descent - ascent)) / 2f - ascent
        canvas.save()
        // A sliding line is cut at the area's sides; the shadow / glow may reach past it otherwise, and above and below
        // always (the panel's transparent room; the parents do not clip).
        val sides = if (textWidth > available) 0 else width
        canvas.clipRect(-sides, -height, width + sides, 2 * height)
        val spans = karaokeSpans
        if (spans != null && accent != 0 && !karaokeEdge.isNaN()) {
            if (accentRasters.peek(text) != null || accentRasters.isUnavailable(text)) {
                // Complementary clips at the karaoke edge: accent before it, the text colour after it.
                val cut = x + karaokeEdge
                canvas.save()
                canvas.clipRect(-width.toFloat(), -height.toFloat(), cut, 2f * height)
                drawAccent(canvas, text, x, baseline)
                canvas.restore()
                canvas.clipRect(cut, -height.toFloat(), 2f * width, 2f * height)
            } else {
                // The accent copy is still being made: the whole line in the text colour until it arrives.
                accentRasters.request(text)
                if (LyricsTrace.on) LyricsTrace.fellBack()
            }
        }
        drawText(canvas, text, x, baseline, 1f)
        canvas.restore()
    }

    private fun drawStrip(canvas: Canvas, texts: List<String>) {
        ensureMetrics()
        val baseline = (height - (descent - ascent)) / 2f - ascent
        // With the centre mask the strip is drawn into a layer the mask is applied to (DST_IN).
        if (maskEm > 0f) canvas.saveLayer(0f, -height.toFloat(), width.toFloat(), 2f * height, null) else canvas.save()
        // The strip is cut at the area's sides; above and below the shadow may reach into the panel's room.
        canvas.clipRect(0, -height, width, 2 * height)
        // Only the cells near the view (one view of margin on each side), not every line of the song.
        val window = FloatingLyricsRules.stripWindowPacked(stripCells, stripWidths, stripOffset + paddingLeft, width.toFloat(), width.toFloat())
        val last = FloatingLyricsRules.windowLast(window)
        for (index in FloatingLyricsRules.windowFirst(window)..last) {
            val value = texts.getOrNull(index) ?: continue
            if (value.isEmpty()) continue
            val cell = (if (index < stripCells.size) stripCells[index] else 0f) + stripOffset + paddingLeft
            val cellWidth = if (index < stripWidths.size) stripWidths[index] else 0f
            if (cell > width || cell + cellWidth < 0f) continue
            val textWidth = if (index < stripTextWidths.size) stripTextWidths[index] else cellWidth
            // The centre mask dims instead of the line fades.
            val weight = if (inactiveFactor < 1f && maskEm <= 0f) FloatingLyricsRules.cellWeight(stripStarts, index, stripLyricsMs) else 1f
            drawText(canvas, value, cell + FloatingLyricsRules.lineStartX(cellWidth, textWidth, FloatingLyricsRules.STRIP_ALIGN, 0f), baseline, weight)
        }
        if (maskEm > 0f) {
            // DST_IN over the row's layer: one static gradient (rebuilt only when the width, em or an opacity changes).
            if (!maskBuilt || maskBuiltWidth != width || maskBuiltEm != maskEm || maskBuiltMain != maskMain || maskBuiltSecondary != maskSecondary) {
                maskBuilt = true
                maskBuiltWidth = width
                maskBuiltEm = maskEm
                maskBuiltMain = maskMain
                maskBuiltSecondary = maskSecondary
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
        val raster = rasters.peek(value)
        if (raster != null) {
            val factor = if (inactiveFactor >= 1f || weight >= 1f) 1f
            else if (weight <= 0f) inactiveFactor
            else FloatingLyricsRules.fadedFactor(inactiveFactor, weight)
            // The image holds text, shadow and outline at full opacity, so its alpha fades all three together.
            bitmapPaint.alpha = Math.round(255 * factor).coerceIn(0, 255)
            val top = baseline + ascent - raster.pad
            canvas.drawBitmap(raster.bitmap, Math.round(x - raster.pad).toFloat(), Math.round(top).toFloat(), bitmapPaint)
            return
        }
        if (!rasters.isUnavailable(value)) {
            // The image is not finished yet: nothing this frame (the request is repeated; the view redraws when it lands).
            rasters.request(value)
            if (LyricsTrace.on) LyricsTrace.fellBack()
            return
        }
        // Too large to be an image: drawn directly.
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
        val raster = accentRasters.peek(value)
        if (raster != null) {
            bitmapPaint.alpha = 255
            val top = baseline + ascent - raster.pad
            canvas.drawBitmap(raster.bitmap, Math.round(x - raster.pad).toFloat(), Math.round(top).toFloat(), bitmapPaint)
            return
        }
        if (strokeWidth > 0f) {
            outlinePaint.alpha = android.graphics.Color.alpha(strokeColor)
            canvas.drawText(value, x, baseline, outlinePaint)
        }
        canvas.drawText(value, x, baseline, accentPaint)
    }

    private companion object {
        /** How many view widths of images are prepared beyond each side of the view in the continuous strip. */
        const val PREFETCH_VIEWS = 2
    }
}
