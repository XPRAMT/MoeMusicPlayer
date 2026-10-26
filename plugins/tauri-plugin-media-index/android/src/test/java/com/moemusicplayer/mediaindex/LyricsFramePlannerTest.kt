package com.moemusicplayer.mediaindex

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** Ten px per UTF-16 unit; glyph centres at the middle of the first / last unit. */
private class FakeMetrics(private val perChar: Float = 10f) : TextMetrics {
    override fun width(text: String) = text.length * perChar
    override fun firstGlyphCenter(text: String) = if (text.isEmpty()) 0f else perChar / 2
    override fun lastGlyphCenter(text: String) = if (text.isEmpty()) 0f else text.length * perChar - perChar / 2
    override fun spans(text: String, ranges: List<Pair<Int, Int>>) = ranges.map { (a, b) -> a * perChar.toDouble() to b * perChar.toDouble() }
}

class LyricsFramePlannerTest {
    private val ms = 1_000_000L
    private val none = ANIMATION_NONE
    private val cont = ANIMATION_CONTINUOUS

    private fun line(
        start: Long, text: String, translation: String = "", times: List<Pair<Double, Double>>? = null, end: Long? = null,
    ): FloatingLyricsOverlay.Line {
        val ranges = times?.let { t ->
            var at = 0
            val per = text.length / t.size
            t.indices.map { i -> val r = at to (if (i == t.size - 1) text.length else at + per); at = r.second; r }
        }
        return FloatingLyricsOverlay.Line(start, text, translation, times, ranges, end)
    }

    private fun layout(reserve: Boolean = false, width: Int = 400) = LyricsLayout(7, 40, width, -20f, 6f, -16f, 5f, reserve)

    private fun inputs(
        lines: List<FloatingLyricsOverlay.Line>,
        animation: Int = none,
        locked: Boolean = true,
        showTranslation: Boolean = true,
        width: Int = 400,
        version: Int = 1,
        trackId: String? = "t",
        offsetMs: Long = 0,
        durationMs: Long = 100_000,
    ): RenderInputs {
        val reserve = showTranslation && lines.any { it.trimmedTranslation.isNotEmpty() }
        return RenderInputs(
            version, trackId, offsetMs, durationMs, lines, animation, "center", "shadow", 0xffffff, 0xffa9c3fe.toInt(),
            20f, 16f, 500, 450, showTranslation, 1f, 0.5f, locked, 3f, layout(reserve, width), 30f,
        )
    }

    private fun state(i: RenderInputs) = RenderBuilder { _, _, _ -> FakeMetrics() }.build(i)

    private fun clock(position: Long = 0, playing: Boolean = true, track: String = "t", active: Boolean = true, duration: Long = 100_000) =
        ClockSnapshot(track, 1, position, 0, playing, 1f, active, duration)

    private fun plan(s: RenderState, c: ClockSnapshot?, pos: Double): FrameParams {
        val out = FrameParams()
        LyricsFramePlanner().plan(s, c, pos, out)
        return out
    }

    // ---- layout

    @Test
    fun theLayoutReservesTheTranslationRowOnlyWhenAsked() {
        val plain = layout(false)
        val reserved = layout(true)
        assertEquals(plain.mainRowH, plain.areaH)
        assertEquals(plain.mainRowH + plain.secondRowH, reserved.areaH)
        assertEquals(FloatingLyricsRules.panelRoomPx(reserved.areaH), reserved.bottomRoom)
        assertEquals(reserved.topRoom + reserved.areaH + reserved.bottomRoom, reserved.surfaceH)
        // The translation row never moves the first row: the room above and the main row's top are the same.
        assertEquals(plain.topRoom, reserved.topRoom)
        assertEquals(plain.mainRowTop, reserved.mainRowTop)
        assertEquals(plain.areaTop, reserved.areaTop)
        assertEquals(reserved.mainRowTop + reserved.mainRowH, reserved.secondRowTop)
        assertTrue(reserved.bottomRoom >= plain.bottomRoom)
        assertEquals(400 + 40 + 14, reserved.surfaceW)
        assertTrue(reserved.surfaceH > plain.surfaceH)
    }

    @Test
    fun theReserveFollowsTheTranslationSwitchAndTheWholeTrack() {
        val lines = listOf(line(0, "one"), line(5000, "two", "zwei"), line(9000, "three"))
        assertTrue(state(inputs(lines)).model.reserveTranslation)
        assertFalse(state(inputs(lines, showTranslation = false)).model.reserveTranslation)
        assertFalse(state(inputs(listOf(line(0, "one")))).model.reserveTranslation)
        // The line without a translation does not shrink the window: the model is one value for the track.
        assertTrue(state(inputs(lines)).look.layout.reserveTranslation)
    }

