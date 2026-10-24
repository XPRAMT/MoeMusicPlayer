package com.moemusicplayer.mediaindex

/**
 * Word-timed lyrics in the floating window (ADR 0041 D10), the same rules as `player-lyrics-rules/src/glide.rs`,
 * `words.rs` and `src/lib/lyrics-glide.js` / `lyrics-word-strip.js` (checked against the shared vectors): the smoothed
 * glide (hard knots exact, soft guides followed within 0.5x-1.5x of each stretch's average speed, monotone, C1) for the
 * karaoke edge, the strip's word glide (a monotone C1 cubic through line start / last-word end knots only), and the
 * static centre mask.
 */
internal object WordLyricsRules {
    const val GLIDE_MIN_SPEED = 0.5
    const val GLIDE_MAX_SPEED = 1.5
    const val GLIDE_KNOT_WINDOW = 0.125
    const val MASK_BAND_EM = 1.0
    const val MASK_EDGE_EM = 1.0

    /** From lyrics time `t` up to the next piece the value is `v + s u + c u^2` (`u` = time - `t`). */
    data class GlidePiece(val t: Double, val v: Double, val s: Double, val c: Double)

    /** One line of the strip: its start, cell left edge, first glyph centre and, with word times, its last word's end and last glyph centre. */
    class KnotLine(val startMs: Double, val cellStart: Double, val anchor: Double, val endMs: Double?, val lastCentre: Double)

    /** `smooth_glide`: hard knots `(t, v)`, guides between knot j and j + 1, eased (speed 0) ends or not. */
    fun smoothGlide(hard: List<Pair<Double, Double>>, guides: List<List<Pair<Double, Double>>>, easeEnds: Boolean): List<GlidePiece> {
        val knots = ArrayList<Pair<Double, Double>>()
        val between = ArrayList<List<Pair<Double, Double>>>()
        var dropped = false
        hard.forEachIndexed { j, (t, v) ->
            if (!t.isFinite() || !v.isFinite()) {
                dropped = true
                return@forEachIndexed
            }
            val prev = knots.lastOrNull()
            if (prev != null) {
                if (t <= prev.first) {
                    dropped = true
                    return@forEachIndexed
                }
                between.add(if (dropped) emptyList() else guides.getOrElse(j - 1) { emptyList() })
                knots.add(t to maxOf(v, prev.second))
            } else {
                knots.add(t to v)
            }
            dropped = false
        }
        val n = knots.size
        if (n == 0) return emptyList()
        if (n == 1) return listOf(GlidePiece(knots[0].first, knots[0].second, 0.0, 0.0))
        fun span(j: Int) = knots[j + 1].first - knots[j].first
        val avg = DoubleArray(n - 1) { (knots[it + 1].second - knots[it].second) / span(it) }
        val tangent = DoubleArray(n)
        val window = DoubleArray(n)
        for (j in 0 until n) {
            if (j == 0 || j == n - 1) {
                val edge = if (j == 0) 0 else n - 2
                tangent[j] = if (easeEnds) 0.0 else avg[edge]
                window[j] = GLIDE_KNOT_WINDOW * span(edge)
                continue
            }
            val aL = avg[j - 1]
            val aR = avg[j]
            val lo = GLIDE_MIN_SPEED * maxOf(aL, aR)
            val hi = GLIDE_MAX_SPEED * minOf(aL, aR)
            val base = if (aL + aR > 0) 2 * aL * aR / (aL + aR) else 0.0
            tangent[j] = if (lo <= hi) base.coerceIn(lo, hi) else hi
            window[j] = GLIDE_KNOT_WINDOW * minOf(span(j - 1), span(j))
        }
        val pt = ArrayList<Double>()
        val pv = ArrayList<Double>()
        val ph = ArrayList<Boolean>()
        fun point(t: Double, v: Double, isHard: Boolean) {
            pt.add(t)
            pv.add(v)
            ph.add(isHard)
        }
        point(knots[0].first, knots[0].second, true)
        for (j in 0 until n - 1) {
            val lo = GLIDE_MIN_SPEED * avg[j]
            val hi = GLIDE_MAX_SPEED * avg[j]
            val at = knots[j].first + window[j]
            val av = knots[j].second + tangent[j] * window[j]
            val bt = knots[j + 1].first - window[j + 1]
            val bv = knots[j + 1].second - tangent[j + 1] * window[j + 1]
            point(at, av, false)
            var prevT = at
            var prevV = av
            for ((t, v) in between[j]) {
                if (!(t > prevT && t < bt && v.isFinite())) continue
                val low = maxOf(prevV + lo * (t - prevT), bv - hi * (bt - t))
                val high = minOf(prevV + hi * (t - prevT), bv - lo * (bt - t))
                val value = if (low <= high) v.coerceIn(low, high) else (low + high) / 2
                prevT = t
                prevV = value
                point(t, value, false)
            }
            point(bt, bv, false)
            point(knots[j + 1].first, knots[j + 1].second, true)
        }
        fun slope(k: Int) = (pv[k + 1] - pv[k]) / (pt[k + 1] - pt[k])
        val pieces = ArrayList<GlidePiece>()
        var cursorT = pt[0]
        var cursorV = pv[0]
        for (k in 1 until pt.size) {
            val t = pt[k]
            val v = pv[k]
            val sIn = slope(k - 1)
            val sOut = if (k + 1 < pt.size) slope(k) else 0.0
            var r = 0.0
            if (!ph[k] && k + 1 < pt.size && kotlin.math.abs(sOut - sIn) > 1e-12) {
                r = minOf(t - pt[k - 1], pt[k + 1] - t) / 2
            }
            if (t - r > cursorT) pieces.add(GlidePiece(cursorT, cursorV, sIn, 0.0))
            if (r > 0) {
                pieces.add(GlidePiece(t - r, v - sIn * r, sIn, (sOut - sIn) / (4 * r)))
                cursorT = t + r
                cursorV = v + sOut * r
            } else {
                cursorT = t
                cursorV = v
            }
        }
        pieces.add(GlidePiece(cursorT, cursorV, 0.0, 0.0))
        return pieces
    }

