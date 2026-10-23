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
}
