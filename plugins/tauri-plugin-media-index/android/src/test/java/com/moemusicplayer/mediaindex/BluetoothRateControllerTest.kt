package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperRequest
import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperResult
import com.moemusicplayer.mediaindex.BluetoothRateRules.Plan
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BluetoothRateControllerTest {
    private val address = "AA:BB:CC:DD:EE:FF"
    private val ldac = 1 or 2 or 4 or 8

    private class FakeStore : BluetoothRateStore {
        val saved = LinkedHashMap<String, BluetoothRateRules.SavedRate>()
        override fun all(): Map<String, BluetoothRateRules.SavedRate> = LinkedHashMap(saved)
        override fun put(address: String, saved: BluetoothRateRules.SavedRate) { this.saved[address] = saved }
        override fun remove(address: String) { saved.remove(address) }
    }

    private class FakeScheduler : BluetoothRateScheduler {
        val tasks = ArrayList<Pair<Long, () -> Unit>>()
        override fun schedule(delayMs: Long, task: () -> Unit) { tasks.add(delayMs to task) }
        fun runAll() {
            while (tasks.isNotEmpty()) tasks.removeAt(0).second()
        }
    }

    private class FakeBackend(var answer: (HelperRequest) -> HelperResult) : BluetoothRateBackend {
        val calls = ArrayList<HelperRequest>()
        override fun run(request: HelperRequest, timeoutMs: Long): HelperResult {
            calls.add(request)
            return answer(request)
        }
    }

    private class Fixture(val backend: FakeBackend) {
        var now = 100_000L
        var enabled = true
        var source = MixerRateRules.Source.ROOT
        var state: BluetoothCodecState? = null
        var heard = true
        val awaited = ArrayList<Int>()
        val logs = ArrayList<String>()
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val env = object : BluetoothRateEnvironment {
            override fun nowMs() = now
            override fun settingEnabled() = enabled
            override fun androidSupported() = true
            override fun source() = source
            override fun backend(source: MixerRateRules.Source) = if (source == MixerRateRules.Source.PLATFORM) null else backend
            override fun codecState(address: String) = state
            override fun awaitRate(address: String, rateFlag: Int, timeoutMs: Long): Boolean {
                awaited.add(rateFlag)
                return heard
            }
            override fun log(message: String) { logs.add(message) }
        }
        val breaker = BluetoothRateBreaker(8_000, 3)
        val controller = BluetoothRateController(env, store, scheduler, breaker)
    }

    private fun request(hz: Int?, address: String? = this.address, usb: Boolean = false, dsd: Boolean = false) =
        BluetoothRateRequest(address, usb, dsd, hz)

    private fun ok(from: Int, to: Int, changed: Boolean = true) = HelperResult.Switched(4, from, to, changed)

    @Test fun aKnownStateSwitchesWithAnExplicitRateAndSavesTheOriginal() {
        val f = Fixture(FakeBackend { ok(2, 1) })
        f.state = BluetoothCodecState(4, 2, ldac)
        val plan = f.controller.process(request(44_100))
        assertEquals(Plan.Switch(1), plan)
        assertEquals(listOf<HelperRequest>(HelperRequest.Set(address, 1)), f.backend.calls)
        assertEquals(BluetoothRateRules.SavedRate(2, 1), f.store.saved[address])
        assertEquals(listOf(1), f.awaited)
    }

    @Test fun anUnknownStateLetsTheHelperDecide() {
        val f = Fixture(FakeBackend { ok(2, 8) })
        val plan = f.controller.process(request(96_000))
        assertEquals(Plan.Ask, plan)
        assertEquals(listOf<HelperRequest>(HelperRequest.Follow(address, 96_000)), f.backend.calls)
        assertEquals(BluetoothRateRules.SavedRate(2, 8), f.store.saved[address])
    }

    @Test fun nothingIsCalledWhenTheCodecAlreadyRunsAtTheRate() {
        val f = Fixture(FakeBackend { ok(2, 2) })
        f.state = BluetoothCodecState(4, 2, ldac)
        assertEquals(Plan.AlreadyOk(2), f.controller.process(request(48_000)))
        assertTrue(f.backend.calls.isEmpty())
        assertTrue(f.store.saved.isEmpty())
    }

    @Test fun theOriginalIsKeptFromTheFirstSwitch() {
        val f = Fixture(FakeBackend { request -> if ((request as HelperRequest.Set).rateFlag == 1) ok(2, 1) else ok(1, 8) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.controller.process(request(44_100))
        f.state = BluetoothCodecState(4, 1, ldac)
        f.controller.process(request(96_000))
        assertEquals(BluetoothRateRules.SavedRate(2, 8), f.store.saved[address])
    }

    @Test fun aHelperThatChangedNothingSavesNothing() {
        val f = Fixture(FakeBackend { ok(2, 2, changed = false) })
        f.controller.process(request(48_000))
        assertTrue(f.store.saved.isEmpty())
        assertTrue(f.awaited.isEmpty())
    }

    @Test fun skippedCasesNeverCallThePrivilegedBackend() {
        val f = Fixture(FakeBackend { ok(2, 1) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.controller.process(request(44_100, address = null))
        f.controller.process(request(44_100, usb = true))
        f.controller.process(request(44_100, dsd = true))
        f.controller.process(request(null))
        f.source = MixerRateRules.Source.PLATFORM
        f.controller.process(request(44_100))
        f.source = MixerRateRules.Source.ROOT
        f.enabled = false
        f.controller.process(request(44_100))
        assertTrue(f.backend.calls.isEmpty())
    }

    @Test fun aFailureLeavesEverythingAsIsAndThreeInARowStopTheFeature() {
        val f = Fixture(FakeBackend { HelperResult.Failed("SecurityException") })
        f.state = BluetoothCodecState(4, 2, ldac)
        repeat(3) { f.controller.process(request(44_100)) }
        assertTrue(f.store.saved.isEmpty())
        assertTrue(f.breaker.tripped)
        f.controller.process(request(44_100))
        assertEquals(3, f.backend.calls.size)
    }

    @Test fun aTimeoutCountsAsAFailure() {
        val f = Fixture(FakeBackend { HelperResult.TimedOut })
        f.controller.process(request(44_100))
        assertTrue(f.store.saved.isEmpty())
        assertFalse(f.breaker.tripped)
    }

    @Test fun anUnconfirmedChangeIsStillRemembered() {
        val f = Fixture(FakeBackend { ok(2, 1) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.heard = false
        f.controller.process(request(44_100))
        assertEquals(BluetoothRateRules.SavedRate(2, 1), f.store.saved[address])
        assertTrue(f.logs.any { it.contains("not confirmed") })
    }

    // ---- scheduling: once per track, spaced, coalesced ------------------------------------------------------------

    @Test fun requestsInAHurryAreCoalescedIntoTheNewestOne() {
        val f = Fixture(FakeBackend { ok(2, 1) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.controller.request(request(48_000))
        f.controller.request(request(96_000))
        f.controller.request(request(44_100))
        assertEquals(1, f.scheduler.tasks.size)
        f.scheduler.runAll()
        assertEquals(listOf<HelperRequest>(HelperRequest.Set(address, 1)), f.backend.calls)
    }

    @Test fun twoSwitchesStartAtLeastFiveSecondsApart() {
        val f = Fixture(FakeBackend { request -> ok(2, (request as HelperRequest.Set).rateFlag) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.controller.request(request(44_100))
        f.scheduler.runAll()
        assertEquals(1, f.backend.calls.size)
        // The next track comes 1 s later: scheduled 4 s from now, not at once.
        f.now += 1_000
        f.controller.request(request(96_000))
        assertEquals(4_000L, f.scheduler.tasks.single().first)
        // If the task fires early it waits for the rest instead of switching.
        f.now += 2_000
        f.scheduler.tasks.removeAt(0).second()
        assertEquals(1, f.backend.calls.size)
        assertEquals(2_000L, f.scheduler.tasks.single().first)
        f.now += 2_000
        f.scheduler.runAll()
        assertEquals(2, f.backend.calls.size)
    }

    @Test fun aSkippedTrackDoesNotUseUpTheInterval() {
        val f = Fixture(FakeBackend { ok(2, 1) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.controller.request(request(48_000))
        f.scheduler.runAll()
        f.now += 100
        f.controller.request(request(44_100))
        assertEquals(0L, f.scheduler.tasks.single().first)
    }

    @Test fun aDriftAfterTheSwitchIsNotCorrectedUntilTheNextTrack() {
        val f = Fixture(FakeBackend { ok(2, 1) })
        f.state = BluetoothCodecState(4, 2, ldac)
        f.controller.request(request(44_100))
        f.scheduler.runAll()
        // The user changes the rate in developer options: nothing is scheduled, nothing is called.
        f.state = BluetoothCodecState(4, 8, ldac)
        f.now += 60_000
        f.scheduler.runAll()
        assertEquals(1, f.backend.calls.size)
        // The next track plans from the new state.
        assertEquals(Plan.Switch(1), f.controller.process(request(44_100)))
    }

    // ---- restoring -------------------------------------------------------------------------------------------------

    @Test fun turningItOffPutsTheOriginalBack() {
        val f = Fixture(FakeBackend { request -> ok(1, (request as HelperRequest.Set).rateFlag) })
        f.store.put(address, BluetoothRateRules.SavedRate(2, 1))
        f.state = BluetoothCodecState(4, 1, ldac)
        f.controller.restoreAll("off")
        f.scheduler.runAll()
        assertEquals(listOf<HelperRequest>(HelperRequest.Set(address, 2)), f.backend.calls)
        assertTrue(f.store.saved.isEmpty())
    }

    @Test fun aRateTheUserChangedMeanwhileIsLeftAlone() {
        val f = Fixture(FakeBackend { ok(1, 2) })
        f.store.put(address, BluetoothRateRules.SavedRate(2, 1))
        f.state = BluetoothCodecState(4, 8, ldac)
        f.controller.restoreNow("off", null)
        assertTrue(f.backend.calls.isEmpty())
        assertTrue(f.store.saved.isEmpty())
    }

    @Test fun aFailedRestoreStaysSavedForTheNextTime() {
        val f = Fixture(FakeBackend { HelperResult.Failed("device not connected") })
        f.store.put(address, BluetoothRateRules.SavedRate(2, 1))
        f.controller.restoreNow("service ended", null)
        assertEquals(BluetoothRateRules.SavedRate(2, 1), f.store.saved[address])
    }

    @Test fun aDisconnectedDeviceGetsOneRestoreAttemptAndAReturnedDeviceIsRestoredWhenOff() {
        val f = Fixture(FakeBackend { HelperResult.Failed("gone") })
        f.store.put(address, BluetoothRateRules.SavedRate(2, 1))
        f.controller.devicesChanged(emptySet())
        f.controller.devicesChanged(emptySet())
        f.scheduler.runAll()
        assertEquals(1, f.backend.calls.size)
        // It comes back with the setting off: restored (this time it works).
        f.backend.answer = { ok(1, 2) }
        f.enabled = false
        f.controller.devicesChanged(setOf(address))
        f.scheduler.runAll()
        assertEquals(2, f.backend.calls.size)
        assertTrue(f.store.saved.isEmpty())
    }

    @Test fun aDeviceThatIsBackWhileFollowingKeepsItsSavedRate() {
        val f = Fixture(FakeBackend { ok(1, 2) })
        f.store.put(address, BluetoothRateRules.SavedRate(2, 1))
        f.controller.devicesChanged(setOf(address))
        f.scheduler.runAll()
        assertTrue(f.backend.calls.isEmpty())
        assertEquals(BluetoothRateRules.SavedRate(2, 1), f.store.saved[address])
    }

    @Test fun aTrippedBreakerStopsRestoresToo() {
        val f = Fixture(FakeBackend { ok(1, 2) })
        f.store.put(address, BluetoothRateRules.SavedRate(2, 1))
        f.breaker.switchStarted(); f.breaker.switchEnded(0, true)
        assertTrue(f.breaker.symptom(1_000, "player error"))
        f.controller.restoreNow("off", null)
        assertTrue(f.backend.calls.isEmpty())
    }
}
