package com.moemusicplayer.mediaindex

/**
 * What one frame of the GL overlay shows (ADR 0032): the output of [LyricsFramePlanner]. A plain mutable holder the
 * render thread reuses every frame (nothing is allocated); [sameAs] compares everything that changes a pixel, so an
 * unchanged frame is not drawn again. Positions are px from the lyric area's left edge.
 */
internal class FrameParams {
    /** Some lyric text is drawn. */
    var visible = false

    /** The move-mode background is drawn. */
    var background = false

    /** The continuous strip (cells) rather than one line per row. */
    var strip = false

    /** The continuous look's centre mask rather than the edge fade. */
    var contMask = false

    // Strip: both rows share the offset; the cells to draw are firstCell..lastCell of the strip model.
    var offset = 0f
    var firstCell = 0
    var lastCell = -1

    // One line per row: line indexes of the model (NO_LINE, NOTE_LINE) and the text's left edge.
    var mainLine = NO_LINE
    var secondLine = NO_LINE

    /** The current line of the model (-1: none yet), whether or not it has text; what the prefetch looks around. */
    var lineIndex = -1
    var mainX = 0f
    var secondX = 0f

    /** Area-local x where the karaoke accent ends and the text colour begins; NaN: no karaoke on this line. */
    var karaokeX = Float.NaN

    /** Something moves with the clock: the next frame is wanted. */
    var animating = false

    /** The lyrics time (ms) of the next change while nothing moves ([FloatingLyricsRules.NO_TIME]: none known). */
    var wakeAtLyricsMs = FloatingLyricsRules.NO_TIME

    /** The lyrics time (ms) of this plan; the unquantised strip offset. For the render thread's wake-up and for tests. */
    var lyricsMs = 0.0
    var rawOffset = 0.0

    fun reset() {
        visible = false
        background = false
        strip = false
        contMask = false
        offset = 0f
        firstCell = 0
        lastCell = -1
        mainLine = NO_LINE
        secondLine = NO_LINE
        lineIndex = -1
        mainX = 0f
        secondX = 0f
        karaokeX = Float.NaN
        animating = false
        wakeAtLyricsMs = FloatingLyricsRules.NO_TIME
        lyricsMs = 0.0
        rawOffset = 0.0
    }

    fun set(o: FrameParams) {
        visible = o.visible
        background = o.background
        strip = o.strip
        contMask = o.contMask
        offset = o.offset
        firstCell = o.firstCell
        lastCell = o.lastCell
        mainLine = o.mainLine
        secondLine = o.secondLine
        lineIndex = o.lineIndex
        mainX = o.mainX
        secondX = o.secondX
        karaokeX = o.karaokeX
        animating = o.animating
        wakeAtLyricsMs = o.wakeAtLyricsMs
        lyricsMs = o.lyricsMs
        rawOffset = o.rawOffset
    }

    /** True when drawing `o` would give the same picture as drawing this. */
    fun sameAs(o: FrameParams): Boolean = visible == o.visible && background == o.background && strip == o.strip &&
        contMask == o.contMask && offset == o.offset && firstCell == o.firstCell && lastCell == o.lastCell &&
        mainLine == o.mainLine && secondLine == o.secondLine && mainX == o.mainX && secondX == o.secondX &&
        (karaokeX == o.karaokeX || (karaokeX.isNaN() && o.karaokeX.isNaN()))

    companion object {
        const val NO_LINE = -2
        const val NOTE_LINE = -1
    }
}

/**
 * The pure core of the GL overlay (ADR 0032): the playback clock, the model and the look in, what to draw out. It is
 * the render-thread twin of the Views path's `render()` / `renderStrip()`, built on the same rules
 * ([FloatingLyricsRules], [WordLyricsRules], checked against the shared vectors), but on a fractional clock and with
 * sub-pixel positions: a moving position is rounded to 1/8 px, a resting one to a whole pixel (so a still line is
 * crisp). One instance per render thread; it keeps only a cache of the line glide's knots.
 */
internal class LyricsFramePlanner {
    private var knotsModel: StripModel? = null
    private var knotsEnd = FloatingLyricsRules.NO_TIME
    private var knots: FloatingLyricsRules.StripKnots? = null

    /**
     * Plans the frame at `positionMs` (fractional, the smoothed clock; NaN: no clock) into `out`. `clock` supplies the
     * playing / active / track / duration facts of that position.
     */
    fun plan(state: RenderState, clock: ClockSnapshot?, positionMs: Double, out: FrameParams) {
        out.reset()
        val look = state.look
        val model = state.model
        out.background = !look.locked
        val have = clock != null && !positionMs.isNaN()
        val matches = have && clock!!.active && clock.trackId == model.trackId
        val strip = model.strip
        if (look.animation == ANIMATION_CONTINUOUS && matches && strip != null) {
            planStrip(look, model, strip, clock!!, positionMs, out)
        } else {
            planLines(look, model, if (have) clock else null, matches, positionMs, out)
        }
    }

    private fun duration(clock: ClockSnapshot, model: RenderModel): Long =
        if (clock.durationMs != FloatingLyricsRules.NO_TIME) clock.durationMs else model.durationMs

