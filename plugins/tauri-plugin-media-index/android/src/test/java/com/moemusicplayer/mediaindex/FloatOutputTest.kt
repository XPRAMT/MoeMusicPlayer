package com.moemusicplayer.mediaindex

import androidx.media3.common.C
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.ToFloatPcmAudioProcessor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class FloatOutputTest {
    @Test
    @Suppress("DEPRECATION")
    fun officialProcessorConvertsPcm16SamplesWithUnityGain() {
        val samples = shortArrayOf(0, 1, -1, 32767, -32768, 12345)
        val input = ByteBuffer.allocateDirect(samples.size * 2).order(ByteOrder.nativeOrder())
        samples.forEach(input::putShort)
        input.flip()

        val processor = ToFloatPcmAudioProcessor()
        val format = processor.configure(AudioProcessor.AudioFormat(48_000, 2, C.ENCODING_PCM_16BIT))
        assertEquals(C.ENCODING_PCM_FLOAT, format.encoding)
        processor.flush()
        processor.queueInput(input)
        val output = processor.output.order(ByteOrder.nativeOrder())
        for (sample in samples) {
            val floatSample = output.getFloat()
            assertEquals(sample / 32768f, floatSample, 0f)
            assertEquals(sample.toInt(), (floatSample * 32768f).toInt())
        }
    }

    @Test
    fun converterIsAlwaysAddedAndPreservesDefaultPlaybackChain() {
        val chain = FloatAwareAudioProcessorChain()
        val defaults = DefaultAudioSink.DefaultAudioProcessorChain()
        val processors = chain.audioProcessors
        assertEquals(defaults.audioProcessors.size + 1, processors.size)
        assertTrue(processors.last() is ToFloatPcmAudioProcessor)
        val parameters = PlaybackParameters(1.5f, 1f)
        defaults.applyPlaybackParameters(parameters)
        assertEquals(parameters, chain.applyPlaybackParameters(parameters))
        assertEquals(defaults.applySkipSilenceEnabled(false), chain.applySkipSilenceEnabled(false))
        assertEquals(defaults.getMediaDuration(48_000L), chain.getMediaDuration(48_000L))
    }

}
