package com.moemusicplayer.mediaindex

/**
 * How a rasterised line becomes textures (ADR 0032, GL overlay): a line image wider than one texture is cut into tiles of
 * at most `chunk` px. Each tile keeps a one-pixel apron of the neighbouring image (or transparent past the ends), so
 * linear filtering at a sub-pixel position shows no seam between two tiles. Pure; the rasteriser and the renderer both
 * derive their geometry from it.
 */
internal object TilePlan {
    /** Pixels of the neighbouring image kept on each side of a tile. */
    const val APRON = 1

    /** The tile's content: image columns `srcX until srcX + width`; its bitmap is `width + 2 * APRON` wide. */
    class Tile(val srcX: Int, val width: Int)

    /** `imageWidth` split into tiles of at most `chunk` content columns each (at least one tile for a non-empty image). */
    fun of(imageWidth: Int, chunk: Int): List<Tile> {
        if (imageWidth <= 0) return emptyList()
        val size = maxOf(1, chunk)
        val tiles = ArrayList<Tile>((imageWidth + size - 1) / size)
        var x = 0
        while (x < imageWidth) {
            val w = minOf(size, imageWidth - x)
            tiles.add(Tile(x, w))
            x += w
        }
        return tiles
    }

    /** The largest tile content width for a GL texture limit: leaves room for the apron, never above 2048. */
    fun chunkFor(maxTextureSize: Int): Int = (minOf(2048, maxTextureSize) - 2 * APRON).coerceAtLeast(64)

    /** Texture coordinate range of a tile's content: `(u0, u1)` for a bitmap `width + 2 * APRON` wide. */
    fun u0(tileWidth: Int): Float = APRON.toFloat() / (tileWidth + 2 * APRON)

    fun u1(tileWidth: Int): Float = (APRON + tileWidth).toFloat() / (tileWidth + 2 * APRON)
}

/**
 * Least-recently-used accounting of the textures' memory: keys with their sizes, a touch on every use, and the keys to
 * drop when the total passes the cap (never one the caller still wants). Pure bookkeeping, no GL, so it is unit-tested.
 */
internal class LruBytes<K : Any> {
    private val order = LinkedHashMap<K, Long>(16, 0.75f, true)

    var totalBytes = 0L
        private set

    val size: Int get() = order.size

    fun put(key: K, bytes: Long) {
        order.put(key, bytes)?.let { totalBytes -= it }
        totalBytes += bytes
    }

    fun touch(key: K) {
        order[key]
    }

    fun remove(key: K): Boolean {
        val bytes = order.remove(key) ?: return false
        totalBytes -= bytes
        return true
    }

    fun contains(key: K): Boolean = order.containsKey(key)

    fun clear() {
        order.clear()
        totalBytes = 0L
    }

    /** The oldest keys to drop until the total is at most `capBytes`, skipping every key `keep` accepts. They are removed. */
    fun evict(capBytes: Long, keep: (K) -> Boolean): List<K> {
        if (totalBytes <= capBytes) return emptyList()
        val dropped = ArrayList<K>()
        val iterator = order.entries.iterator()
        while (iterator.hasNext() && totalBytes > capBytes) {
            val entry = iterator.next()
            if (keep(entry.key)) continue
            totalBytes -= entry.value
            dropped.add(entry.key)
            iterator.remove()
        }
        return dropped
    }
}
