package com.moemusicplayer.mediaindex

import android.content.Context
import android.media.AudioTrack
import android.os.Build
import android.os.SystemClock
import android.util.Log
import org.json.JSONObject
import java.lang.ref.WeakReference
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit

/**
 * The real mixer rate of the playing AudioTrack from a privileged `dumpsys` (ADR 0048). Safety rules, from a real
 * incident (audioserver aborted about 2 s after a flinger dump): the dump runs on its own thread, at most once per
 * track creation or route change, never closer than [MixerRateConfig.MIN_INTERVAL_MS] to the last one (requests in
 * between are coalesced), never while the USB exclusive output or native DSD owns the device, never when a kill switch
 * is off, and an audio-server symptom soon after a dump stops it for the rest of the process.
 */
internal class MixerRateLookup(
    context: Context,
    /** The hidden `settings.json` boolean (`androidRealMixerRate`). */
    private val settingEnabled: () -> Boolean,
    /** Called (any thread) when the answer changed. */
    private val onChange: () -> Unit,
) {
    private val appContext = context.applicationContext
    private val executor = ScheduledThreadPoolExecutor(1) { runnable ->
        Thread(runnable, "moe-mixer-rate").apply { isDaemon = true }
    }.apply { removeOnCancelPolicy = true }
    private val gate = DumpGate(MixerRateConfig.MIN_INTERVAL_MS, MixerRateConfig.SETTLE_MS)

    @Volatile private var track: WeakReference<AudioTrack>? = null
    @Volatile private var routeKey: Int? = null

    /** What the status shows for the current track. */
    @Volatile var lookup: MixerRateRules.Lookup = MixerRateRules.Lookup.NotUsed
        private set

    /** A track exists or was re-routed: look up its thread rate (coalesced, off every app thread that matters). */
    fun request(newTrack: AudioTrack, routedDeviceId: Int?) {
        track = WeakReference(newTrack)
        routeKey = routedDeviceId
        if (!allowed()) {
            setLookup(MixerRateRules.Lookup.NotUsed)
            return
        }
        // A privileged source is probably in use: show nothing for the 「系統」 step instead of the stream-level value.
        if (MixerRatePrefs.rootOptIn(appContext) || ShizukuAccess.binderAlive()) setLookup(MixerRateRules.Lookup.Pending)
        val delay = synchronized(gate) { gate.request(SystemClock.elapsedRealtime()) } ?: return
        schedule(delay)
    }

    /** The track is gone. */
    fun forget() {
        track = null
        routeKey = null
        setLookup(MixerRateRules.Lookup.NotUsed)
    }

    /** The user asked to check the sources again: the lookup of the current track may run again. */
    fun recheck() {
        cache.clear()
        val current = track?.get() ?: return
        request(current, routeKey)
    }

    fun shutdown() {
        executor.shutdownNow()
    }

    /** An audio-server symptom (called from the player and routing callbacks, any thread). */
    fun symptom(what: String) {
        val tripped = synchronized(breaker) { breaker.symptom(SystemClock.elapsedRealtime(), what) }
        if (tripped) {
            Log.w(TAG, "mixer rate lookup disabled for this process: ${breaker.reason}")
            setLookup(MixerRateRules.Lookup.NotUsed)
        }
    }

    private fun allowed() = MixerRateConfig.PRIVILEGED_LOOKUP_ENABLED && settingEnabled() && !breaker.tripped

    private fun schedule(delayMs: Long) {
        try {
            executor.schedule({ runOnce() }, delayMs, TimeUnit.MILLISECONDS)
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            // The service is gone.
        }
    }

    private fun setLookup(value: MixerRateRules.Lookup) {
        if (lookup == value) return
        lookup = value
        onChange()
    }

    /** Executor thread. */
    private fun runOnce() {
        val audioTrack = track?.get()
        val rootState = currentRoot()
        val shizuku = ShizukuAccess.state(appContext)
        val source = MixerRateRules.selectSource(
            rootState, shizuku, MixerRateConfig.PRIVILEGED_LOOKUP_ENABLED, settingEnabled(), breaker.tripped,
        )
        val portId = audioTrack?.let(::portIdOf)
        val key = portId?.let { cache.key(it, routeKey) }
        if (source == MixerRateRules.Source.PLATFORM || audioTrack == null || portId == null) {
            synchronized(gate) { gate.abort() }
            setLookup(MixerRateRules.Lookup.NotUsed)
            return
        }
        if (key != null && cache.has(key)) {
            synchronized(gate) { gate.abort() }
            setLookup(cache.get(key)?.let { MixerRateRules.Lookup.Found(it, source) } ?: MixerRateRules.Lookup.Failed)
            return
        }
        if (!MixerRateRules.mayDump(source, usbOutputActive(), UsbExclusiveEngine.source.nativeDsd, portId)) {
            synchronized(gate) { gate.abort() }
            setLookup(MixerRateRules.Lookup.NotUsed)
            return
        }
        val started = SystemClock.elapsedRealtime()
        if (!synchronized(gate) { gate.begin(started) }) return
        var rate: Int? = null
        try {
            rate = dump(source, portId)
        } catch (error: Exception) {
            Log.w(TAG, "mixer rate lookup failed: $error")
            if (source == MixerRateRules.Source.ROOT) RootAccess.forget()
        }
        if (breaker.tripped) {
            setLookup(MixerRateRules.Lookup.NotUsed)
        } else if (key != null) {
            cache.put(key, rate)
            Log.i(TAG, "mixer rate of port $portId via ${source.wire}: ${rate ?: "unknown"}")
            setLookup(rate?.let { MixerRateRules.Lookup.Found(it, source) } ?: MixerRateRules.Lookup.Failed)
        }
        val next = synchronized(gate) { gate.finish(SystemClock.elapsedRealtime()) }
        if (next != null && !breaker.tripped) schedule(next)
    }

    /** One run: the dumps of [MixerRateConfig.DUMP_ORDER] until one gives the rate, stopping at a tripped breaker. */
    private fun dump(source: MixerRateRules.Source, portId: Int): Int? {
        for (which in MixerRateConfig.DUMP_ORDER) {
            if (breaker.tripped) return null
            val service = if (which == MixerRateRules.Dump.FLINGER) "media.audio_flinger" else "media.metrics"
            synchronized(breaker) { breaker.dumpStarted() }
            val result = try {
                if (source == MixerRateRules.Source.ROOT) {
                    RootAccess.dumpsys(service, MixerRateConfig.DUMP_TIMEOUT_MS, MixerRateConfig.DUMP_MAX_BYTES)
                } else {
                    ShizukuAccess.dumpsys(service, MixerRateConfig.DUMP_TIMEOUT_MS, MixerRateConfig.DUMP_MAX_BYTES)
                }
            } catch (error: Exception) {
                synchronized(breaker) { breaker.dumpEnded(SystemClock.elapsedRealtime(), error.toString()) }
                throw error
            }
            // Only a short or failed output is read for the death text: a full dump may mention such words in its logs.
            val errorText = result.stderr + (if (result.stdout.length < SHORT_OUTPUT) result.stdout else "")
            synchronized(breaker) { breaker.dumpEnded(SystemClock.elapsedRealtime(), errorText) }
            if (breaker.tripped) {
                Log.w(TAG, "mixer rate lookup disabled for this process: ${breaker.reason}")
                return null
            }
            if (result.timedOut) {
                Log.w(TAG, "dumpsys $service timed out")
                continue
            }
            val rate = if (which == MixerRateRules.Dump.FLINGER) {
                MixerRateDumpParsers.fromFlingerDump(result.stdout, portId)
            } else {
                MixerRateDumpParsers.fromMetricsDump(result.stdout, portId)
            }
            if (rate != null) return rate
        }
        return null
    }

    private fun currentRoot(): MixerRateRules.RootState {
        if (RootAccess.state == MixerRateRules.RootState.UNKNOWN && MixerRatePrefs.rootOptIn(appContext)) {
            RootAccess.probe(MixerRateConfig.ROOT_SILENT_TIMEOUT_MS)
        }
        return RootAccess.state
    }

    private fun usbOutputActive(): Boolean = UsbExclusiveEngine.held != null

    companion object {
        private const val TAG = "MoeMediaIndex"
        private const val SHORT_OUTPUT = 2048

        /** `AudioTrack.MetricsConstants.PORT_ID` (API 29); the key is used directly so the call links on older systems. */
        private const val PORT_ID_KEY = "android.media.audiotrack.portId"

        /** Process-wide: a tripped breaker outlives a restarted playback service. */
        private val breaker = DumpBreaker(MixerRateConfig.BREAKER_WINDOW_MS)
        private val cache = MixerRateCache()

        val breakerTripped: Boolean get() = breaker.tripped
        val breakerReason: String? get() = breaker.reason

        /** The track's port id in AudioFlinger (its metrics), or null when unknown. */
        fun portIdOf(track: AudioTrack): Int? {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return null
            return try {
                track.metrics.getInt(PORT_ID_KEY, -1).takeIf { it > 0 }
            } catch (_: RuntimeException) {
                null
            }
        }
    }
}

