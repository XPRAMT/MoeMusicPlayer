package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MixerRateDumpParsersTest {
    private fun thread(name: String, rate: String, rows: List<String>, extra: String = ""): String =
        "Output thread 0xb400007a6a5f6c00, name $name, tid 1234, type 0 (MIXER):\n" +
            "  I/O handle: 29\n" +
            "  Standby: no\n" +
            "  Sample rate: $rate\n" +
            "  HAL frame count: 960\n" +
            "  Channel count: 2\n" +
            "  Output device: 0x20 (AUDIO_DEVICE_OUT_BLUETOOTH_A2DP)\n" +
            "  Blocked in write: no\n" +
            "  ${rows.size} Tracks of which ${rows.size} are active\n" +
            "    Type     Id Active Client Session Port Id S  Flags   Format Chn mask  SRate ST Usg CT  G db  L dB  R dB  VS dB      Server FrmCnt  FrmRdy F Underruns  Flushed   Main Buf  Aux Buf\n" +
            rows.joinToString("\n") + "\n" + extra + "\n"

    private fun row(id: Int, active: String, port: Int, rate: Int) =
        "    F  $id    $active 4321  1000  $port A 0x000 00000005 00000003 $rate  3   1  2 0.00 -0.0 -0.0 -0.0  0x00000000   1920  960 y 0 0 0x0 0x0"

    private val dump = "Audio flinger dump\n" +
        thread("AudioOut_1D", "44100 Hz", listOf(row(0, "yes", 214, 44100))) +
        thread("AudioOut_2A", "48000 Hz", listOf(row(0, "yes", 215, 48000))) +
        thread("AudioOut_2F", "96000 Hz", listOf(row(1, "no", 301, 96000), row(2, "yes", 216, 96000)))

    @Test fun readsTheRateOfTheThreadThatHoldsThePortId() {
        assertEquals(44100, MixerRateDumpParsers.fromFlingerDump(dump, 214))
        assertEquals(48000, MixerRateDumpParsers.fromFlingerDump(dump, 215))
        assertEquals(96000, MixerRateDumpParsers.fromFlingerDump(dump, 216))
        assertEquals(96000, MixerRateDumpParsers.fromFlingerDump(dump, 301))
    }

    @Test fun aMissingRowIsNullNotAGuess() {
        assertNull(MixerRateDumpParsers.fromFlingerDump(dump, 999))
        assertNull(MixerRateDumpParsers.fromFlingerDump("", 214))
        assertNull(MixerRateDumpParsers.fromFlingerDump("garbage\nlines only", 214))
    }

    @Test fun localLogRowsThatRepeatRemovedTracksAreIgnored() {
        val logged = thread(
            "AudioOut_1D", "44100 Hz", listOf(row(0, "yes", 214, 44100)),
            "  Local log:\n    03-28 12:00:00.000 removeTrack_l: " + row(3, "no", 777, 44100).trim() + "\n" +
                "    Type Id Active Client Session Port Id S\n    " + row(3, "no", 777, 44100).trim(),
        )
        assertNull(MixerRateDumpParsers.fromFlingerDump(logged, 777))
        assertEquals(44100, MixerRateDumpParsers.fromFlingerDump(logged, 214))
    }

    @Test fun aTerminatedTracksTableIsIgnored() {
        val text = thread(
            "AudioOut_1D", "44100 Hz", listOf(row(0, "yes", 214, 44100)),
            "  Terminated tracks:\n    Type Id Active Client Session Port Id S Flags\n" + row(5, "no", 555, 44100),
        )
        assertNull(MixerRateDumpParsers.fromFlingerDump(text, 555))
    }

    @Test fun anInputThreadIsNotAMixerThread() {
        val text = "Input thread 0xb4, name AudioIn_1D, tid 1:\n  Sample rate: 16000 Hz\n    Type Id Active Client Session Port Id S\n    F 0 yes 1 2 500 A\n"
        assertNull(MixerRateDumpParsers.fromFlingerDump(text, 500))
    }

    @Test fun anActiveRowWinsAndDisagreeingInactiveRowsAreNotGuessed() {
        val twice = thread("AudioOut_1D", "44100 Hz", listOf(row(0, "no", 214, 44100))) +
            thread("AudioOut_2A", "48000 Hz", listOf(row(0, "yes", 214, 48000)))
        assertEquals(48000, MixerRateDumpParsers.fromFlingerDump(twice, 214))
        val inactive = thread("AudioOut_1D", "44100 Hz", listOf(row(0, "no", 214, 44100))) +
            thread("AudioOut_2A", "48000 Hz", listOf(row(0, "no", 214, 48000)))
        assertNull(MixerRateDumpParsers.fromFlingerDump(inactive, 214))
    }

    @Test fun otherRomsWithAnotherColumnOrderAndExtraColumnsStillWork() {
        val other = "Playback thread 0xabc, name AudioOut_77:\n" +
            "  Sample rate: 192000\n" +
            "    Id Active Client(pid/uid) Session SomethingNew Port Id Flags ExtraA ExtraB\n" +
            "    0 yes 111/10234 55 xyz 410 0x0 a b\n" +
            "    1 no 111/10234 56 xyz 411 0x0 a b\n"
        assertEquals(192000, MixerRateDumpParsers.fromFlingerDump(other, 410))
        assertEquals(192000, MixerRateDumpParsers.fromFlingerDump(other, 411))
    }

    @Test fun aNonNumericRateIsNull() {
        val bad = "Output thread 0x1, name AudioOut_1:\n  Sample rate: unknown\n    Type Id Active Client Session Port Id S\n    F 0 yes 1 2 214 A\n"
        assertNull(MixerRateDumpParsers.fromFlingerDump(bad, 214))
    }

    private val metrics = "Dumping audio analytics...\n" +
        "audio.track.214:\n" +
        "  event#=create threadId=29 sampleRate=44100 effective.sampleRate=44100 flags=0x0 routedDeviceId=44\n" +
        "audio.track.216 [2026-10-28 14:03:21] event#=create threadId=31 sampleRate=96000 flags=0x0\n" +
        "audio.thread.29:\n" +
        "  event#=create type=MIXER\n" +
        "  event#=readParameters sampleRate=44100 channelMask=3\n" +
        "audio.thread.31:\n" +
        "  event#=readParameters sampleRate=48000\n" +
        "  event#=readParameters sampleRate=96000\n" +
        "audio.track.300:\n" +
        "  event#=create sampleRate=44100\n"

    @Test fun metricsDumpFollowsTheTrackToItsThread() {
        assertEquals(44100, MixerRateDumpParsers.fromMetricsDump(metrics, 214))
        assertEquals(96000, MixerRateDumpParsers.fromMetricsDump(metrics, 216))
    }

    @Test fun metricsDumpWithoutThreadOrTrackIsNull() {
        assertNull(MixerRateDumpParsers.fromMetricsDump(metrics, 300))
        assertNull(MixerRateDumpParsers.fromMetricsDump(metrics, 1))
        assertNull(MixerRateDumpParsers.fromMetricsDump("", 1))
    }

    @Test fun effectiveSampleRateIsNotTheThreadRate() {
        val text = "audio.track.5:\n  event#=create threadId=7\naudio.thread.7:\n  event#=readParameters effective.sampleRate=11111\n"
        assertNull(MixerRateDumpParsers.fromMetricsDump(text, 5))
    }
}
