package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.MixerAttributesRules.Entry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The system mixer formats of a USB output (ADR 0045), as `getSupportedMixerAttributes` reports them. */
class MixerAttributesRulesTest {
    private val stereo = 0x0C

    private fun entry(encoding: Int, rate: Int, behavior: Int, mask: Int = stereo, count: Int = 2) =
        Entry(encoding = encoding, sampleRate = rate, channelMask = mask, channelCount = count, behavior = behavior)

    private fun rates(group: JSONObject): List<Int> {
        val array = group.getJSONArray("sampleRates")
        return (0 until array.length()).map { array.getInt(it) }
    }

    private fun list(array: JSONArray): List<JSONObject> = (0 until array.length()).map { array.getJSONObject(it) }

    @Test
    fun anEmptyListHasNoGroupsAndNoBitPerfect() {
        val device = MixerAttributesRules.deviceJson(7, 11, "DAC", emptyList(), null)
        assertEquals(0, device.getJSONArray("groups").length())
        assertFalse(device.getBoolean("bitPerfect"))
        assertTrue(device.isNull("preferred"))
        assertFalse(device.has("error"))
    }

    @Test
    fun defaultOnlyEntriesFormOneGroupWithSortedDistinctRates() {
        val entries = listOf(
            entry(MixerAttributesRules.ENCODING_PCM_16BIT, 96_000, MixerAttributesRules.BEHAVIOR_DEFAULT),
            entry(MixerAttributesRules.ENCODING_PCM_16BIT, 48_000, MixerAttributesRules.BEHAVIOR_DEFAULT),
            entry(MixerAttributesRules.ENCODING_PCM_16BIT, 48_000, MixerAttributesRules.BEHAVIOR_DEFAULT),
        )
        val groups = list(MixerAttributesRules.groups(entries))
        assertEquals(1, groups.size)
        val group = groups[0]
        assertEquals("default", group.getString("behavior"))
        assertEquals("pcm16", group.getString("encoding"))
        assertEquals(16, group.getInt("bits"))
        assertEquals(2, group.getInt("channels"))
        assertEquals(listOf(48_000, 96_000), rates(group))
        assertFalse(MixerAttributesRules.hasBitPerfect(entries))
    }

    @Test
    fun bitPerfectGroupsComeFirstAndDeepestEncodingFirst() {
        val entries = listOf(
            entry(MixerAttributesRules.ENCODING_PCM_16BIT, 48_000, MixerAttributesRules.BEHAVIOR_DEFAULT),
            entry(MixerAttributesRules.ENCODING_PCM_16BIT, 44_100, MixerAttributesRules.BEHAVIOR_BIT_PERFECT),
            entry(MixerAttributesRules.ENCODING_PCM_32BIT, 192_000, MixerAttributesRules.BEHAVIOR_BIT_PERFECT),
            entry(MixerAttributesRules.ENCODING_PCM_32BIT, 48_000, MixerAttributesRules.BEHAVIOR_BIT_PERFECT),
            entry(MixerAttributesRules.ENCODING_PCM_32BIT, 96_000, MixerAttributesRules.BEHAVIOR_BIT_PERFECT),
        )
        val groups = list(MixerAttributesRules.groups(entries))
        assertEquals(
            listOf("bitPerfect/pcm32", "bitPerfect/pcm16", "default/pcm16"),
            groups.map { "${it.getString("behavior")}/${it.getString("encoding")}" },
        )
        assertEquals(listOf(48_000, 96_000, 192_000), rates(groups[0]))
        assertTrue(MixerAttributesRules.hasBitPerfect(entries))
        val device = MixerAttributesRules.deviceJson(
            7, 11, "DAC", entries,
            entry(MixerAttributesRules.ENCODING_PCM_32BIT, 96_000, MixerAttributesRules.BEHAVIOR_BIT_PERFECT),
        )
        val preferred = device.getJSONObject("preferred")
        assertEquals("bitPerfect", preferred.getString("behavior"))
        assertEquals(96_000, preferred.getInt("sampleRate"))
        assertEquals(32, preferred.getInt("bits"))
    }

    @Test
    fun belowAndroid14TheQueryIsNotSupported() {
        val result = MixerAttributesRules.resultJson(33, listOf(JSONObject()))
        assertFalse(result.getBoolean("supported"))
        assertEquals("requiresAndroid14", result.getString("reason"))
        assertEquals(0, result.getJSONArray("devices").length())
        val modern = MixerAttributesRules.resultJson(34, listOf(JSONObject().put("id", 1)))
        assertTrue(modern.getBoolean("supported"))
        assertEquals(1, modern.getJSONArray("devices").length())
    }

    @Test
    fun unknownBehaviorAndEncodingKeepTheirValues() {
        val odd = entry(encoding = 5, rate = 0, behavior = 9, mask = 1, count = 0)
        val groups = list(MixerAttributesRules.groups(listOf(odd)))
        val group = groups.single()
        assertEquals("unknown", group.getString("behavior"))
        assertEquals(9, group.getInt("behaviorValue"))
        assertEquals("other", group.getString("encoding"))
        assertEquals(5, group.getInt("encodingValue"))
        assertTrue(group.isNull("bits"))
        assertTrue(group.isNull("channels"))
        assertEquals(emptyList<Int>(), rates(group))
        assertTrue(MixerAttributesRules.entryJson(odd).isNull("sampleRate"))
    }

    @Test
    fun theChannelCountFallsBackToTheMask() {
        assertEquals(6, MixerAttributesRules.channelsOf(entry(2, 48_000, 0, mask = 0xFC, count = 0)))
        assertEquals(2, MixerAttributesRules.channelsOf(entry(2, 48_000, 0, mask = 0, count = 2)))
    }

    @Test
    fun anErrorIsReportedAsText() {
        val device = MixerAttributesRules.deviceJson(3, 22, "Headset", emptyList(), null, "java.lang.SecurityException: no")
        assertEquals("java.lang.SecurityException: no", device.getString("error"))
        assertEquals(0, device.getJSONArray("groups").length())
    }
}