    // ---- none mode

    @Test
    fun theCurrentLineIsPickedByTheLyricsTimeAndOffset() {
        val lines = listOf(line(1000, "one"), line(5000, "two"), line(9000, "three"))
        val s = state(inputs(lines, offsetMs = 500))
        assertEquals(FrameParams.NO_LINE, plan(s, clock(), 400.0).mainLine) // before the first line, locked
        assertEquals(0, plan(s, clock(), 500.0).mainLine)
        assertEquals(1, plan(s, clock(), 4500.0).mainLine)
        assertEquals(2, plan(s, clock(), 20_000.0).mainLine)
    }

    @Test
    fun anUnlockedOverlayShowsTheNoteWhereNothingIsCurrent() {
        val s = state(inputs(listOf(line(1000, "one")), locked = false))
        val p = plan(s, clock(), 100.0)
        assertEquals(FrameParams.NOTE_LINE, p.mainLine)
        assertTrue(p.background)
        assertTrue(p.visible)
        val locked = plan(state(inputs(listOf(line(1000, "one")), locked = true)), clock(), 100.0)
        assertFalse(locked.visible)
        assertFalse(locked.background)
    }

    @Test
    fun aShortLineIsPlacedByTheAlignmentAndNotAnimated() {
        val s = state(inputs(listOf(line(0, "abcdefghij"), line(8000, "next"))))
        val p = plan(s, clock(), 1000.0)
        assertEquals(0, p.mainLine)
        assertEquals((400f - 100f) / 2f, p.mainX, 0f)
        assertFalse(p.animating)
        // Idle until the next line starts.
        assertEquals(8000L, p.wakeAtLyricsMs)
    }

    @Test
    fun aPausedClockDoesNotAnimateOrWake() {
        val s = state(inputs(listOf(line(0, "x".repeat(60)), line(8000, "next"))))
        val p = plan(s, clock(playing = false), 3000.0)
        assertFalse(p.animating)
        assertEquals(FloatingLyricsRules.NO_TIME, p.wakeAtLyricsMs)
    }

    @Test
    fun aLongLineSlidesWithSubPixelSteps() {
        val s = state(inputs(listOf(line(0, "x".repeat(60)), line(10_000, "end"))))
        val c = clock()
        val a = plan(s, c, 3000.0)
        val b = plan(s, c, 3008.3)
        assertTrue(a.animating)
        assertTrue(a.mainX < 0f)
        assertTrue("moves", b.mainX < a.mainX)
        // 600 px wide in a 400 px area; progress = elapsed / (10000 * 0.9)
        val expected = -(600f - 400f) * (3000f / 9000f)
        assertEquals(expected, a.mainX, 0.07f)
        // 1/8 px grid
        assertEquals(0f, (a.mainX * 8f) - Math.round(a.mainX * 8f), 1e-4f)
    }

    @Test
    fun theTranslationRowFollowsTheCurrentLineOnly() {
        val s = state(inputs(listOf(line(0, "one", "eins"), line(5000, "two"))))
        assertEquals(0, plan(s, clock(), 100.0).secondLine)
        assertEquals(FrameParams.NO_LINE, plan(s, clock(), 6000.0).secondLine)
        assertTrue(s.look.layout.reserveTranslation)
    }

    @Test
    fun aTrackThatIsNotTheModelsOrInactiveShowsNothing() {
        val s = state(inputs(listOf(line(0, "one"))))
        assertFalse(plan(s, clock(track = "other"), 100.0).visible)
        assertFalse(plan(s, clock(active = false), 100.0).visible)
        assertFalse(plan(s, null, Double.NaN).visible)
    }

    @Test
    fun karaokeEdgeFollowsTheWordGlide() {
        val words = listOf(1000.0 to 2000.0, 2000.0 to 3000.0)
        val s = state(inputs(listOf(line(0, "abcdefghij", times = words), line(8000, "next"))))
        val before = plan(s, clock(), 500.0)
        val mid = plan(s, clock(), 2000.0)
        val after = plan(s, clock(), 3500.0)
        val x0 = (400f - 100f) / 2f
        assertEquals(x0, before.karaokeX, 0.01f)
        assertEquals(x0 + 50f, mid.karaokeX, 1.0f)
        assertEquals(x0 + 100f, after.karaokeX, 0.01f)
        assertTrue(mid.animating)
        assertFalse(after.animating)
    }

