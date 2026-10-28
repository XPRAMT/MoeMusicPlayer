package com.moemusicplayer.mediaindex

import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.ToFloatPcmAudioProcessor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DsdPassthroughRulesTest {
    private val ok = DsdPassthroughRules.FilePlan(88_200, null)

    private fun decide(
        featureOn: Boolean = true,
        isDsd: Boolean = true,
        scheme: String? = "moedsd",
        exclusive: Boolean = true,
        routedToUsb: Boolean = true,
        permitted: Boolean = true,
        filePlan: DsdPassthroughRules.FilePlan = ok,
        deviceReason: String? = null,
        onFile: () -> Unit = {},
        onDevice: () -> Unit = {},
    ): DsdPassthroughRules.Decision = DsdPassthroughRules.decide(
        featureOn = featureOn,
        isDsd = isDsd,
        scheme = scheme,
        exclusive = exclusive,
        routedToUsb = routedToUsb,
        permitted = permitted,
        filePlan = { onFile(); filePlan },
        deviceReason = { onDevice(); deviceReason },
    )

    @Test
    fun aLocalStereoDsfOnAnExclusiveUsbDeviceThatOffersTheRateIsPlayedNatively() {
        assertEquals(DsdPassthroughRules.Decision.Raw(88_200), decide())
        assertEquals(
            DsdPassthroughRules.Decision.Raw(352_800),
            decide(filePlan = DsdPassthroughRules.FilePlan(352_800, null)),
        )
    }

    @Test
    fun theSettingOffOrANonDsdTrackSaysNothing() {
        assertEquals(DsdPassthroughRules.Decision.Off, decide(featureOn = false))
        assertEquals(DsdPassthroughRules.Decision.Off, decide(isDsd = false))
        // Not even the file or the device is looked at.
        var looked = false
        decide(featureOn = false, onFile = { looked = true }, onDevice = { looked = true })
        decide(isDsd = false, onFile = { looked = true }, onDevice = { looked = true })
        assertFalse(looked)
    }

    @Test
    fun everyConditionHasItsOwnReasonAndAnEarlierOneWins() {
        fun pcm(text: String) = DsdPassthroughRules.Decision.Pcm(text)
        assertEquals(pcm("只支援本機的 DSF 檔"), decide(scheme = "moewv"))
        assertEquals(pcm("只支援本機的 DSF 檔"), decide(scheme = "moeremote"))
        assertEquals(pcm("不是獨佔模式"), decide(exclusive = false))
        assertEquals(pcm("沒有 USB 音訊裝置"), decide(routedToUsb = false))
        assertEquals(pcm("未取得 USB 權限"), decide(permitted = false))
        assertEquals(
            pcm("不是雙聲道"),
            decide(filePlan = DsdPassthroughRules.FilePlan(0, "不是雙聲道")),
        )
        assertEquals(
            pcm("DSD512 超出 USB 音訊的取樣率上限"),
            decide(filePlan = DsdPassthroughRules.FilePlan(0, "DSD512 超出 USB 音訊的取樣率上限")),
        )
        assertEquals(pcm("音響沒有 DSD 取樣率 88.2 kHz"), decide(deviceReason = "音響沒有 DSD 取樣率 88.2 kHz"))
        assertEquals(pcm("音響沒有 DSD 直通格式"), decide(deviceReason = "音響沒有 DSD 直通格式"))
        // Order: scheme before mode before route before permission before file before device.
        assertEquals(pcm("只支援本機的 DSF 檔"), decide(scheme = null, exclusive = false, routedToUsb = false))
        assertEquals(pcm("不是獨佔模式"), decide(exclusive = false, routedToUsb = false, permitted = false))
        assertEquals(pcm("沒有 USB 音訊裝置"), decide(routedToUsb = false, permitted = false))
        assertEquals(
            pcm("未取得 USB 權限"),
            decide(permitted = false, filePlan = DsdPassthroughRules.FilePlan(0, "x"), deviceReason = "y"),
        )
        assertEquals(
            pcm("x"),
            decide(filePlan = DsdPassthroughRules.FilePlan(0, "x"), deviceReason = "y"),
        )
    }

    @Test
    fun theFileAndTheDeviceAreOnlyReadWhenEverythingBeforeThemPassed() {
        var file = 0
        var device = 0
        decide(exclusive = false, onFile = { file++ }, onDevice = { device++ })
        decide(routedToUsb = false, onFile = { file++ }, onDevice = { device++ })
        decide(permitted = false, onFile = { file++ }, onDevice = { device++ })
        assertEquals(0, file)
        assertEquals(0, device)
        decide(filePlan = DsdPassthroughRules.FilePlan(0, "no"), onFile = { file++ }, onDevice = { device++ })
        assertEquals(1, file)
        assertEquals(0, device)
        decide(onFile = { file++ }, onDevice = { device++ })
        assertEquals(2, file)
        assertEquals(1, device)
    }

    @Test
    fun theDeviceIsAskedAboutTheFilesFrameRate() {
        var asked = 0
        DsdPassthroughRules.decide(true, true, "moedsd", true, true, true, { DsdPassthroughRules.FilePlan(176_400, null) }) {
            asked = it
            null
        }
        assertEquals(176_400, asked)
    }

    @Test
    fun theRawUriReplacesOnlyTheSchemeOfALocalDsdUri() {
        assertEquals("moedsdraw:///storage/emulated/0/a.dsf", DsdPassthroughRules.rawUriOf("moedsd:///storage/emulated/0/a.dsf"))
        assertEquals("moedsdraw:///x/A%20B.dsf", DsdPassthroughRules.rawUriOf("moedsd:///x/A%20B.dsf"))
        assertNull(DsdPassthroughRules.rawUriOf("moewv:///x/a.wv"))
        assertNull(DsdPassthroughRules.rawUriOf("file:///x/a.dsf"))
        assertNull(DsdPassthroughRules.rawUriOf("moeremote://id/a.dsf"))
        assertNull(DsdPassthroughRules.rawUriOf("moedsdraw:///x/a.dsf"))
        // The raw scheme is one of the schemes the Rust data source serves, and maps back to the file.
        assertTrue(isRustServedScheme(DSD_RAW_URI_SCHEME))
        assertEquals("file:///x/a.dsf", fileUriOfConverted("moedsdraw:///x/a.dsf"))
    }

    @Test
    fun theFilePlanIsReadFromTheNativeAnswer() {
        assertEquals(
            DsdPassthroughRules.FilePlan(88_200, null),
            DsdPassthroughRules.parseFilePlan("""{"ok":true,"rate":88200,"name":"DSD64"}"""),
        )
        assertEquals(
            DsdPassthroughRules.FilePlan(0, "不是雙聲道"),
            DsdPassthroughRules.parseFilePlan("""{"ok":false,"reason":"不是雙聲道"}"""),
        )
        assertEquals("不支援直通", DsdPassthroughRules.parseFilePlan("""{"ok":false}""").reason)
        assertEquals("無法讀取 DSD 檔案", DsdPassthroughRules.parseFilePlan(null).reason)
        assertEquals("無法讀取 DSD 檔案", DsdPassthroughRules.parseFilePlan("not json").reason)
    }

    @Test
    fun theAppVolumeAndTheDuckingAreIgnoredForNativeDsdAndPassedOtherwise() {
        assertNull(DsdPassthroughRules.gainFor(true, 0.3f))
        assertNull(DsdPassthroughRules.gainFor(true, 0f))
        assertNull(DsdPassthroughRules.gainFor(true, 1f))
        assertEquals(0.3f, DsdPassthroughRules.gainFor(false, 0.3f))
        assertEquals(1f, DsdPassthroughRules.gainFor(false, 1f))
    }

    @Test
    fun aRawTrackNeverGetsAnythingButTheUsbOutput() {
        assertTrue(DsdPassthroughRules.mustFail(nativeDsd = true, usb = false))
        assertFalse(DsdPassthroughRules.mustFail(nativeDsd = true, usb = true))
        assertFalse(DsdPassthroughRules.mustFail(nativeDsd = false, usb = false))
        assertFalse(DsdPassthroughRules.mustFail(nativeDsd = false, usb = true))
    }

    @Test
    fun theCarrierIsSixteenBitFourChannels() {
        assertTrue(DsdPassthroughRules.isCarrier(UsbExclusiveRules.ENCODING_PCM_16BIT, 4))
        assertFalse(DsdPassthroughRules.isCarrier(UsbExclusiveRules.ENCODING_PCM_16BIT, 2))
        assertFalse(DsdPassthroughRules.isCarrier(UsbExclusiveRules.ENCODING_PCM_32BIT, 4))
        assertFalse(DsdPassthroughRules.isCarrier(UsbExclusiveRules.ENCODING_PCM_FLOAT, 4))
    }

    @Test
    fun theDiscardCoversMediaThreesTwentyMillisecondRampAtEveryRate() {
        for (rate in listOf(88_200, 176_400, 352_800)) {
            val ramp = rate * 20 / 1000
            val discard = DsdPassthroughRules.discardFrames(rate)
            assertTrue("$rate", discard > ramp)
            // 25 ms, rounded up to whole frames (two slots each), so a slot is never split.
            assertEquals((rate * 25 + 999) / 1000, discard)
        }
        assertEquals(2_205, DsdPassthroughRules.discardFrames(88_200))
    }

    @Test
    fun theReasonForNotUsingUsbNamesTheFirstMissingCondition() {
        val facts = UsbExclusiveRules.Facts(
            enabled = true,
            chosenKind = null,
            deviceAttached = true,
            permitted = true,
            encoding = UsbExclusiveRules.ENCODING_PCM_16BIT,
            trackFallback = null,
            unsupported = null,
        )
        assertNull(DsdPassthroughRules.notUsbReason(facts, UsbExclusiveRules.Choice.Usb))
        val track = UsbExclusiveRules.Choice.AudioTrack(null)
        assertEquals("不是獨佔模式", DsdPassthroughRules.notUsbReason(facts.copy(enabled = false), track))
        assertEquals("播放裝置不是 USB", DsdPassthroughRules.notUsbReason(facts.copy(chosenKind = "bluetooth"), track))
        assertEquals("沒有 USB 音訊裝置", DsdPassthroughRules.notUsbReason(facts.copy(deviceAttached = false), track))
        assertEquals(
            "未取得 USB 權限",
            DsdPassthroughRules.notUsbReason(facts.copy(permitted = false), UsbExclusiveRules.Choice.AudioTrack("未取得 USB 權限")),
        )
    }

    @Test
    fun theFloatConverterIsLeftOutOnlyWhileTheTrackIsNativeDsd() {
        var native = false
        val chain = FloatAwareAudioProcessorChain { native }
        val defaults = DefaultAudioSink.DefaultAudioProcessorChain().audioProcessors.size
        // A normal track (a genuine 88.2 kHz 16-bit PCM looks like the carrier, so the flag decides, not the format).
        assertEquals(defaults + 1, chain.audioProcessors.size)
        assertTrue(chain.audioProcessors.last() is ToFloatPcmAudioProcessor)
        native = true
        assertEquals(defaults, chain.audioProcessors.size)
        assertFalse(chain.audioProcessors.any { it is ToFloatPcmAudioProcessor })
        native = false
        assertEquals(defaults + 1, chain.audioProcessors.size)
    }

    @Test
    fun everyProcessorLeftInTheNativeChainIsInactiveForTheCarrier() {
        // No speed change, no silence skipping: the sink hands the output the bytes it was given (ADR 0049).
        val chain = FloatAwareAudioProcessorChain { true }
        val format = AudioProcessor.AudioFormat(88_200, 4, C.ENCODING_PCM_16BIT)
        for (processor in chain.audioProcessors) {
            processor.configure(format)
            assertFalse(processor.javaClass.simpleName, processor.isActive)
        }
    }
}
