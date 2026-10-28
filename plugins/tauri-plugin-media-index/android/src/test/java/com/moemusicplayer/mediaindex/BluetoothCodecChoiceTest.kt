package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.BluetoothCodecChoiceRules.Plan
import com.moemusicplayer.mediaindex.BluetoothCodecChoiceRules.SkipReason
import com.moemusicplayer.mediaindex.BluetoothCodecChoiceRules.Trigger
import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperRequest
import com.moemusicplayer.mediaindex.BluetoothRateRules.HelperResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BluetoothCodecChoiceTest {
    private fun offer(key: String, priority: Long = 0, rate: Int = 0, bits: Int = 0, channel: Int = 0) =
        CodecOffer(key, "codec $key", priority, rate, bits, channel)

    // The phone's codecs in the stack's order: LDAC, aptX HD, AAC, SBC.
    private val local = listOf(offer("t4", 400), offer("t3", 300), offer("t1", 200), offer("t0", 100))

    @Test fun keysAreTypesOrExtendedIds() {
        assertEquals("t4", BluetoothCodecChoiceRules.keyOf(4, null))
        assertEquals("t0", BluetoothCodecChoiceRules.keyOf(0, 99L))
        assertEquals("x123", BluetoothCodecChoiceRules.keyOf(BluetoothCodecRules.CODEC_INVALID, 123L))
        assertNull(BluetoothCodecChoiceRules.keyOf(BluetoothCodecRules.CODEC_INVALID, null))
        assertTrue(BluetoothCodecChoiceRules.validKey("x123"))
        assertFalse(BluetoothCodecChoiceRules.validKey("4"))
        assertFalse(BluetoothCodecChoiceRules.validKey("t4; rm"))
    }

    @Test fun theStacksOrderIsTheLargerPriorityFirstAndDuplicatesAreDropped() {
        val order = BluetoothCodecChoiceRules.deviceOrder(listOf(offer("t0", 1), offer("t4", 9), offer("t0", 5), offer("t1", 9)))
        assertEquals(listOf("t4", "t1", "t0"), order.map { it.key })
    }

    @Test fun firstUseShowsTheStacksOrderAndAStoredOrderWinsWithNewCodecsAtTheEnd() {
        assertEquals(listOf("t4", "t3", "t1", "t0"), BluetoothCodecChoiceRules.shown(null, local).map { it.key })
        assertEquals(listOf("t1", "t4", "t3", "t0"), BluetoothCodecChoiceRules.shown(listOf("t1", "t4"), local).map { it.key })
        // A stored codec the phone does not list is skipped.
        assertEquals(listOf("t1", "t4", "t3", "t0"), BluetoothCodecChoiceRules.shown(listOf("x9", "t1", "t4"), local).map { it.key })
    }

    @Test fun reorderingKeepsUnlistedCodecsInTheirPlace() {
        val reordered = BluetoothCodecChoiceRules.reorder(listOf("x9", "t4", "t3"), local, listOf("t0", "t1", "t3", "t4"))
        assertEquals(listOf("x9", "t0", "t1", "t3", "t4"), reordered)
        assertEquals(listOf("t1", "t4", "t3", "t0"), BluetoothCodecChoiceRules.reorder(null, local, listOf("t1", "t4", "t3", "t0")))
    }

    @Test fun theTargetIsTheFirstListedCodecTheHeadsetSupports() {
        val headset = listOf(offer("t1"), offer("t0"))
        assertEquals("t1", BluetoothCodecChoiceRules.target(null, local, headset))
        assertEquals("t0", BluetoothCodecChoiceRules.target(listOf("t0", "t1"), local, headset))
        assertEquals("t1", BluetoothCodecChoiceRules.target(listOf("t4", "t3", "t1"), local, headset))
        assertNull(BluetoothCodecChoiceRules.target(null, local, listOf(offer("t2"))))
        assertNull(BluetoothCodecChoiceRules.target(null, local, emptyList()))
    }

    @Test fun aSwitchKeepsTheCurrentValuesWhenTheNewCodecOffersThemAndOtherwiseTheNearest() {
        // 96 kHz (8) 24 bit (2) stereo (2) to a codec that offers 44.1/48 kHz (3), 16/24 bit (3) and stereo.
        val values = BluetoothCodecChoiceRules.switchValues(8, 2, 2, offer("t1", rate = 3, bits = 3, channel = 2))
        assertEquals(BluetoothCodecChoiceRules.SwitchValues(2, 2, 2), values)
        // 44.1 kHz stays when offered; 32 bit falls to the best the codec has; mono falls to stereo.
        val kept = BluetoothCodecChoiceRules.switchValues(1, 4, 1, offer("t0", rate = 3, bits = 1, channel = 2))
        assertEquals(BluetoothCodecChoiceRules.SwitchValues(1, 1, 2), kept)
        // A codec that reports nothing keeps everything.
        assertEquals(BluetoothCodecChoiceRules.SwitchValues(8, 2, 2), BluetoothCodecChoiceRules.switchValues(8, 2, 2, offer("t9")))
    }

    @Test fun theHelperWireFormatRoundTrips() {
        val request = HelperRequest.SetCodec("AA:BB:CC:DD:EE:FF", "x123")
        assertEquals(request, BluetoothRateRules.parseRequest(BluetoothRateRules.requestArgs(request)))
        assertEquals(HelperRequest.Codecs("AA:BB:CC:DD:EE:FF"), BluetoothRateRules.parseRequest(listOf("codecs", "aa:bb:cc:dd:ee:ff", "0")))
        assertNull(BluetoothRateRules.parseRequest(listOf("codec", "AA:BB:CC:DD:EE:FF", "t4;id")))
        assertNull(BluetoothRateRules.shellCommand("/x'y/base.apk", request))
        assertTrue(BluetoothRateRules.shellCommand("/data/app/base.apk", request)!!.endsWith("codec AA:BB:CC:DD:EE:FF x123"))

        val codecs = HelperResult.Codecs(
            "t4",
            listOf(CodecOffer("t4", "LDAC", 4, 15, 7, 2), CodecOffer("x77", "LHDC V5", 1, 63, 2, 2)),
            listOf(CodecOffer("t4", "LDAC", 4, 15, 7, 2)),
        )
        assertEquals(codecs, BluetoothRateRules.parseResult(BluetoothRateRules.formatResult(codecs)))
        val none = HelperResult.Codecs(null, emptyList(), emptyList())
        assertEquals(none, BluetoothRateRules.parseResult(BluetoothRateRules.formatResult(none)))
        val switched = HelperResult.CodecSwitched("t1", "t4", true)
        assertEquals(switched, BluetoothRateRules.parseResult(BluetoothRateRules.formatResult(switched)))
        assertTrue(BluetoothRateRules.parseResult("RESULT CODEC t1 t4 maybe") is HelperResult.Failed)
        assertTrue(BluetoothRateRules.parseResult("RESULT CODECS t4 broken -") is HelperResult.Failed)
    }

    // ---- the plan ---------------------------------------------------------------------------------------------------

    private fun input(
        choices: BluetoothCodecChoices? = BluetoothCodecChoices("t1", listOf(offer("t4"), offer("t1")), local),
        stored: List<String>? = listOf("t4"),
        trigger: Trigger = Trigger.USER,
        enabled: Boolean = true,
        source: MixerRateRules.Source = MixerRateRules.Source.ROOT,
        tripped: Boolean = false,
        supported: Boolean = true,
    ) = BluetoothCodecChoiceRules.Input(true, enabled, supported, tripped, source, stored, choices, trigger)

    @Test fun thePlanSwitchesToTheTargetUnlessItIsAlreadyInUse() {
        assertEquals(Plan.Switch("t4"), BluetoothCodecChoiceRules.plan(input()))
        assertEquals(
            Plan.AlreadyOk("t1"),
            BluetoothCodecChoiceRules.plan(input(choices = BluetoothCodecChoices("t1", listOf(offer("t1")), local), stored = listOf("t1"))),
        )
    }

    @Test fun nothingHappensWhenTheSwitchIsOffWithoutASourceOrAfterTheBreaker() {
        assertEquals(Plan.Skip(SkipReason.DISABLED), BluetoothCodecChoiceRules.plan(input(enabled = false)))
        assertEquals(Plan.Skip(SkipReason.NO_SOURCE), BluetoothCodecChoiceRules.plan(input(source = MixerRateRules.Source.PLATFORM)))
        assertEquals(Plan.Skip(SkipReason.BREAKER), BluetoothCodecChoiceRules.plan(input(tripped = true)))
        assertEquals(Plan.Skip(SkipReason.UNSUPPORTED), BluetoothCodecChoiceRules.plan(input(supported = false)))
    }

    @Test fun aConnectionWithoutAStoredOrderChangesNothingButTheUserActionDoes() {
        assertEquals(Plan.Skip(SkipReason.NO_STORED_ORDER), BluetoothCodecChoiceRules.plan(input(stored = null, trigger = Trigger.CONNECT)))
        assertEquals(Plan.Switch("t4"), BluetoothCodecChoiceRules.plan(input(stored = null, trigger = Trigger.USER)))
    }

    @Test fun unknownCodecsAreReadFirstAndAReadOnlyRequestNeverSwitches() {
        assertEquals(Plan.NeedOffers, BluetoothCodecChoiceRules.plan(input(choices = null)))
        assertEquals(Plan.Skip(SkipReason.READ_ONLY), BluetoothCodecChoiceRules.plan(input(trigger = Trigger.READ)))
        assertEquals(
            Plan.Skip(SkipReason.NO_OFFERS),
            BluetoothCodecChoiceRules.plan(input(choices = BluetoothCodecChoices("t1", listOf(offer("t2")), local))),
        )
    }

    @Test fun aSavedOriginalIsRestoredOnlyWhileTheCodecIsStillTheOneWeSet() {
        val saved = BluetoothCodecChoiceRules.SavedCodec("t1", "t4")
        assertTrue(BluetoothCodecChoiceRules.shouldRestore(saved, "t4"))
        assertTrue(BluetoothCodecChoiceRules.shouldRestore(saved, null))
        assertFalse(BluetoothCodecChoiceRules.shouldRestore(saved, "t0"))
        assertFalse(BluetoothCodecChoiceRules.shouldRestore(BluetoothCodecChoiceRules.SavedCodec("t1", "t1"), "t1"))
    }

    @Test fun thePrefsAreReadLeniently() {
        assertEquals(BluetoothCodecPrefs(true, listOf("t4", "x7")), BluetoothCodecPrefs.parse("""{"enabled":true,"order":["t4","bad","x7","t4"]}"""))
        assertEquals(BluetoothCodecPrefs.OFF, BluetoothCodecPrefs.parse("not json"))
        assertEquals(BluetoothCodecPrefs.OFF, BluetoothCodecPrefs.parse(null))
    }

    // ---- the controller ---------------------------------------------------------------------------------------------

    private class FakeStore : BluetoothCodecStore {
        val saved = LinkedHashMap<String, BluetoothCodecChoiceRules.SavedCodec>()
        override fun all(): Map<String, BluetoothCodecChoiceRules.SavedCodec> = LinkedHashMap(saved)
        override fun put(address: String, saved: BluetoothCodecChoiceRules.SavedCodec) { this.saved[address] = saved }
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

    private val address = "AA:BB:CC:DD:EE:FF"

    private inner class Fixture(val backend: FakeBackend) {
        var now = 100_000L
        var prefs = BluetoothCodecPrefs(true, listOf("t4"))
        var source = MixerRateRules.Source.ROOT
        val known = HashMap<String, BluetoothCodecChoices>()
        val failures = ArrayList<String?>()
        val logs = ArrayList<String>()
        val awaited = ArrayList<String>()
        val store = FakeStore()
        val scheduler = FakeScheduler()
        val env = object : BluetoothCodecChoiceEnvironment {
            override fun nowMs() = now
            override fun supported() = true
            override fun source() = source
            override fun backend(source: MixerRateRules.Source) = if (source == MixerRateRules.Source.PLATFORM) null else backend
            override fun prefs() = prefs
            override fun choices(address: String) = known[address]
            override fun storeChoices(address: String, choices: BluetoothCodecChoices) { known[address] = choices }
            override fun awaitCodec(address: String, key: String, timeoutMs: Long): Boolean { awaited.add(key); return true }
            override fun reportFailure(address: String, text: String?) { failures.add(text) }
            override fun log(message: String) { logs.add(message) }
        }
        val controller = BluetoothCodecChoiceController(env, store, scheduler, BluetoothRateBreaker(8_000, 3))
    }

    private val headsetChoices = BluetoothCodecChoices("t1", listOf(offer("t4"), offer("t1")), local)

    @Test fun aSwitchSavesTheOriginalCodecOnceAndWaitsForTheBroadcast() {
        val f = Fixture(FakeBackend { HelperResult.CodecSwitched("t1", "t4", true) })
        f.known[address] = headsetChoices
        assertEquals(Plan.Switch("t4"), f.controller.process(address, Trigger.USER))
        assertEquals(listOf<HelperRequest>(HelperRequest.SetCodec(address, "t4")), f.backend.calls)
        assertEquals(BluetoothCodecChoiceRules.SavedCodec("t1", "t4"), f.store.saved[address])
        assertEquals(listOf("t4"), f.awaited)
        f.prefs = BluetoothCodecPrefs(true, listOf("t3"))
        f.backend.answer = { HelperResult.CodecSwitched("t4", "t1", true) }
        f.known[address] = BluetoothCodecChoices("t4", listOf(offer("t4"), offer("t1")), local)
        f.prefs = BluetoothCodecPrefs(true, listOf("t1"))
        f.controller.process(address, Trigger.USER)
        assertEquals(BluetoothCodecChoiceRules.SavedCodec("t1", "t1"), f.store.saved[address])
    }

    @Test fun unknownCodecsAreReadThroughTheHelperThenSwitched() {
        val f = Fixture(FakeBackend { request ->
            when (request) {
                is HelperRequest.Codecs -> HelperResult.Codecs("t1", local, listOf(offer("t4"), offer("t1")))
                else -> HelperResult.CodecSwitched("t1", "t4", true)
            }
        })
        f.controller.process(address, Trigger.USER)
        assertEquals(listOf(HelperRequest.Codecs(address), HelperRequest.SetCodec(address, "t4")), f.backend.calls)
        assertEquals("t1", f.known[address]!!.activeKey)
    }

    @Test fun aFailedSwitchShowsAShortMessageAndKeepsTheCodec() {
        val f = Fixture(FakeBackend { HelperResult.Failed("denied") })
        f.known[address] = headsetChoices
        f.controller.process(address, Trigger.USER)
        assertTrue(f.store.saved.isEmpty())
        assertEquals(listOf<String?>(BluetoothCodecChoiceController.FAILURE_TEXT), f.failures)
    }

    @Test fun threeFailedCallsInARowStopTheFeature() {
        val f = Fixture(FakeBackend { HelperResult.Failed("denied") })
        f.known[address] = headsetChoices
        repeat(3) { f.controller.process(address, Trigger.USER) }
        assertEquals(Plan.Skip(SkipReason.BREAKER), f.controller.process(address, Trigger.USER))
        assertEquals(3, f.backend.calls.size)
    }

    @Test fun withTheSwitchOffNothingIsCalled() {
        val f = Fixture(FakeBackend { HelperResult.CodecSwitched("t1", "t4", true) })
        f.prefs = BluetoothCodecPrefs(false, listOf("t4"))
        f.known[address] = headsetChoices
        f.controller.devicesChanged(setOf(address))
        f.controller.prefsChanged()
        f.scheduler.runAll()
        assertTrue(f.backend.calls.isEmpty())
    }

    @Test fun turningTheSwitchOnAppliesAtOnceAndConnectionsNeedAStoredOrder() {
        val f = Fixture(FakeBackend { HelperResult.CodecSwitched("t1", "t4", true) })
        f.prefs = BluetoothCodecPrefs(false, listOf("t4"))
        f.known[address] = headsetChoices
        f.controller.start(setOf(address))
        f.prefs = BluetoothCodecPrefs(true, listOf("t4"))
        f.controller.prefsChanged()
        f.scheduler.runAll()
        assertEquals(listOf<HelperRequest>(HelperRequest.SetCodec(address, "t4")), f.backend.calls)

        val g = Fixture(FakeBackend { HelperResult.CodecSwitched("t1", "t4", true) })
        g.prefs = BluetoothCodecPrefs(true, emptyList())
        g.known[address] = headsetChoices
        g.controller.start(setOf(address))
        g.scheduler.runAll()
        assertTrue(g.backend.calls.isEmpty())
    }

    @Test fun aNewConnectionAppliesTheStoredOrderAndReorderingAppliesToAttachedHeadsets() {
        val f = Fixture(FakeBackend { HelperResult.CodecSwitched("t1", "t4", true) })
        f.known[address] = headsetChoices
        f.controller.start(emptySet())
        f.controller.devicesChanged(setOf(address))
        f.scheduler.runAll()
        assertEquals(1, f.backend.calls.size)
        f.known[address] = BluetoothCodecChoices("t4", listOf(offer("t4"), offer("t1")), local)
        f.controller.devicesChanged(setOf(address))
        f.scheduler.runAll()
        assertEquals(1, f.backend.calls.size)
        f.prefs = BluetoothCodecPrefs(true, listOf("t1", "t4"))
        f.now += 5_000
        f.controller.prefsChanged()
        f.scheduler.runAll()
        assertEquals(HelperRequest.SetCodec(address, "t1"), f.backend.calls.last())
    }

    @Test fun twoSwitchesAreSpacedByTheMinimumInterval() {
        val f = Fixture(FakeBackend { HelperResult.CodecSwitched("t1", "t4", true) })
        f.known[address] = headsetChoices
        f.prefs = BluetoothCodecPrefs(true, listOf("t4"))
        f.controller.start(setOf(address))
        f.scheduler.tasks.removeAt(0).second()
        f.prefs = BluetoothCodecPrefs(true, listOf("t1"))
        f.controller.prefsChanged()
        assertTrue(f.scheduler.tasks.first().first > 0L)
    }

    @Test fun switchingOffPutsTheOriginalBackOnlyWhileTheCodecIsStillOurs() {
        val f = Fixture(FakeBackend { HelperResult.CodecSwitched("t4", "t1", true) })
        f.store.saved[address] = BluetoothCodecChoiceRules.SavedCodec("t1", "t4")
        f.known[address] = BluetoothCodecChoices("t4", listOf(offer("t4"), offer("t1")), local)
        f.controller.restoreNow("switch turned off")
        assertEquals(listOf<HelperRequest>(HelperRequest.SetCodec(address, "t1")), f.backend.calls)
        assertTrue(f.store.saved.isEmpty())

        val g = Fixture(FakeBackend { HelperResult.CodecSwitched("t0", "t1", true) })
        g.store.saved[address] = BluetoothCodecChoiceRules.SavedCodec("t1", "t4")
        g.known[address] = BluetoothCodecChoices("t0", listOf(offer("t0")), local)
        g.controller.restoreNow("switch turned off")
        assertTrue(g.backend.calls.isEmpty())
        assertTrue(g.store.saved.isEmpty())
    }

    @Test fun aListPageReadOnlyReadsAndStopsAfterAFewFailures() {
        val f = Fixture(FakeBackend { HelperResult.Failed("no") })
        f.prefs = BluetoothCodecPrefs(false, emptyList())
        repeat(5) {
            f.controller.ensureChoices(address)
            f.scheduler.runAll()
            f.now += 5_000
        }
        assertEquals(BluetoothCodecChoiceConfig.READ_FAILURES_PER_DEVICE, f.backend.calls.size)
        assertTrue(f.backend.calls.all { it is HelperRequest.Codecs })
    }
}
