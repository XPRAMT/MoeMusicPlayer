package com.moemusicplayer.mediaindex

import android.graphics.Typeface
import android.os.Build
import org.json.JSONArray
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/** One upright face file of the chosen floating-lyrics font (ADR 0042), resolved by Rust from its font cache. */
internal data class LyricFontFace(val path: String, val faceIndex: Int, val weight: Int)

/** The chosen font of the floating lyrics: face files from `fontFaces` of a settings update; none is the default font. */
internal object LyricFonts {
    /** `settings.fontFaces` (`[{path, faceIndex, weight}]`); malformed entries are skipped, null or empty: the default font. */
    fun parse(array: JSONArray?): List<LyricFontFace> {
        if (array == null) return emptyList()
        val faces = ArrayList<LyricFontFace>(array.length())
        for (i in 0 until array.length()) {
            val face = array.optJSONObject(i) ?: continue
            val path = face.optString("path", "")
            if (path.isEmpty()) continue
            faces.add(LyricFontFace(path, face.optInt("faceIndex", 0).coerceAtLeast(0), face.optInt("weight", 400).coerceIn(1, 1000)))
        }
        return faces
    }

    /** The face whose weight is closest to `weight` (a tie goes to the heavier face from 400 up, the lighter below). */
    fun nearest(faces: List<LyricFontFace>, weight: Int): LyricFontFace? = faces.minWithOrNull(
        compareBy<LyricFontFace> { kotlin.math.abs(it.weight - weight) }.thenBy { if (weight >= 400) -it.weight else it.weight },
    )

    /** Typefaces loaded from files (null: the file could not be loaded, so the default font draws). */
    private val loaded = ConcurrentHashMap<LyricFontFace, Any>()
    private val failed = Any()

    /** The typeface of `face`'s file, or null when it is missing or unreadable. */
    fun typeface(face: LyricFontFace): Typeface? {
        val cached = loaded.getOrPut(face) {
            val file = File(face.path)
            val built = try {
                when {
                    !file.isFile -> null
                    Build.VERSION.SDK_INT >= Build.VERSION_CODES.O -> Typeface.Builder(file).setTtcIndex(face.faceIndex).build()
                    // Before API 26 a collection opens at its first face only.
                    face.faceIndex == 0 -> Typeface.createFromFile(file)
                    else -> null
                }
            } catch (error: Throwable) {
                null
            }
            built ?: failed
        }
        return cached as? Typeface
    }
}
