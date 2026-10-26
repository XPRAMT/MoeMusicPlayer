package com.moemusicplayer.mediaindex

import org.json.JSONArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LyricFontsTest {
    @Test
    fun fontFacesParseAndSkipMalformedEntries() {
        val faces = LyricFonts.parse(
            JSONArray(
                """[{"path": "/system/fonts/A.ttf", "faceIndex": 0, "weight": 400},
                    {"path": "", "faceIndex": 1, "weight": 700},
                    {"faceIndex": 2},
                    {"path": "/system/fonts/B.ttc", "faceIndex": 3, "weight": 5000},
                    7]""",
            ),
        )
        assertEquals(
            listOf(LyricFontFace("/system/fonts/A.ttf", 0, 400), LyricFontFace("/system/fonts/B.ttc", 3, 1000)),
            faces,
        )
        assertTrue(LyricFonts.parse(null).isEmpty())
        assertTrue(LyricFonts.parse(JSONArray()).isEmpty())
    }

    @Test
    fun theFaceNearestTheWeightIsTheBase() {
        val light = LyricFontFace("l", 0, 300)
        val regular = LyricFontFace("r", 0, 400)
        val bold = LyricFontFace("b", 0, 700)
        val faces = listOf(bold, light, regular)
        assertEquals(regular, LyricFonts.nearest(faces, 450))
        assertEquals(bold, LyricFonts.nearest(faces, 600))
        assertEquals(light, LyricFonts.nearest(faces, 100))
        // A tie: heavier from 400 up, lighter below.
        assertEquals(bold, LyricFonts.nearest(listOf(regular, bold), 550))
        val thin = LyricFontFace("t", 0, 100)
        assertEquals(thin, LyricFonts.nearest(listOf(light, thin), 200))
        assertNull(LyricFonts.nearest(emptyList(), 500))
    }
}
