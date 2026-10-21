package com.moemusicplayer.mediaindex

import androidx.media3.common.PlaybackParameters
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.ToFloatPcmAudioProcessor

/** The "always output float32" switch (Settings -> output, Android). Rust sends it with every playback command. */
object FloatOutputSwitch {
    @Volatile
    var enabled: Boolean = false
}

internal fun shouldPreservePositionForFloatOutputChange(
    operation: String,
    policyChanged: Boolean,
    currentTrackId: String?,
    requestedTrackId: String?,
): Boolean = operation == "load" && policyChanged && currentTrackId != null && currentTrackId == requestedTrackId

/** The default Media3 chain with its official converter after the speed and silence processors. */
@UnstableApi
class FloatAwareAudioProcessorChain : DefaultAudioSink.AudioProcessorChain {
    private val defaults = DefaultAudioSink.DefaultAudioProcessorChain()
    private val converter = ToFloatPcmAudioProcessor()

    override fun getAudioProcessors(): Array<AudioProcessor> =
        if (FloatOutputSwitch.enabled) defaults.audioProcessors + converter else defaults.audioProcessors

    override fun applyPlaybackParameters(playbackParameters: PlaybackParameters): PlaybackParameters =
        defaults.applyPlaybackParameters(playbackParameters)

    override fun applySkipSilenceEnabled(skipSilenceEnabled: Boolean): Boolean =
        defaults.applySkipSilenceEnabled(skipSilenceEnabled)

    override fun getMediaDuration(playoutDuration: Long): Long = defaults.getMediaDuration(playoutDuration)

    override fun getSkippedOutputFrameCount(): Long = defaults.skippedOutputFrameCount
}
