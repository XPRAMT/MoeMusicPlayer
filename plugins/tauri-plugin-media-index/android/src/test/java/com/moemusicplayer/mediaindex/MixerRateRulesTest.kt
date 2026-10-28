package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.MixerRateRules.Panel
import com.moemusicplayer.mediaindex.MixerRateRules.RootState
import com.moemusicplayer.mediaindex.MixerRateRules.ShizukuState
import com.moemusicplayer.mediaindex.MixerRateRules.Source
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MixerRateRulesTest {
    private fun select(root: RootState, shizuku: ShizukuState, code: Boolean = true, setting: Boolean = true, tripped: Boolean = false) =
        MixerRateRules.selectSource(root, shizuku, code, setting, tripped)

    @Test fun rootComesFirstThenAGrantedShizukuThenThePlatformValue() {
        assertEquals(Source.ROOT, select(RootState.AVAILABLE, ShizukuState.GRANTED))
        assertEquals(Source.SHIZUKU, select(RootState.UNAVAILABLE, ShizukuState.GRANTED))
        assertEquals(Source.SHIZUKU, select(RootState.UNKNOWN, ShizukuState.GRANTED))
        assertEquals(Source.PLATFORM, select(RootState.UNKNOWN, ShizukuState.NOT_RUNNING))
        assertEquals(Source.PLATFORM, select(RootState.UNAVAILABLE, ShizukuState.NEEDS_PERMISSION))
        assertEquals(Source.PLATFORM, select(RootState.UNAVAILABLE, ShizukuState.NOT_INSTALLED))
        assertEquals(Source.PLATFORM, select(RootState.UNAVAILABLE, ShizukuState.DENIED))
    }

    @Test fun killSwitchesAndATrippedBreakerForcePlatform() {
        assertEquals(Source.PLATFORM, select(RootState.AVAILABLE, ShizukuState.GRANTED, code = false))
        assertEquals(Source.PLATFORM, select(RootState.AVAILABLE, ShizukuState.GRANTED, setting = false))
        assertEquals(Source.PLATFORM, select(RootState.AVAILABLE, ShizukuState.GRANTED, tripped = true))
    }

    @Test fun noDumpWhenUsbOrDsdOwnsTheDeviceOrThereIsNoPort() {
        assertTrue(MixerRateRules.mayDump(Source.ROOT, false, false, 214))
        assertFalse(MixerRateRules.mayDump(Source.ROOT, true, false, 214))
        assertFalse(MixerRateRules.mayDump(Source.SHIZUKU, false, true, 214))
        assertFalse(MixerRateRules.mayDump(Source.PLATFORM, false, false, 214))
        assertFalse(MixerRateRules.mayDump(Source.ROOT, false, false, null))
        assertFalse(MixerRateRules.mayDump(Source.ROOT, false, false, 0))
    }

    @Test fun theStatusShowsNothingWhilePendingAndThePlatformValueWhenNotUsedOrFailed() {
        val found = MixerRateRules.resolve(44100, MixerRateRules.Lookup.Found(96000, Source.ROOT))
        assertEquals(96000, found.rate)
        assertEquals(Source.ROOT, found.source)
        assertNull(MixerRateRules.resolve(44100, MixerRateRules.Lookup.Pending).rate)
        assertEquals(44100, MixerRateRules.resolve(44100, MixerRateRules.Lookup.Failed).rate)
        assertEquals(Source.PLATFORM, MixerRateRules.resolve(44100, MixerRateRules.Lookup.NotUsed).source)
        assertNull(MixerRateRules.resolve(null, MixerRateRules.Lookup.Failed).rate)
        assertNull(MixerRateRules.resolve(null, MixerRateRules.Lookup.NotUsed).source)
    }

    @Test fun serverDeathTextIsRecognised() {
        assertTrue(MixerRateRules.looksLikeServerDeath("Failure from system: DeadObjectException"))
        assertTrue(MixerRateRules.looksLikeServerDeath("AudioFlinger server died!"))
        assertTrue(MixerRateRules.looksLikeServerDeath("Can't find service: media.audio_flinger"))
        assertFalse(MixerRateRules.looksLikeServerDeath("Output thread 0x1, name AudioOut_1"))
    }

    @Test fun panelStatesFollowThePriority() {
        fun p(root: RootState, s: ShizukuState, setting: Boolean = true, tripped: Boolean = false) =
            MixerRateRules.panelState(root, s, true, setting, tripped)
        assertEquals(Panel.ROOT_ACTIVE, p(RootState.AVAILABLE, ShizukuState.NOT_INSTALLED))
        assertEquals(Panel.SHIZUKU_ACTIVE, p(RootState.UNKNOWN, ShizukuState.GRANTED))
        assertEquals(Panel.SHIZUKU_NEEDS_PERMISSION, p(RootState.UNKNOWN, ShizukuState.NEEDS_PERMISSION))
        assertEquals(Panel.SHIZUKU_DENIED, p(RootState.UNAVAILABLE, ShizukuState.DENIED))
        assertEquals(Panel.SHIZUKU_NOT_RUNNING, p(RootState.UNKNOWN, ShizukuState.NOT_RUNNING))
        assertEquals(Panel.SHIZUKU_NOT_RUNNING, p(RootState.UNKNOWN, ShizukuState.TOO_OLD))
        assertEquals(Panel.NONE, p(RootState.UNKNOWN, ShizukuState.NOT_INSTALLED))
        assertEquals(Panel.TRIPPED, p(RootState.AVAILABLE, ShizukuState.GRANTED, tripped = true))
        assertEquals(Panel.DISABLED, p(RootState.AVAILABLE, ShizukuState.GRANTED, setting = false))
    }

    @Test fun shizukuStateFollowsTheApiFacts() {
        fun s(installed: Boolean, alive: Boolean, old: Boolean = false, granted: Boolean = false, rationale: Boolean = false) =
            MixerRateRules.shizukuState(installed, alive, old, granted, rationale)
        assertEquals(ShizukuState.NOT_INSTALLED, s(false, false))
        assertEquals(ShizukuState.NOT_RUNNING, s(true, false))
        assertEquals(ShizukuState.TOO_OLD, s(true, true, old = true))
        assertEquals(ShizukuState.GRANTED, s(true, true, granted = true))
        assertEquals(ShizukuState.DENIED, s(true, true, rationale = true))
        assertEquals(ShizukuState.NEEDS_PERMISSION, s(true, true))
    }

    @Test fun theDefaultOrderAsksTheFlingerFirst() {
        assertEquals(listOf(MixerRateRules.Dump.FLINGER, MixerRateRules.Dump.METRICS), MixerRateConfig.DUMP_ORDER)
        assertEquals(MixerRateRules.Dump.METRICS, MixerRateRules.FLINGER_SECOND.first())
    }

    @Test fun theGateSpacesRunsAndCoalescesRequests() {
        val gate = DumpGate(5_000, 1_000)
        assertEquals(1_000L, gate.request(0))
        assertNull(gate.request(100)) // coalesced into the pending run
        assertTrue(gate.begin(1_000))
        assertNull(gate.request(1_500)) // arrives while running: one follow-up
        assertNull(gate.request(1_600))
        // the follow-up starts at least 5 s after the start of the first run (1_000 + 5_000), asked at 2_000
        assertEquals(4_000L, gate.finish(2_000))
        assertTrue(gate.begin(6_000))
        assertNull(gate.finish(6_500))
        // a request long after: only the settle time applies
        assertEquals(1_000L, gate.request(60_000))
    }

    @Test fun aRequestRightAfterARunWaitsForTheMinimumInterval() {
        val gate = DumpGate(5_000, 1_000)
        gate.request(0)
        gate.begin(1_000)
        gate.finish(1_100)
        // last start 1_000 + 5_000 = 6_000; asked at 1_200
        assertEquals(4_800L, gate.request(1_200))
    }

    @Test fun anAbortedRunLeavesNoTraceAndAllowsTheNextRequest() {
        val gate = DumpGate(5_000, 1_000)
        gate.request(0)
        gate.abort()
        assertEquals(1_000L, gate.request(100))
    }

    @Test fun aSymptomSoonAfterADumpTripsTheBreakerAndALateOneDoesNot() {
        val soon = DumpBreaker(8_000)
        soon.dumpStarted()
        soon.dumpEnded(1_000, "")
        assertFalse(soon.symptom(20_000, "late"))
        assertFalse(soon.tripped)
        assertTrue(soon.symptom(5_000, "player error"))
        assertTrue(soon.tripped)
        val none = DumpBreaker(8_000)
        assertFalse(none.symptom(1_000, "no dump yet"))
    }

    @Test fun aSymptomDuringADumpTrips() {
        val breaker = DumpBreaker(8_000)
        breaker.dumpStarted()
        assertTrue(breaker.symptom(1, "sink error"))
    }

    @Test fun aDeadServerInTheDumpOutputTripsAtOnce() {
        val breaker = DumpBreaker(8_000)
        breaker.dumpStarted()
        breaker.dumpEnded(10, "Failure from system")
        assertTrue(breaker.tripped)
    }

    @Test fun theCacheRemembersFailuresToo() {
        val cache = MixerRateCache()
        val key = cache.key(214, 44)
        assertFalse(cache.has(key))
        cache.put(key, null)
        assertTrue(cache.has(key))
        assertNull(cache.get(key))
        assertFalse(cache.has(cache.key(214, 45)))
        cache.put(key, 96000)
        assertEquals(96000, cache.get(key))
    }
}
