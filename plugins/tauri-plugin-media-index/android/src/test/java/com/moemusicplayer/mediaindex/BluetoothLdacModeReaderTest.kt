package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperRequest
import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BluetoothLdacModeReaderTest {
    private val address = "AA:BB:CC:DD:EE:FF"

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

    private class Fixture(val backend: FakeBackend) : BluetoothLdacModeEnvironment {
        var now = 100_000L
        var supported = true
        var source = MixerRateRules.Source.ROOT
        var codec: Int? = BluetoothCodecRules.CODEC_LDAC
        val modes = HashMap<String, Long>()
        val logs = ArrayList<String>()
        override fun nowMs() = now
        override fun supported() = supported
        override fun source() = source
        override fun backend(source: MixerRateRules.Source): BluetoothRateBackend? =
            if (source == MixerRateRules.Source.PLATFORM) null else backend
        override fun codecType(address: String) = codec
        override fun known(address: String) = modes.containsKey(address)
        override fun store(address: String, mode: Long) { modes[address] = mode }
        override fun log(message: String) { logs.add(message) }
    }

    private fun ldacState(mode: Long) = HelperResult.State(4, 8, 15, listOf(mode, 0L, 0L, 0L))

    private fun fixture(answer: (HelperRequest) -> HelperResult = { ldacState(1003) }) = Fixture(FakeBackend(answer))

    @Test
    fun theModeComesFromTheHelperStateAnswer() {
        assertEquals(1003L, BluetoothLdacModeRules.modeOf(ldacState(1003)))
        assertEquals(0L, BluetoothLdacModeRules.modeOf(ldacState(0)))
        assertNull("an older helper has no values", BluetoothLdacModeRules.modeOf(HelperResult.State(4, 8, 15)))
        assertNull("another codec has no LDAC mode", BluetoothLdacModeRules.modeOf(HelperResult.State(1, 2, 3, listOf(1000L, 0L, 0L, 0L))))
        assertNull(BluetoothLdacModeRules.modeOf(HelperResult.Failed("x")))
    }

    @Test
    fun onlyLdacWithASourceAndNoKnownModeIsRead() {
        val ldac = BluetoothCodecRules.CODEC_LDAC
        assertTrue(BluetoothLdacModeRules.wanted(ldac, false, true, true, false))
        assertFalse(BluetoothLdacModeRules.wanted(1, false, true, true, false))
        assertFalse(BluetoothLdacModeRules.wanted(null, false, true, true, false))
        assertFalse("known since the last announcement", BluetoothLdacModeRules.wanted(ldac, true, true, true, false))
        assertFalse("no root or Shizuku", BluetoothLdacModeRules.wanted(ldac, false, true, false, false))
        assertFalse("before Android 13", BluetoothLdacModeRules.wanted(ldac, false, false, true, false))
        assertFalse("stopped after failures", BluetoothLdacModeRules.wanted(ldac, false, true, true, true))
    }

    @Test
    fun aRequestReadsTheModeOnTheSchedulerThreadAndStoresIt() {
        val f = fixture()
        val scheduler = FakeScheduler()
        val reader = BluetoothLdacModeReader(f, scheduler)
        reader.request(address)
        assertTrue("nothing runs on the caller's thread", f.backend.calls.isEmpty())
        assertEquals(BluetoothLdacModeConfig.DEBOUNCE_MS, scheduler.tasks.single().first)
        scheduler.runAll()
        assertEquals(listOf<HelperRequest>(HelperRequest.State(address)), f.backend.calls)
        assertEquals(1003L, f.modes[address])
    }

    @Test
    fun aKnownModeIsNotReadAgainAtTheNextTrack() {
        val f = fixture()
        val scheduler = FakeScheduler()
        val reader = BluetoothLdacModeReader(f, scheduler)
        reader.request(address)
        scheduler.runAll()
        reader.request(address)
        assertTrue(scheduler.tasks.isEmpty())
        assertEquals(1, f.backend.calls.size)
        // A new announcement clears the mode (the watch does that); it is read again after the minimum interval.
        f.now += BluetoothLdacModeConfig.MIN_INTERVAL_MS
        f.modes.clear()
        reader.request(address)
        scheduler.runAll()
        assertEquals(2, f.backend.calls.size)
    }

    @Test
    fun requestsInABurstAreCoalescedAndSpacedByTheMinimumInterval() {
        val f = fixture()
        val scheduler = FakeScheduler()
        val reader = BluetoothLdacModeReader(f, scheduler)
        reader.request(address)
        reader.request(address)
        reader.request(address)
        assertEquals(1, scheduler.tasks.size)
        scheduler.runAll()
        assertEquals(1, f.backend.calls.size)
        f.modes.clear()
        f.now += 500L
        reader.request(address)
        // The next read waits for the rest of the minimum interval.
        assertEquals(BluetoothLdacModeConfig.MIN_INTERVAL_MS - 500L, scheduler.tasks.single().first)
    }

    @Test
    fun withoutASourceOrForOtherCodecsNothingIsCalled() {
        val f = fixture()
        val scheduler = FakeScheduler()
        val reader = BluetoothLdacModeReader(f, scheduler)
        f.source = MixerRateRules.Source.PLATFORM
        reader.request(address)
        scheduler.runAll()
        assertTrue(f.backend.calls.isEmpty())
        assertTrue(f.modes.isEmpty())
        f.source = MixerRateRules.Source.ROOT
        f.codec = BluetoothCodecRules.CODEC_AAC
        reader.request(address)
        assertTrue(scheduler.tasks.isEmpty())
        f.codec = BluetoothCodecRules.CODEC_LDAC
        f.supported = false
        reader.request(address)
        assertTrue(scheduler.tasks.isEmpty())
    }

    @Test
    fun repeatedFailuresStopTheReadingForTheProcess() {
        val f = fixture { HelperResult.Failed("profile-proxy: refused") }
        val scheduler = FakeScheduler()
        val reader = BluetoothLdacModeReader(f, scheduler)
        repeat(BluetoothLdacModeConfig.FAILURES_TO_STOP) {
            f.now += 10_000L
            reader.request(address)
            scheduler.runAll()
        }
        assertEquals(BluetoothLdacModeConfig.FAILURES_TO_STOP, f.backend.calls.size)
        assertTrue(f.modes.isEmpty())
        f.now += 10_000L
        reader.request(address)
        assertTrue(scheduler.tasks.isEmpty())
        assertEquals(BluetoothLdacModeConfig.FAILURES_TO_STOP, f.backend.calls.size)
    }
}
