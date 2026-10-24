package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LyricsTexturePlanTest {
    @Test
    fun aNarrowImageIsOneTile() {
        val tiles = TilePlan.of(300, 2046)
        assertEquals(1, tiles.size)
        assertEquals(0, tiles[0].srcX)
        assertEquals(300, tiles[0].width)
    }

    @Test
    fun aWideImageIsCutIntoContiguousTilesThatCoverItExactly() {
        for (width in listOf(2046, 2047, 4093, 5000, 16_000, 20_001)) {
            val chunk = 2046
            val tiles = TilePlan.of(width, chunk)
            assertEquals((width + chunk - 1) / chunk, tiles.size)
            var x = 0
            for (tile in tiles) {
                assertEquals(x, tile.srcX)
                assertTrue(tile.width in 1..chunk)
                x += tile.width
            }
            assertEquals(width, x)
        }
    }

    @Test
    fun noImageNoTiles() {
        assertTrue(TilePlan.of(0, 2046).isEmpty())
    }

    @Test
    fun theChunkLeavesRoomForTheApronAndTheTextureLimit() {
        assertEquals(2046, TilePlan.chunkFor(16_384))
        assertEquals(2046, TilePlan.chunkFor(2048))
        assertEquals(1022, TilePlan.chunkFor(1024))
        assertEquals(64, TilePlan.chunkFor(16))
    }

    @Test
    fun textureCoordinatesSkipTheApron() {
        assertEquals(1f / 102f, TilePlan.u0(100), 1e-7f)
        assertEquals(101f / 102f, TilePlan.u1(100), 1e-7f)
        assertTrue(TilePlan.u0(500) < TilePlan.u1(500))
    }

    @Test
    fun theLruDropsTheOldestFirstAndKeepsWhatIsWanted() {
        val lru = LruBytes<String>()
        lru.put("a", 100)
        lru.put("b", 100)
        lru.put("c", 100)
        lru.touch("a")
        assertEquals(300L, lru.totalBytes)
        assertEquals(emptyList<String>(), lru.evict(300) { false })
        val dropped = lru.evict(200) { false }
        assertEquals(listOf("b"), dropped)
        assertEquals(200L, lru.totalBytes)
        assertFalse(lru.contains("b"))
        // c is the oldest now but wanted: a goes instead.
        assertEquals(listOf("a"), lru.evict(100) { it == "c" })
        assertEquals(100L, lru.totalBytes)
    }

    @Test
    fun replacingAKeyAdjustsTheTotal() {
        val lru = LruBytes<String>()
        lru.put("a", 100)
        lru.put("a", 40)
        assertEquals(40L, lru.totalBytes)
        assertTrue(lru.remove("a"))
        assertFalse(lru.remove("a"))
        assertEquals(0L, lru.totalBytes)
    }

    @Test
    fun whenEverythingIsWantedNothingIsDroppedEvenOverTheCap() {
        val lru = LruBytes<String>()
        lru.put("a", 100)
        lru.put("b", 100)
        assertTrue(lru.evict(10) { true }.isEmpty())
        assertEquals(200L, lru.totalBytes)
    }
}
