package com.moemusicplayer.mediaindex

import android.content.Context
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.util.Log
import androidx.media3.common.C
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.util.Clock
import androidx.media3.common.util.UnstableApi
import androidx.media3.common.util.Util
import androidx.media3.exoplayer.analytics.PlayerId
import androidx.media3.exoplayer.audio.AudioOutput
import androidx.media3.exoplayer.audio.AudioOutputProvider
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.CopyOnWriteArrayList

/**
 * A Media3 [AudioOutput] whose PCM goes to the USB exclusive engine (ADR 0047) instead of an AudioTrack.
 *
 * - [write] copies whole frames into the Rust ring and never blocks: it answers whether the buffer was taken whole.
 * - The position ([getPositionUs]) is the frames the device has been sent in completed URBs (the ring's `played`),
 *   so the URBs still queued (at most 16 ms) are not counted; Media3 caps it at the frames written.
 * - [play] / [pause] stream / stop streaming (the claim stays); [flush] drops what was written (a new ring, the stream
 *   keeps running with silence until new data); [stop] means "no more data": an empty ring is the end, not an underrun;
 *   [release] detaches the ring (the stream keeps running for the next output a moment, see `engine.rs`).
 * - When the device fails, is unplugged or the mode is switched to shared, the ring is failed: [write] takes nothing and
 *   [isStalled] is true, so Media3 releases this output and asks the provider for a new one, which is an AudioTrack.
 * - [retire] (the mode, the chosen device or the attached USB devices changed) does the same on purpose: no fallback reason is recorded.
 * - Native DSD (ADR 0049, [UsbExclusiveRules.Source.nativeDsd]): the PCM Media3 writes is the raw DSD slots in a 16-bit,
 *   4-channel carrier. The ring replaces the first 25 ms of it by idle fill (Media3 scales that much with its volume
 *   ramp), the engine sends the bytes unchanged, and [setVolume] (the app volume and the audio-focus ducking) is ignored.
 */
