package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperRequest
import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperResult
import com.moemusicplayer.mediaindex.BluetoothRateRules.Plan
import com.moemusicplayer.mediaindex.BluetoothRateRules.SkipReason
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BluetoothRateRulesTest {
    private val all = 1 or 2 or 4 or 8 or 16 or 32
    private val ldac = 1 or 2 or 4 or 8 // 44.1, 48, 88.2, 96
    private val sbc = 1 or 2
    private val aptxHd = 2

    private fun hz(flag: Int?) = flag?.let(BluetoothRateRules::hzOfFlag)

    @Test fun anExactlyOfferedRateIsTaken() {
        assertEquals(44_100, hz(BluetoothRateRules.pickRate(44_100, ldac)))
        assertEquals(96_000, hz(BluetoothRateRules.pickRate(96_000, ldac)))
        assertEquals(192_000, hz(BluetoothRateRules.pickRate(192_000, all)))
    }

    @Test fun otherwiseTheNearestRateOfTheSameFamilyWins() {
        // 176.4 kHz on LDAC: 88.2 kHz (same family) rather than 96 kHz.
        assertEquals(88_200, hz(BluetoothRateRules.pickRate(176_400, ldac)))
        // 192 kHz on LDAC: 96 kHz.
        assertEquals(96_000, hz(BluetoothRateRules.pickRate(192_000, ldac)))
        // 88.2 kHz on SBC (44.1 and 48 kHz): 44.1 kHz.
        assertEquals(44_100, hz(BluetoothRateRules.pickRate(88_200, sbc)))
        // 96 kHz on SBC: 48 kHz.
        assertEquals(48_000, hz(BluetoothRateRules.pickRate(96_000, sbc)))
    }

    @Test fun withoutAFamilyMemberTheNearestRateWins() {
        assertEquals(48_000, hz(BluetoothRateRules.pickRate(44_100, aptxHd)))
        assertEquals(48_000, hz(BluetoothRateRules.pickRate(88_200, aptxHd)))
        // 32 kHz is in the 48 kHz family; 22.05 kHz in the 44.1 kHz one.
        assertEquals(48_000, hz(BluetoothRateRules.pickRate(32_000, sbc)))
        assertEquals(44_100, hz(BluetoothRateRules.pickRate(22_050, sbc)))
        // An odd rate: the nearest overall.
        assertEquals(44_100, hz(BluetoothRateRules.pickRate(43_000, ldac)))
    }

    @Test fun aTieGoesToTheLowerRate() {
        // 46 050 is exactly between 44 100 and 48 000.
        assertEquals(44_100, hz(BluetoothRateRules.pickRate(46_050, sbc)))
    }

    @Test fun nothingIsPickedWithoutRatesOrTrackRate() {
        assertNull(BluetoothRateRules.pickRate(44_100, 0))
        assertNull(BluetoothRateRules.pickRate(0, ldac))
        assertNull(BluetoothRateRules.pickRate(-1, ldac))
    }

    private fun input(
        code: Boolean = true,
        setting: Boolean = true,
        supported: Boolean = true,
        source: MixerRateRules.Source = MixerRateRules.Source.ROOT,
        tripped: Boolean = false,
        address: String? = "AA:BB:CC:DD:EE:FF",
        usb: Boolean = false,
        dsd: Boolean = false,
        trackHz: Int? = 44_100,
        state: BluetoothCodecState? = BluetoothCodecState(4, 2, ldac),
    ) = BluetoothRateRules.Input(code, setting, supported, source, tripped, address, usb, dsd, trackHz, state)

    @Test fun aSwitchIsPlannedWhenTheTrackWantsAnotherRate() {
        assertEquals(Plan.Switch(1), BluetoothRateRules.plan(input()))
    }

    @Test fun noSwitchWhenTheCodecAlreadyRunsAtTheRate() {
        assertEquals(Plan.AlreadyOk(2), BluetoothRateRules.plan(input(trackHz = 48_000)))
        assertEquals(Plan.AlreadyOk(2), BluetoothRateRules.plan(input(trackHz = 32_000)))
    }

    @Test fun anUnknownCodecStateAsksTheHelper() {
        assertEquals(Plan.Ask, BluetoothRateRules.plan(input(state = null)))
    }

    @Test fun everySkipHasItsOwnReason() {
        fun reason(i: BluetoothRateRules.Input) = (BluetoothRateRules.plan(i) as Plan.Skip).reason
        assertEquals(SkipReason.DISABLED, reason(input(setting = false)))
        assertEquals(SkipReason.DISABLED, reason(input(code = false)))
        assertEquals(SkipReason.BREAKER, reason(input(tripped = true)))
        assertEquals(SkipReason.UNSUPPORTED, reason(input(supported = false)))
        assertEquals(SkipReason.NOT_BLUETOOTH, reason(input(address = null)))
        assertEquals(SkipReason.USB_EXCLUSIVE, reason(input(usb = true)))
        assertEquals(SkipReason.DSD, reason(input(dsd = true)))
        assertEquals(SkipReason.UNKNOWN_RATE, reason(input(trackHz = null)))
        assertEquals(SkipReason.UNKNOWN_RATE, reason(input(trackHz = 0)))
        assertEquals(SkipReason.NO_SOURCE, reason(input(source = MixerRateRules.Source.PLATFORM)))
        assertEquals(SkipReason.NO_RATE, reason(input(state = BluetoothCodecState(4, 2, 0))))
    }

    @Test fun aRestoreHappensOnlyWhileTheCodecStillRunsAtTheRateWeSet() {
        val saved = BluetoothRateRules.SavedRate(original = 2, applied = 1)
        assertTrue(BluetoothRateRules.shouldRestore(saved, BluetoothCodecState(4, 1, ldac)))
        assertTrue(BluetoothRateRules.shouldRestore(saved, null))
        // The user or the system changed it since: leave their choice.
        assertFalse(BluetoothRateRules.shouldRestore(saved, BluetoothCodecState(4, 8, ldac)))
        // Nothing to put back when we never moved it.
        assertFalse(BluetoothRateRules.shouldRestore(BluetoothRateRules.SavedRate(2, 2), null))
    }

    // ---- the helper protocol -----------------------------------------------------------------------------------

    @Test fun requestsRoundTripThroughTheirArguments() {
        for (request in listOf(
            HelperRequest.Follow("AA:BB:CC:DD:EE:FF", 96_000),
            HelperRequest.Set("AA:BB:CC:DD:EE:FF", 8),
            HelperRequest.State("AA:BB:CC:DD:EE:FF"),
        )) {
            assertEquals(request, BluetoothRateRules.parseRequest(BluetoothRateRules.requestArgs(request)))
        }
    }

    @Test fun badArgumentsAreRefused() {
        assertNull(BluetoothRateRules.parseRequest(listOf("follow", "not-an-address", "44100")))
        assertNull(BluetoothRateRules.parseRequest(listOf("follow", "AA:BB:CC:DD:EE:FF; reboot", "44100")))
        assertNull(BluetoothRateRules.parseRequest(listOf("follow", "AA:BB:CC:DD:EE:FF", "abc")))
        assertNull(BluetoothRateRules.parseRequest(listOf("set", "AA:BB:CC:DD:EE:FF", "3")))
        assertNull(BluetoothRateRules.parseRequest(listOf("format", "AA:BB:CC:DD:EE:FF", "1")))
        assertNull(BluetoothRateRules.parseRequest(listOf("follow", "AA:BB:CC:DD:EE:FF")))
    }

    @Test fun theShellCommandNamesTheHelperAndQuotesThePath() {
        val command = BluetoothRateRules.shellCommand("/data/app/x/base.apk", HelperRequest.Follow("AA:BB:CC:DD:EE:FF", 44_100))
        assertEquals(
            "CLASSPATH='/data/app/x/base.apk' app_process /system/bin com.moemusicplayer.mediaindex.BluetoothRateHelper follow AA:BB:CC:DD:EE:FF 44100",
            command,
        )
        assertNull(BluetoothRateRules.shellCommand("/data/app/x'; reboot; '/base.apk", HelperRequest.State("AA:BB:CC:DD:EE:FF")))
        assertNull(BluetoothRateRules.shellCommand("", HelperRequest.State("AA:BB:CC:DD:EE:FF")))
    }

    @Test fun resultsRoundTripAndTheLastResultLineWins() {
        for (result in listOf(
            HelperResult.Switched(4, 2, 1, true),
            HelperResult.Switched(0, 1, 1, false),
            HelperResult.State(4, 2, 15),
        )) {
            assertEquals(result, BluetoothRateRules.parseResult("log noise\n" + BluetoothRateRules.formatResult(result) + "\n"))
        }
        assertEquals(
            HelperResult.Failed("SecurityException: nope"),
            BluetoothRateRules.parseResult(BluetoothRateRules.formatResult(HelperResult.Failed("SecurityException: nope"))),
        )
        assertEquals(HelperResult.Failed("timeout"), BluetoothRateRules.parseResult(BluetoothRateRules.formatResult(HelperResult.TimedOut)))
    }

    @Test fun missingOrGarbledOutputIsAFailure() {
        assertTrue(BluetoothRateRules.parseResult("") is HelperResult.Failed)
        assertTrue(BluetoothRateRules.parseResult("RESULT OK 1 2") is HelperResult.Failed)
        assertTrue(BluetoothRateRules.parseResult("RESULT WHAT") is HelperResult.Failed)
        assertTrue(BluetoothRateRules.parseResult("Exception in thread main") is HelperResult.Failed)
    }

    // ---- the breaker -------------------------------------------------------------------------------------------

    @Test fun threeFailuresInARowTripTheBreakerAndASuccessResetsTheCount() {
        val breaker = BluetoothRateBreaker(8_000, 3)
        breaker.switchStarted(); breaker.switchEnded(0, false)
        breaker.switchStarted(); breaker.switchEnded(10, false)
        breaker.switchStarted(); breaker.switchEnded(20, true)
        breaker.switchStarted(); breaker.switchEnded(30, false)
        breaker.switchStarted(); breaker.switchEnded(40, false)
        assertFalse(breaker.tripped)
        breaker.switchStarted(); breaker.switchEnded(50, false)
        assertTrue(breaker.tripped)
    }

    @Test fun aSymptomSoonAfterASwitchTripsButALateOneDoesNot() {
        val breaker = BluetoothRateBreaker(8_000, 3)
        assertFalse(breaker.symptom(0, "player error"))
        breaker.switchStarted(); breaker.switchEnded(1_000, true)
        assertFalse(breaker.symptom(20_000, "player error"))
        assertFalse(breaker.tripped)
        assertTrue(breaker.symptom(5_000, "audio sink error"))
        assertTrue(breaker.tripped)
    }

    @Test
    fun helperIdentityMatchesTheUid() {
        assertEquals("root", BluetoothRateRules.packageForUid(0))
        assertEquals("android", BluetoothRateRules.packageForUid(1000))
        assertEquals("com.android.shell", BluetoothRateRules.packageForUid(2000))
    }

    @Test
    fun stepErrorsNameTheStep() {
        assertEquals("adapter: no Bluetooth adapter", BluetoothRateRules.stepError("adapter", "no Bluetooth adapter"))
        val parsed = BluetoothRateRules.parseResult(
            BluetoothRateRules.formatResult(BluetoothRateRules.HelperResult.Failed("codec-status: boom")),
        )
        assertEquals(BluetoothRateRules.HelperResult.Failed("codec-status: boom"), parsed)
    }

    @Test
    fun stateCommandPrintsReadableInfoAndStillParses() {
        val mask = BluetoothRateRules.RATE_44100 or BluetoothRateRules.RATE_48000 or BluetoothRateRules.RATE_96000
        val info = BluetoothRateRules.describeState(4, BluetoothRateRules.RATE_48000, mask)
        assertEquals("INFO codec=LDAC(4) sampleRate=48000 selectable=44100,48000,96000", info)
        val output = info + "\n" + BluetoothRateRules.formatResult(BluetoothRateRules.HelperResult.State(4, 2, mask))
        assertEquals(BluetoothRateRules.HelperResult.State(4, 2, mask), BluetoothRateRules.parseResult(output))
        assertEquals("INFO codec=SBC(0) sampleRate=flag 0 selectable=none", BluetoothRateRules.describeState(0, 0, 0))
    }
}
