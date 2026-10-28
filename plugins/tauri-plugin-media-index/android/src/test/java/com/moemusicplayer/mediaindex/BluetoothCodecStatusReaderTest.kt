package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperRequest
import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BluetoothCodecStatusReaderTest {
    private val address = "AA:BB:CC:DD:EE:FF"

    private class FakeScheduler : BluetoothRateScheduler {
        val tasks = ArrayList<Pair<Long, () -> Unit>>()
        override fun schedule(delayMs: Long, task: () -> Unit) { tasks.add(delayMs to task) }
        fun runNext() = tasks.removeAt(0).second()
    }

    private class FakeBackend(var answer: (HelperRequest) -> HelperResult) : BluetoothRateBackend {
        val calls = ArrayList<HelperRequest>()
        override fun run(request: HelperRequest, timeoutMs: Long): HelperResult {
            calls.add(request)
            return answer(request)
        }
    }

    private class Fixture(val backend: FakeBackend) {
        var now = 1_000L
        var source = MixerRateRules.Source.ROOT
        val known = HashSet<String>()
        val stored = ArrayList<HelperResult.State>()
        val scheduler = FakeScheduler()
        val env = object : BluetoothCodecStatusEnvironment {
            override fun nowMs() = now
            override fun supported() = true
            override fun source() = source
            override fun backend(source: MixerRateRules.Source) = if (source == MixerRateRules.Source.PLATFORM) null else backend
            override fun known(address: String) = address in known
            override fun store(address: String, state: HelperResult.State) { stored.add(state); known.add(address) }
            override fun log(message: String) {}
        }
        val reader = BluetoothCodecStatusReader(env, scheduler, BluetoothRateBreaker(8_000, 3))
    }

    private val state = HelperResult.State(4, 8, 15, listOf(1003, 0, 0, 0))

    @Test fun anUnknownCodecIsReadAfterTheFirstWaitAndStoredAtOnce() {
        val f = Fixture(FakeBackend { state })
        f.reader.request(address)
        assertEquals(listOf(500L), f.scheduler.tasks.map { it.first })
        f.scheduler.runNext()
        assertEquals(listOf<HelperRequest>(HelperRequest.State(address)), f.backend.calls)
        assertEquals(listOf(state), f.stored)
        assertTrue(f.scheduler.tasks.isEmpty())
    }

    @Test fun aKnownCodecIsNeverRead() {
        val f = Fixture(FakeBackend { state })
        f.known.add(address)
        f.reader.request(address)
        f.reader.request(address, rearm = true)
        assertTrue(f.scheduler.tasks.isEmpty())
        assertTrue(f.backend.calls.isEmpty())
    }

    @Test fun aCodecAnnouncedWhileWaitingIsNotReadEither() {
        val f = Fixture(FakeBackend { state })
        f.reader.request(address)
        f.known.add(address)
        f.scheduler.runNext()
        assertTrue(f.backend.calls.isEmpty())
    }

    @Test fun aFailedReadIsRetriedWithAGrowingWaitAndThenStops() {
        val f = Fixture(FakeBackend { HelperResult.Failed("no") })
        // A breaker that tolerates the four tries of this test.
        val reader = BluetoothCodecStatusReader(f.env, f.scheduler, BluetoothRateBreaker(8_000, 10))
        reader.request(address)
        val waits = ArrayList<Long>()
        while (f.scheduler.tasks.isNotEmpty()) {
            waits.add(f.scheduler.tasks.first().first)
            f.scheduler.runNext()
        }
        assertEquals(listOf(500L, 1_000L, 2_000L, 4_000L), waits)
        assertEquals(4, f.backend.calls.size)
        // Asking again without re-arming does not start over.
        reader.request(address)
        assertTrue(f.scheduler.tasks.isEmpty())
        // A connection or a route change re-arms the backoff.
        reader.request(address, rearm = true)
        assertEquals(listOf(500L), f.scheduler.tasks.map { it.first })
    }

    @Test fun aRouteChangeReReadsAfterTheCodecWasForgotten() {
        val f = Fixture(FakeBackend { state })
        f.reader.request(address)
        f.scheduler.runNext()
        f.known.remove(address)
        f.reader.request(address, rearm = true)
        f.scheduler.runNext()
        assertEquals(2, f.backend.calls.size)
    }

    @Test fun aForcedRefreshReadsAKnownCodecOnce() {
        val f = Fixture(FakeBackend { state })
        f.known.add(address)
        f.reader.request(address, force = true)
        f.scheduler.runNext()
        assertEquals(1, f.backend.calls.size)
        assertTrue(f.scheduler.tasks.isEmpty())
    }

    @Test fun aFailedForcedRefreshDoesNotLoopForever() {
        val f = Fixture(FakeBackend { HelperResult.Failed("no") })
        val reader = BluetoothCodecStatusReader(f.env, f.scheduler, BluetoothRateBreaker(8_000, 10))
        f.known.add(address)
        reader.request(address, force = true)
        var runs = 0
        while (f.scheduler.tasks.isNotEmpty() && runs < 20) {
            f.scheduler.runNext()
            runs++
        }
        assertEquals(4, runs)
    }

    @Test fun withoutRootOrShizukuNothingIsReadOrGuessed() {
        val f = Fixture(FakeBackend { state })
        f.source = MixerRateRules.Source.PLATFORM
        f.reader.request(address)
        f.scheduler.runNext()
        assertTrue(f.backend.calls.isEmpty())
        assertTrue(f.stored.isEmpty())
    }

    @Test fun aRunnerBugIsNotCountedAgainstTheBreaker() {
        val f = Fixture(FakeBackend { HelperResult.Failed("java.lang.IllegalArgumentException: process hasn't exited") })
        val breaker = BluetoothRateBreaker(8_000, 3)
        val reader = BluetoothCodecStatusReader(f.env, f.scheduler, breaker)
        reader.request(address)
        while (f.scheduler.tasks.isNotEmpty()) f.scheduler.runNext()
        assertFalse(breaker.tripped)
        // Real failures trip it after three.
        val g = Fixture(FakeBackend { HelperResult.Failed("denied") })
        val real = BluetoothRateBreaker(8_000, 3)
        val realReader = BluetoothCodecStatusReader(g.env, g.scheduler, real)
        realReader.request(address)
        while (g.scheduler.tasks.isNotEmpty()) g.scheduler.runNext()
        assertTrue(real.tripped)
        assertEquals(3, g.backend.calls.size)
    }

    @Test fun theReadStateBecomesTheCodecJsonOfThePathLine() {
        val facts = BluetoothCodecFacts(state.codecType, state.rateFlag, 0, 0, null)
        val json = BluetoothCodecRules.codecJson(facts)!!
        assertEquals("LDAC", json.getString("codec"))
        assertEquals(96_000, json.getInt("sampleRate"))
    }
}
