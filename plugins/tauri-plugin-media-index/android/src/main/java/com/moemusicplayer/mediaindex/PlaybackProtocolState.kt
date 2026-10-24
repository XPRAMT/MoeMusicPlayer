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

/** Which commands the service may fetch while another one has not been answered yet. */
internal object PlaybackCommandGate {
    /**
     * A load that is only waiting (for its file to be checked, for the media source to become ready, or for its artwork)
     * can be replaced by the next command.
     */
    fun mayPreempt(loadAwaitingReady: Boolean, artworkPreparing: Boolean, preflighting: Boolean = false): Boolean =
        loadAwaitingReady || artworkPreparing || preflighting
}

/**
 * Which load may still switch tracks once its file has been checked (ADR 0036). The newest load wins, and Rust says when
 * it gave up waiting for one (`cancel_load`): a check that finishes after that is dropped, so a stale choice never
 * replaces the track that plays.
 */
internal class LoadPreflightGate {
    private var latestCommandId: String? = null
    private val cancelled = HashSet<String>()

    /** A load command arrived. */
    fun begin(commandId: String) {
        latestCommandId = commandId
    }

    /** Rust gave up waiting for [commandId]. */
    fun cancel(commandId: String) {
        if (commandId.isNotBlank()) cancelled.add(commandId)
    }

    /** Whether the load whose file was just checked may go on to switch tracks. Asking again gives the same answer. */
    fun mayCommit(commandId: String): Boolean = latestCommandId == commandId && commandId !in cancelled

    /** The load is dealt with. */
    fun finish(commandId: String) {
        cancelled.remove(commandId)
    }
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
