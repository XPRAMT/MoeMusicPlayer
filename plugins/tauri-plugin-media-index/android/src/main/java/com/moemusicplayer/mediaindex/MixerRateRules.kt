package com.moemusicplayer.mediaindex

import org.json.JSONObject

/**
 * Constants of the privileged mixer-rate lookup (ADR 0048). Every value that decides how often and whether `dumpsys` runs
 * lives here, so the owner can change it in one place.
 */
internal object MixerRateConfig {
    /**
     * Kill switch in code: false makes the lookup skip to `getNativeOutputSampleRate` whatever else is set. The second
     * kill switch is the hidden `settings.json` boolean `androidRealMixerRate` (read over JNI).
     */
    const val PRIVILEGED_LOOKUP_ENABLED = true

    /** The dumps of one lookup run, in order, until one gives the rate. Flip to [FLINGER_SECOND] to ask `media.metrics` first. */
    val DUMP_ORDER: List<MixerRateRules.Dump> = MixerRateRules.FLINGER_FIRST

    /** Two lookup runs start at least this far apart; requests in between are coalesced into one run. */
    const val MIN_INTERVAL_MS = 5_000L

    /** A request waits this long so a track that is created, pinned and re-routed within a moment costs one run. */
    const val SETTLE_MS = 1_000L

    /** An audio-server symptom this soon after a dump trips the circuit breaker. */
    const val BREAKER_WINDOW_MS = 8_000L

    /** One `dumpsys` is given this long, and its output is read up to this size. */
    const val DUMP_TIMEOUT_MS = 6_000L
    const val DUMP_MAX_BYTES = 4 * 1024 * 1024

    /** `su -c id` after the user asked (the root manager may show its grant dialog), and silently before a lookup. */
    const val ROOT_ASK_TIMEOUT_MS = 20_000L
    const val ROOT_SILENT_TIMEOUT_MS = 3_000L
}

/** Pure rules of the lookup (ADR 0048): which source, when a dump may run, the breaker and the panel state. */
internal object MixerRateRules {
    enum class Dump { FLINGER, METRICS }

    val FLINGER_FIRST = listOf(Dump.FLINGER, Dump.METRICS)
    val FLINGER_SECOND = listOf(Dump.METRICS, Dump.FLINGER)

    /** Which source gives the 「系統」 step. */
    enum class Source(val wire: String) {
        ROOT("root"),
        SHIZUKU("shizuku"),
        /** `AudioTrack.getNativeOutputSampleRate`: the stream-level value. */
        PLATFORM("platform"),
    }

    enum class ShizukuState { NOT_INSTALLED, NOT_RUNNING, TOO_OLD, NEEDS_PERMISSION, DENIED, GRANTED }

    /** Root is known only after a probe the user asked for (the grant dialog belongs to the root manager). */
    enum class RootState { UNKNOWN, AVAILABLE, UNAVAILABLE }

    /**
     * The source for the next lookup. Root first, then a Shizuku that is running and permitted; otherwise the platform
     * value. A tripped breaker or either kill switch means the platform value, whatever else is available.
     */
    fun selectSource(
        root: RootState,
        shizuku: ShizukuState,
        killSwitchCode: Boolean,
        settingEnabled: Boolean,
        breakerTripped: Boolean,
    ): Source = when {
        !killSwitchCode || !settingEnabled || breakerTripped -> Source.PLATFORM
        root == RootState.AVAILABLE -> Source.ROOT
        shizuku == ShizukuState.GRANTED -> Source.SHIZUKU
        else -> Source.PLATFORM
    }

    /**
     * Whether a dump may run now. Never when the USB exclusive output (or native DSD) owns the device: the track does not
     * go through AudioFlinger then. Never without a track port id.
     */
    fun mayDump(source: Source, usbOutputActive: Boolean, nativeDsdActive: Boolean, portId: Int?): Boolean =
        source != Source.PLATFORM && !usbOutputActive && !nativeDsdActive && portId != null && portId > 0