@UnstableApi
internal class UsbAudioOutput private constructor(
    /** The USB device this output drives ([UsbExclusiveRules.UsbDeviceFacts.key]). */
    val deviceKey: String,
    private val config: AudioOutputProvider.OutputConfig,
    private val channels: Int,
    private val source: UsbExclusiveRules.Source,
    private val reopen: () -> Boolean,
    private val onReleasedByMedia3: (UsbAudioOutput) -> Unit,
) : AudioOutput {
    private var ring: Long = 0
    private val listeners = CopyOnWriteArrayList<AudioOutput.Listener>()
    private var playing = false
    private var released = false
    private var advancingReported = false
    private var underruns = 0L
    private var scratch: ByteBuffer? = null
    @Volatile private var retired = false

    /** The engine said no (a failed attach, a failed step, the device gone, the shared mode), or [retire]d. */
    val failed: Boolean
        get() = retired || released || ring == 0L || UsbExclusiveNative.nativeRingFailed(ring)

    /**
     * Any thread. The user's choice changed: this output takes nothing more and reports itself stalled, so Media3's
     * playback thread releases it and asks the provider for the next output, which follows the new choice. A paused
     * output whose claim ended meanwhile does not claim the device again either.
     */
    fun retire() {
        retired = true
    }

    private fun newRing(): Boolean {
        ring = UsbExclusiveNative.nativeCreateRing(
            UsbExclusiveRules.ringFrames(config.sampleRate),
            config.encoding,
            channels,
            if (source.nativeDsd) config.sampleRate else 0,
        )
        return ring != 0L && attach()
    }

    /** Attach the ring to the device (claim, alt and clock as needed); false with the reason as the track's fallback. */
    private fun attach(): Boolean {
        val answer = UsbExclusiveNative.nativeAttach(ring, config.sampleRate, source.bits, source.dsd, source.nativeDsd)
            ?.let { runCatching { org.json.JSONObject(it) }.getOrNull() }
        if (answer?.optBoolean("ok") == true) return true
        fallBack(answer?.optString("reason")?.takeIf { it.isNotBlank() } ?: "USB 音訊裝置無法開始播放")
        return false
    }

    private fun fallBack(reason: String) {
        Log.w(TAG, "USB exclusive: falling back to Android: $reason")
        UsbExclusiveEngine.trackFallback = reason
    }

    override fun play() {
        playing = true
        if (failed) return
        var error = UsbExclusiveNative.nativePlay(ring)
        if (error != null && !UsbExclusiveNative.nativeAlive()) {
            // The claim ended while this output was paused (idle for 10 s): claim again where the ring left off.
            error = if (reopen() && attach()) UsbExclusiveNative.nativePlay(ring) else error
        }
        if (error != null) fallBack(error)
    }

    override fun pause() {
        playing = false
        if (!failed) UsbExclusiveNative.nativePause(ring)
    }

    override fun write(buffer: ByteBuffer, encodedAccessUnitCount: Int, presentationTimeUs: Long): Boolean {
        if (failed) return false
        val start = buffer.position()
        val length = buffer.remaining()
        val taken = if (buffer.isDirect) {
            UsbExclusiveNative.nativeWrite(ring, buffer, start, length)
        } else {
            val direct = scratch?.takeIf { it.capacity() >= length }
                ?: ByteBuffer.allocateDirect(length).order(ByteOrder.nativeOrder()).also { scratch = it }
            direct.clear()
            direct.put(buffer.duplicate())
            UsbExclusiveNative.nativeWrite(ring, direct, 0, length)
        }
        buffer.position(start + taken)
        reportUnderruns()
        return !buffer.hasRemaining()
    }

    private fun reportUnderruns() {
        val now = UsbExclusiveNative.nativeRingUnderruns(ring)
        if (now > underruns) {
            underruns = now
            listeners.forEach { it.onUnderrun() }
        }
    }

    override fun flush() {
        if (released) return
        val old = ring
        if (old != 0L) {
            UsbExclusiveNative.nativeDetach(old)
            UsbExclusiveNative.nativeDestroyRing(old)
        }
        ring = 0
        advancingReported = false
        underruns = 0
        if (newRing() && playing) UsbExclusiveNative.nativePlay(ring)
    }

    override fun stop() {
        if (ring != 0L) UsbExclusiveNative.nativeRingEnd(ring)
    }

    override fun release() {
        if (released) return
        released = true
        if (ring != 0L) {
            UsbExclusiveNative.nativeDetach(ring)
            UsbExclusiveNative.nativeDestroyRing(ring)
            ring = 0
        }
        if (UsbExclusiveEngine.current === this) UsbExclusiveEngine.current = null
        onReleasedByMedia3(this)
        listeners.forEach { it.onReleased() }
    }

    // The app volume and the ducking arrive here; native DSD ignores them (the slots are not audio to scale).
    override fun setVolume(volume: Float) {
        DsdPassthroughRules.gainFor(source.nativeDsd, volume)?.let(UsbExclusiveNative::nativeSetGain)
    }

    override fun isOffloadedPlayback(): Boolean = false

    override fun getAudioSessionId(): Int = config.audioSessionId

    override fun getSampleRate(): Int = config.sampleRate

    override fun getBufferSizeInFrames(): Long = UsbExclusiveRules.ringFrames(config.sampleRate).toLong()

    override fun getPositionUs(): Long {
        if (ring == 0L) return 0
        val positionUs = UsbExclusiveRules.framesToUs(UsbExclusiveNative.nativePlayedFrames(ring), config.sampleRate)
        if (!advancingReported && playing && positionUs > 0) {
            advancingReported = true
            val startedAt = System.currentTimeMillis() - positionUs / 1000
            listeners.forEach { it.onPositionAdvancing(startedAt) }
        }
        return positionUs
    }

    override fun getPlaybackParameters(): PlaybackParameters = PlaybackParameters.DEFAULT

    override fun isStalled(): Boolean {
        if (released || !failed) return false
        // Media3 now releases this output and opens the next one through the provider: that one is an AudioTrack.
        // A retired output or the shared mode (the user changed the mode or the device) is not a failure of
        // this track.
        if (!retired && UsbExclusiveEngine.enabled && UsbExclusiveEngine.trackFallback == null) fallBack(engineError() ?: "USB 音訊裝置中斷")
        return true
    }

    private fun engineError(): String? = try {
        UsbExclusiveNative.nativeStatus()?.let { org.json.JSONObject(it).optString("error").takeIf { e -> e.isNotBlank() && e != "null" } }
    } catch (_: Throwable) {
        null
    }

    override fun addListener(listener: AudioOutput.Listener) {
        listeners.addIfAbsent(listener)
    }

    override fun removeListener(listener: AudioOutput.Listener) {
        listeners.remove(listener)
    }

    // Speed changes are done by Media3's processors before the output (the config never asks the output for them).
    override fun setPlaybackParameters(playbackParams: PlaybackParameters) = Unit

    override fun setOffloadDelayPadding(delayInFrames: Int, paddingInFrames: Int) = Unit

    override fun setOffloadEndOfStream() = Unit

    override fun attachAuxEffect(effectId: Int) = Unit

    override fun setAuxEffectSendLevel(level: Float) = Unit

    // The device is the USB device itself; Android's routing does not apply.
    override fun setPreferredDevice(preferredDevice: AudioDeviceInfo?) = Unit

    companion object {
        private const val TAG = "MoeMediaIndex"

        /** A started output, or null (the reason is the track's fallback) when the device cannot take it. */
        fun open(
            deviceKey: String,
            config: AudioOutputProvider.OutputConfig,
            channels: Int,
            source: UsbExclusiveRules.Source,
            reopen: () -> Boolean,
            onReleased: (UsbAudioOutput) -> Unit,
        ): UsbAudioOutput? {
            val output = UsbAudioOutput(deviceKey, config, channels, source, reopen, onReleased)
            if (!output.newRing()) {
                output.release()
                return null
            }
            return output
        }
    }
}

