package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Byte fixtures built by hand from the USB Audio Class 1.0 and 2.0 specifications (ADR 0045). */
class UsbAudioDescriptorsTest {
    private fun bytes(vararg values: Int) = values.map { it.toByte() }

    /** One descriptor: bLength is worked out from the body (type first). */
    private fun d(vararg body: Int): List<Byte> = listOf((body.size + 1).toByte()) + bytes(*body)

    private fun u16(value: Int) = intArrayOf(value and 0xFF, (value shr 8) and 0xFF)
    private fun u24(value: Int) = intArrayOf(value and 0xFF, (value shr 8) and 0xFF, (value shr 16) and 0xFF)
    private fun u32(value: Long) = intArrayOf(
        (value and 0xFF).toInt(), ((value shr 8) and 0xFF).toInt(), ((value shr 16) and 0xFF).toInt(),
        ((value shr 24) and 0xFF).toInt(),
    )

    private fun device(usb: Int, vid: Int, pid: Int) =
        d(0x01, *u16(usb), 0xEF, 0x02, 0x01, 64, *u16(vid), *u16(pid), *u16(0x0100), 1, 2, 3, 1)

    private fun configuration(value: Int, interfaces: Int) = d(0x02, *u16(0), interfaces, value, 0, 0x80, 50)
    private fun iface(number: Int, alt: Int, endpoints: Int, subclass: Int, protocol: Int) =
        d(0x04, number, alt, endpoints, 0x01, subclass, protocol, 0)

    /** Sony "Sony Audio" (VID 054C, PID 0B8C) as the user's device reports it: UAC2, async OUT with feedback. */
    private fun sonyUac2(): ByteArray {
        val out = mutableListOf<Byte>()
        out += device(0x0200, 0x054C, 0x0B8C)
        out += configuration(1, 2)
        // Interface association: interfaces 0..1 are one audio function.
        out += d(0x0B, 0, 2, 0x01, 0x00, 0x20, 0)
        out += iface(0, 0, 0, 0x01, 0x20)
        out += d(0x24, 0x01, *u16(0x0200), 0x01, *u16(0), 0x00) // header, bcdADC 2.00
        out += d(0x24, 0x0A, 0x29, 0x03, 0x07, 0x00, 0) // clock 0x29: internal programmable, freq r/w, validity r
        out += d(0x24, 0x02, 0x02, *u16(0x0101), 0x00, 0x29, 2, *u32(0x3), 0, *u16(0), 0) // IT USB streaming
        // Feature unit 0x0A from IT 2: master mute + volume r/w (0x0F), two channels with nothing.
        out += d(0x24, 0x06, 0x0A, 0x02, *u32(0x0F), *u32(0), *u32(0), 0)
        out += d(0x24, 0x03, 0x14, *u16(0x0302), 0x00, 0x0A, 0x29, *u16(0), 0) // OT headphones
        out += iface(1, 0, 0, 0x02, 0x20) // zero bandwidth
        for (alt in 1..7) {
            val bits = when (alt) {
                1 -> 16
                2 -> 24
                3 -> 32
                else -> 32
            }
            val formats = if (alt >= 6) 0x8000_0000L else 0x1L
            out += iface(1, alt, 2, 0x02, 0x20)
            out += d(0x24, 0x01, 0x02, 0x00, 0x01, *u32(formats), 2, *u32(0x3), 0)
            out += d(0x24, 0x02, 0x01, if (bits == 16) 2 else if (bits == 24) 3 else 4, bits)
            out += d(0x05, 0x01, 0x05, *u16(1024), 1)
            out += d(0x25, 0x01, 0x00, 0x00, 0x00, *u16(0))
            out += d(0x05, 0x81, 0x11, *u16(4), 4)
        }
        return out.toByteArray()
    }

