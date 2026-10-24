package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Random

/**
 * The allocation-free / binary-search replacements of the per-frame lyrics rules must give exactly the numbers of the
 * original linear, allocating implementations (kept here as the reference), and the background raster cache must
 * discard stale work and bound what it keeps.
 */
class LyricsFramePerformanceTest {
    // --- the original implementations (linear scans, closures) ---

    private fun legacyStripOffset(
        startTimesMs: LongArray, cellStarts: FloatArray, cellWidths: FloatArray, align: String, lyricsMs: Long, viewWidth: Float,
        targetAlign: String, finalArrivalMs: Long?, mainTextWidths: FloatArray, lastGlyphCenters: FloatArray, firstGlyphCenters: FloatArray,
    ): Float {
        val count = minOf(startTimesMs.size, cellStarts.size)
        if (count == 0) return 0f
        val target = when (targetAlign) {
            "right" -> viewWidth.toDouble()
            "center" -> viewWidth / 2.0
            else -> 0.0
        }
        val last = count - 1
        val end = finalArrivalMs?.takeIf { it > startTimesMs[last] }
        val knots = count + if (end != null) 1 else 0
        fun time(k: Int): Double = (if (k < count) startTimesMs[k] else end!!).toDouble()
        fun value(k: Int): Double {
            if (k < count) return FloatingLyricsRules.stripAnchor(cellStarts, cellWidths, k, align, firstGlyphCenters).toDouble()
            val mainWidth = maxOf(0f, mainTextWidths.getOrElse(last) { cellWidths.getOrElse(last) { 0f } })
            val glyphCenter = lastGlyphCenters.getOrElse(last) { mainWidth }.coerceIn(0f, mainWidth)
            return maxOf(
                FloatingLyricsRules.stripAnchor(cellStarts, cellWidths, last, align, firstGlyphCenters).toDouble(),
                cellStarts.getOrElse(last) { 0f }.toDouble() + glyphCenter,
            )
        }
        fun tangent(k: Int): Double {
            if (k <= 0 || k >= knots - 1) return 0.0
            val h0 = time(k) - time(k - 1)
            val h1 = time(k + 1) - time(k)
            if (h0 <= 0 || h1 <= 0) return 0.0
            val d0 = (value(k) - value(k - 1)) / h0
            val d1 = (value(k + 1) - value(k)) / h1
            if (d0 <= 0 || d1 <= 0) return 0.0
            val w0 = 2 * h1 + h0
            val w1 = h1 + 2 * h0
            return (w0 + w1) / (w0 / d0 + w1 / d1)
        }
        val p = lyricsMs.toDouble()
        var index = -1
        while (index + 1 < knots && time(index + 1) <= p) index++
        if (index < 0) return (target - value(0)).toFloat()
        if (index >= knots - 1) return (target - value(knots - 1)).toFloat()
        val t0 = time(index)
        val h = time(index + 1) - t0
        val v0 = value(index)
        val m0 = tangent(index)
        val m1 = tangent(index + 1)
        val d = (value(index + 1) - v0) / h
        val c2 = (3 * d - 2 * m0 - m1) / h
        val c3 = (m0 + m1 - 2 * d) / (h * h)
        val u = p - t0
        return (target - (v0 + u * (m0 + u * (c2 + u * c3)))).toFloat()
    }

    private fun legacyKnotCubic(knots: List<Pair<Double, Double>>, time: Double): DoubleArray {
        val count = knots.size
        if (count == 0) return doubleArrayOf(0.0, 0.0, 0.0, 0.0)
        fun tangent(k: Int): Double {
            if (k == 0 || k >= count - 1) return 0.0
            val h0 = knots[k].first - knots[k - 1].first
            val h1 = knots[k + 1].first - knots[k].first
            if (h0 <= 0 || h1 <= 0) return 0.0
            val d0 = (knots[k].second - knots[k - 1].second) / h0
            val d1 = (knots[k + 1].second - knots[k].second) / h1
            if (d0 <= 0 || d1 <= 0) return 0.0
            val w0 = 2 * h1 + h0
            val w1 = h1 + 2 * h0
            return (w0 + w1) / (w0 / d0 + w1 / d1)
        }
        var index = -1
        while (index + 1 < count && knots[index + 1].first <= time) index++
        if (index < 0) return doubleArrayOf(knots[0].second, 0.0, 0.0, 0.0)
        if (index >= count - 1) return doubleArrayOf(knots[count - 1].second, 0.0, 0.0, 0.0)
        val t0 = knots[index].first
        val v0 = knots[index].second
        val h = knots[index + 1].first - t0
        val m0 = tangent(index)
        val m1 = tangent(index + 1)
        val d = (knots[index + 1].second - v0) / h
        val c2 = (3 * d - 2 * m0 - m1) / h
        val c3 = (m0 + m1 - 2 * d) / (h * h)
        val u = time - t0
        return doubleArrayOf(v0 + u * (m0 + u * (c2 + u * c3)), m0 + u * (2 * c2 + 3 * c3 * u), c2 + 3 * c3 * u, c3)
    }