    @Test
    fun aKaraokeLineThatHasNotStartedWakesAtItsFirstWord() {
        val words = listOf(1000.0 to 2000.0, 2000.0 to 3000.0)
        val s = state(inputs(listOf(line(0, "abcdefghij", times = words), line(8000, "next"))))
        val p = plan(s, clock(), 10.0)
        assertEquals(1000L, p.wakeAtLyricsMs)
    }

    @Test
    fun aLineWithItsOwnEndAndNoWordTimesIsSweptAsOneWord() {
        // ADR 0041 D10: start 1000, own end 3000, no words: one word over the whole line; without an end: no sweep.
        val s = state(inputs(listOf(line(1000, "abcdefghij", end = 3000), line(8000, "next"))))
        assertTrue(s.model.karaoke[0] != null)
        assertTrue(s.model.karaoke[1] == null)
        val x0 = (400f - 100f) / 2f
        assertEquals(x0, plan(s, clock(), 1000.0).karaokeX, 0.01f)
        assertEquals(x0 + 50f, plan(s, clock(), 2000.0).karaokeX, 1.0f)
        assertEquals(x0 + 100f, plan(s, clock(), 3500.0).karaokeX, 0.01f)
        val plain = state(inputs(listOf(line(1000, "abcdefghij"), line(8000, "next"))))
        assertTrue(plain.model.karaoke[0] == null)
        // Word-timed lines keep their own words, whatever their end.
        val words = listOf(1000.0 to 2000.0, 2000.0 to 3000.0)
        val timed = state(inputs(listOf(line(0, "abcdefghij", times = words, end = 9000), line(8000, "next"))))
        assertEquals(x0 + 100f, plan(timed, clock(), 3500.0).karaokeX, 0.01f)
    }

    @Test
    fun plansFollowTheSharedActiveLineVectors() {
        val vectors = vectors()
        val cases = vectors.getJSONArray("active")
        for (k in 0 until cases.length()) {
            val c = cases.getJSONObject(k)
            val starts = c.getJSONArray("starts")
            val lines = (0 until starts.length()).map { line(starts.getDouble(it).toLong(), "line $it") }
            val expected = c.getInt("expected")
            val s = state(inputs(lines, offsetMs = c.getLong("offsetMs"), locked = true))
            val p = plan(s, clock(c.getLong("positionMs")), c.getLong("positionMs").toDouble())
            if (expected < 0) assertEquals(c.toString(), FrameParams.NO_LINE, p.mainLine) else assertEquals(c.toString(), expected, p.mainLine)
        }
    }

    // ---- continuous

    private fun stripState(lines: List<FloatingLyricsOverlay.Line>, width: Int = 400) = state(inputs(lines, animation = cont, width = width))

    @Test
    fun theLineGlideIsTheRulesStripOffsetAtTheFractionalClock() {
        val lines = listOf(line(1000, "aaaa bbbb"), line(5000, "cccc dddd eeee"), line(9000, "ffff"))
        val s = stripState(lines)
        val strip = s.model.strip!!
        for (position in listOf(0.0, 999.5, 1000.0, 2500.25, 4999.9, 7000.0, 9000.0, 9100.5, 11_000.0, 20_000.0)) {
            val p = plan(s, clock(), position)
            val arrival = FloatingLyricsRules.finalLineArrivalFromEstimate(9000, strip.lastEstimateMs, 100_000)
            val knots = FloatingLyricsRules.stripKnots(
                strip.starts, strip.cells, strip.widths, FloatingLyricsRules.STRIP_ALIGN, arrival,
                strip.mainWidths, strip.lastGlyphCenters, strip.firstGlyphCenters,
            )!!
            val expected = FloatingLyricsRules.stripKnotsOffset(knots, position, 200.0)
            assertEquals("t=$position", expected.toDouble(), p.rawOffset, 1e-4)
            assertEquals("t=$position", expected, p.offset, 0.51f)
            assertTrue(p.strip && p.contMask && p.visible)
        }
    }

    @Test
    fun aFirstLineStartsOnTheCentreExactlyAtItsStart() {
        val lines = listOf(line(1000, "abcdefgh"), line(5000, "ijklmnop"))
        val s = stripState(lines)
        val p = plan(s, clock(), 1000.0)
        // The first glyph's centre (5 px) sits on the area's centre (200 px).
        assertEquals(195f, p.offset, 0.001f)
    }