    /** What the status shows for the 「系統」 step: the value and where it came from; null: leave the step out. */
    class Resolved(val rate: Int?, val source: Source?)

    /** The privileged lookup of the current track. */
    sealed class Lookup {
        /** No privileged source in use: the platform value is the answer. */
        object NotUsed : Lookup()
        /** A source is in use and the run has not finished: nothing is shown rather than a stream-level guess. */
        object Pending : Lookup()
        /** The source could not tell (row missing, unparsable, command failed): the next source, the platform value. */
        object Failed : Lookup()
        data class Found(val rate: Int, val source: Source) : Lookup()
    }

    fun resolve(platformRate: Int?, lookup: Lookup): Resolved = when (lookup) {
        is Lookup.Found -> Resolved(lookup.rate, lookup.source)
        Lookup.Pending -> Resolved(null, null)
        Lookup.NotUsed, Lookup.Failed -> Resolved(platformRate, platformRate?.let { Source.PLATFORM })
    }

    /** Text an audio-server death leaves in a `dumpsys` result (or its error stream). */
    fun looksLikeServerDeath(output: String): Boolean {
        val lower = output.lowercase()
        return lower.contains("server died") || lower.contains("deadobject") || lower.contains("dead object") ||
            lower.contains("failure from system") || lower.contains("can't find service")
    }

    // ---- the settings panel (ADR 0039 convention: explanation first) ---------------------------------------------

    /** The state the explanation panel shows. */
    enum class Panel(val wire: String) {
        /** The circuit breaker stopped the lookup for this process. */
        TRIPPED("tripped"),
        ROOT_ACTIVE("rootActive"),
        SHIZUKU_ACTIVE("shizukuActive"),
        /** Shizuku runs and can be asked. */
        SHIZUKU_NEEDS_PERMISSION("shizukuNeedsPermission"),
        /** "Deny and don't ask again": only the Shizuku app can allow it. */
        SHIZUKU_DENIED("shizukuDenied"),
        /** Shizuku is installed but not running (or too old). */
        SHIZUKU_NOT_RUNNING("shizukuNotRunning"),
        /** No root, no Shizuku. */
        NONE("none"),
        /** The hidden switch (or the code constant) turned the lookup off. */
        DISABLED("disabled"),
    }

    fun panelState(
        root: RootState,
        shizuku: ShizukuState,
        killSwitchCode: Boolean,
        settingEnabled: Boolean,
        breakerTripped: Boolean,
    ): Panel = when {
        !killSwitchCode || !settingEnabled -> Panel.DISABLED
        breakerTripped -> Panel.TRIPPED
        root == RootState.AVAILABLE -> Panel.ROOT_ACTIVE
        shizuku == ShizukuState.GRANTED -> Panel.SHIZUKU_ACTIVE
        shizuku == ShizukuState.NEEDS_PERMISSION -> Panel.SHIZUKU_NEEDS_PERMISSION
        shizuku == ShizukuState.DENIED -> Panel.SHIZUKU_DENIED
        shizuku == ShizukuState.NOT_RUNNING || shizuku == ShizukuState.TOO_OLD -> Panel.SHIZUKU_NOT_RUNNING
        else -> Panel.NONE
    }

    /**
     * The Shizuku state from the facts the API gives (every call guarded by the caller): installed, `pingBinder`,
     * `isPreV11`, `checkSelfPermission == GRANTED`, `shouldShowRequestPermissionRationale`.
     */
    fun shizukuState(
        installed: Boolean,
        binderAlive: Boolean,
        preV11: Boolean,
        granted: Boolean,
        rationale: Boolean,
    ): ShizukuState = when {
        !binderAlive -> if (installed) ShizukuState.NOT_RUNNING else ShizukuState.NOT_INSTALLED
        preV11 -> ShizukuState.TOO_OLD
        granted -> ShizukuState.GRANTED
        rationale -> ShizukuState.DENIED
        else -> ShizukuState.NEEDS_PERMISSION
    }