    private fun randomStrip(random: Random): Array<Any> {
        val n = 1 + random.nextInt(40)
        var t = random.nextInt(3000).toLong()
        val starts = LongArray(n) { t.also { t += random.nextInt(4000).toLong() } } // equal starts happen (step 0)
        val widths = FloatArray(n) { 20f + random.nextInt(600) }
        val gap = 28f
        val cells = FloatArray(n)
        var x = 0f
        for (i in 0 until n) {
            cells[i] = x
            x += widths[i] + gap
        }
        val last = FloatArray(n) { random.nextFloat() * widths[it] }
        val first = FloatArray(n) { random.nextFloat() * 20f }
        return arrayOf(starts, cells, widths, last, first)
    }

    @Test
    fun stripOffsetBinarySearchEqualsTheLinearScan() {
        val random = Random(7)
        repeat(300) {
            val parts = randomStrip(random)
            val s = parts[0] as LongArray
            val c = parts[1] as FloatArray
            val w = parts[2] as FloatArray
            val l = parts[3] as FloatArray
            val f = parts[4] as FloatArray
            val end = if (random.nextBoolean()) s.last() + 1 + random.nextInt(8000) else s.last() - random.nextInt(3)
            val finalArrival: Long? = if (random.nextInt(4) == 0) null else end.toLong()
            val view = 200f + random.nextInt(800)
            for (probe in 0 until 40) {
                val ms = s.first() - 500 + random.nextInt((s.last() - s.first()).toInt() + 9000).toLong()
                val align = if (random.nextBoolean()) "left" else "center"
                val target = if (random.nextBoolean()) "center" else "right"
                val expected = legacyStripOffset(s, c, w, align, ms, view, target, finalArrival, w, l, f)
                val actual = FloatingLyricsRules.stripOffset(s, c, w, align, ms, view, target, finalArrival, w, l, f)
                assertEquals(expected, actual, 0f)
                val knots = FloatingLyricsRules.stripKnots(s, c, w, align, finalArrival ?: FloatingLyricsRules.NO_TIME, w, l, f)
                assertNotNull(knots)
                assertEquals(expected, FloatingLyricsRules.stripKnotsOffset(knots!!, ms, FloatingLyricsRules.stripTarget(view, target)), 0f)
            }
        }
    }

    @Test
    fun stripOffsetWithoutLinesIsZero() {
        assertNull(FloatingLyricsRules.stripKnots(LongArray(0), FloatArray(0), FloatArray(0), "left", FloatingLyricsRules.NO_TIME))
        assertEquals(0f, FloatingLyricsRules.stripOffset(LongArray(0), FloatArray(0), FloatArray(0), "left", 5, 300f), 0f)
    }

    @Test
    fun packedStripWindowEqualsTheRange() {
        val random = Random(11)
        repeat(300) {
            val n = random.nextInt(50)
            val widths = FloatArray(n) { 10f + random.nextInt(500) }
            val cells = FloatArray(n)
            var x = 0f
            for (i in 0 until n) {
                cells[i] = x
                x += widths[i] + 28f
            }
            val offset = -random.nextInt(8000).toFloat() + random.nextInt(500)
            val view = 100f + random.nextInt(900)
            val margin = random.nextInt(1500).toFloat()
            val range = FloatingLyricsRules.stripWindow(cells, widths, offset, view, margin)
            val packed = FloatingLyricsRules.stripWindowPacked(cells, widths, offset, view, margin)
            assertEquals(range.first, FloatingLyricsRules.windowFirst(packed))
            assertEquals(range.last, FloatingLyricsRules.windowLast(packed))
        }
    }

