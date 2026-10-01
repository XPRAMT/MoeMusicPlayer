package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class SafPathPolicyTest {
    @Test
    fun normalizesRelativeComponentsAndPreservesUnicodeAndSpaces() {
        assertEquals(
            listOf("音樂資料夾", "artist feat. guest", "track.flac"),
            normalizeSafPlaylistLocator("./音樂資料夾\\artist feat. guest/../artist feat. guest/track.flac"),
        )
    }

    @Test
    fun absoluteUriAndEmptyLocatorsRemainUnmatched() {
        assertNull(normalizeSafPlaylistLocator("/storage/emulated/0/Music/song.flac"))
        assertNull(normalizeSafPlaylistLocator("C:\\Music\\song.flac"))
        assertNull(normalizeSafPlaylistLocator("content://media/external/audio/1"))
        assertNull(normalizeSafPlaylistLocator("https://example.invalid/song.flac"))
        assertNull(normalizeSafPlaylistLocator("././"))
    }

    @Test
    fun rejectsTraversalThatEscapesTheGrantedTree() {
        assertThrows(IllegalArgumentException::class.java) {
            normalizeSafPlaylistLocator("../../outside/song.flac")
        }
        assertThrows(IllegalArgumentException::class.java) {
            normalizeSafPlaylistLocator("folder/../../../outside/song.flac")
        }
    }
}