    @Test
    fun theStripRestsBeforeTheFirstLineAndWakesAtItsStart() {
        val s = stripState(listOf(line(1000, "abcdefgh"), line(5000, "ijklmnop")))
        val p = plan(s, clock(), 200.0)
        assertFalse(p.animating)
        assertEquals(1000L, p.wakeAtLyricsMs)
        assertEquals(195f, p.offset, 0.001f)
        assertTrue(plan(s, clock(), 2000.0).animating)
        assertFalse(plan(s, clock(playing = false), 2000.0).animating)
    }

    @Test
    fun theStripWindowIsTheRulesWindowAtTheQuantisedOffset() {
        val lines = (0 until 40).map { line(it * 3000L, "line number $it") }
        val s = stripState(lines, width = 300)
        val strip = s.model.strip!!
        val p = plan(s, clock(), 61_000.0)
        val window = FloatingLyricsRules.stripWindowPacked(strip.cells, strip.widths, p.offset, 300f, 300f)
        assertEquals(FloatingLyricsRules.windowFirst(window), p.firstCell)
        assertEquals(FloatingLyricsRules.windowLast(window), p.lastCell)
        assertTrue("never the whole song", p.lastCell - p.firstCell < 10)
    }

    @Test
    fun theWordGlideMatchesTheSharedStripVectors() {
        val vectors = vectors().getJSONArray("stripGlide")
        for (k in 0 until vectors.length()) {
            val c = vectors.getJSONObject(k)
            val linesJson = c.getJSONArray("lines")
            val knotLines = (0 until linesJson.length()).map { linesJson.getJSONObject(it) }.map {
                WordLyricsRules.KnotLine(
                    it.getDouble("startMs"), it.getDouble("cellStart"), it.getDouble("anchor"),
                    if (it.isNull("endMs")) null else it.getDouble("endMs"), it.getDouble("lastCentre"),
                    if (it.isNull("capMs")) null else it.getDouble("capMs"),
                )
            }
            val n = knotLines.size
            // The planner adds the terminal knot itself from the track length: a huge estimate makes the arrival
            // the track end, set to the vector's `finalArrivalMs` (none: the last start, which adds no knot).
            val arrival = if (c.isNull("finalArrivalMs")) knotLines[n - 1].startMs.toLong() else c.getLong("finalArrivalMs")
            val strip = StripModel(
                LongArray(n) { knotLines[it].startMs.toLong() }, Array(n) { "x" }, Array(n) { "" },
                FloatArray(n) { knotLines[it].cellStart.toFloat() }, FloatArray(n) { 10f }, FloatArray(n) { 10f }, FloatArray(n),
                FloatArray(n), FloatArray(n), knotLines, 500_000_000L,
            )
            val base = state(inputs(listOf(line(0, "x")), animation = cont))
            val model = RenderModel(
                "t", 0, 100_000, strip.starts, strip.texts, strip.translations, FloatArray(n), FloatArray(n),
                Array<List<WordLyricsRules.GlidePiece>?>(n) { null }, strip, 10f, false, 1,
            )
            val s = RenderState(1, base.look, model)
            val samples = c.getJSONArray("samples")
            for (i in 0 until samples.length()) {
                val sample = samples.getJSONObject(i)
                val t = sample.getDouble("t")
                val p = plan(s, clock(duration = arrival), t)
                val value = sample.getJSONArray("expected").getDouble(0)
                assertEquals(sample.toString(), 200.0 - value, p.rawOffset, 1e-3)
            }
        }
    }

    @Test
    fun aLastLineWithoutWordTimesGlidesToItsLastGlyphInAWordTimedSong() {
        // Each line on its own (ADR 0041 D10): the second line is word-timed, the last (啦啦啦啦啦啦啦啦) is not, so it
        // glides from its first glyph to its last over 500 ms per grapheme (9000 -> 13000) instead of resting.
        val s = stripState(listOf(
            line(1000, "aaaa"),
            line(3000, "bbbbbb", times = listOf(3000.0 to 3500.0, 3500.0 to 4200.0)),
            line(9000, "啦啦啦啦啦啦啦啦"),
        ))
        val strip = s.model.strip!!
        val last = strip.starts.size - 1
        val atStart = plan(s, clock(), 9000.0)
        val mid = plan(s, clock(), 11000.0)
        val end = plan(s, clock(), 13000.0)
        val after = plan(s, clock(), 15000.0)
        assertEquals(200.0 - strip.cells[last] - strip.firstGlyphCenters[last], atStart.rawOffset, 1e-3)
        assertTrue("moving through the last line", mid.rawOffset < atStart.rawOffset - 1.0 && mid.rawOffset > end.rawOffset + 1.0)
        assertTrue(mid.animating)
        assertEquals(200.0 - strip.cells[last] - strip.lastGlyphCenters[last], end.rawOffset, 1e-3)
        assertEquals(end.rawOffset, after.rawOffset, 0.0)
    }

