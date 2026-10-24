package com.moemusicplayer.mediaindex

/**
 * Word-timed lyrics in the floating window (ADR 0041), the same rules as `player-lyrics-rules/src/words.rs` and
 * `src/lib/lyrics-word-strip.js` (checked against the shared vectors): the word glide's knots with the speed cap, the
 * monotone C1 glide, the static centre mask and the karaoke clip edge.
 */
internal object WordLyricsRules {
    const val WORD_SPEED_FACTOR = 2.0
    const val WORD_MAX_LEAD_MS = 600.0
    const val MASK_BAND_EM = 1.0
    const val MASK_SOFT_EM = 0.1

    class KnotLine(val startMs: Double, val cellStart: Double, val wordStarts: DoubleArray, val wordCentres: DoubleArray)

    private class Knot(var t: Double, var v: Double, val line: Int)

    private fun wordKnots(lines: List<KnotLine>): MutableList<Knot> {
        val out = ArrayList<Knot>()
        lines.forEachIndexed { line, l ->
            val n = minOf(l.wordStarts.size, l.wordCentres.size)
            if (n == 0) {
                out.add(Knot(l.startMs, l.cellStart, line))
            } else {
                for (i in 0 until n) out.add(Knot(l.wordStarts[i], l.cellStart + maxOf(0.0, l.wordCentres[i]), line))
            }
        }
        return out
    }

    /** The knots `(t, v)` of the word glide, the speed cap applied (`word_glide_knots`). */
    fun wordGlideKnots(lines: List<KnotLine>): List<Pair<Double, Double>> {
        val k = wordKnots(lines)
        val caps = (lines.indices).map { line ->
            val mine = k.filter { it.line == line }
            if (mine.size < 2) null else {
                val dt = mine.last().t - mine.first().t
                val dv = mine.last().v - mine.first().v
                if (dt > 0 && dv > 0) WORD_SPEED_FACTOR * dv / dt else null
            }
        }
        for (i in 1 until k.size) if (k[i].v < k[i - 1].v) k[i].v = k[i - 1].v
        val original = DoubleArray(k.size) { k[it].t }
        for (i in k.size - 1 downTo 1) {
            val cap = caps.getOrNull(k[i].line) ?: continue
            val need = (k[i].v - k[i - 1].v) / cap
            if (k[i].t - k[i - 1].t >= need) continue
            var t = maxOf(k[i].t - need, original[i - 1] - WORD_MAX_LEAD_MS)
            if (i >= 2) t = maxOf(t, k[i - 2].t)
            k[i - 1].t = minOf(k[i - 1].t, t)
        }
        val out = ArrayList<Knot>(k.size)
        for (knot in k) {
            while (out.isNotEmpty() && out.last().t >= knot.t) out.removeAt(out.size - 1)
            out.add(knot)
        }
        return out.map { it.t to it.v }
    }

    /** The strip offset at `positionMs` (`target - v` along the monotone C1 curve through the knots). */
    fun glideOffset(knots: List<Pair<Double, Double>>, positionMs: Double, target: Double): Double {
        val n = knots.size
        if (n == 0) return 0.0
        fun time(k: Int) = knots[k].first
        fun value(k: Int) = knots[k].second
        fun tangent(k: Int): Double {
            if (k == 0 || k >= n - 1) return 0.0
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
        var i = -1
        while (i + 1 < n && time(i + 1) <= positionMs) i += 1
        if (i < 0) return target - value(0)
        if (i >= n - 1) return target - value(n - 1)
        val t0 = time(i)
        val h = time(i + 1) - t0
        val v0 = value(i)
        val m0 = tangent(i)
        val m1 = tangent(i + 1)
        val d = (value(i + 1) - v0) / h
        val c2 = (3 * d - 2 * m0 - m1) / h
        val c3 = (m0 + m1 - 2 * d) / (h * h)
        val u = positionMs - t0
        return target - (v0 + u * (m0 + u * (c2 + u * c3)))
    }

    /** The centre mask's gradient stops `(offset 0..1, opacity)` (`centre_mask_stops`): an opaque em, straight ramps. */
    fun centreMaskStops(areaWidth: Double, emPx: Double): List<Pair<Double, Double>> {
        if (!(areaWidth > 0) || !(emPx > 0)) return emptyList()
        val c = areaWidth / 2
        val a = MASK_BAND_EM * emPx / 2
        val s = MASK_SOFT_EM * emPx / 2
        val q = if (c - a > 0) maxOf(0.0, (c - a - s) / (c - a)) else 1.0
        return listOf(0.0 to 0.0, c - a - s to q, c - a + s to 1.0, c + a - s to 1.0, c + a + s to q, areaWidth to 0.0)
            .map { (x, o) -> x.coerceIn(0.0, areaWidth) / areaWidth to o }
    }

    fun centreMaskOpacity(x: Double, areaWidth: Double, emPx: Double): Double {
        val stops = centreMaskStops(areaWidth, emPx)
        if (stops.isEmpty()) return 0.0
        val p = (x / areaWidth).coerceIn(0.0, 1.0)
        var value = stops[0].second
        for (i in 0 until stops.size - 1) {
            val (a, va) = stops[i]
            val (b, vb) = stops[i + 1]
            if (p >= a && p <= b) {
                value = if (b > a) va + (vb - va) * (p - a) / (b - a) else vb
                if (p < b) break
            }
        }
        return value
    }

    /** Karaoke clip edge (the none mode): lit up to the sung position (`karaoke_clip_x`). */
    fun karaokeClipX(spans: List<Pair<Double, Double>>, words: List<Pair<Double, Double>>, t: Double): Double {
        val n = minOf(spans.size, words.size)
        if (n == 0) return 0.0
        var x = spans[0].first
        for (i in 0 until n) {
            val (s, e) = words[i]
            val (l, r) = spans[i]
            if (t < s) break
            if (t >= e || e <= s) {
                x = maxOf(x, r)
                continue
            }
            x = maxOf(x, l + (r - l) * (t - s) / (e - s))
            break
        }
        return x
    }

    /** True while a word is being sung at `t` (the karaoke edge moves, so frames are wanted). */
    fun wiping(words: List<Pair<Double, Double>>, t: Double): Boolean = words.any { (s, e) -> t >= s && t < e && e > s }
}
