package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Bluetooth codec mapping (`BluetoothCodecConfig` values to what the page shows; ADR 0044). */
class BluetoothCodecRulesTest {
    @Test
    fun codecTypesHaveTheirNames() {
        assertEquals("SBC", BluetoothCodecRules.codecName(0, null))
        assertEquals("AAC", BluetoothCodecRules.codecName(1, null))
        assertEquals("aptX", BluetoothCodecRules.codecName(2, null))
        assertEquals("aptX HD", BluetoothCodecRules.codecName(3, null))
        assertEquals("LDAC", BluetoothCodecRules.codecName(4, null))
        assertEquals("LC3", BluetoothCodecRules.codecName(5, null))
        assertEquals("Opus", BluetoothCodecRules.codecName(6, null))
        // A vendor codec without a constant is named by the platform (Android 15+), else unknown.
        assertEquals("LHDC V5", BluetoothCodecRules.codecName(7, " LHDC V5 "))
        assertNull(BluetoothCodecRules.codecName(7, null))
    }

    @Test
    fun rateBitsAndChannelFlagsMapToValues() {
        assertEquals(44_100, BluetoothCodecRules.sampleRateHz(1))
        assertEquals(48_000, BluetoothCodecRules.sampleRateHz(2))
        assertEquals(88_200, BluetoothCodecRules.sampleRateHz(4))
        assertEquals(96_000, BluetoothCodecRules.sampleRateHz(8))
        assertEquals(176_400, BluetoothCodecRules.sampleRateHz(16))
        assertEquals(192_000, BluetoothCodecRules.sampleRateHz(32))
        assertNull(BluetoothCodecRules.sampleRateHz(0))
        assertEquals(16, BluetoothCodecRules.bitsPerSample(1))
        assertEquals(24, BluetoothCodecRules.bitsPerSample(2))
        assertEquals(32, BluetoothCodecRules.bitsPerSample(4))
        assertNull(BluetoothCodecRules.bitsPerSample(0))
        assertEquals(1, BluetoothCodecRules.channels(1))
        assertEquals(2, BluetoothCodecRules.channels(2))
        assertNull(BluetoothCodecRules.channels(0))
    }

    @Test
    fun ldacQualityIsItsBitrateInEachRateFamily() {
        assertEquals(990, BluetoothCodecRules.ldacKbps(1000, 96_000))
        assertEquals(660, BluetoothCodecRules.ldacKbps(1001, 48_000))
        assertEquals(330, BluetoothCodecRules.ldacKbps(1002, 96_000))
        assertEquals(909, BluetoothCodecRules.ldacKbps(1000, 44_100))
        assertEquals(606, BluetoothCodecRules.ldacKbps(1001, 88_200))
        assertEquals(303, BluetoothCodecRules.ldacKbps(1002, 44_100))
        assertNull(BluetoothCodecRules.ldacKbps(1003, 96_000))
        assertNull(BluetoothCodecRules.ldacKbps(0, 96_000))
    }

    @Test
    fun theCodecJsonIsWhatThePageShows() {
        val ldac = BluetoothCodecRules.codecJson(BluetoothCodecFacts(4, 8, 4, 2, 1003))!!
        assertEquals("LDAC", ldac.getString("codec"))
        assertEquals(96_000, ldac.getInt("sampleRate"))
        assertEquals(32, ldac.getInt("bitsPerSample"))
        assertEquals(2, ldac.getInt("channels"))
        assertTrue(ldac.isNull("ldacKbps"))
        assertTrue(ldac.getBoolean("ldacAdaptive"))
        val high = BluetoothCodecRules.codecJson(BluetoothCodecFacts(4, 8, 4, 2, 1000))!!
        assertEquals(990, high.getInt("ldacKbps"))
        assertFalse(high.getBoolean("ldacAdaptive"))
        val aac = BluetoothCodecRules.codecJson(BluetoothCodecFacts(1, 1, 1, 2, 1000))!!
        assertTrue("codecSpecific1 means nothing outside LDAC", aac.isNull("ldacKbps"))
        assertNull(BluetoothCodecRules.codecJson(BluetoothCodecFacts(1_000_000, 0, 0, 0, 0)))
        assertNull(BluetoothCodecRules.codecJson(BluetoothCodecFacts(9, 2, 1, 2, 0)))
        // A mode that was not read (no root / Shizuku) names nothing; 0 is "not set" and claims no mode.
        val unread = BluetoothCodecRules.codecJson(BluetoothCodecFacts(4, 8, 4, 2, null))!!
        assertTrue(unread.isNull("ldacKbps"))
        assertFalse(unread.getBoolean("ldacAdaptive"))
        assertFalse(unread.getBoolean("ldacDefault"))
        val unset = BluetoothCodecRules.codecJson(BluetoothCodecFacts(4, 8, 4, 2, 0))!!
        assertTrue(unset.isNull("ldacKbps"))
        assertFalse(unset.getBoolean("ldacAdaptive"))
        assertTrue(unset.getBoolean("ldacDefault"))
        assertEquals(660, BluetoothCodecRules.codecJson(BluetoothCodecFacts(4, 2, 4, 2, 1001))!!.getInt("ldacKbps"))
        assertEquals(330, BluetoothCodecRules.codecJson(BluetoothCodecFacts(4, 2, 4, 2, 1002))!!.getInt("ldacKbps"))
        assertFalse(BluetoothCodecRules.codecJson(BluetoothCodecFacts(1, 1, 1, 2, 0))!!.getBoolean("ldacDefault"))
        assertEquals("AA:BB:CC:DD:EE:FF", BluetoothCodecRules.addressKey(" aa:bb:cc:dd:ee:ff "))
        assertNull(BluetoothCodecRules.addressKey(""))
    }
}