    /** A UAC1 adaptive DAC: discrete rates 44.1 / 48 kHz, 16-bit stereo, feature unit with mute + volume. */
    private fun uac1(): ByteArray {
        val out = mutableListOf<Byte>()
        out += device(0x0110, 0x1234, 0x5678)
        out += configuration(1, 2)
        out += iface(0, 0, 0, 0x01, 0x00)
        out += d(0x24, 0x01, *u16(0x0100), *u16(0), 1, 1)
        out += d(0x24, 0x02, 0x01, *u16(0x0101), 0x00, 2, *u16(0x0003), 0, 0) // IT, 2 channels
        out += d(0x24, 0x06, 0x02, 0x01, 1, 0x03, 0x00, 0x00, 0) // FU: master mute+volume
        out += d(0x24, 0x03, 0x03, *u16(0x0301), 0x00, 0x02, 0) // OT speaker
        out += iface(1, 0, 0, 0x02, 0x00)
        out += iface(1, 1, 1, 0x02, 0x00)
        out += d(0x24, 0x01, 0x01, 1, *u16(0x0001))
        out += d(0x24, 0x02, 0x01, 2, 2, 16, 2, *u24(44100), *u24(48000))
        out += listOf(9, 0x05, 0x02, 0x09, 0xC4, 0x00, 1, 0, 0).map { it.toByte() } // iso OUT adaptive 196 bytes
        out += d(0x25, 0x01, 0x01, 0x00, *u16(0))
        // Alt 2: a continuous range.
        out += iface(1, 2, 1, 0x02, 0x00)
        out += d(0x24, 0x01, 0x01, 1, *u16(0x0001))
        out += d(0x24, 0x02, 0x01, 2, 3, 24, 0, *u24(8000), *u24(96000))
        out += listOf(9, 0x05, 0x02, 0x09, 0x40, 0x02, 1, 0, 0).map { it.toByte() }
        return out.toByteArray()
    }

    @Test
    fun sonyUac2IsParsedAsTheDeviceReportsIt() {
        val device = UsbAudioDescriptors.parse(sonyUac2())
        assertEquals(0x054C, device.vendorId)
        assertEquals(0x0B8C, device.productId)
        val config = device.configurations.single()
        assertEquals(2, config.uacVersion)
        assertEquals(0x0200, config.adcVersion)
        val clock = config.clockSources.single()
        assertEquals(0x29, clock.id)
        assertEquals(3, clock.clockType)
        assertEquals(3, clock.frequencyControl)
        assertEquals(1, clock.validityControl)
        assertEquals(listOf(0x0101, 0x0302), config.terminals.map { it.terminalType })
        assertEquals(0x29, config.terminals[0].clockSourceId)
        val feature = config.featureUnits.single()
        assertTrue(feature.masterMute)
        assertTrue(feature.masterVolume)
        assertFalse(feature.channelVolume)
        assertEquals(2, feature.channels)

        assertEquals((0..7).toList(), config.streams.map { it.alternateSetting })
        assertTrue(config.streams[0].endpoints.isEmpty())
        val first = config.streams[1]
        assertEquals(2, first.terminalLink)
        assertEquals(2, first.channels)
        assertEquals(2, first.subslotBytes)
        assertEquals(16, first.bits)
        assertNull(first.rates)
        assertEquals("pcm", UsbAudioDescriptors.formatName(first, 2))
        assertEquals("raw", UsbAudioDescriptors.formatName(config.streams[6], 2))
        val (data, feedback) = first.endpoints
        assertEquals(0x01, data.address)
        assertFalse(data.input)
        assertEquals(1, data.transferType)
        assertEquals(1, data.syncType)
        assertEquals(0, data.usageType)
        assertEquals(1024, data.maxPacketSize)
        assertEquals(1, data.interval)
        assertEquals(0x81, feedback.address)
        assertTrue(feedback.input)
        assertEquals(1, feedback.usageType)
        assertEquals(4, feedback.maxPacketSize)
        assertEquals(4, feedback.interval)
    }

    @Test
    fun theJsonCarriesTheStreamsAndTheRawHex() {
        val raw = sonyUac2()
        val json = UsbAudioDescriptors.deviceJson(raw)
        assertEquals(2, json.getInt("uacVersion"))
        val stream = json.getJSONArray("streams").getJSONObject(1)
        assertEquals("pcm", stream.getString("format"))
        assertTrue(stream.isNull("rates"))
        val endpoint = stream.getJSONArray("endpoints").getJSONObject(0)
        assertEquals("async", endpoint.getString("sync"))
        assertEquals("out", endpoint.getString("direction"))
        assertEquals("feedback", stream.getJSONArray("endpoints").getJSONObject(1).getString("usage"))
        assertEquals("internalProgrammable", json.getJSONObject("controls").getJSONArray("clockSources").getJSONObject(0).getString("type"))
        assertEquals(raw.size * 2, json.getString("raw").length)
        assertTrue(json.getString("raw").startsWith("12010002"))
    }