    @Test
    fun wordKnotsBinarySearchEqualsTheLinearScan() {
        val random = Random(3)
        repeat(300) {
            val n = random.nextInt(30)
            var t = random.nextInt(2000).toDouble()
            var v = random.nextInt(100).toDouble()
            val knots = ArrayList<Pair<Double, Double>>()
            repeat(n) {
                t += 1 + random.nextInt(3000)
                v += random.nextInt(400)
                knots.add(t to v)
            }
            val prepared = WordLyricsRules.WordKnots.of(knots)
            repeat(40) {
                val time = random.nextInt(100_000) - 2000 + random.nextDouble()
                val expected = legacyKnotCubic(knots, time)
                val actual = WordLyricsRules.knotCubic(knots, time)
                for (i in 0 until 4) assertEquals(expected[i], actual[i], 0.0)
                assertEquals(expected[0], WordLyricsRules.knotValue(prepared, time), 0.0)
                assertEquals(expected[0], WordLyricsRules.knotValue(knots, time), 0.0)
            }
        }
    }

    @Test
    fun glideValueEqualsGlideAt() {
        val pieces = WordLyricsRules.smoothGlide(
            listOf(0.0 to 0.0, 1000.0 to 100.0, 3000.0 to 150.0, 3500.0 to 400.0),
            listOf(listOf(500.0 to 40.0), emptyList(), listOf(3200.0 to 200.0)),
            false,
        )
        val random = Random(5)
        repeat(500) {
            val time = random.nextDouble() * 5000 - 500
            assertEquals(WordLyricsRules.glideAt(pieces, time)[0], WordLyricsRules.glideValue(pieces, time), 0.0)
        }
        assertEquals(0.0, WordLyricsRules.glideValue(emptyList(), 5.0), 0.0)
    }

    @Test
    fun fractionalClockSmoothingEqualsTheWholeMillisecondOne() {
        val random = Random(9)
        repeat(500) {
            val previous = random.nextInt(100_000).toDouble()
            val previousAt = random.nextInt(100_000).toLong()
            val reported = random.nextInt(100_000).toLong()
            val reportedAt = random.nextInt(100_000).toLong()
            val now = previousAt + random.nextInt(40)
            val playing = random.nextBoolean()
            val has = random.nextBoolean()
            val expected = FloatingLyricsRules.smoothPositionMs(if (has) previous else null, previousAt, reported, reportedAt, now, playing)
            val actual = FloatingLyricsRules.smoothPosition(previous, has, previousAt.toDouble(), reported, reportedAt.toDouble(), now.toDouble(), playing)
            assertEquals(expected, actual, 0.0)
        }
    }

    @Test
    fun finalArrivalFromCountedGraphemes() {
        for (text in listOf("", "a", "最後一句", "Aが🙂")) {
            for (end in listOf(null, 6_200L, 100_000L)) {
                val expected = FloatingLyricsRules.finalLineArrivalMs(5_000L, text, end)
                val actual = FloatingLyricsRules.finalLineArrivalFromGraphemes(5_000L, FloatingLyricsRules.graphemeCount(text), end ?: FloatingLyricsRules.NO_TIME)
                assertEquals(expected, actual)
            }
        }
    }

    @Test
    fun lineDurationWithoutBoxingMatches() {
        val starts = longArrayOf(0, 1000, 5000)
        assertEquals(FloatingLyricsRules.lineDurationMs(starts, 2, 10_500L, 500), FloatingLyricsRules.lineDurationMs(starts, 2, 10_500L as Long?, 500))
        assertEquals(5_000L, FloatingLyricsRules.lineDurationMs(starts, 2, FloatingLyricsRules.NO_TIME, 0))
    }

    // --- the background raster cache ---

    private class Harness {
        val work = ArrayDeque<Runnable>()
        val delivered = ArrayDeque<Runnable>()
        val created = ArrayList<String>()
        val released = ArrayList<String>()
        var ready = 0
        val cache = LineRasterCache<String>(
            create = { text, look -> if (text == "huge") null else { created.add(text); "$text@${look.sizePx}" } },
            release = { released.add(it) },
            submit = { work.addLast(it) },
            deliver = { delivered.addLast(it) },
            onReady = { ready++ },
        )

        fun runWorker() {
            while (work.isNotEmpty()) work.removeFirst().run()
        }

        fun deliverAll() {
            while (delivered.isNotEmpty()) delivered.removeFirst().run()
        }
    }

