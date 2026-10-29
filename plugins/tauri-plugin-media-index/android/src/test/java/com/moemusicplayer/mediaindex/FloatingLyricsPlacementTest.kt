package com.moemusicplayer.mediaindex

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The floating panel placement (ADR 0032): the ratio <-> pixel rule and the clamp match the shared vectors that the
 * Rust reference (`player-lyrics-rules` placement) generates and the Windows placement is checked against; the restore
 * path, the orientation choice, the store's change rule and the one-time carry-over of the old SharedPreferences values.
 */
class FloatingLyricsPlacementTest {
    private val vectors: JSONObject by lazy {
        var dir: File? = File("").absoluteFile
        while (dir != null && !File(dir, "tests/fixtures/lyrics-rules-vectors.json").exists()) dir = dir.parentFile
        JSONObject(File(dir!!, "tests/fixtures/lyrics-rules-vectors.json").readText())
    }

    private fun cases(key: String) = vectors.getJSONArray(key).let { a -> (0 until a.length()).map { a.getJSONObject(it) } }
    private fun JSONArray.ints() = IntArray(length()) { getInt(it) }

    @Test
    fun ratioAndPixelsMatchTheSharedVectors() {
        for (c in cases("placementRatio")) {
            val actual = FloatingLyricsRules.positionRatio(c.getInt("px"), c.getInt("origin"), c.getInt("extent"))
            if (c.isNull("ratio")) assertNull(c.toString(), actual)
            else assertEquals(c.toString(), c.getDouble("ratio"), actual!!, 1e-12)
        }
        for (c in cases("placementPx")) {
            assertEquals(c.toString(), c.getInt("px"),
                FloatingLyricsRules.positionPx(c.getDouble("ratio"), c.getInt("origin"), c.getInt("extent")))
        }
    }

    @Test
    fun theClampMatchesTheSharedVectors() {
        for (c in cases("placementClamp")) {
            val (o, size, b, expected) = listOf("origin", "size", "bounds", "expected").map { c.getJSONArray(it).ints() }
            assertEquals(c.toString(), Pair(expected[0], expected[1]),
                FloatingLyricsRules.clampKeptBox(o[0], o[1], size[0], size[1], b[0], b[1], b[2], b[3]))
        }
    }

    @Test
    fun theRestorePathMatchesTheSharedVectors() {
        for (c in cases("placementRestore")) {
            val ratio = c.getJSONArray("ratio")
            val (size, screen, expected) = listOf("size", "screen", "expected").map { c.getJSONArray(it).ints() }
            assertEquals(0, screen[0])
            assertEquals(0, screen[1])
            assertEquals(c.toString(), Pair(expected[0], expected[1]),
                FloatingLyricsRules.restoreAreaOrigin(ratio.getDouble(0), ratio.getDouble(1), screen[2], screen[3], size[0], size[1]))
        }
    }

    @Test
    fun theUserExampleAndSaveThenRestoreIsTheIdentity() {
        // Screen 1080 x 2400, ratio (0.5, 0.1): the lyric area's top-left at (540, 240).
        assertEquals(Pair(540, 240), FloatingLyricsRules.restoreAreaOrigin(0.5, 0.1, 1_080, 2_400, 300, 60))
        val saved = FloatingLyricsRules.ratioOf(540, 240, 1_080, 2_400)!!
        assertEquals(0.5, saved.x, 0.0)
        assertEquals(0.1, saved.y, 0.0)
        for (x in listOf(0, 1, 333, 779)) for (y in listOf(0, 7, 1_999, 2_340)) {
            val r = FloatingLyricsRules.ratioOf(x, y, 1_080, 2_400)!!
            assertEquals(Pair(x, y), FloatingLyricsRules.restoreAreaOrigin(r.x, r.y, 1_080, 2_400, 300, 60))
        }
        assertNull(FloatingLyricsRules.ratioOf(1, 1, 0, 2_400))
    }

