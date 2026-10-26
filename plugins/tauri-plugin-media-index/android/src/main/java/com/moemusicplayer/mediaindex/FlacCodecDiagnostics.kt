package com.moemusicplayer.mediaindex

import android.content.Context
import android.util.Log
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.DecoderReuseEvaluation
import androidx.media3.exoplayer.analytics.AnalyticsListener
import androidx.media3.exoplayer.mediacodec.MediaCodecInfo
import androidx.media3.exoplayer.mediacodec.MediaCodecUtil

/**
 * Diagnostic log of how the FLAC decoder is chosen (always on, FLAC only, one block per track start; tag
 * `MoeMediaIndex`, prefix `flacdiag`). It does not change the choice: it repeats the renderer's own query
 * (soft match through [SoftwareFirstFlacCodecSelector], then sort by format support) for the track's Format and
 * logs which decoder Media3 actually initialised.
 */
@UnstableApi
object FlacCodecDiagnostics {
    private const val TAG = "MoeMediaIndex"

    fun describe(info: MediaCodecInfo): String =
        "${info.name}(sw=${info.softwareOnly},hw=${info.hardwareAccelerated},vendor=${info.vendor})"

    /** Called by the selector for every FLAC query. */
    fun logSelector(secure: Boolean, tunneling: Boolean, defaults: List<MediaCodecInfo>, ordered: List<MediaCodecInfo>) {
        Log.i(
            TAG,
            "flacdiag selector secure=$secure tunnel=$tunneling default=[${defaults.joinToString { describe(it) }}] " +
                "ordered=[${ordered.joinToString { it.name }}]",
        )
    }

    private fun audioLimits(info: MediaCodecInfo): String {
        val audio = info.capabilities?.audioCapabilities ?: return "caps=none"
        val rates = audio.supportedSampleRates?.joinToString(",") ?: "-"
        val ranges = audio.supportedSampleRateRanges?.joinToString(",") { "${it.lower}-${it.upper}" } ?: "-"
        return "rates=[$rates] ranges=[$ranges] maxCh=${audio.maxInputChannelCount}"
    }

    /** Log, for the track's FLAC Format, each candidate's support and limits and the renderer's resulting order. */
    fun logFormat(context: Context, format: Format) {
        if (format.sampleMimeType != MimeTypes.AUDIO_FLAC) return
        try {
            Log.i(
                TAG,
                "flacdiag format rate=${format.sampleRate} ch=${format.channelCount} pcm=${format.pcmEncoding} " +
                    "codecs=${format.codecs} csd=${format.initializationData.size} maxIn=${format.maxInputSize}",
            )
            SoftwareFirstFlacCodecSelector.quiet.set(true)
            val soft = try {
                MediaCodecUtil.getDecoderInfosSoftMatch(SoftwareFirstFlacCodecSelector, format, false, false)
            } finally {
                SoftwareFirstFlacCodecSelector.quiet.set(false)
            }
            for (info in soft) {
                Log.i(
                    TAG,
                    "flacdiag candidate ${describe(info)} supported=${info.isFormatSupported(context, format)} " +
                        "functional=${info.isFormatFunctionallySupported(context, format)} " +
                        "rateOk=${info.isAudioSampleRateSupportedV21(format.sampleRate)} " +
                        "chOk=${info.isAudioChannelCountSupportedV21(format.channelCount)} ${audioLimits(info)}",
                )
            }
            val sorted = MediaCodecUtil.getDecoderInfosSortedByFormatSupport(context, soft, format)
            Log.i(TAG, "flacdiag renderer-order=[${sorted.joinToString { it.name }}]")
        } catch (error: Throwable) {
            Log.i(TAG, "flacdiag query failed: $error")
        }
    }

    /** Analytics listener that logs the Format, the candidate list and the decoder Media3 initialises. */
    fun listener(context: Context): AnalyticsListener = object : AnalyticsListener {
        @Volatile private var flacTrack = false

        override fun onAudioInputFormatChanged(
            eventTime: AnalyticsListener.EventTime,
            format: Format,
            decoderReuseEvaluation: DecoderReuseEvaluation?,
        ) {
            flacTrack = format.sampleMimeType == MimeTypes.AUDIO_FLAC
            logFormat(context.applicationContext, format)
        }

        override fun onAudioDecoderInitialized(
            eventTime: AnalyticsListener.EventTime,
            decoderName: String,
            initializedTimestampMs: Long,
            initializationDurationMs: Long,
        ) {
            if (flacTrack) Log.i(TAG, "flacdiag initialized $decoderName in ${initializationDurationMs}ms")
        }

        override fun onAudioCodecError(eventTime: AnalyticsListener.EventTime, audioCodecError: Exception) {
            if (flacTrack) Log.i(TAG, "flacdiag codec-error $audioCodecError")
        }

        override fun onPlayerError(
            eventTime: AnalyticsListener.EventTime,
            error: androidx.media3.common.PlaybackException,
        ) {
            if (!flacTrack) return
            var cause: Throwable? = error
            val chain = StringBuilder()
            while (cause != null && chain.length < 600) {
                chain.append(" <- ").append(cause.javaClass.simpleName).append(": ").append(cause.message)
                cause = cause.cause
            }
            Log.i(TAG, "flacdiag player-error ${error.errorCodeName}$chain")
        }
    }
}
