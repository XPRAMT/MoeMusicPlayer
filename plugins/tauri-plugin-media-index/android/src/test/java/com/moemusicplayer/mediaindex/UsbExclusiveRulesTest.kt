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
    fun sharedModeANonUsbChoiceOrNoUsbDeviceTakeTheAudioTrackPathSilently() {
        val silent = UsbExclusiveRules.Choice.AudioTrack(null)
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(enabled = false)))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(enabled = false, chosenKind = null)))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(chosenKind = "bluetooth")))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(chosenKind = "speaker")))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(deviceAttached = false)))
        assertEquals(silent, UsbExclusiveRules.choose(usable.copy(chosenKind = null, deviceAttached = false)))
    }

    @Test
    fun theSystemDefaultInTheExclusiveModePlaysOnTheUsbDevice() {
        assertEquals(UsbExclusiveRules.Choice.Usb, UsbExclusiveRules.choose(usable.copy(chosenKind = null)))
        assertEquals(
            UsbExclusiveRules.Choice.AudioTrack("未取得 USB 權限"),
            UsbExclusiveRules.choose(usable.copy(chosenKind = null, permitted = false)),
        )
    }

    private val sonyUsb = UsbExclusiveRules.UsbDeviceFacts("/dev/bus/usb/001/002", "Sony Audio")
    private val ka3Usb = UsbExclusiveRules.UsbDeviceFacts("/dev/bus/usb/001/003", "KA3")
    private val sonyChoice = OutputDevicePreference("usb", "Sony Audio", null)
    private val speakerChoice = OutputDevicePreference("speaker", "", null)
    private val headsetChoice = OutputDevicePreference("bluetooth", "WH-1000XM5", "AA:BB:CC:DD:EE:FF")

    @Test
    fun theRoutingTable() {
        fun exclusive(chosen: OutputDevicePreference?, devices: List<UsbExclusiveRules.UsbDeviceFacts>) =
            UsbExclusiveRules.route(UsbExclusiveRules.Wish(true, chosen), devices)
        val shared = UsbExclusiveRules.Route.Shared(noUsbDevice = false)
        val noUsb = UsbExclusiveRules.Route.Shared(noUsbDevice = true)
        // System default with a USB audio device attached: exclusive on it, on the first one when there are several.
        assertEquals(UsbExclusiveRules.Route.Exclusive(sonyUsb), exclusive(null, listOf(sonyUsb)))
        assertEquals(UsbExclusiveRules.Route.Exclusive(sonyUsb), exclusive(null, listOf(sonyUsb, ka3Usb)))
        // A USB device, attached: exclusive on that one.
        assertEquals(UsbExclusiveRules.Route.Exclusive(sonyUsb), exclusive(sonyChoice, listOf(ka3Usb, sonyUsb)))
        // The speaker, Bluetooth, anything that is not USB: shared on it, whatever USB is attached.
        assertEquals(shared, exclusive(speakerChoice, listOf(sonyUsb)))
        assertEquals(shared, exclusive(headsetChoice, listOf(sonyUsb, ka3Usb)))
        assertEquals(shared, exclusive(headsetChoice, emptyList()))
        // The system default or a USB device with no USB audio device attached: shared as chosen, with the hint.
        assertEquals(noUsb, exclusive(null, emptyList()))
        assertEquals(noUsb, exclusive(sonyChoice, emptyList()))
        // The shared mode: shared, no USB at all.
        assertEquals(shared, UsbExclusiveRules.route(UsbExclusiveRules.Wish(false, null), listOf(sonyUsb)))
        assertEquals(shared, UsbExclusiveRules.route(UsbExclusiveRules.Wish(false, sonyChoice), listOf(sonyUsb)))
        assertEquals(sonyUsb.key, UsbExclusiveRules.target(UsbExclusiveRules.Wish(true, null), listOf(sonyUsb)))
        assertNull(UsbExclusiveRules.target(UsbExclusiveRules.Wish(true, headsetChoice), listOf(sonyUsb)))
    }

    @Test
    fun unpluggingThePlayingUsbDeviceHandsItBackAndGoesOnShared() {
        val wish = UsbExclusiveRules.Wish(true, null)
        assertTrue(UsbExclusiveRules.usbDevicesChanged(wish, listOf(sonyUsb), emptyList()))
        val target = UsbExclusiveRules.target(wish, emptyList())
        assertNull(target)
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = true, newOutput = true),
            UsbExclusiveRules.outputChange(liveOn = sonyUsb.key, claimed = sonyUsb.key, target = target, hasOutput = true),
        )
        // The USB output already failed and an AudioTrack plays: nothing more to re-create.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = false, newOutput = false),
            UsbExclusiveRules.outputChange(liveOn = null, claimed = null, target = target, hasOutput = true),
        )
    }

    @Test
    fun attachingAUsbDeviceInTheExclusiveModeGoesExclusiveNow() {
        val wish = UsbExclusiveRules.Wish(true, null)
        assertTrue(UsbExclusiveRules.usbDevicesChanged(wish, emptyList(), listOf(sonyUsb)))
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = false, newOutput = true),
            UsbExclusiveRules.outputChange(liveOn = null, claimed = null, target = sonyUsb.key, hasOutput = true),
        )
        // A second device attached behind the first: the first one stays.
        assertFalse(UsbExclusiveRules.usbDevicesChanged(wish, listOf(sonyUsb), listOf(sonyUsb, ka3Usb)))
        // Shared mode, or a non-USB device chosen: USB devices coming and going change nothing.
        assertFalse(UsbExclusiveRules.usbDevicesChanged(UsbExclusiveRules.Wish(false, null), emptyList(), listOf(sonyUsb)))
        assertFalse(UsbExclusiveRules.usbDevicesChanged(UsbExclusiveRules.Wish(true, headsetChoice), emptyList(), listOf(sonyUsb)))
        assertFalse(UsbExclusiveRules.usbDevicesChanged(wish, listOf(sonyUsb), listOf(sonyUsb)))
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
    fun aChangeOfTheModeOrDeviceReCreatesTheOutputOnlyWhereItShouldPlayChanges() {
        val usb = listOf(sonyUsb)
        fun change(
            before: UsbExclusiveRules.Wish,
            after: UsbExclusiveRules.Wish,
            hasOutput: Boolean = true,
            devices: List<UsbExclusiveRules.UsbDeviceFacts> = usb,
        ): UsbExclusiveRules.OutputChange {
            // What plays now follows the wish before (the USB output when it routed there).
            val now = UsbExclusiveRules.target(before, devices)
            return UsbExclusiveRules.outputChange(now, now, UsbExclusiveRules.target(after, devices), hasOutput)
        }
        val none = UsbExclusiveRules.OutputChange(handBack = false, newOutput = false)
        val both = UsbExclusiveRules.OutputChange(handBack = true, newOutput = true)
        val renew = UsbExclusiveRules.OutputChange(handBack = false, newOutput = true)
        fun wish(enabled: Boolean, chosen: OutputDevicePreference?) = UsbExclusiveRules.Wish(enabled, chosen)

        // To exclusive while a track plays on the USB device through Android: a new output, now the app's own.
        assertEquals(renew, change(wish(false, sonyChoice), wish(true, sonyChoice)))
        // To exclusive with the system default chosen and a USB device attached: the same.
        assertEquals(renew, change(wish(false, null), wish(true, null)))
        // To shared: the device goes back and the track goes on through an AudioTrack.
        assertEquals(both, change(wish(true, sonyChoice), wish(false, sonyChoice)))
        assertEquals(both, change(wish(true, null), wish(false, null)))
        // Exclusive, to the speaker or Bluetooth: at once to that device, shared; the USB device goes back.
        assertEquals(both, change(wish(true, sonyChoice), wish(true, speakerChoice)))
        assertEquals(both, change(wish(true, null), wish(true, headsetChoice)))
        // Exclusive, from Bluetooth to the system default or the USB device: exclusive now.
        assertEquals(renew, change(wish(true, headsetChoice), wish(true, null)))
        assertEquals(renew, change(wish(true, headsetChoice), wish(true, sonyChoice)))
        // Another USB device: hand back and re-create.
        val two = listOf(sonyUsb, ka3Usb)
        assertEquals(both, change(wish(true, sonyChoice), wish(true, OutputDevicePreference("usb", "KA3", null)), devices = two))
        // The system default and the USB device it plays on: the same device, nothing to do.
        assertEquals(none, change(wish(true, null), wish(true, sonyChoice)))
        assertEquals(none, change(wish(true, sonyChoice), wish(true, null)))
        // Nothing to do: shared to shared (the router re-pins the AudioTrack), exclusive without any USB device.
        assertEquals(none, change(wish(true, headsetChoice), wish(true, speakerChoice)))
        assertEquals(none, change(wish(false, headsetChoice), wish(true, headsetChoice)))
        assertEquals(none, change(wish(false, sonyChoice), wish(false, null)))
        assertEquals(none, change(wish(false, null), wish(true, null), devices = emptyList()))
        // No track loaded: the device still goes back, but there is no output to re-create.
        assertEquals(
            UsbExclusiveRules.OutputChange(handBack = true, newOutput = false),
            change(wish(true, sonyChoice), wish(false, sonyChoice), hasOutput = false),
        )
        assertEquals(none, change(wish(false, sonyChoice), wish(true, sonyChoice), hasOutput = false))
    }

    @Test
    fun theExclusiveDeviceOfAChoice() {
        assertEquals(sonyUsb, UsbExclusiveRules.pickExclusiveDevice(null, listOf(sonyUsb, ka3Usb)))
        assertEquals(ka3Usb, UsbExclusiveRules.pickExclusiveDevice(OutputDevicePreference("usb", "KA3", null), listOf(sonyUsb, ka3Usb)))
        assertNull(UsbExclusiveRules.pickExclusiveDevice(speakerChoice, listOf(sonyUsb)))
        assertNull(UsbExclusiveRules.pickExclusiveDevice(null, emptyList()))
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
        val facts = usable.copy(
            rate = 48_000,
            channels = 2,
            source = UsbExclusiveRules.Source(16, false),
            device = "/dev/bus/usb/002/002",
        )
        assertEquals(
            "USB exclusive: getOutputConfig → USB: every check passed; mode=exclusive chosen=usb " +
                "device=/dev/bus/usb/002/002 deviceAttached=true permitted=true encoding=4 rate=48000 channels=2 " +
                "sourceBits=16 dsd=false trackFallback=null unsupported=null",
            UsbExclusiveRules.decisionLog("getOutputConfig", facts, UsbExclusiveRules.choose(facts)),
        )
        val off = facts.copy(enabled = false)
        assertTrue(
            UsbExclusiveRules.decisionLog("getAudioOutput", off, UsbExclusiveRules.choose(off))
                .startsWith("USB exclusive: getAudioOutput → AudioTrack: shared mode; mode=shared"),
        )
        val unsupported = facts.copy(unsupported = "USB 音訊裝置不支援 705600 Hz")
        assertTrue(
            UsbExclusiveRules.decisionLog("getAudioOutput", unsupported, UsbExclusiveRules.choose(unsupported))
                .startsWith("USB exclusive: getAudioOutput → AudioTrack (USB 音訊裝置不支援 705600 Hz): the device cannot take the format;"),
        )
        // The reasons follow the order of the choice.
        val systemDefault = facts.copy(chosenKind = null)
        assertEquals("every check passed", UsbExclusiveRules.explain(systemDefault))
        assertTrue(
            UsbExclusiveRules.decisionLog("getAudioOutput", systemDefault, UsbExclusiveRules.choose(systemDefault))
                .contains("mode=exclusive chosen=system default device=/dev/bus/usb/002/002"),
        )
        assertEquals("chosen output is not USB (bluetooth), shared", UsbExclusiveRules.explain(facts.copy(chosenKind = "bluetooth")))
        assertEquals(
            "no USB audio device attached for system default, shared",
            UsbExclusiveRules.explain(facts.copy(chosenKind = null, deviceAttached = false)),
        )
        assertEquals("no USB audio device attached for usb, shared", UsbExclusiveRules.explain(facts.copy(deviceAttached = false)))
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