    @Test
    fun theWindowClampIsTheKeptBoxClamp() {
        // Window coordinates around a kept box: margin 7 left of the area, room 20 above it, the area 60 tall.
        val margin = 7
        val top = 20
        for ((x, y) in listOf(Pair(-300, -300), Pair(900, 3_000), Pair(100, 200), Pair(5_000, 10))) {
            val (wx, wy) = FloatingLyricsRules.clampWindowToArea(x, y, 1_000, 60, top, margin, 1_080, 2_400)
            val (kx, ky) = FloatingLyricsRules.clampKeptBox(x + margin, y + top, 1_000 - 2 * margin, 60, 0, 0, 1_080, 2_400)
            assertEquals(Pair(kx - margin, ky - top), Pair(wx, wy))
        }
    }

    @Test
    fun eachOrientationHasItsOwnPosition() {
        val portrait = FloatingLyricsRules.Ratio(0.5, 0.1)
        val landscape = FloatingLyricsRules.Ratio(0.2, 0.8)
        val saved = FloatingLyricsRules.SavedPlacement(true, portrait, landscape)
        assertEquals(portrait, saved.position(landscape = false))
        assertEquals(landscape, saved.position(landscape = true))
        val reset = saved.withPosition(landscape = true, ratio = null)
        assertNull(reset.position(landscape = true))
        assertEquals(portrait, reset.position(landscape = false))
        assertEquals("position", FloatingLyricsRules.positionKey(false))
        assertEquals("landscapePosition", FloatingLyricsRules.positionKey(true))
    }

    @Test
    fun theStoreReadsRustsStateAndAppliesChangesLikeRust() {
        val state = FloatingLyricsStore.parseState("""{"locked":true,"position":{"x":0.5,"y":0.1},"landscapePosition":null}""")!!
        assertEquals(FloatingLyricsRules.SavedPlacement(true, FloatingLyricsRules.Ratio(0.5, 0.1), null), state)
        assertNull(FloatingLyricsStore.parseState("not json"))
        // An invalid ratio reads as none (settings.rs drops it too).
        assertNull(FloatingLyricsStore.parseState("""{"position":{"x":9,"y":0}}""")!!.portrait)
        val changed = FloatingLyricsStore.applyChange(
            state,
            JSONObject().put("locked", false).put("landscapePosition", FloatingLyricsStore.ratioJson(FloatingLyricsRules.Ratio(0.25, 0.75))),
        )
        assertEquals(FloatingLyricsRules.SavedPlacement(false, FloatingLyricsRules.Ratio(0.5, 0.1), FloatingLyricsRules.Ratio(0.25, 0.75)), changed)
        val cleared = FloatingLyricsStore.applyChange(changed, JSONObject().put("position", JSONObject.NULL))
        assertNull(cleared.portrait)
        assertEquals(changed.landscape, cleared.landscape)
        assertEquals(changed, FloatingLyricsStore.applyChange(changed, JSONObject()))
    }

    @Test
    fun theOldValuesBecomeRatiosOfTheirOwnOrientation() {
        // Taken while the phone is landscape (2400 x 1080): portrait uses 1080 x 2400, landscape 2400 x 1080.
        val (locked, portrait, landscape) = FloatingLyricsRules.legacyPlacement(true, Pair(540, 240), Pair(1_200, 108), 2_400, 1_080)
        assertEquals(true, locked)
        assertEquals(FloatingLyricsRules.Ratio(0.5, 0.1), portrait)
        assertEquals(FloatingLyricsRules.Ratio(0.5, 0.1), landscape)
        // The same while portrait.
        assertEquals(Triple(true, portrait, landscape),
            FloatingLyricsRules.legacyPlacement(true, Pair(540, 240), Pair(1_200, 108), 1_080, 2_400))
        // Missing values stay missing; the payload carries only what existed.
        val none = FloatingLyricsRules.legacyPlacement(null, null, Pair(0, 0), 1_080, 2_400)
        assertEquals(Triple(null, null, FloatingLyricsRules.Ratio(0.0, 0.0)), none)
        val payload = FloatingLyricsStore.legacyJson(none.first, none.second, none.third)
        assertFalse(payload.has("locked"))
        assertFalse(payload.has("position"))
        assertTrue(payload.has("landscapePosition"))
        val full = FloatingLyricsStore.legacyJson(false, portrait, null)
        assertEquals(false, full.getBoolean("locked"))
        assertEquals(0.5, full.getJSONObject("position").getDouble("x"), 0.0)
        assertFalse(full.has("landscapePosition"))
    }
}