    private fun look(size: Float = 20f) = LineLook(size, 0xffffffff.toInt(), 4f, 0xbf000000.toInt(), 0f, 0, 3f)

    @Test
    fun anImageIsOnlyAvailableAfterTheWorkerAndTheHandOver() {
        val h = Harness()
        h.cache.setLook(look())
        h.cache.request("a")
        assertNull(h.cache.peek("a"))
        assertFalse(h.cache.isUnavailable("a"))
        h.runWorker()
        assertNull(h.cache.peek("a"))
        h.deliverAll()
        assertEquals("a@20.0", h.cache.peek("a"))
        assertEquals(1, h.ready)
        // asking again for a finished or an in-flight line does no new work
        h.cache.request("a")
        h.cache.request("b")
        h.cache.request("b")
        assertEquals(1, h.work.size)
    }

    @Test
    fun aNewLookDiscardsResultsStillInFlight() {
        val h = Harness()
        h.cache.setLook(look(20f))
        val before = h.cache.generation
        h.cache.request("a")
        h.cache.request("b")
        h.runWorker()
        h.cache.setLook(look(24f))
        assertTrue(h.cache.generation != before)
        h.deliverAll()
        assertNull(h.cache.peek("a"))
        assertNull(h.cache.peek("b"))
        assertEquals(listOf("a@20.0", "b@20.0"), h.released)
        assertEquals(0, h.cache.size)
        // the same look again changes nothing
        val gen = h.cache.generation
        h.cache.setLook(look(24f))
        assertEquals(gen, h.cache.generation)
        h.cache.request("a")
        h.runWorker()
        h.deliverAll()
        assertEquals("a@24.0", h.cache.peek("a"))
    }

    @Test
    fun workQueuedUnderAnOldLookIsSkippedByTheWorker() {
        val h = Harness()
        h.cache.setLook(look(20f))
        h.cache.request("a")
        h.cache.setLook(look(30f))
        h.runWorker()
        assertTrue(h.created.isEmpty())
    }

    @Test
    fun retainDropsTheFarLinesAndCancelsTheirPendingWork() {
        val h = Harness()
        h.cache.setLook(look())
        for (text in listOf("a", "b", "c")) h.cache.request(text)
        h.runWorker()
        h.deliverAll()
        h.cache.request("d")
        h.cache.request("e")
        h.cache.retain(setOf("b", "d"))
        assertEquals(setOf("a@20.0", "c@20.0"), h.released.toSet())
        assertEquals("b@20.0", h.cache.peek("b"))
        assertNull(h.cache.peek("a"))
        h.runWorker()   // "e" was dropped before it ran; "d" is still wanted
        h.deliverAll()
        assertEquals("d@20.0", h.cache.peek("d"))
        assertNull(h.cache.peek("e"))
        assertFalse(h.created.contains("e"))
        assertEquals(2, h.cache.size)
        assertEquals(0, h.cache.pendingCount)
    }

    @Test
    fun aResultForALineDroppedWhileRunningIsReleasedNotKept() {
        val h = Harness()
        h.cache.setLook(look())
        h.cache.request("a")
        h.runWorker()               // built, hand-over still queued
        h.cache.retain(emptySet())  // the line left the window meanwhile
        h.deliverAll()
        assertNull(h.cache.peek("a"))
        assertEquals(listOf("a@20.0"), h.released)
        assertEquals(0, h.cache.size)
    }

    @Test
    fun aLineThatCannotBeAnImageIsRememberedAsUnavailable() {
        val h = Harness()
        h.cache.setLook(look())
        h.cache.request("huge")
        h.runWorker()
        h.deliverAll()
        assertNull(h.cache.peek("huge"))
        assertTrue(h.cache.isUnavailable("huge"))
        assertEquals(0, h.ready)
        h.cache.request("huge")
        assertTrue(h.work.isEmpty())
        // empty text is never asked for
        h.cache.request("")
        assertTrue(h.work.isEmpty())
    }

    @Test
    fun nothingIsRequestedBeforeTheFirstLook() {
        val h = Harness()
        h.cache.request("a")
        assertTrue(h.work.isEmpty())
        assertNull(h.cache.peek("a"))
    }

    @Test
    fun looksThatDifferOnlyInTheFaceAreOtherImages() {
        assertFalse(look() == look().copy(defaultFace = true))
        assertTrue(look() == look())
    }
}
