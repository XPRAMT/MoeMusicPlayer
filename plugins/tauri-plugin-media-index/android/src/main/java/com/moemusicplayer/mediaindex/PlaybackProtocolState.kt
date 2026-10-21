package com.moemusicplayer.mediaindex

import org.json.JSONObject


/** Main-thread protocol state shared by the service and its bounded-load tests. */
internal class PlaybackLoadFence {
    private var nextInstance = 0L
    private var pendingCommandId: String? = null
    private var pendingTrackId: String? = null

    fun begin(commandId: String, trackId: String): Long {
        nextInstance += 1L
        pendingCommandId = commandId
        pendingTrackId = trackId
        return nextInstance
    }

    fun matchesReady(trackId: String?, mediaTag: String?): Boolean =
        pendingCommandId != null && pendingTrackId == trackId && pendingCommandId == mediaTag

    fun finish(commandId: String?): Boolean {
        if (commandId == null || pendingCommandId != commandId) return false
        pendingCommandId = null
        pendingTrackId = null
        return true
    }

    fun cancel(commandId: String?): Boolean = finish(commandId)
}

internal data class QueueCapabilities(val canNext: Boolean, val canPrevious: Boolean) {
    fun allowsNext(): Boolean = canNext
    fun allowsPrevious(): Boolean = canPrevious
}

/** Adds the process-ordered wire fields without changing command payloads. */
internal object PlaybackEventEnvelope {
    fun sequence(event: JSONObject, serviceGeneration: Long, eventId: String): JSONObject = event.apply {
        if (!has("v")) put("v", 1)
        if (!has("serviceGeneration")) put("serviceGeneration", serviceGeneration)
        if (!has("eventId")) put("eventId", eventId)
    }

    fun snapshot(
        serviceGeneration: Long,
        loadInstance: Long,
        trackId: String?,
        mediaKey: String?,
        state: String,
        positionMs: Long,
        durationMs: Long?,
        volume: Double,
        error: String?,
    ): JSONObject = JSONObject().apply {
        put("serviceGeneration", serviceGeneration)
        put("loadInstance", loadInstance)
        put("trackId", trackId ?: JSONObject.NULL)
        put("mediaKey", mediaKey ?: JSONObject.NULL)
        put("state", state)
        put("positionMs", positionMs)
        put("durationMs", durationMs ?: JSONObject.NULL)
        put("volume", volume)
        put("error", error ?: JSONObject.NULL)
    }
}