    @Test
    fun aLineWithoutWordTimesEndsAtItsCapBeforeALongInterludeInALineTimedSong() {
        // ADR 0041 D10: "abcd" (4 narrow graphemes, cap 2000 ms) is followed 19 s later by the next line, so its last
        // glyph reaches the centre at 3000 ms and the interlude takes the rest; the last line keeps its estimate.
        val s = stripState(listOf(line(1000, "abcd"), line(20_000, "efgh")))
        val strip = s.model.strip!!
        assertEquals(2000.0, strip.wordLines!![0].capMs!!, 0.0)
        assertEquals(200.0 - strip.cells[0] - strip.firstGlyphCenters[0], plan(s, clock(), 1000.0).rawOffset, 1e-3)
        assertEquals(200.0 - strip.cells[0] - strip.lastGlyphCenters[0], plan(s, clock(), 3000.0).rawOffset, 1e-3)
        val interlude = plan(s, clock(), 11_500.0).rawOffset
        assertTrue(interlude < 200.0 - strip.cells[0] - strip.lastGlyphCenters[0] - 1.0)
        assertTrue(interlude > 200.0 - strip.cells[1] - strip.firstGlyphCenters[1] + 1.0)
        assertEquals(200.0 - strip.cells[1] - strip.firstGlyphCenters[1], plan(s, clock(), 20_000.0).rawOffset, 1e-3)
    }

    @Test
    fun aLineWithItsOwnEndEndsThereInsteadOfAtItsCap() {
        // ADR 0041 D10, per line: "abcd" has its own end 6000 (not the 2000 ms cap), "efgh" has neither (its cap,
        // 22000), and the last line "ijkl" ends at its own end 33000 (not the 1000 ms estimate).
        val s = stripState(listOf(line(1000, "abcd", end = 6000), line(20_000, "efgh"), line(30_000, "ijkl", end = 33_000)))
        val strip = s.model.strip!!
        assertEquals(6000.0, strip.wordLines!![0].endMs!!, 0.0)
        assertEquals(null, strip.wordLines!![0].capMs)
        assertEquals(null, strip.wordLines!![1].endMs)
        assertEquals(2000.0, strip.wordLines!![1].capMs!!, 0.0)
        assertEquals(3000L, strip.lastEstimateMs)
        assertEquals(200.0 - strip.cells[0] - strip.lastGlyphCenters[0], plan(s, clock(), 6000.0).rawOffset, 1e-3)
        assertTrue(plan(s, clock(), 3000.0).rawOffset > 200.0 - strip.cells[0] - strip.lastGlyphCenters[0] + 1.0)
        assertEquals(200.0 - strip.cells[1] - strip.lastGlyphCenters[1], plan(s, clock(), 22_000.0).rawOffset, 1e-3)
        assertEquals(200.0 - strip.cells[2] - strip.lastGlyphCenters[2], plan(s, clock(duration = 60_000), 33_000.0).rawOffset, 1e-3)
    }

    @Test
    fun aSeekOrAPauseJustEvaluatesTheCurveAtTheNewPosition() {
        val s = stripState(listOf(line(1000, "aaaa bbbb"), line(5000, "cccc dddd"), line(9000, "ffff")))
        val here = plan(s, clock(), 6000.0)
        val back = plan(s, clock(), 1200.0)
        val again = plan(s, clock(), 6000.0)
        assertNotEquals(here.rawOffset, back.rawOffset, 1.0)
        assertEquals(here.rawOffset, again.rawOffset, 0.0)
        val paused = plan(s, clock(playing = false), 6000.0)
        assertEquals(here.rawOffset, paused.rawOffset, 0.0)
        assertFalse(paused.animating)
        // A resting (paused) position is on a whole pixel, a moving one on the 1/8 grid.
        assertEquals(Math.round(paused.rawOffset).toFloat(), paused.offset, 0f)
    }

    @Test
    fun aStaleTrackFallsBackToTheSingleLinePlan() {
        val s = stripState(listOf(line(1000, "abc")))
        val p = plan(s, clock(track = "other"), 1500.0)
        assertFalse(p.strip)
        assertFalse(p.contMask)
    }

