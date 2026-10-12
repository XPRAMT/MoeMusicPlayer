package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MediaIndexMappingTest {
    @Test
    fun decodesMediaStoreTrackNumberWithOptionalDiscPrefix() {
        assertEquals(1 to 3, MediaIndexPlugin.splitTrackNumber(1003))
        assertEquals(null to 7, MediaIndexPlugin.splitTrackNumber(7))
        assertEquals(2 to null, MediaIndexPlugin.splitTrackNumber(2000))
        assertEquals(null to null, MediaIndexPlugin.splitTrackNumber(null))
        assertEquals(null to null, MediaIndexPlugin.splitTrackNumber(-1))
    }

    @Test
    fun includesAudioAndKnownAudioFilesButDoesNotGuessUnknownDocuments() {
        assertEquals(AudioCandidate.YES, MediaIndexPlugin.audioCandidate("audio/mpeg", "song"))
        assertEquals(AudioCandidate.YES, MediaIndexPlugin.audioCandidate("application/octet-stream", "song.FLAC"))
        assertEquals(AudioCandidate.NO, MediaIndexPlugin.audioCandidate("image/jpeg", "cover.jpg"))
        assertEquals(AudioCandidate.UNKNOWN, MediaIndexPlugin.audioCandidate(null, "unknown.bin"))
        assertFalse(MediaIndexPlugin.audioCandidate("image/jpeg", "cover.jpg") == AudioCandidate.YES)
        assertTrue(MediaIndexPlugin.audioCandidate("audio/ogg", null) == AudioCandidate.YES)
    }

    @Test
    fun incompleteScanAlwaysIncludesTheRequiredStateReason() {
        assertEquals(
            "Could not enumerate SAF directory",
            MediaIndexPlugin.scanStateReason("incomplete", null, "Could not enumerate SAF directory"),
        )
        assertEquals(
            "MediaStore changed while scanning",
            MediaIndexPlugin.scanStateReason("incomplete", "MediaStore changed while scanning", null),
        )
        assertEquals(
            "Android source scan did not complete",
            MediaIndexPlugin.scanStateReason("incomplete", null, null),
        )
        assertEquals(null, MediaIndexPlugin.scanStateReason("complete", null, null))
    }

    @Test
    fun hiddenSafArtifactsAreNotReportedAsUnknownAudio() {
        assertTrue(MediaIndexPlugin.isHiddenDocument(".nomedia"))
        assertTrue(MediaIndexPlugin.isHiddenDocument(".database_uuid"))
        assertTrue(MediaIndexPlugin.isHiddenDocument(".thumbnails"))
        assertFalse(MediaIndexPlugin.isHiddenDocument("song.m4a"))
        assertFalse(MediaIndexPlugin.isHiddenDocument(null))
    }
}
