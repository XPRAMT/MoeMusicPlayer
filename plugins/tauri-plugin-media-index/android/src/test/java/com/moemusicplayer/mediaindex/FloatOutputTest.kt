package com.moemusicplayer.mediaindex

import androidx.media3.common.C
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.ToFloatPcmAudioProcessor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

@OptIn(UnstableApi::class)
class FloatOutputTest {
    @After
    fun resetSwitch() {
        FloatOutputSwitch.enabled = false
    }

    @Test
    fun officialProcessorConvertsPcm16SamplesWithUnityGain() {
        val samples = shortArrayOf(0, 1, -1, 32767, -32768, 12345)
        val input = ByteBuffer.allocate(samples.size * 2).order(ByteOrder.nativeOrder())
        samples.forEach(input::putShort)
        input.flip()

        val processor = ToFloatPcmAudioProcessor()
        val format = processor.configure(AudioProcessor.AudioFormat(48_000, 2, C.ENCODING_PCM_16BIT))
        assertEquals(C.ENCODING_PCM_FLOAT, format.encoding)
        processor.queueInput(input)
        val output = processor.output.order(ByteOrder.nativeOrder())
        for (sample in samples) {
            val floatSample = output.getFloat()
            assertEquals(sample / 32768f, floatSample, 0f)
            assertEquals(sample.toInt(), (floatSample * 32768f).toInt())
        }
    }

    @Test
    fun converterIsAddedOnlyWhenTheSwitchIsEnabledAndPreservesDefaultPlaybackChain() {
        val chain = FloatAwareAudioProcessorChain()
        val defaults = DefaultAudioSink.DefaultAudioProcessorChain()
        FloatOutputSwitch.enabled = false
        val disabled = chain.audioProcessors
        assertEquals(defaults.audioProcessors.size, disabled.size)
        assertTrue(disabled.none { it is ToFloatPcmAudioProcessor })

        FloatOutputSwitch.enabled = true
        val enabled = chain.audioProcessors
        assertEquals(defaults.audioProcessors.size + 1, enabled.size)
        assertTrue(enabled.last() is ToFloatPcmAudioProcessor)
        val parameters = PlaybackParameters(1.5f, 1f)
        defaults.applyPlaybackParameters(parameters)
        assertEquals(parameters, chain.applyPlaybackParameters(parameters))
        assertEquals(defaults.applySkipSilenceEnabled(false), chain.applySkipSilenceEnabled(false))
        assertEquals(defaults.getMediaDuration(48_000L), chain.getMediaDuration(48_000L))
    }

    @Test
    fun positionIsPreservedOnlyWhenReloadingTheSameTrackForAChangedPolicy() {
        assertTrue(shouldPreservePositionForFloatOutputChange("load", true, "track-1", "track-1"))
        assertFalse(shouldPreservePositionForFloatOutputChange("load", false, "track-1", "track-1"))
        assertFalse(shouldPreservePositionForFloatOutputChange("load", true, "track-1", "track-2"))
        assertFalse(shouldPreservePositionForFloatOutputChange("pause", true, "track-1", "track-1"))
        assertFalse(shouldPreservePositionForFloatOutputChange("load", true, null, "track-1"))
    }
}
