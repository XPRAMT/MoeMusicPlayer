package com.moemusicplayer.mediaindex

import org.json.JSONObject

/** The codec a Bluetooth (A2DP) device streams with, in plain values (from `BluetoothCodecConfig`; ADR 0044). */
internal data class BluetoothCodecFacts(
    val codecType: Int,
    val sampleRateFlag: Int,
    val bitsPerSampleFlag: Int,
    val channelModeFlag: Int,
    val codecSpecific1: Long,
    /** `BluetoothCodecType.getCodecName()` (Android 15+), for codecs without a `SOURCE_CODEC_TYPE_*` constant. */
    val extendedName: String? = null,
)

/** Pure mapping of `BluetoothCodecConfig` values to what the settings page shows (ADR 0044). */
internal object BluetoothCodecRules {
    // BluetoothCodecConfig.SOURCE_CODEC_TYPE_* (public since Android 13).
    const val CODEC_SBC = 0
    const val CODEC_AAC = 1
    const val CODEC_APTX = 2
    const val CODEC_APTX_HD = 3
    const val CODEC_LDAC = 4
    const val CODEC_LC3 = 5
    const val CODEC_OPUS = 6
    const val CODEC_INVALID = 1_000_000

    /** LDAC's playback quality in `codecSpecific1` (the values Android's developer options set). */
    const val LDAC_QUALITY_HIGH = 1000L
    const val LDAC_QUALITY_STANDARD = 1001L
    const val LDAC_QUALITY_CONNECTION = 1002L
    const val LDAC_QUALITY_ADAPTIVE = 1003L

    fun codecName(type: Int, extendedName: String?): String? = when (type) {
        CODEC_SBC -> "SBC"
        CODEC_AAC -> "AAC"
        CODEC_APTX -> "aptX"
        CODEC_APTX_HD -> "aptX HD"
        CODEC_LDAC -> "LDAC"
        CODEC_LC3 -> "LC3"
        CODEC_OPUS -> "Opus"
        else -> extendedName?.trim()?.takeIf { it.isNotEmpty() }
    }

    /** `SAMPLE_RATE_*` is one bit per rate; the highest bit set is the rate in use (one is set in a current config). */
    fun sampleRateHz(flag: Int): Int? = when {
        flag and 32 != 0 -> 192_000
        flag and 16 != 0 -> 176_400
        flag and 8 != 0 -> 96_000
        flag and 4 != 0 -> 88_200
        flag and 2 != 0 -> 48_000
        flag and 1 != 0 -> 44_100
        else -> null
    }

    /** `BITS_PER_SAMPLE_*`: 1 = 16, 2 = 24, 4 = 32. */
    fun bitsPerSample(flag: Int): Int? = when {
        flag and 4 != 0 -> 32
        flag and 2 != 0 -> 24
        flag and 1 != 0 -> 16
        else -> null
    }

    /** `CHANNEL_MODE_*`: 1 = mono, 2 = stereo. */
    fun channels(flag: Int): Int? = when {
        flag and 2 != 0 -> 2
        flag and 1 != 0 -> 1
        else -> null
    }

    /**
     * LDAC's bitrate for its quality setting: 990 / 660 / 330 kbps in the 48 kHz family and 909 / 606 / 303 kbps in the
     * 44.1 kHz family; null for "best effort" (adaptive) and for anything else.
     */
    fun ldacKbps(quality: Long, sampleRateHz: Int?): Int? {
        val family44 = sampleRateHz != null && sampleRateHz % 44_100 == 0
        return when (quality) {
            LDAC_QUALITY_HIGH -> if (family44) 909 else 990
            LDAC_QUALITY_STANDARD -> if (family44) 606 else 660
            LDAC_QUALITY_CONNECTION -> if (family44) 303 else 330
            else -> null
        }
    }

    /** What the page shows, or null when the codec is not known (invalid or unnamed). */
    fun codecJson(facts: BluetoothCodecFacts): JSONObject? {
        if (facts.codecType == CODEC_INVALID) return null
        val name = codecName(facts.codecType, facts.extendedName) ?: return null
        val rate = sampleRateHz(facts.sampleRateFlag)
        val ldac = facts.codecType == CODEC_LDAC
        return JSONObject().apply {
            put("codec", name)
            put("sampleRate", rate ?: JSONObject.NULL)
            put("bitsPerSample", bitsPerSample(facts.bitsPerSampleFlag) ?: JSONObject.NULL)
            put("channels", channels(facts.channelModeFlag) ?: JSONObject.NULL)
            put("ldacKbps", if (ldac) ldacKbps(facts.codecSpecific1, rate) ?: JSONObject.NULL else JSONObject.NULL)
            put("ldacAdaptive", ldac && facts.codecSpecific1 == LDAC_QUALITY_ADAPTIVE)
        }
    }

    /** Addresses are kept upper case, the way AudioDeviceInfo reports them. */
    fun addressKey(address: String?): String? = address?.trim()?.uppercase()?.takeIf { it.isNotEmpty() }
}