    private fun pieceIndex(pieces: List<GlidePiece>, time: Double): Int {
        var lo = 0
        var hi = pieces.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (pieces[mid].t <= time) lo = mid + 1 else hi = mid
        }
        return lo - 1
    }

    /** `[value, speed, half acceleration]` at `time` (`glide_at`). */
    fun glideAt(pieces: List<GlidePiece>, time: Double): DoubleArray {
        if (pieces.isEmpty()) return doubleArrayOf(0.0, 0.0, 0.0)
        val i = pieceIndex(pieces, time)
        if (i < 0) return doubleArrayOf(pieces[0].v, 0.0, 0.0)
        val p = pieces[i]
        val u = time - p.t
        return doubleArrayOf(p.v + u * (p.s + p.c * u), p.s + 2 * p.c * u, p.c)
    }

    fun glideValue(pieces: List<GlidePiece>, time: Double): Double = glideAt(pieces, time)[0]

    /** True while the glide still moves at `time` (frames are wanted). */
    fun glideMoving(pieces: List<GlidePiece>, time: Double): Boolean =
        pieces.size > 1 && time >= pieces[0].t && time < pieces[pieces.size - 1].t

    /** The karaoke edge of one line (`karaoke_glide`): hard at the first word's start and the last word's end. */
    fun karaokeGlide(spans: List<Pair<Double, Double>>, words: List<Pair<Double, Double>>): List<GlidePiece> {
        val n = minOf(spans.size, words.size)
        if (n == 0) return emptyList()
        val start = words[0].first
        var end = words[n - 1].second
        if (!(end > start)) end = start + 1
        val guides = ArrayList<Pair<Double, Double>>()
        for (i in 0 until n) {
            if (i > 0) guides.add(words[i].first to spans[i].first)
            if (i < n - 1 && words[i].second < words[i + 1].first) guides.add(words[i].second to spans[i].second)
        }
        return smoothGlide(listOf(start to spans[0].first, end to spans[n - 1].second), listOf(guides), false)
    }

    /** The continuous strip's word glide knots `(lyrics ms, strip px)` (`word_strip_knots`), see [wordStripKnots]. */
    fun wordStripKnots(lines: List<KnotLine>): List<Pair<Double, Double>> {
        val knots = ArrayList<Pair<Double, Double>>(lines.size * 2)
        fun push(t: Double, v: Double) {
            if (!t.isFinite() || !v.isFinite()) return
            while (knots.isNotEmpty() && knots[knots.size - 1].first >= t) knots.removeAt(knots.size - 1)
            knots.add(t to if (knots.isNotEmpty()) maxOf(v, knots[knots.size - 1].second) else v)
        }
        for (line in lines) {
            val start = line.cellStart + maxOf(0.0, line.anchor)
            push(line.startMs, start)
            val end = line.endMs
            if (end != null) {
                val value = line.cellStart + maxOf(0.0, line.lastCentre)
                if (end > line.startMs && value > start) push(end, value)
            }
        }
        return knots
    }

    /**
     * The word glide at lyrics time `time` as `[value, speed, c2, c3]` (`knot_cubic`): the monotone C1 cubic Hermite
     * through [knots] (tangents 0 at the first and last knot, inside the weighted harmonic mean of the neighbouring
     * slopes, Fritsch-Butland); the value at `time + u` is `value + speed u + c2 u^2 + c3 u^3` up to the next knot.
     */
    fun knotCubic(knots: List<Pair<Double, Double>>, time: Double): DoubleArray {
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

    fun knotValue(knots: List<Pair<Double, Double>>, time: Double): Double = knotCubic(knots, time)[0]

    /**
     * The static continuous-mode mask's gradient stops `(offset 0..1, opacity)` (`centre_mask_stops`, experimental):
     * 0 → inactive over the edge em, inactive → 1 up to an opaque em at the centre, mirrored; below four em the em
     * shrinks to a quarter of the area.
     */
    fun centreMaskStops(areaWidth: Double, emPx: Double, inactive: Double): List<Pair<Double, Double>> {
        if (!(areaWidth > 0) || !(emPx > 0)) return emptyList()
        val a = if (inactive.isNaN()) 1.0 else inactive.coerceIn(0.0, 1.0)
        val e = minOf(emPx, areaWidth / 4)
        val edge = MASK_EDGE_EM * e
        val half = MASK_BAND_EM * e / 2
        val c = areaWidth / 2
        return listOf(0.0 to 0.0, edge to a, c - half to 1.0, c + half to 1.0, areaWidth - edge to a, areaWidth to 0.0)
            .map { (x, o) -> x / areaWidth to o }
    }

    fun centreMaskOpacity(x: Double, areaWidth: Double, emPx: Double, inactive: Double): Double {
        val stops = centreMaskStops(areaWidth, emPx, inactive)
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
}
