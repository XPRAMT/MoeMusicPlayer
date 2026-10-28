package com.moemusicplayer.mediaindex

import org.json.JSONArray
import org.json.JSONObject

/**
 * The system mixer formats Android 14+ reports for an output device (`AudioManager.getSupportedMixerAttributes`,
 * `getPreferredMixerAttributes`), normalised and grouped for the 「讀取 USB 規格」 result (ADR 0045). Pure: the reader
 * turns each `AudioMixerAttributes` into an [Entry]; nothing here talks to the system, and nothing is ever set.
 */
internal object MixerAttributesRules {
    /** `Build.VERSION_CODES.UPSIDE_DOWN_CAKE`: the mixer attribute queries exist from here on. */
    const val MIN_SDK = 34

    /** `AudioMixerAttributes.MIXER_BEHAVIOR_*`. */
    const val BEHAVIOR_DEFAULT = 0
    const val BEHAVIOR_BIT_PERFECT = 1

    /** `AudioFormat.ENCODING_*` values the page names. */
    const val ENCODING_PCM_16BIT = 2
    const val ENCODING_PCM_8BIT = 3
    const val ENCODING_PCM_FLOAT = 4
    const val ENCODING_PCM_24BIT_PACKED = 21
    const val ENCODING_PCM_32BIT = 22

    /** `AudioFormat.CHANNEL_OUT_DEFAULT`: no mask given, so no channel count can be derived from it. */
    private const val CHANNEL_OUT_DEFAULT = 1

    /** One `AudioMixerAttributes` as read: its `AudioFormat` and its mixer behavior. */
    data class Entry(
        val encoding: Int,
        val sampleRate: Int,
        val channelMask: Int,
        val channelCount: Int,
        val behavior: Int,
    )

    fun behaviorName(behavior: Int): String = when (behavior) {
        BEHAVIOR_DEFAULT -> "default"
        BEHAVIOR_BIT_PERFECT -> "bitPerfect"
        else -> "unknown"
    }

    fun encodingName(encoding: Int): String = when (encoding) {
        ENCODING_PCM_8BIT -> "pcm8"
        ENCODING_PCM_16BIT -> "pcm16"
        ENCODING_PCM_24BIT_PACKED -> "pcm24"
        ENCODING_PCM_32BIT -> "pcm32"
        ENCODING_PCM_FLOAT -> "pcmFloat"
        else -> "other"
    }

    /** Bits per sample of a PCM encoding; null for anything else. */
    fun bitsOf(encoding: Int): Int? = when (encoding) {
        ENCODING_PCM_8BIT -> 8
        ENCODING_PCM_16BIT -> 16
        ENCODING_PCM_24BIT_PACKED -> 24
        ENCODING_PCM_32BIT, ENCODING_PCM_FLOAT -> 32
        else -> null
    }

    /** The platform's count, else one per bit of a real output mask; null when neither says. */
    fun channelsOf(entry: Entry): Int? = when {
        entry.channelCount > 0 -> entry.channelCount
        entry.channelMask != 0 && entry.channelMask != CHANNEL_OUT_DEFAULT -> Integer.bitCount(entry.channelMask)
        else -> null
    }

    private fun behaviorOrder(behavior: Int): Int = when (behavior) {
        BEHAVIOR_BIT_PERFECT -> 0
        BEHAVIOR_DEFAULT -> 1
        else -> 2
    }

    private fun encodingOrder(encoding: Int): Int = when (encoding) {
        ENCODING_PCM_32BIT -> 0
        ENCODING_PCM_FLOAT -> 1
        ENCODING_PCM_24BIT_PACKED -> 2
        ENCODING_PCM_16BIT -> 3
        ENCODING_PCM_8BIT -> 4
        else -> 5
    }

    private fun describe(entry: Entry, into: JSONObject): JSONObject = into
        .put("behavior", behaviorName(entry.behavior))
        .put("behaviorValue", entry.behavior)
        .put("encoding", encodingName(entry.encoding))
        .put("encodingValue", entry.encoding)
        .put("bits", bitsOf(entry.encoding) ?: JSONObject.NULL)
        .put("channelMask", entry.channelMask)
        .put("channels", channelsOf(entry) ?: JSONObject.NULL)

    /** One entry on its own (the preferred one): its rate, null when unspecified (0). */
    fun entryJson(entry: Entry): JSONObject =
        describe(entry, JSONObject()).put("sampleRate", if (entry.sampleRate > 0) entry.sampleRate else JSONObject.NULL)

    /**
     * The supported entries grouped by behavior (bit-perfect first), then encoding (deepest first) and channel mask;
     * each group lists its distinct rates in ascending order (an unspecified rate, 0, is left out).
     */
    fun groups(entries: List<Entry>): JSONArray {
        val keyed = entries.groupBy { Triple(it.behavior, it.encoding, it.channelMask) }
        val ordered = keyed.entries.sortedWith(
            compareBy(
                { behaviorOrder(it.key.first) },
                { it.key.first },
                { encodingOrder(it.key.second) },
                { it.key.second },
                { -(channelsOf(it.value.first()) ?: 0) },
                { it.key.third },
            ),
        )
        val out = JSONArray()
        for ((_, members) in ordered) {
            val rates = JSONArray()
            members.map { it.sampleRate }.filter { it > 0 }.distinct().sorted().forEach { rates.put(it) }
            out.put(describe(members.first(), JSONObject()).put("sampleRates", rates))
        }
        return out
    }

    /** Whether any entry lets the mixer pass the stream through unchanged. */
    fun hasBitPerfect(entries: List<Entry>): Boolean = entries.any { it.behavior == BEHAVIOR_BIT_PERFECT }

    /** One device's answer. `error` is the text of an exception the query threw (the lists are then empty). */
    fun deviceJson(
        id: Int,
        type: Int,
        productName: String,
        supported: List<Entry>,
        preferred: Entry?,
        error: String? = null,
    ): JSONObject {
        val out = JSONObject()
            .put("id", id)
            .put("type", type)
            .put("productName", productName)
            .put("groups", groups(supported))
            .put("bitPerfect", hasBitPerfect(supported))
            .put("preferred", preferred?.let(::entryJson) ?: JSONObject.NULL)
        if (error != null) out.put("error", error)
        return out
    }

    /** The whole `mixerAttributes` field: below Android 14 the query does not exist. */
    fun resultJson(sdk: Int, devices: List<JSONObject>): JSONObject {
        if (sdk < MIN_SDK) {
            return JSONObject().put("supported", false).put("reason", "requiresAndroid14").put("devices", JSONArray())
        }
        val list = JSONArray()
        devices.forEach { list.put(it) }
        return JSONObject().put("supported", true).put("devices", list)
    }
}
