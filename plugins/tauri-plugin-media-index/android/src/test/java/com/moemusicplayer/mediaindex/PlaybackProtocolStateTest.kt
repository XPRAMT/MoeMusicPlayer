package com.moemusicplayer.mediaindex

import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.json.JSONObject

class PlaybackProtocolStateTest {
    @Test
    fun expiredLoadCannotAcknowledgeALaterRequestForTheSameTrack() {
        val fence = PlaybackLoadFence()
        val firstInstance = fence.begin("command-1", "track-7")

        assertTrue(fence.cancel("command-1"))
        val secondInstance = fence.begin("command-2", "track-7")

        assertTrue(secondInstance > firstInstance)
        assertFalse(fence.matchesReady("track-7", "command-1"))
        assertTrue(fence.matchesReady("track-7", "command-2"))
        assertFalse(fence.finish("command-1"))
        assertFalse("a stale timeout must not cancel the current load", fence.cancel("command-1"))
        assertTrue(fence.matchesReady("track-7", "command-2"))
    }

    @Test
    fun queueCapabilitiesExposeOnlyRustAuthorizedTransportActions() {
        val empty = QueueCapabilities(canNext = false, canPrevious = false)
        assertFalse(empty.allowsNext())
        assertFalse(empty.allowsPrevious())

        val singleTrack = QueueCapabilities(canNext = true, canPrevious = false)
        assertTrue(singleTrack.allowsNext())
        assertFalse(singleTrack.allowsPrevious())

        val multiTrack = QueueCapabilities(canNext = true, canPrevious = true)
        assertTrue(multiTrack.allowsNext())
        assertTrue(multiTrack.allowsPrevious())
    }

    @Test
    fun ackWireFixtureCarriesGlobalStringEventIdAndNumericGenerationAndLoadInstance() {
        val snapshot = PlaybackEventEnvelope.snapshot(
            serviceGeneration = 7L,
            loadInstance = 12L,
            trackId = "track-7",
            mediaKey = "command-12",
            state = "paused",
            positionMs = 1234L,
            durationMs = 9000L,
            volume = 0.75,
            error = null,
        )
        val ack = PlaybackEventEnvelope.sequence(
            JSONObject()
                .put("kind", "ack")
                .put("commandId", "12")
                .put("requestId", "19")
                .put("snapshot", snapshot),
            serviceGeneration = 7L,
            eventId = "184",
        )
        val encodedFixture = ack.toString()
        val decoded = JSONObject(encodedFixture)
        val decodedSnapshot = decoded.getJSONObject("snapshot")

        assertEquals("ack", decoded.getString("kind"))
        assertEquals("184", decoded.getString("eventId"))
        assertEquals("12", decoded.getString("commandId"))
        assertEquals("19", decoded.getString("requestId"))
        assertEquals(1, decoded.getInt("v"))
        assertEquals(7L, decoded.getLong("serviceGeneration"))
        assertEquals(12L, decodedSnapshot.getLong("loadInstance"))
        assertEquals(7L, decodedSnapshot.getLong("serviceGeneration"))
        assertEquals("track-7", decodedSnapshot.getString("trackId"))
        assertEquals("command-12", decodedSnapshot.getString("mediaKey"))
        assertTrue(decoded.get("eventId") is String)
        assertTrue(decoded.get("serviceGeneration") is Number)
        assertTrue(decodedSnapshot.get("loadInstance") is Number)
        assertTrue(decodedSnapshot.get("serviceGeneration") is Number)
        assertTrue(decodedSnapshot.isNull("error"))
    }
}