/**
 * An output of an AudioTrack provider that can be [retire]d: it then takes nothing more and reports itself stalled,
 * so Media3 releases it and asks the provider again (Media3's own "Resetting stalled audio output" path). This is how
 * a change of the output mode, of the chosen device or of the attached USB devices reaches the track that is playing (ADR 0047).
 */
@UnstableApi
internal class RetirableAudioOutput(
    private val inner: AudioOutput,
    private val onReleasedByMedia3: (RetirableAudioOutput) -> Unit,
) : AudioOutput by inner {
    @Volatile private var retired = false

    /** Any thread. */
    fun retire() {
        retired = true
    }

    override fun write(buffer: ByteBuffer, encodedAccessUnitCount: Int, presentationTimeUs: Long): Boolean =
        if (retired) false else inner.write(buffer, encodedAccessUnitCount, presentationTimeUs)

    override fun isStalled(): Boolean = retired || inner.isStalled()

    override fun setPlayerId(playerId: PlayerId) = inner.setPlayerId(playerId)

    override fun canReuseAudioOutput(
        currentConfig: AudioOutputProvider.OutputConfig,
        newFormat: AudioOutputProvider.FormatConfig,
        newConfig: AudioOutputProvider.OutputConfig,
    ): Boolean = !retired && inner.canReuseAudioOutput(currentConfig, newFormat, newConfig)

    override fun release() {
        inner.release()
        onReleasedByMedia3(this)
    }
}

/**
 * Media3's output provider with the USB exclusive output in front (ADR 0047). In the exclusive mode, when the choice
 * (the system default or a USB device) plays on a permitted USB audio device ([UsbExclusiveRules.route]) and the
 * device's descriptors offer the track's rate, channels and bit depth,
 * Media3 gets a [UsbAudioOutput]; otherwise everything is [delegate]'s (the AudioTrack provider, watched by the router),
 * unchanged. Format support is always the delegate's, so Media3 decodes and processes exactly as before.
 *
 * The choice is made when Media3 configures an output ([getOutputConfig]) and again when it opens one
 * ([getAudioOutput], from the config's own encoding, rate and channels), so an output re-created for the same config
 * (a seek, a stalled output, [retireCurrent]) follows the mode and the chosen device as they are now.
 */