    fun panelJson(
        panel: Panel,
        root: RootState,
        shizuku: ShizukuState,
        shizukuInstalled: Boolean,
        source: Source,
    ): JSONObject = JSONObject().apply {
        put("state", panel.wire)
        put("root", root.name.lowercase())
        put("shizuku", shizuku.name.lowercase())
        put("shizukuInstalled", shizukuInstalled)
        put("source", source.wire)
    }
}

/**
 * Spaces the dump runs (ADR 0048 safety): at most one run in flight, runs start at least [minIntervalMs] apart, requests
 * wait [settleMs] and every request made while one is pending or running is coalesced into it (a run that is running
 * when a request arrives is followed by exactly one more, after the interval). Pure: the caller passes the clock.
 */
internal class DumpGate(private val minIntervalMs: Long, private val settleMs: Long) {
    private var lastStartMs: Long? = null
    private var pending = false
    private var running = false
    private var rerun = false

    /** The delay after which to call [begin], or null when the request is coalesced into a pending or running run. */
    fun request(nowMs: Long): Long? {
        if (pending) return null
        if (running) {
            rerun = true
            return null
        }
        pending = true
        return delayFor(nowMs)
    }

    private fun delayFor(nowMs: Long): Long {
        val earliest = maxOf(nowMs + settleMs, (lastStartMs ?: Long.MIN_VALUE / 2) + minIntervalMs)
        return earliest - nowMs
    }

    /** The scheduled time came: true when the run starts now (it is then "running" until [finish]). */
    fun begin(nowMs: Long): Boolean {
        if (!pending || running) return false
        pending = false
        running = true
        lastStartMs = nowMs
        return true
    }

    /** The run found nothing to do (a skip condition): no dump was made, nothing is remembered. */
    fun abort() {
        pending = false
        running = false
        rerun = false
    }

    /** The run ended; the delay of the one follow-up run when a request arrived meanwhile, else null. */
    fun finish(nowMs: Long): Long? {
        running = false
        if (!rerun) return null
        rerun = false
        pending = true
        return delayFor(nowMs)
    }
}

/**
 * The circuit breaker (ADR 0048 safety): once an audio-server symptom shows within [windowMs] after a dump, or a dump
 * itself reports a dead audio server, the privileged lookup stays off for the rest of the process. Pure.
 */
internal class DumpBreaker(private val windowMs: Long) {
    var tripped = false
        private set
    var reason: String? = null
        private set
    private var lastDumpEndMs: Long? = null
    private var dumpRunning = false

    fun dumpStarted() {
        dumpRunning = true
    }

    /** A dump ended; its output (or error text) may already say the audio server is gone. */
    fun dumpEnded(nowMs: Long, output: String?) {
        dumpRunning = false
        lastDumpEndMs = nowMs
        if (output != null && MixerRateRules.looksLikeServerDeath(output)) trip("the dump reported a dead audio server")
    }

    /** A symptom (a sink error, a player error, the playing track's route lost). True when it tripped the breaker. */
    fun symptom(nowMs: Long, what: String): Boolean {
        if (tripped) return false
        val last = lastDumpEndMs
        val soon = dumpRunning || (last != null && nowMs - last in 0..windowMs)
        if (!soon) return false
        trip("$what within ${windowMs} ms of a dump")
        return true
    }

    private fun trip(why: String) {
        if (tripped) return
        tripped = true
        reason = why
    }
}

/** The rates found for tracks, by port id and route, so a track is dumped once (a failed lookup is remembered too). */
internal class MixerRateCache {
    private val entries = HashMap<String, Int?>()

    fun key(portId: Int, routeKey: Int?): String = "$portId/${routeKey ?: -1}"

    fun has(key: String): Boolean = entries.containsKey(key)

    fun get(key: String): Int? = entries[key]

    fun put(key: String, rate: Int?) {
        // The cache only needs the live tracks: keep it small.
        if (entries.size >= 16) entries.clear()
        entries[key] = rate
    }

    fun clear() = entries.clear()
}
