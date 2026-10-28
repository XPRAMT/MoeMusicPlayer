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
    fun aChangeOfTheSwitchOrDeviceReCreatesTheOutputOnlyWhenUsbIsInvolved() {
        val sony = OutputDevicePreference("usb", "Sony Audio", null)
        val ka3 = OutputDevicePreference("usb", "KA3", null)
        val headset = OutputDevicePreference("bluetooth", "WH-1000XM5", "AA:BB:CC:DD:EE:FF")
        fun change(before: UsbExclusiveRules.Wish, after: UsbExclusiveRules.Wish, hasOutput: Boolean = true) =
            UsbExclusiveRules.outputChange(before, after, hasOutput)
        val none = UsbExclusiveRules.OutputChange(handBack = false, newOutput = false)

        // Switch on while a track plays on the USB device through Android: a new output, now the app's own.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = false, newOutput = true),
            change(UsbExclusiveRules.Wish(false, sony), UsbExclusiveRules.Wish(true, sony)),
        )
        // Switch off: the device goes back and the track goes on through an AudioTrack.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = true, newOutput = true),
            change(UsbExclusiveRules.Wish(true, sony), UsbExclusiveRules.Wish(false, sony)),
        )
        // Another USB device, or away from USB, with the switch on: hand back and re-create.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = true, newOutput = true),
            change(UsbExclusiveRules.Wish(true, sony), UsbExclusiveRules.Wish(true, ka3)),
        )
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = true, newOutput = true),
            change(UsbExclusiveRules.Wish(true, sony), UsbExclusiveRules.Wish(true, null)),
        )
        // To the USB device with the switch on.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = false, newOutput = true),
            change(UsbExclusiveRules.Wish(true, headset), UsbExclusiveRules.Wish(true, sony)),
        )
        // Nothing to do: no change, no USB involved (the router re-pins the AudioTrack), the switch for a non-USB choice.
        assertEquals(none, change(UsbExclusiveRules.Wish(true, sony), UsbExclusiveRules.Wish(true, sony)))
        assertEquals(none, change(UsbExclusiveRules.Wish(true, headset), UsbExclusiveRules.Wish(true, null)))
        assertEquals(none, change(UsbExclusiveRules.Wish(false, headset), UsbExclusiveRules.Wish(true, headset)))
        assertEquals(none, change(UsbExclusiveRules.Wish(false, sony), UsbExclusiveRules.Wish(false, ka3)))
        // No track loaded: the device still goes back, but there is no output to re-create.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = true, newOutput = false),
            change(UsbExclusiveRules.Wish(true, sony), UsbExclusiveRules.Wish(false, sony), hasOutput = false),
        )
        assertEquals(none, change(UsbExclusiveRules.Wish(false, sony), UsbExclusiveRules.Wish(true, sony), hasOutput = false))
    }

    @Test
    fun onlyPlainPcmConfigsCanGoToTheUsbOutput() {
        assertTrue(UsbExclusiveRules.takesConfig(UsbExclusiveRules.ENCODING_PCM_FLOAT, false, false, false))
        assertTrue(UsbExclusiveRules.takesConfig(UsbExclusiveRules.ENCODING_PCM_16BIT, false, false, false))
        assertFalse(UsbExclusiveRules.takesConfig(5, false, false, false))
        assertFalse(UsbExclusiveRules.takesConfig(UsbExclusiveRules.ENCODING_PCM_FLOAT, true, false, false))
        assertFalse(UsbExclusiveRules.takesConfig(UsbExclusiveRules.ENCODING_PCM_FLOAT, false, true, false))
        assertFalse(UsbExclusiveRules.takesConfig(UsbExclusiveRules.ENCODING_PCM_FLOAT, false, false, true))
    }

    @Test
    fun everyDecisionIsLoggedWithItsReasonAndFacts() {
        val facts = usable.copy(rate = 48_000, channels = 2, source = UsbExclusiveRules.Source(16, false))
        assertEquals(
            "USB exclusive: getOutputConfig → USB: every check passed; enabled=true chosenKind=usb " +
                "deviceAttached=true permitted=true encoding=4 rate=48000 channels=2 sourceBits=16 dsd=false " +
                "trackFallback=null unsupported=null",
            UsbExclusiveRules.decisionLog("getOutputConfig", facts, UsbExclusiveRules.choose(facts)),
        )
        val off = facts.copy(enabled = false)
        assertTrue(
            UsbExclusiveRules.decisionLog("getAudioOutput", off, UsbExclusiveRules.choose(off))
                .startsWith("USB exclusive: getAudioOutput → AudioTrack: switch off; enabled=false"),
        )
        val unsupported = facts.copy(unsupported = "USB 音訊裝置不支援 705600 Hz")
        assertTrue(
            UsbExclusiveRules.decisionLog("getAudioOutput", unsupported, UsbExclusiveRules.choose(unsupported))
                .startsWith("USB exclusive: getAudioOutput → AudioTrack (USB 音訊裝置不支援 705600 Hz): the device cannot take the format;"),
        )
        // The reasons follow the order of the choice.
        assertEquals("chosen output is not USB (system default)", UsbExclusiveRules.explain(facts.copy(chosenKind = null)))
        assertEquals("chosen output is not USB (bluetooth)", UsbExclusiveRules.explain(facts.copy(chosenKind = "bluetooth")))
        assertEquals("no matching USB audio device attached", UsbExclusiveRules.explain(facts.copy(deviceAttached = false)))
        assertEquals("no USB permission", UsbExclusiveRules.explain(facts.copy(permitted = false)))
        assertEquals("this track already fell back", UsbExclusiveRules.explain(facts.copy(trackFallback = "x")))
        assertEquals("encoding 5 is not linear PCM", UsbExclusiveRules.explain(facts.copy(encoding = 5)))
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