@UnstableApi
internal class UsbAudioOutputProvider(
    context: Context,
    private val delegate: AudioOutputProvider,
    /** The user's chosen output device (the router keeps it). */
    private val preference: () -> OutputDevicePreference?,
) : AudioOutputProvider by delegate {
    private val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager
    private val audio = context.getSystemService(AudioManager::class.java)

    /** The output handed to Media3 last and not released yet (ours or a wrapped AudioTrack). */
    @Volatile private var live: AudioOutput? = null

    /** Media3 has an output open (a track is loaded). */
    val hasLiveOutput: Boolean
        get() = live != null

    /** The USB device the output Media3 plays through now drives, null for an AudioTrack (or a failed USB output). */
    val liveOn: String?
        get() = (live as? UsbAudioOutput)?.takeIf { !it.failed }?.deviceKey

    private class Decision(
        val choice: UsbExclusiveRules.Choice,
        val facts: UsbExclusiveRules.Facts,
        val manager: UsbManager?,
        val device: UsbDevice?,
    )

    private fun pcmEncoding(formatConfig: AudioOutputProvider.FormatConfig): Int {
        val format = formatConfig.format
        return if (MimeTypes.AUDIO_RAW == format.sampleMimeType) format.pcmEncoding else C.ENCODING_INVALID
    }

    private fun decide(encoding: Int, rate: Int, channels: Int): Decision {
        val wanted = preference()
        val manager = usb
        val device = manager?.let { UsbExclusiveEngine.findDevice(it, wanted) }
        val permitted = device != null && manager.hasPermission(device)
        val source = UsbExclusiveEngine.source
        val facts = UsbExclusiveRules.Facts(
            enabled = UsbExclusiveEngine.enabled,
            chosenKind = wanted?.kind,
            deviceAttached = device != null,
            permitted = permitted,
            encoding = encoding,
            trackFallback = UsbExclusiveEngine.trackFallback,
            unsupported = null,
            rate = rate,
            channels = channels,
            source = source,
            device = device?.deviceName,
        )
        val first = UsbExclusiveRules.choose(facts)
        if (first != UsbExclusiveRules.Choice.Usb || manager == null || device == null) {
            return Decision(first, facts, manager, device)
        }
        val raw = UsbExclusiveEngine.descriptorsOf(manager, device)
        val unsupported = if (raw == null) {
            "無法讀取 USB 描述元"
        } else {
            try {
                if (source.nativeDsd) {
                    UsbExclusiveNative.nativeSupportsDsd(raw, rate)
                } else {
                    UsbExclusiveNative.nativeSupports(raw, rate, channels, source.bits, source.dsd)
                }
            } catch (error: Throwable) {
                error.message ?: "USB 獨佔輸出無法使用"
            }
        }
        val checked = facts.copy(unsupported = unsupported)
        return Decision(UsbExclusiveRules.choose(checked), checked, manager, device)
    }

    /** Logs [decision] (once per output config or output, never per write) and records the fallback reason. */
    private fun record(where: String, decision: Decision) {
        Log.i(TAG, UsbExclusiveRules.decisionLog(where, decision.facts, decision.choice))
        val choice = decision.choice
        // Said only when the user expects the USB output (exclusive mode, a USB device to play on).
        if (choice is UsbExclusiveRules.Choice.AudioTrack && choice.reason != null) {
            UsbExclusiveEngine.trackFallback = choice.reason
        }
    }

    /**
     * The carrier of a native DSD track (16-bit, 4 channels, at the frame rate: up to 352.8 kHz, beyond what an AudioTrack
     * is asked about) is supported directly: only this provider's USB output ever plays it (ADR 0049).
     */
    override fun getFormatSupport(formatConfig: AudioOutputProvider.FormatConfig): AudioOutputProvider.FormatSupport {
        val format = formatConfig.format
        return if (UsbExclusiveEngine.source.nativeDsd && DsdPassthroughRules.isCarrier(pcmEncoding(formatConfig), format.channelCount)) {
            AudioOutputProvider.FormatSupport.Builder()
                .setFormatSupportLevel(AudioOutputProvider.FORMAT_SUPPORTED_DIRECTLY)
                .build()
        } else {
            delegate.getFormatSupport(formatConfig)
        }
    }

    override fun getOutputConfig(formatConfig: AudioOutputProvider.FormatConfig): AudioOutputProvider.OutputConfig {
        val format = formatConfig.format
        val encoding = pcmEncoding(formatConfig)
        val decision = decide(encoding, format.sampleRate, format.channelCount)
        record("getOutputConfig", decision)
        // A native DSD track has no AudioTrack config: getAudioOutput fails it (and the service reloads it as PCM).
        if (decision.choice is UsbExclusiveRules.Choice.AudioTrack && !UsbExclusiveEngine.source.nativeDsd) {
            return delegate.getOutputConfig(formatConfig)
        }
        val frameBytes = Util.getPcmFrameSize(encoding, format.channelCount)
        return AudioOutputProvider.OutputConfig.Builder()
            .setEncoding(encoding)
            .setSampleRate(format.sampleRate)
            .setChannelMask(Util.getAudioTrackChannelConfig(format.channelCount))
            .setBufferSize(UsbExclusiveRules.ringFrames(format.sampleRate) * frameBytes)
            .setAudioAttributes(formatConfig.audioAttributes)
            .setAudioSessionId(formatConfig.audioSessionId)
            .setVirtualDeviceId(formatConfig.virtualDeviceId)
            .setIsOffload(false)
            .setIsTunneling(false)
            .setUsePlaybackParameters(false)
            .build()
    }

    override fun getAudioOutput(config: AudioOutputProvider.OutputConfig): AudioOutput {
        val plain = UsbExclusiveRules.takesConfig(
            config.encoding,
            config.isOffload,
            config.isTunneling,
            config.usePlaybackParameters,
        )
        var notUsbReason: String? = null
        val usbOutput = if (plain) {
            val decision = decide(config.encoding, config.sampleRate, Integer.bitCount(config.channelMask))
            record("getAudioOutput", decision)
            notUsbReason = DsdPassthroughRules.notUsbReason(decision.facts, decision.choice)
            if (decision.choice == UsbExclusiveRules.Choice.Usb) openUsb(config, decision) else null
        } else {
            Log.i(
                TAG,
                "USB exclusive: getAudioOutput → AudioTrack: the config is not plain PCM (encoding=${config.encoding} " +
                    "offload=${config.isOffload} tunneling=${config.isTunneling} " +
                    "playbackParameters=${config.usePlaybackParameters})",
            )
            null
        }
        if (DsdPassthroughRules.mustFail(UsbExclusiveEngine.source.nativeDsd, usbOutput != null)) {
            // The raw slots must never reach an AudioTrack. The service reloads the track through the PCM converter.
            val reason = UsbExclusiveEngine.trackFallback ?: notUsbReason ?: "USB 獨佔輸出無法使用"
            Log.w(TAG, "USB exclusive: native DSD output not possible, the track is reloaded as PCM: $reason")
            UsbExclusiveEngine.trackFallback = reason
            throw AudioOutputProvider.InitializationException()
        }
        // Not ours (or it could not start): Android plays it, through an AudioTrack that a change can retire.
        val output: AudioOutput = usbOutput ?: RetirableAudioOutput(delegate.getAudioOutput(config)) { released ->
            if (live === released) live = null
        }
        live = output
        return output
    }

    private fun openUsb(config: AudioOutputProvider.OutputConfig, decision: Decision): UsbAudioOutput? {
        val manager = decision.manager ?: return null
        val device = decision.device ?: return null
        val wanted = preference()
        val channels = Integer.bitCount(config.channelMask)
        val reopen = {
            val facts = currentFacts(wanted)
            UsbExclusiveEngine.ensureRunning(manager, device, facts)
        }
        if (!reopen()) {
            UsbExclusiveEngine.trackFallback = "無法開啟 USB 音訊裝置"
            Log.w(TAG, "USB exclusive: openUsb → AudioTrack (無法開啟 USB 音訊裝置): ${device.deviceName}")
            return null
        }
        val output = UsbAudioOutput.open(device.deviceName, config, channels, UsbExclusiveEngine.source, reopen) { released ->
            if (live === released) live = null
        }
        if (output == null) {
            Log.w(
                TAG,
                "USB exclusive: openUsb → AudioTrack (${UsbExclusiveEngine.trackFallback}): attach failed; " +
                    "rate=${config.sampleRate} channels=$channels encoding=${config.encoding}",
            )
            return null
        }
        Log.i(
            TAG,
            "USB exclusive: openUsb → USB: ${device.deviceName} rate=${config.sampleRate} channels=$channels " +
                "encoding=${config.encoding} sourceBits=${UsbExclusiveEngine.source.bits} " +
                "dsd=${UsbExclusiveEngine.source.dsd} nativeDsd=${UsbExclusiveEngine.source.nativeDsd}",
        )
        UsbExclusiveEngine.current = output
        return output
    }

    /**
     * Any thread. The mode, the chosen device or the attached USB devices changed: the output Media3 plays through now is retired, so on its
     * next write Media3's playback thread releases it and opens the next one through this provider, which follows the
     * new choice (ADR 0047). False when there was none.
     */
    fun retireCurrent(): Boolean {
        when (val output = live) {
            is UsbAudioOutput -> output.retire()
            is RetirableAudioOutput -> output.retire()
            else -> return false
        }
        return true
    }

    /** The chosen output device as Android lists it now (null once the app holds it: it is not listed then). */
    private fun currentFacts(wanted: OutputDevicePreference?): OutputDeviceFacts? {
        val attached = audio?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)?.map(AudioOutputRouter::factsOf) ?: emptyList()
        return AudioOutputRules.find(wanted, attached) ?: UsbExclusiveEngine.held
    }

    override fun setClock(clock: Clock) = delegate.setClock(clock)

    override fun hasPendingReleases(): Boolean = delegate.hasPendingReleases()

    override fun release() {
        live = null
        UsbExclusiveEngine.release("播放器已釋放")
        delegate.release()
    }

    private companion object {
        const val TAG = "MoeMediaIndex"
    }
}
