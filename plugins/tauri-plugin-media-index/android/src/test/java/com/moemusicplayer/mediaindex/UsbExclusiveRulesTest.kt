package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UsbExclusiveRulesTest {
    private val usable = UsbExclusiveRules.Facts(
        enabled = true,
        chosenKind = "usb",
        deviceAttached = true,
        permitted = true,
        encoding = UsbExclusiveRules.ENCODING_PCM_FLOAT,
        trackFallback = null,
        unsupported = null,
    )

    private fun usbFacts(id: Int, name: String) = OutputDeviceFacts(
        id = id,
        type = AudioOutputRules.TYPE_USB_DEVICE,
        productName = name,
        address = "card=1;device=0",
        sampleRates = listOf(44_100, 48_000),
        channelCounts = listOf(2),
        encodings = emptyList(),
    )

    private val speaker = OutputDeviceFacts(2, AudioOutputRules.TYPE_BUILTIN_SPEAKER, "Phone", "", emptyList(), emptyList(), emptyList())

    @Test
    fun theUsbOutputIsChosenOnlyWhenEverythingAllowsIt() {
        assertEquals(UsbExclusiveRules.Choice.Usb, UsbExclusiveRules.choose(usable))
        for (encoding in listOf(2, 4, 21, 22)) {
            assertEquals(UsbExclusiveRules.Choice.Usb, UsbExclusiveRules.choose(usable.copy(encoding = encoding)))
        }
    }

    @Test
    fun withoutTheSwitchOrAChosenUsbDeviceTheAudioTrackPathIsSilent() {
        val silent = UsbExclusiveRules.Choice.AudioTrack(null)
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(enabled = false)))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(chosenKind = null)))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(chosenKind = "bluetooth")))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(deviceAttached = false)))
    }

    @Test
    fun aFallbackSaysWhy() {
        assertEquals(
            UsbExclusiveRules.Choice.AudioTrack("未取得 USB 權限"),
            UsbExclusiveRules.choose(usable.copy(permitted = false)),
        )
        assertEquals(
            UsbExclusiveRules.Choice.AudioTrack("USB 音訊裝置不支援 705600 Hz"),
            UsbExclusiveRules.choose(usable.copy(trackFallback = "USB 音訊裝置不支援 705600 Hz")),
        )
        assertEquals(
            UsbExclusiveRules.Choice.AudioTrack("不是 PCM 音訊"),
            UsbExclusiveRules.choose(usable.copy(encoding = 5)),
        )
        assertEquals(
            UsbExclusiveRules.Choice.AudioTrack("USB 音訊裝置沒有 6 聲道的 PCM 格式"),
            UsbExclusiveRules.choose(usable.copy(unsupported = "USB 音訊裝置沒有 6 聲道的 PCM 格式")),
        )
    }

    @Test
    fun theUsbDeviceIsFoundByItsProductNameOrAsTheOnlyOne() {
        val sony = UsbExclusiveRules.UsbDeviceFacts("/dev/bus/usb/001/002", "Sony Audio")
        val other = UsbExclusiveRules.UsbDeviceFacts("/dev/bus/usb/001/003", "KA3")
        assertEquals(sony, UsbExclusiveRules.pickDevice(" sony audio ", listOf(other, sony)))
        assertEquals(other, UsbExclusiveRules.pickDevice("USB-Audio - KA3", listOf(other)))
        assertNull(UsbExclusiveRules.pickDevice("Unknown", listOf(other, sony)))
        assertNull(UsbExclusiveRules.pickDevice("Sony Audio", emptyList()))
        val unnamed = UsbExclusiveRules.UsbDeviceFacts("/dev/bus/usb/001/004", null)
        assertEquals(unnamed, UsbExclusiveRules.pickDevice("Sony Audio", listOf(unnamed)))
    }

    @Test
    fun theHeldDeviceStaysListedAndKeepsItsPin() {
        val held = usbFacts(31, "Sony Audio")
        val preference = AudioOutputRules.identityOf(held)!!
        // Android removed it while the app drives it: it is listed as it was.
        assertEquals(listOf(speaker, held), UsbExclusiveRules.withHeld(listOf(speaker), held))
        assertEquals(held, AudioOutputRules.find(preference, UsbExclusiveRules.withHeld(listOf(speaker), held)))
        // Back with Android (another id): not listed twice.
        val back = usbFacts(40, "Sony Audio")
        assertEquals(listOf(speaker, back), UsbExclusiveRules.withHeld(listOf(speaker, back), held))
        assertEquals(listOf(speaker), UsbExclusiveRules.withHeld(listOf(speaker), null))
        assertTrue(UsbExclusiveRules.keepsPin(preference, held))
        assertFalse(UsbExclusiveRules.keepsPin(preference, null))
        assertFalse(UsbExclusiveRules.keepsPin(null, held))
        assertFalse(UsbExclusiveRules.keepsPin(OutputDevicePreference("usb", "KA3", null), held))
    }

    @Test
    fun ringSizeAndPositionMath() {
        assertEquals(11_025, UsbExclusiveRules.ringFrames(44_100))
        assertEquals(96_000, UsbExclusiveRules.ringFrames(384_000))
        assertEquals(1, UsbExclusiveRules.ringFrames(0))
        assertEquals(1_000_000L, UsbExclusiveRules.framesToUs(44_100, 44_100))
        assertEquals(22L, UsbExclusiveRules.framesToUs(1, 44_100))
        assertEquals(0L, UsbExclusiveRules.framesToUs(100, 0))
        assertEquals(111_848_106_640L, UsbExclusiveRules.framesToUs(42_949_672_950L, 384_000))
    }

    @Test
    fun theSourceKeepsOnlyRealBitDepths() {
        assertEquals(UsbExclusiveRules.Source(16, false), UsbExclusiveRules.sourceOf(16, false))
        assertEquals(UsbExclusiveRules.Source(0, false), UsbExclusiveRules.sourceOf(0, false))
        assertEquals(UsbExclusiveRules.Source(0, false), UsbExclusiveRules.sourceOf(null, false))
        assertEquals(UsbExclusiveRules.Source(0, true), UsbExclusiveRules.sourceOf(1, true))
        assertEquals(UsbExclusiveRules.Source(0, false), UsbExclusiveRules.sourceOf(64, false))
    }
}
