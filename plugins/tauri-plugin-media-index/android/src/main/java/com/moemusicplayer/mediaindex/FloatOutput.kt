package com.moemusicplayer.mediaindex

import androidx.media3.common.PlaybackParameters
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.ToFloatPcmAudioProcessor

/**
 * The default Media3 chain with its official float converter after the speed and silence processors.
 *
 * While [passthrough] is true (the track in the sink is a DSD file played natively, ADR 0049) the converter is left
 * out: the data is raw DSD slots in a 16-bit carrier and must reach the USB output untouched. The default processors
 * stay; they are inactive (speed 1, silence skipping off, no trimming). Media3 asks for the list each time it
 * configures for a track, so the flag set at load applies to that track. A genuine 16-bit track is converted as
 * before: the flag, not the format, decides.
 */
@UnstableApi
class FloatAwareAudioProcessorChain(private val passthrough: () -> Boolean = { false }) : DefaultAudioSink.AudioProcessorChain {
    private val defaults = DefaultAudioSink.DefaultAudioProcessorChain()
    private val converter = ToFloatPcmAudioProcessor()

    override fun getAudioProcessors(): Array<AudioProcessor> =
        if (passthrough()) defaults.audioProcessors else defaults.audioProcessors + converter

    override fun applyPlaybackParameters(playbackParameters: PlaybackParameters): PlaybackParameters =
        defaults.applyPlaybackParameters(playbackParameters)

    override fun applySkipSilenceEnabled(skipSilenceEnabled: Boolean): Boolean =
        defaults.applySkipSilenceEnabled(skipSilenceEnabled)

    override fun getMediaDuration(playoutDuration: Long): Long = defaults.getMediaDuration(playoutDuration)

    override fun getSkippedOutputFrameCount(): Long = defaults.skippedOutputFrameCount
}