    @Test
    fun uac1RatesAndAdaptiveEndpoint() {
        val config = UsbAudioDescriptors.parse(uac1()).configurations.single()
        assertEquals(1, config.uacVersion)
        assertEquals(0x0100, config.adcVersion)
        assertEquals(2, config.terminals[0].channels)
        val feature = config.featureUnits.single()
        assertTrue(feature.masterMute && feature.masterVolume)
        assertEquals(2, feature.channels)
        val alt1 = config.streams[1]
        assertEquals("pcm", UsbAudioDescriptors.formatName(alt1, 1))
        assertEquals(listOf(44100, 48000), alt1.rates)
        assertEquals(16, alt1.bits)
        assertEquals(2, alt1.subslotBytes)
        assertEquals(2, alt1.endpoints.single().syncType)
        assertEquals(196, alt1.endpoints.single().maxPacketSize)
        val alt2 = config.streams[2]
        assertNull(alt2.rates)
        assertEquals(listOf(8000, 96000), alt2.rateRange)
        assertEquals(24, alt2.bits)
    }

    @Test
    fun highBandwidthBitsAreSplitFromThePacketSize() {
        val raw = (device(0x0200, 1, 2) + configuration(1, 1) + iface(1, 1, 1, 0x02, 0x20) +
            d(0x05, 0x01, 0x05, *u16(0x0400 or (2 shl 11)), 1)).toByteArray()
        val endpoint = UsbAudioDescriptors.parse(raw).configurations.single().streams.single().endpoints.single()
        assertEquals(1024, endpoint.maxPacketSize)
        assertEquals(2, endpoint.extraTransactions)
    }

    @Test
    fun malformedInputNeverThrows() {
        assertTrue(UsbAudioDescriptors.parse(null).configurations.isEmpty())
        assertTrue(UsbAudioDescriptors.parse(ByteArray(0)).configurations.isEmpty())
        // A zero bLength stops the walk instead of looping.
        UsbAudioDescriptors.parse(byteArrayOf(0, 0, 0, 0))
        // Truncated descriptors at every cut of a real device.
        val full = sonyUac2()
        for (cut in full.indices) UsbAudioDescriptors.parse(full.copyOf(cut))
        // A bLength past the end, an unknown type, descriptors before any configuration.
        UsbAudioDescriptors.parse(byteArrayOf(40, 0x24, 0x02, 1))
        UsbAudioDescriptors.parse(bytes(3, 0x77, 1, 9, 0x24, 0x02, 1, 2, 3, 4, 5, 6, 7).toByteArray())
        // Random bytes.
        val random = java.util.Random(45)
        repeat(500) {
            val noise = ByteArray(random.nextInt(300)).also(random::nextBytes)
            UsbAudioDescriptors.deviceJson(noise)
        }
        // Too short class-specific descriptors are left out, the rest is still read.
        val short = (device(0x0200, 1, 2) + configuration(1, 2) + iface(0, 0, 0, 0x01, 0x20) +
            d(0x24, 0x06) + d(0x24, 0x0A, 0x29) + iface(1, 1, 1, 0x02, 0x20) + d(0x24, 0x02, 0x01) +
            d(0x05, 0x01, 0x05, *u16(512), 1)).toByteArray()
        val config = UsbAudioDescriptors.parse(short).configurations.single()
        assertTrue(config.featureUnits.isEmpty())
        assertTrue(config.clockSources.isEmpty())
        assertEquals(512, config.streams.single().endpoints.single().maxPacketSize)
    }

    @Test
    fun nonAudioInterfacesAreIgnored() {
        val raw = (device(0x0200, 1, 2) + configuration(1, 1) + d(0x04, 0, 0, 1, 0x03, 0, 0, 0) +
            d(0x05, 0x81, 0x03, *u16(8), 10)).toByteArray()
        val json = UsbAudioDescriptors.deviceJson(raw)
        assertTrue(json.isNull("uacVersion"))
        assertEquals(0, json.getJSONArray("streams").length())
    }
}
