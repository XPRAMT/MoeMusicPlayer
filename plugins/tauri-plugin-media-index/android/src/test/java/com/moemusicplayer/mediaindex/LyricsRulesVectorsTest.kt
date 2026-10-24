package com.moemusicplayer.mediaindex

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.File

/**
 * The Android rules match the shared vectors in tests/fixtures/lyrics-rules-vectors.json (generated from the JS
 * rules; the Rust crate player-lyrics-rules checks the same file, ADR 0034).
 */
class LyricsRulesVectorsTest {
    private val vectors: JSONObject by lazy {
        var dir: File? = File("").absoluteFile
        while (dir != null && !File(dir, "tests/fixtures/lyrics-rules-vectors.json").exists()) dir = dir.parentFile
        JSONObject(File(dir!!, "tests/fixtures/lyrics-rules-vectors.json").readText())
    }

    private fun JSONArray.longs() = LongArray(length()) { getDouble(it).toLong() }
    private fun JSONArray.floats() = FloatArray(length()) { getDouble(it).toFloat() }
    private fun JSONObject.cases(key: String) = vectors.getJSONArray(key).let { a -> (0 until a.length()).map { a.getJSONObject(it) } }

    @Test
    fun activeLine() {
        for (c in vectors.cases("active")) {
            assertEquals(c.toString(), c.getInt("expected"),
                FloatingLyricsRules.activeIndex(c.getJSONArray("starts").longs(), c.getLong("positionMs"), c.getLong("offsetMs")))
        }
    }

    @Test
    fun finalArrival() {
        for (c in vectors.cases("finalArrival")) {
            val end = if (c.isNull("trackEndMs")) null else c.getLong("trackEndMs")
            assertEquals(c.toString(), c.getLong("expected"),
                FloatingLyricsRules.finalLineArrivalMs(c.getLong("lineStartMs"), c.getString("text"), end))
        }
    }

    @Test
    fun stripOffsetAndWindow() {
        for (c in vectors.cases("strip")) {
            val main = c.getJSONArray("main").floats()
            val (cells, widths) = FloatingLyricsRules.stripLayout(main, c.getJSONArray("translation").floats(), c.getDouble("gap").toFloat())
            val fin = c.optJSONObject("finalLine")
            val last = main.size - 1
            val centers = FloatArray(main.size) { main[it] }.also { if (fin != null) it[last] = fin.getDouble("lastGlyphCenter").toFloat() }
            val actual = FloatingLyricsRules.stripOffset(
                c.getJSONArray("starts").longs(), cells, widths, c.getString("align"), c.getLong("positionMs"),
                c.getDouble("viewWidth").toFloat(), c.getString("target"), fin?.getLong("endMs"), main, centers,
                c.optJSONArray("anchors")?.floats() ?: FloatArray(0),
            )
            assertEquals(c.toString(), c.getDouble("expected").toFloat(), actual, 0.01f)
        }
        for (c in vectors.cases("windows")) {
            if (c.getDouble("fromOffset") != c.getDouble("toOffset")) continue // Kotlin takes one offset per frame
            val (cells, widths) = FloatingLyricsRules.stripLayout(
                c.getJSONArray("main").floats(), c.getJSONArray("translation").floats(), c.getDouble("gap").toFloat())
            val range = FloatingLyricsRules.stripWindow(cells, widths, c.getDouble("fromOffset").toFloat(),
                c.getDouble("viewWidth").toFloat(), c.getDouble("margin").toFloat())
            if (c.getInt("last") < c.getInt("first")) assertEquals(c.toString(), true, range.isEmpty())
            else assertEquals(c.toString(), c.getInt("first")..c.getInt("last"), range)
        }
    }

    @Test
    fun weightsAndOpacity() {
        for (c in vectors.cases("weights")) {
            if (!c.getBoolean("android")) continue
            val starts = c.getJSONArray("starts").longs()
            val expected = c.getJSONArray("expected")
            for (i in starts.indices) {
                assertEquals(c.toString(), expected.getDouble(i).toFloat(),
                    FloatingLyricsRules.cellWeight(starts, i, c.getLong("positionMs")), 0.0001f)
            }
        }
        for (c in vectors.cases("opacity")) {
            assertEquals(c.toString(), c.getDouble("expected").toFloat(),
                FloatingLyricsRules.inactiveFactor(c.getString("animation"), c.getInt("inactive"), c.getDouble("weight").toFloat()), 0.0001f)
        }
    }