    @Test
    fun theKnotCacheFollowsTheTrackLength() {
        val s = stripState(listOf(line(1000, "aaaa"), line(5000, "bbbb bbbb bbbb bbbb bbbb")))
        val planner = LyricsFramePlanner()
        val a = FrameParams()
        val b = FrameParams()
        planner.plan(s, clock(duration = 100_000), 5500.0, a)
        planner.plan(s, clock(duration = 5600), 5500.0, b)
        // The track ends right after the last line starts: the terminal glide is cut short, so the curve differs.
        assertNotEquals(a.rawOffset, b.rawOffset, 0.001)
    }

    // ---- quantisation and change detection

    @Test
    fun quantisationIsAnEighthWhileMovingAndAWholePixelAtRest() {
        assertEquals(10.125f, LyricsFramePlanner.quantise(10.1f, true), 0f)
        assertEquals(10f, LyricsFramePlanner.quantise(10.1f, false), 0f)
        assertEquals(-3.5f, LyricsFramePlanner.quantise(-3.52f, true), 0f)
        assertEquals(-4f, LyricsFramePlanner.quantise(-3.52f, false), 0f)
    }

    @Test
    fun anUnchangedPlanIsTheSameFrame() {
        val s = stripState(listOf(line(0, "aaaa"), line(100_000, "bbbb")))
        val a = plan(s, clock(), 50_000.0)
        val b = plan(s, clock(), 50_000.01)
        assertTrue(a.sameAs(b))
        val c = plan(s, clock(), 60_000.0)
        assertFalse(a.sameAs(c))
        val copy = FrameParams().also { it.set(c) }
        assertTrue(copy.sameAs(c))
    }

    // ---- builder

    @Test
    fun theBuilderReusesTheModelUntilTheLinesOrTheMeasuresChange() {
        val builder = RenderBuilder { _, _, _ -> FakeMetrics() }
        val lines = listOf(line(0, "one"))
        val a = builder.build(inputs(lines, version = 1))
        val b = builder.build(inputs(lines, version = 1))
        assertSame(a.model, b.model)
        val c = builder.build(inputs(lines, version = 2))
        assertNotEquals(a.model, c.model)
        assertTrue(c.version > a.version)
    }

    @Test
    fun theRasterGenerationRisesOnlyWhenALookChanges() {
        val builder = RenderBuilder { _, _, _ -> FakeMetrics() }
        val lines = listOf(line(0, "one"))
        val a = builder.build(inputs(lines))
        val b = builder.build(inputs(lines, locked = false))
        assertEquals(a.look.generation, b.look.generation)
        val i = inputs(lines)
        val c = builder.build(RenderInputs(i.linesVersion, i.trackId, i.offsetMs, i.durationMs, i.lines, i.animation, i.align, i.textEffect,
            0x112233, i.accentArgb, i.mainPx, i.secondPx, i.mainWeight, i.secondWeight, i.showTranslation, i.mainOpacity,
            i.secondaryOpacity, i.locked, i.density, i.layout, i.backgroundRadiusPx))
        assertTrue(c.look.generation > b.look.generation)
    }

    @Test
    fun theMasksAreTheRulesStops() {
        val s = state(inputs(listOf(line(0, "one")), animation = cont))
        val look = s.look
        val stops = WordLyricsRules.centreMaskStops(400.0, 20.0, 1.0, 0.5)
        for (k in 0 until LyricsMasks.STOPS) {
            assertEquals(stops[k].first * 400.0, look.contMaskX[k].toDouble(), 1e-3)
            assertEquals(stops[k].second, look.contMaskV[k].toDouble(), 1e-6)
        }
        val n = state(inputs(listOf(line(0, "one")))).look
        val zone = FloatingLyricsRules.edgeFadeWidth(400f, n.layout.areaH.toFloat())
        assertEquals(zone, n.noneMaskX[1], 0f)
        assertEquals(400f - zone, n.noneMaskX[3], 0f)
        assertEquals(1f, n.noneMaskV[1], 0f)
        assertEquals(0f, n.noneMaskV[0], 0f)
    }

    private fun vectors(): JSONObject {
        var dir: File? = File("").absoluteFile
        while (dir != null && !File(dir, "tests/fixtures/lyrics-rules-vectors.json").exists()) dir = dir.parentFile
        return JSONObject(File(dir!!, "tests/fixtures/lyrics-rules-vectors.json").readText())
    }
}