    private fun planStrip(look: RenderLook, model: RenderModel, strip: StripModel, clock: ClockSnapshot, positionMs: Double, out: FrameParams) {
        val areaW = look.layout.areaWidthPx.toFloat()
        val lyricsMs = positionMs + model.offsetMs
        val offset: Float
        val firstT: Double
        val lastT: Double
        val wordKnots = strip.wordKnots
        if (wordKnots != null && wordKnots.times.isNotEmpty()) {
            offset = (areaW / 2.0 - WordLyricsRules.knotValue(wordKnots, lyricsMs)).toFloat()
            firstT = wordKnots.times[0]
            lastT = wordKnots.times[wordKnots.times.size - 1]
        } else {
            val duration = duration(clock, model)
            val trackEnd = if (duration != FloatingLyricsRules.NO_TIME) duration + model.offsetMs else FloatingLyricsRules.NO_TIME
            val arrival = FloatingLyricsRules.finalLineArrivalFromGraphemes(strip.starts[strip.starts.size - 1], strip.lastGraphemes, trackEnd)
            if (knotsModel !== strip || knotsEnd != arrival) {
                knots = FloatingLyricsRules.stripKnots(
                    strip.starts, strip.cells, strip.widths, FloatingLyricsRules.STRIP_ALIGN, arrival,
                    strip.mainWidths, strip.lastGlyphCenters, strip.firstGlyphCenters,
                )
                knotsModel = strip
                knotsEnd = arrival
            }
            val k = knots
            offset = if (k == null) 0f else FloatingLyricsRules.stripKnotsOffset(k, lyricsMs, FloatingLyricsRules.stripTarget(areaW, FloatingLyricsRules.STRIP_TARGET))
            firstT = k?.times?.get(0) ?: Double.POSITIVE_INFINITY
            lastT = k?.times?.get(k.times.size - 1) ?: Double.NEGATIVE_INFINITY
        }
        val moving = clock.playing && lyricsMs >= firstT && lyricsMs < lastT
        out.strip = true
        out.contMask = true
        out.visible = true
        out.rawOffset = offset.toDouble()
        out.lyricsMs = lyricsMs
        out.offset = quantise(offset, moving)
        val window = FloatingLyricsRules.stripWindowPacked(strip.cells, strip.widths, out.offset, areaW, areaW)
        out.firstCell = FloatingLyricsRules.windowFirst(window)
        out.lastCell = FloatingLyricsRules.windowLast(window)
        out.animating = moving
        if (clock.playing && lyricsMs < firstT && firstT.isFinite()) out.wakeAtLyricsMs = firstT.toLong()
    }

    private fun planLines(look: RenderLook, model: RenderModel, clock: ClockSnapshot?, matches: Boolean, positionMs: Double, out: FrameParams) {
        val areaW = look.layout.areaWidthPx.toFloat()
        val lyricsMs = if (clock != null) positionMs + model.offsetMs else 0.0
        out.lyricsMs = lyricsMs
        val index = if (matches) FloatingLyricsRules.activeIndex(model.starts, Math.round(positionMs), model.offsetMs) else -1
        val text = if (index >= 0) model.texts[index] else ""
        val translation = if (index >= 0) model.translations[index] else ""
        val shownNote = text.isEmpty() && !look.locked
        out.contMask = false
        out.lineIndex = index
        out.mainLine = if (text.isNotEmpty()) index else if (shownNote) FrameParams.NOTE_LINE else FrameParams.NO_LINE
        out.secondLine = if (translation.isNotEmpty()) index else FrameParams.NO_LINE
        out.visible = out.mainLine != FrameParams.NO_LINE || out.secondLine != FrameParams.NO_LINE
        // How far a line wider than the area has slid (the web marquee rule).
        val progress = if (index >= 0 && matches && clock != null) {
            val duration = FloatingLyricsRules.lineDurationMs(model.starts, index, duration(clock, model), model.offsetMs)
            FloatingLyricsRules.scrollProgress(positionMs + model.offsetMs - model.starts[index], duration)
        } else {
            0f
        }
        val playing = clock != null && clock.playing && matches
        var sliding = false
        var mainW = 0f
        if (out.mainLine != FrameParams.NO_LINE) {
            mainW = if (out.mainLine == FrameParams.NOTE_LINE) model.noteWidth else model.textWidths[index]
            if (mainW > areaW) sliding = true
        }
        var secondW = 0f
        if (out.secondLine != FrameParams.NO_LINE) {
            secondW = model.translationWidths[index]
            if (secondW > areaW) sliding = true
        }
        val slideMoving = playing && sliding
        val mainX = FloatingLyricsRules.lineStartX(areaW, mainW, look.align, progress)
        val secondX = FloatingLyricsRules.lineStartX(areaW, secondW, look.align, progress)
        out.mainX = quantise(mainX, slideMoving)
        out.secondX = quantise(secondX, slideMoving)
        var animating = slideMoving
        var wake = FloatingLyricsRules.NO_TIME
        // Word-timed line: karaoke, one smooth accent edge over the line (ADR 0041 D10); not for the placeholder.
        val glide = if (index >= 0 && out.mainLine == index) model.karaoke[index] else null
        if (glide != null && glide.isNotEmpty() && matches) {
            val edge = WordLyricsRules.glideValue(glide, lyricsMs)
            val glideMoving = playing && WordLyricsRules.glideMoving(glide, lyricsMs)
            out.karaokeX = quantise(mainX + edge.toFloat(), glideMoving || slideMoving)
            if (glideMoving) animating = true
            if (playing && lyricsMs < glide[0].t) wake = glide[0].t.toLong()
        }
        if (playing && !animating) {
            val next = FloatingLyricsRules.firstStartAfter(model.starts, Math.round(positionMs) + model.offsetMs)
            if (next < model.starts.size) wake = if (wake == FloatingLyricsRules.NO_TIME) model.starts[next] else minOf(wake, model.starts[next])
        }
        out.animating = animating
        out.wakeAtLyricsMs = wake
    }

    companion object {
        /** A moving position is rounded to 1/8 px (sub-pixel motion); a resting one to a whole pixel (a crisp still line). */
        fun quantise(x: Float, moving: Boolean): Float = if (moving) Math.round(x * 8f) / 8f else Math.round(x).toFloat()
    }
}