    @Test
    fun edgeFade() {
        for (c in vectors.cases("edgeFade")) {
            val w = c.getDouble("areaWidth").toFloat()
            val h = c.getDouble("areaHeight").toFloat()
            assertEquals(c.toString(), c.getDouble("width").toFloat(), FloatingLyricsRules.edgeFadeWidth(w, h), 0.0001f)
            assertEquals(c.toString(), c.getDouble("expected").toFloat(),
                FloatingLyricsRules.edgeFadeOpacity(c.getDouble("x").toFloat(), w, h), 0.0001f)
        }
    }

    private fun JSONArray.doubles() = DoubleArray(length()) { getDouble(it) }
    private fun JSONArray.pairs() = (0 until length()).map { getJSONArray(it).let { p -> p.getDouble(0) to p.getDouble(1) } }

    @Test
    fun glideMaskAndKaraoke() {
        fun samePieces(actual: List<WordLyricsRules.GlidePiece>, expected: JSONArray, what: String) {
            assertEquals(what, expected.length(), actual.size)
            actual.forEachIndexed { i, p ->
                val e = expected.getJSONArray(i)
                for ((k, x) in listOf(p.t, p.v, p.s, p.c).withIndex()) assertEquals("$what piece $i", e.getDouble(k), x, 1e-9)
            }
        }
        for (c in vectors.cases("glide")) {
            val guidesJson = c.getJSONArray("guides")
            val guides = (0 until guidesJson.length()).map { guidesJson.getJSONArray(it).pairs() }
            val pieces = WordLyricsRules.smoothGlide(c.getJSONArray("hard").pairs(), guides, c.getBoolean("easeEnds"))
            samePieces(pieces, c.getJSONArray("pieces"), c.getJSONArray("hard").toString())
            val samples = c.getJSONArray("samples")
            for (i in 0 until samples.length()) {
                val s = samples.getJSONObject(i)
                val at = WordLyricsRules.glideAt(pieces, s.getDouble("t"))
                for (k in 0 until 3) assertEquals(s.toString(), s.getJSONArray("expected").getDouble(k), at[k], 1e-9)
            }
        }
        for (c in vectors.cases("karaokeGlide")) {
            val pieces = WordLyricsRules.karaokeGlide(c.getJSONArray("spans").pairs(), c.getJSONArray("words").pairs())
            samePieces(pieces, c.getJSONArray("pieces"), "karaoke")
        }
        val g = vectors.getJSONObject("stripGlide")
        val linesJson = g.getJSONArray("lines")
        val lines = (0 until linesJson.length()).map { linesJson.getJSONObject(it) }.map {
            WordLyricsRules.KnotLine(it.getDouble("startMs"), it.getDouble("cellStart"), it.getDouble("anchor"),
                it.getJSONArray("wordStarts").doubles(), it.getJSONArray("wordCentres").doubles())
        }
        samePieces(WordLyricsRules.wordStripGlide(lines), g.getJSONArray("pieces"), "strip glide")
        for (c in vectors.cases("centreMask")) {
            assertEquals(c.toString(), c.getDouble("expected"),
                WordLyricsRules.centreMaskOpacity(c.getDouble("x"), c.getDouble("areaWidth"), c.getDouble("emPx")), 1e-9)
        }
    }

    @Test
    fun theKaraokeEdgeIsMonotoneAndHitsBothEnds() {
        val spans = listOf(0.0 to 20.0, 20.0 to 30.0, 34.0 to 60.0)
        val words = listOf(1000.0 to 1200.0, 1200.0 to 1200.0, 1300.0 to 1500.0)
        val glide = WordLyricsRules.karaokeGlide(spans, words)
        assertEquals(0.0, WordLyricsRules.glideValue(glide, 1000.0), 1e-9)
        assertEquals(60.0, WordLyricsRules.glideValue(glide, 1500.0), 1e-9)
        var last = Double.NEGATIVE_INFINITY
        var t = 900.0
        while (t <= 1600.0) {
            val x = WordLyricsRules.glideValue(glide, t)
            org.junit.Assert.assertTrue("$t", x >= last - 1e-12)
            last = x
            t += 1000.0 / 240
        }
    }
}