/** The explanation panel's state and actions (ADR 0048), for the page. */
internal object MixerRatePanel {
    /** Executor thread or any non-main thread: probes `su` and Shizuku and answers the panel's state. */
    fun status(context: Context, settingEnabled: Boolean): JSONObject {
        val root = RootAccess.state
        val shizuku = ShizukuAccess.state(context)
        val panel = MixerRateRules.panelState(
            root, shizuku, MixerRateConfig.PRIVILEGED_LOOKUP_ENABLED, settingEnabled, MixerRateLookup.breakerTripped,
        )
        val source = MixerRateRules.selectSource(
            root, shizuku, MixerRateConfig.PRIVILEGED_LOOKUP_ENABLED, settingEnabled, MixerRateLookup.breakerTripped,
        )
        return MixerRateRules.panelJson(panel, root, shizuku, ShizukuAccess.installed(context), source)
            .put("rootOptIn", MixerRatePrefs.rootOptIn(context))
    }

    /** `su -c id`, asked for by the user: the root manager shows its own dialog; remembers the answer. */
    fun checkRoot(context: Context) {
        val state = RootAccess.probe(MixerRateConfig.ROOT_ASK_TIMEOUT_MS)
        MixerRatePrefs.setRootOptIn(context, state == MixerRateRules.RootState.AVAILABLE)
    }
}
