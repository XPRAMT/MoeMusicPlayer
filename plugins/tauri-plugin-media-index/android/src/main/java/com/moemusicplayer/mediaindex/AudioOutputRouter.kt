package com.moemusicplayer.mediaindex

import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.AudioRouting
import android.media.AudioTrack
import android.os.Build
import android.os.Handler
import android.util.Log
import androidx.media3.common.util.Clock
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.AudioOutput
import androidx.media3.exoplayer.audio.AudioOutputProvider
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.AudioTrackAudioOutput
import org.json.JSONArray
import org.json.JSONObject
import java.lang.ref.WeakReference

/**
 * The output device of the playback service (ADR 0044): keeps the user's choice, follows the attached devices through
 * an [AudioDeviceCallback] and pins the player to the chosen device whenever it is attached (system default otherwise),
 * and remembers what the AudioTrack really opened and where it is routed. Main thread (the player's thread) only,
 * except [observeTrack], which Media3 calls from its playback thread.
 */
@UnstableApi
internal class AudioOutputRouter(
    context: Context,
    private val handler: Handler,
    /** Pin the player to a device (`ExoPlayer.setPreferredAudioDevice`), null for the system's choice. */
    private val pin: (AudioDeviceInfo?) -> Unit,
    /** The hidden `settings.json` switch `androidRealMixerRate` (ADR 0048). */
    realMixerRateEnabled: () -> Boolean = { true },
) {
    private val audioManager = context.getSystemService(AudioManager::class.java)
    private var preference: OutputDevicePreference? = null
    private var pinnedId: Int? = null
    private var pinApplied = false
    private var registered = false
    private val liveTrack = LiveTrackOutput<AudioSink.AudioTrackConfig>()
    @Volatile private var trackOutput: AudioSink.AudioTrackConfig? = null
    @Volatile private var routedDevice: AudioDeviceInfo? = null
    /** The rate the Android mixer runs at for the stream's output (null: unknown). */
    @Volatile private var mixerRate: Int? = null
    private var observedTrack: WeakReference<AudioTrack>? = null

    /** The privileged lookup of the track's real mixer rate (ADR 0048). */
    val mixerLookup = MixerRateLookup(context, realMixerRateEnabled) { /* the status reads the answer when asked */ }

    /** Called (main thread) after the attached devices or the device the app holds changed (ADR 0047). */
    var onDevicesChanged: (() -> Unit)? = null

    private fun devicesChanged() {
        reconcile()
        onDevicesChanged?.invoke()
    }

    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) = devicesChanged()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) = devicesChanged()
    }

    private val routingListener = AudioRouting.OnRoutingChangedListener { router ->
        val previous = routedDevice
        routedDevice = router.routedDevice
        mixerRate = readMixerRate()
        // The playing track lost its route: a possible audio-server symptom (ADR 0048 safety).
        val track = observedTrack?.get()
        if (previous != null && routedDevice == null && track != null && playing(track)) mixerLookup.symptom("track route lost")
        else if (track != null) mixerLookup.request(track, routedDevice?.id)
    }

    private fun playing(track: AudioTrack): Boolean = try {
        track.playState == AudioTrack.PLAYSTATE_PLAYING
    } catch (_: RuntimeException) {
        false
    }

    /** The user's choice as applied now (null: the system default). */
    fun currentPreference(): OutputDevicePreference? = preference

    private val heldChanged: () -> Unit = { handler.post { devicesChanged() } }

    fun start() {
        if (registered) return
        registered = true
        // The USB exclusive output took or gave back the chosen device (ADR 0047): apply the choice again.
        UsbExclusiveEngine.onHeldChanged = heldChanged
        // The callback is also called at once with every attached device, which applies the choice.
        audioManager?.registerAudioDeviceCallback(deviceCallback, handler)
        reconcile()
    }

    fun stop() {
        if (UsbExclusiveEngine.onHeldChanged === heldChanged) UsbExclusiveEngine.onHeldChanged = null
        if (registered) audioManager?.unregisterAudioDeviceCallback(deviceCallback)
        registered = false
        forgetTrack()
    }

    /** A choice read from the Rust side; "not known yet" keeps the current one. */
    fun setPreference(reading: PreferenceReading) {
        if (reading !is PreferenceReading.Known) return
        if (reading.preference == preference && pinApplied) return
        preference = reading.preference
        reconcile()
    }

    /** Pin the player to the chosen device if it is attached, else let the system choose; only when that changes. */
    fun reconcile() {
        // While the app drives the chosen USB device itself, Android lists it as gone: that is not an unplug, the pin
        // stays until the device is handed back (ADR 0047).
        if (UsbExclusiveRules.keepsPin(preference, UsbExclusiveEngine.held)) return
        val attached = outputDevices()
        val target = AudioOutputRules.find(preference, attached.map(::factsOf))
            ?.let { facts -> attached.firstOrNull { it.id == facts.id } }
        if (pinApplied && target?.id == pinnedId) return
        pinApplied = true
        pinnedId = target?.id
        try {
            pin(target)
        } catch (error: RuntimeException) {
            Log.w(TAG, "Could not set the preferred audio device", error)
        }
    }

    /** `AnalyticsListener.onAudioTrackInitialized` (main thread). */
    fun onTrackInitialized(config: AudioSink.AudioTrackConfig) {
        liveTrack.initialized(config)
        trackOutput = liveTrack.current
        mixerRate = readMixerRate()
        Log.i(TAG, "AudioTrack initialized: ${config.sampleRate} Hz, encoding ${config.encoding}, offload ${config.offload}")
    }

    /** `AnalyticsListener.onAudioTrackReleased` (main thread). */
    fun onTrackReleased() {
        // A release reported after the next track opened is the old track's: the live one keeps playing.
        val none = liveTrack.released()
        Log.i(TAG, "AudioTrack released; another still open: ${!none}")
        if (!none) return
        trackOutput = null
        mixerRate = null
        mixerLookup.forget()
        forgetTrack()
    }

    /** Release the lookup thread (the service is going away). */
    fun shutdown() = mixerLookup.shutdown()

    /** The AudioTrack Media3 just created: follow where it is routed. Media3's playback thread. */
    fun observeTrack(track: AudioTrack) {
        handler.post {
            forgetTrack()
            observedTrack = WeakReference(track)
            try {
                track.addOnRoutingChangedListener(routingListener, handler)
                routedDevice = track.routedDevice
                mixerRate = readMixerRate()
                mixerLookup.request(track, routedDevice?.id)
            } catch (error: RuntimeException) {
                Log.w(TAG, "Could not follow the audio routing", error)
            }
        }
    }

    private fun forgetTrack() {
        observedTrack?.get()?.let { track ->
            try {
                track.removeOnRoutingChangedListener(routingListener)
            } catch (_: RuntimeException) {
                // The track is already gone.
            }
        }
        observedTrack = null
        routedDevice = null
    }

    /** The status the settings page shows; [saved] is the choice in `settings.json`. */
    fun status(saved: OutputDevicePreference?): JSONObject {
        val attached = outputDevices()
        val listed = UsbExclusiveRules.withHeld(attached.map(::factsOf), UsbExclusiveEngine.held)
        val trackRoute = routedDevice?.takeIf { trackOutput != null }
        // AudioTrack.playState is what the track itself does now (Media3 pauses the track on pause and stop), so it
        // is the most direct answer to "is this route live"; a paused track's route can be stale.
        val trackPlaying = try {
            observedTrack?.get()?.playState == AudioTrack.PLAYSTATE_PLAYING
        } catch (_: RuntimeException) {
            false
        }
        val source = AudioOutputRules.routeSource(trackPlaying, trackRoute != null, Build.VERSION.SDK_INT)
        val routed = when (source) {
            AudioOutputRules.RouteSource.TRACK -> trackRoute?.let(::factsOf)
            AudioOutputRules.RouteSource.SYSTEM -> systemMediaRoute(audioManager, attached)
        }
        val resolved = MixerRateRules.resolve(mixerRate, mixerLookup.lookup)
        return statusJson(listed, saved, pinnedId, routed, trackOutput, resolved, source)
    }

    /**
     * The rate of the mixer output media goes to (`AudioTrack.getNativeOutputSampleRate(STREAM_MUSIC)`, public API; it
     * follows the stream's current route). Null when it fails or reports 0.
     */
    private fun readMixerRate(): Int? = AudioOutputRules.mixerRateOf(
        try {
            @Suppress("DEPRECATION")
            AudioTrack.getNativeOutputSampleRate(AudioManager.STREAM_MUSIC)
        } catch (_: RuntimeException) {
            null
        },
    )

    private fun outputDevices(): List<AudioDeviceInfo> =
        audioManager?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)?.toList() ?: emptyList()

    companion object {
        private const val TAG = "MoeMediaIndex"

        fun factsOf(device: AudioDeviceInfo): OutputDeviceFacts = OutputDeviceFacts(
            id = device.id,
            type = device.type,
            productName = device.productName?.toString() ?: "",
            address = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) device.address ?: "" else "",
            sampleRates = device.sampleRates?.toList() ?: emptyList(),
            channelCounts = device.channelCounts?.toList() ?: emptyList(),
            encodings = device.encodings?.toList() ?: emptyList(),
        )

        /** The status while the playback service is not running: the attached devices only. */
        fun idleStatus(context: Context, saved: OutputDevicePreference?): JSONObject {
            val audioManager = context.getSystemService(AudioManager::class.java)
            val attached = audioManager?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)?.toList() ?: emptyList()
            return statusJson(
                UsbExclusiveRules.withHeld(attached.map(::factsOf), UsbExclusiveEngine.held),
                saved,
                null,
                systemMediaRoute(audioManager, attached),
                null,
                MixerRateRules.Resolved(null, null),
                AudioOutputRules.RouteSource.SYSTEM,
            )
        }

        /**
         * Where the system sends media now (Android 13+), as one of the attached devices; null before Android 13 or when
         * it is none of them.
         */
        private fun systemMediaRoute(audioManager: AudioManager?, attached: List<AudioDeviceInfo>): OutputDeviceFacts? {
            if (audioManager == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return null
            return try {
                val attributes = android.media.AudioAttributes.Builder()
                    .setUsage(android.media.AudioAttributes.USAGE_MEDIA)
                    .setContentType(android.media.AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build()
                val route = audioManager.getAudioDevicesForAttributes(attributes).firstOrNull() ?: return null
                attached.firstOrNull { it.type == route.type && (it.address ?: "") == route.address }
                    ?.let(::factsOf)
            } catch (_: RuntimeException) {
                null
            }
        }

        private fun statusJson(
            attached: List<OutputDeviceFacts>,
            saved: OutputDevicePreference?,
            pinnedId: Int?,
            routed: OutputDeviceFacts?,
            output: AudioSink.AudioTrackConfig?,
            mixerRate: MixerRateRules.Resolved,
            source: AudioOutputRules.RouteSource,
        ): JSONObject {
            val listed = AudioOutputRules.userDevices(attached)
            return JSONObject().apply {
                put("devices", JSONArray(listed.map(AudioOutputRules::deviceJson)))
                put("chosenId", AudioOutputRules.find(saved, attached)?.id ?: JSONObject.NULL)
                put("pinnedId", pinnedId ?: JSONObject.NULL)
                put("routed", routed?.let(AudioOutputRules::deviceJson) ?: JSONObject.NULL)
                // "track": where the playing AudioTrack goes; "system": the system's media route (paused or no track).
                put("routedFrom", if (source == AudioOutputRules.RouteSource.TRACK) "track" else "system")
                put(
                    "output",
                    output?.let {
                        JSONObject().apply {
                            put("encoding", it.encoding)
                            put("sampleRate", it.sampleRate)
                            put("channelCount", AudioOutputRules.channelCountOfMask(it.channelConfig))
                            put("offload", it.offload)
                            put("mixerSampleRate", mixerRate.rate ?: JSONObject.NULL)
                            // Where the 「系統」 rate came from: root, shizuku or platform (the stream-level API).
                            put("mixerRateSource", mixerRate.source?.wire ?: JSONObject.NULL)
                        }
                    } ?: JSONObject.NULL,
                )
                // The USB exclusive output (ADR 0047): whether it plays the track, its format and why not.
                put("exclusive", UsbExclusiveEngine.statusJson())
            }
        }
    }
}

/**
 * Media3's AudioTrack provider with one addition: every AudioTrack it creates is handed to [onTrack], so the router can
 * read where the track is really routed (`AudioTrack.getRoutedDevice`). Everything else is the default provider's.
 */
@UnstableApi
internal class RouteObservingOutputProvider(
    private val delegate: AudioOutputProvider,
    private val onTrack: (AudioTrack) -> Unit,
) : AudioOutputProvider by delegate {
    override fun getAudioOutput(config: AudioOutputProvider.OutputConfig): AudioOutput {
        val output = delegate.getAudioOutput(config)
        (output as? AudioTrackAudioOutput)?.let { audioOutput ->
            try {
                onTrack(audioOutput.audioTrack)
            } catch (_: RuntimeException) {
                // Only the routing display depends on it.
            }
        }
        return output
    }

    override fun setClock(clock: Clock) = delegate.setClock(clock)

    override fun hasPendingReleases(): Boolean = delegate.hasPendingReleases()
}
