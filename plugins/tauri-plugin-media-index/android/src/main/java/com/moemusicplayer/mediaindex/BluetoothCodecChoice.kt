package com.moemusicplayer.mediaindex

/** Constants of the Bluetooth codec priority list (ADR 0052). */
internal object BluetoothCodecChoiceConfig {
    /** Kill switch in code: false makes the feature do nothing whatever the setting says. */
    const val ENABLED = true

    /** Two privileged calls start at least this far apart; requests in between are coalesced. */
    const val MIN_INTERVAL_MS = 1_000L

    /** After a switch, wait this long for the Bluetooth stack's codec broadcast that shows the new codec. */
    const val WAIT_CHANGE_MS = 3_000L

    /** One privileged helper process is given this long. */
    const val COMMAND_TIMEOUT_MS = 8_000L

    /** This many failed calls in a row stop the feature for the rest of the process. */
    const val FAILURES_TO_TRIP = 3

    /** An audio-server symptom this soon after a switch stops the feature for the rest of the process. */
    const val BREAKER_WINDOW_MS = 8_000L

    /** How long a failure text stays in the settings page's status. */
    const val ERROR_VISIBLE_MS = 12_000L

    /** This many failed reads of one headset's codecs stop reading for that headset (until it reconnects). */
    const val READ_FAILURES_PER_DEVICE = 2

    /** The priority that makes the stack pick a codec (`BluetoothCodecConfig.CODEC_PRIORITY_HIGHEST`). */
    const val PRIORITY_HIGHEST = 1_000_000
}

/** One codec of the Bluetooth stack (a local capability or a selectable one), in plain values. */
internal data class CodecOffer(
    /** `t<source codec type>` or `x<extended codec id>`; the same key is stored in settings.json. */
    val key: String,
    val name: String,
    /** The codec's own priority as the stack reports it; larger is preferred by the device. */
    val priority: Long,
    /** `SAMPLE_RATE_*`, `BITS_PER_SAMPLE_*` and `CHANNEL_MODE_*` unions the codec offers (0: not reported). */
    val rateMask: Int,
    val bitsMask: Int,
    val channelMask: Int,
)

/**
 * What the stack announced for a headset: the codec in use, the codecs the headset can be switched to ([selectable],
 * with the values each allows) and the codecs this phone can encode ([local]); both in the stack's own priority order.
 */
internal data class BluetoothCodecChoices(
    val activeKey: String?,
    val selectable: List<CodecOffer>,
    val local: List<CodecOffer>,
)

/** Pure rules of the Bluetooth codec priority list (ADR 0052). */
internal object BluetoothCodecChoiceRules {
    private val KEY = Regex("^[tx][0-9]{1,18}$")

    fun validKey(key: String): Boolean = KEY.matches(key)

    /**
     * The key of a codec: its source codec type when it has one (SBC 0 ... Opus 6), else the extended codec id of an
     * Android 15+ vendor codec; null when neither is known.
     */
    fun keyOf(codecType: Int, extendedId: Long?): String? = when {
        codecType in 0 until BluetoothCodecRules.CODEC_INVALID -> "t$codecType"
        extendedId != null && extendedId >= 0 -> "x$extendedId"
        else -> null
    }

    /** The stack's own order: the larger priority first, equal priorities keep the order the stack reported. */
    fun deviceOrder(offers: List<CodecOffer>): List<CodecOffer> =
        offers.distinctBy { it.key }.sortedByDescending { it.priority }

    /**
     * The list as it is shown (one order for every headset): the phone's codecs in the stored order, then the phone's
     * codecs the order does not know yet in the stack's order. Stored codecs the phone does not list are skipped (they
     * stay in the stored order, see [reorder]). [local] are the phone's codecs in the stack's order.
     */
    fun shown(stored: List<String>?, local: List<CodecOffer>): List<CodecOffer> {
        val byKey = local.associateBy { it.key }
        val first = stored.orEmpty().distinct().mapNotNull { byKey[it] }
        val known = first.map { it.key }.toSet()
        return first + local.filter { it.key !in known }
    }

    /**
     * The order to store after the user dragged the shown list into [newShown]: the listed codecs fill, in their new
     * order, the places the listed codecs had in the stored order; a codec the phone does not list keeps its place.
     */
    fun reorder(stored: List<String>?, local: List<CodecOffer>, newShown: List<String>): List<String> {
        val listed = local.map { it.key }.toSet()
        val full = stored.orEmpty().distinct().toMutableList()
        for (offer in local) if (offer.key !in full) full.add(offer.key)
        val order = newShown.filter { it in listed }.distinct().toMutableList()
        for (key in full) if (key in listed && key !in order) order.add(key)
        var next = 0
        return full.map { key -> if (key in listed) order[next++] else key }
    }

    /**
     * The codec to use for a headset: the first of the shown list that the headset can be switched to ([selectable]),
     * or null when none is.
     */
    fun target(stored: List<String>?, local: List<CodecOffer>, selectable: List<CodecOffer>): String? {
        val supported = selectable.map { it.key }.toSet()
        return shown(stored, local).firstOrNull { it.key in supported }?.key
    }

    /** What a switch asks of the codec: the sample rate nearest to the current one and compatible bits and channels. */
    data class SwitchValues(val rateFlag: Int, val bitsFlag: Int, val channelFlag: Int)

    /**
     * The sample rate, bits per sample and channel mode for switching to [offer]: the current ones when the codec offers
     * them, else the nearest rate (the same pick as the sample-rate follow) or the best bit depth and stereo. A field
     * the codec did not report (mask 0) keeps the current value. Codec-specific values are not carried over: they
     * mean something else in every codec.
     */
    fun switchValues(currentRate: Int, currentBits: Int, currentChannel: Int, offer: CodecOffer): SwitchValues {
        val hz = BluetoothRateRules.hzOfFlag(currentRate)
        val rate = when {
            offer.rateMask == 0 || currentRate and offer.rateMask != 0 -> currentRate
            hz != null -> BluetoothRateRules.pickRate(hz, offer.rateMask) ?: currentRate
            else -> highestBit(offer.rateMask)
        }
        val bits = if (offer.bitsMask == 0 || currentBits and offer.bitsMask != 0) currentBits else highestBit(offer.bitsMask)
        val channel = when {
            offer.channelMask == 0 || currentChannel and offer.channelMask != 0 -> currentChannel
            offer.channelMask and 2 != 0 -> 2
            else -> highestBit(offer.channelMask)
        }
        return SwitchValues(rate, bits, channel)
    }

    private fun highestBit(mask: Int): Int = if (mask == 0) 0 else Integer.highestOneBit(mask)

    // ---- the helper's wire format --------------------------------------------------------------------------------

    /** `key:priority:rate:bits:channel:name`, the name with spaces as `_` and without `:`, `,` and non-ASCII. */
    fun formatOffer(offer: CodecOffer): String {
        val name = offer.name.map { if (it == ' ') '_' else it }
            .filter { it != ':' && it != ',' && it.code in 33..126 }.joinToString("").ifEmpty { offer.key }
        return "${offer.key}:${offer.priority}:${offer.rateMask}:${offer.bitsMask}:${offer.channelMask}:$name"
    }

    fun parseOffer(text: String): CodecOffer? {
        val parts = text.split(':')
        if (parts.size != 6 || !validKey(parts[0])) return null
        val priority = parts[1].toLongOrNull() ?: return null
        val masks = (2..4).map { parts[it].toIntOrNull() ?: return null }
        return CodecOffer(parts[0], parts[5].replace('_', ' '), priority, masks[0], masks[1], masks[2])
    }

    /** A list of offers as one token (`-` when empty). */
    fun formatOffers(offers: List<CodecOffer>): String =
        if (offers.isEmpty()) "-" else offers.joinToString(",") { formatOffer(it) }

    /** The inverse of [formatOffers]; null when a part is unreadable. */
    fun parseOffers(token: String?): List<CodecOffer>? {
        if (token == null) return null
        if (token == "-") return emptyList()
        val offers = token.split(',').map { parseOffer(it) }
        return if (offers.any { it == null }) null else offers.map { it!! }
    }

    // ---- when it may run --------------------------------------------------------------------------------------------

    enum class Trigger {
        /** The headset connected (or the service started with it connected): nothing is applied without a stored order. */
        CONNECT,

        /** The switch was turned on, or the user reordered the list. */
        USER,

        /** Only read the headset's codecs (for the settings page); nothing is switched. */
        READ,
    }

    enum class SkipReason(val wire: String) {
        DISABLED("the switch or the code switch is off"),
        UNSUPPORTED("needs Android 13 or newer"),
        BREAKER("the circuit breaker stopped the feature"),
        NO_SOURCE("neither root nor an authorized Shizuku"),
        NO_STORED_ORDER("no stored order yet, the headset's own codec stays"),
        NO_OFFERS("the headset supports none of the listed codecs"),
        READ_ONLY("only reading the codecs"),
    }

    sealed class Plan {
        data class Skip(val reason: SkipReason) : Plan()
        data class AlreadyOk(val key: String) : Plan()
        data class Switch(val key: String) : Plan()

        /** The headset's codecs are not known here: read them through the helper first. */
        object NeedOffers : Plan()
    }

    data class Input(
        val killSwitchCode: Boolean,
        val settingEnabled: Boolean,
        val androidSupported: Boolean,
        val breakerTripped: Boolean,
        val source: MixerRateRules.Source,
        val stored: List<String>?,
        val choices: BluetoothCodecChoices?,
        val trigger: Trigger,
    )

    fun plan(input: Input): Plan = when {
        !input.killSwitchCode -> Plan.Skip(SkipReason.DISABLED)
        input.breakerTripped -> Plan.Skip(SkipReason.BREAKER)
        !input.androidSupported -> Plan.Skip(SkipReason.UNSUPPORTED)
        input.source == MixerRateRules.Source.PLATFORM -> Plan.Skip(SkipReason.NO_SOURCE)
        input.choices == null -> Plan.NeedOffers
        input.trigger == Trigger.READ -> Plan.Skip(SkipReason.READ_ONLY)
        !input.settingEnabled -> Plan.Skip(SkipReason.DISABLED)
        input.trigger == Trigger.CONNECT && input.stored.isNullOrEmpty() -> Plan.Skip(SkipReason.NO_STORED_ORDER)
        else -> {
            val target = target(input.stored, input.choices.local, input.choices.selectable)
            when {
                target == null -> Plan.Skip(SkipReason.NO_OFFERS)
                target == input.choices.activeKey -> Plan.AlreadyOk(target)
                else -> Plan.Switch(target)
            }
        }
    }

    /** Whether a saved original codec should be put back now: only while the codec still is the one we set. */
    fun shouldRestore(saved: SavedCodec, activeKey: String?): Boolean =
        saved.original != saved.applied && (activeKey == null || activeKey == saved.applied)

    /** What a switch left behind for an address: the codec before our first switch and the one we set last. */
    data class SavedCodec(val original: String, val applied: String)
}

/** The prefs the page saved: the switch and the one stored order (codec keys, best first). */
internal data class BluetoothCodecPrefs(val enabled: Boolean, val order: List<String>) {
    companion object {
        val OFF = BluetoothCodecPrefs(false, emptyList())

        /** Reads {"enabled":bool,"order":[key]}; anything unreadable is off. */
        fun parse(json: String?): BluetoothCodecPrefs {
            if (json.isNullOrBlank()) return OFF
            return try {
                val root = org.json.JSONObject(json)
                val array = root.optJSONArray("order")
                val keys = if (array == null) emptyList() else (0 until array.length())
                    .mapNotNull { array.optString(it).takeIf(BluetoothCodecChoiceRules::validKey) }
                BluetoothCodecPrefs(root.optBoolean("enabled", false), keys.distinct())
            } catch (_: org.json.JSONException) {
                OFF
            }
        }
    }
}

/** Where the saved original codecs live (SharedPreferences on the device). */
internal interface BluetoothCodecStore {
    fun all(): Map<String, BluetoothCodecChoiceRules.SavedCodec>
    fun put(address: String, saved: BluetoothCodecChoiceRules.SavedCodec)
    fun remove(address: String)
}

/** What the controller needs from the device; a fake in the unit tests. */
internal interface BluetoothCodecChoiceEnvironment {
    fun nowMs(): Long
    fun supported(): Boolean
    fun source(): MixerRateRules.Source
    fun backend(source: MixerRateRules.Source): BluetoothRateBackend?
    fun prefs(): BluetoothCodecPrefs

    /** The codecs and the one in use as announced or read before, or null. */
    fun choices(address: String): BluetoothCodecChoices?
    fun storeChoices(address: String, choices: BluetoothCodecChoices)

    /** Blocks up to [timeoutMs] until the stack announced the codec [key] for the address. */
    fun awaitCodec(address: String, key: String, timeoutMs: Long): Boolean

    /** A short failure text for the settings page (null clears it). */
    fun reportFailure(address: String, text: String?)
    fun log(message: String)
}

/**
 * The Bluetooth codec priority list (ADR 0052). With the switch on, the first codec of the one stored order that the
 * headset supports is applied when the headset connects, when the switch is turned on and when the user reorders;
 * with it off nothing is applied and the codec in use before the first switch is put back (only while it is still the
 * one we set). Everything runs on the scheduler's own thread, rate-limited and coalesced, behind a circuit breaker;
 * nothing blocks playback.
 */
internal class BluetoothCodecChoiceController(
    private val env: BluetoothCodecChoiceEnvironment,
    private val store: BluetoothCodecStore,
    private val scheduler: BluetoothRateScheduler,
    private val breaker: BluetoothRateBreaker = sharedBreaker,
) {
    private val lock = Any()
    private val wanted = LinkedHashMap<String, BluetoothCodecChoiceRules.Trigger>()
    private var scheduled = false
    private var lastSwitchStartMs: Long? = null
    private var attached: Set<String> = emptySet()
    private var lastPrefs: BluetoothCodecPrefs = BluetoothCodecPrefs.OFF
    private val readFailures = HashMap<String, Int>()

    /** Addresses that connected while their codecs were not known; looked at when the stack announces. */
    private val waitingForAnnouncement = HashSet<String>()

    /** Remember the prefs at the start without acting on them, then treat the attached headsets as connections. */
    fun start(initial: Set<String>) {
        synchronized(lock) { lastPrefs = env.prefs() }
        devicesChanged(initial)
    }

    /** The attached A2DP devices changed (main thread; only memory is touched). A device that appears is a connection. */
    fun devicesChanged(now: Set<String>) {
        if (!BluetoothCodecChoiceConfig.ENABLED) return
        val connected: Set<String>
        synchronized(lock) {
            connected = now - attached
            attached = now
            waitingForAnnouncement.retainAll(now)
            readFailures.keys.retainAll(now)
        }
        for (address in connected) request(address, BluetoothCodecChoiceRules.Trigger.CONNECT)
    }

    /** The stack announced a codec for [address] (broadcast thread; memory only). */
    fun announced(address: String) {
        val waiting = synchronized(lock) { waitingForAnnouncement.remove(address) }
        if (waiting) request(address, BluetoothCodecChoiceRules.Trigger.CONNECT)
    }

    /** The settings page is open and [address] has no known codecs: read them (a few tries per connection). */
    fun ensureChoices(address: String) {
        if (env.choices(address) != null) return
        if (synchronized(lock) { (readFailures[address] ?: 0) >= BluetoothCodecChoiceConfig.READ_FAILURES_PER_DEVICE }) return
        request(address, BluetoothCodecChoiceRules.Trigger.READ)
    }

    /**
     * The prefs changed (main thread): the switch turned on applies to every attached headset, the switch turned off
     * puts the original codecs back, and a changed order applies to every attached headset while the switch is on.
     */
    fun prefsChanged() {
        val now = env.prefs()
        val before: BluetoothCodecPrefs
        val devices: Set<String>
        synchronized(lock) {
            before = lastPrefs
            lastPrefs = now
            devices = attached
        }
        when {
            now.enabled && !before.enabled -> devices.forEach { request(it, BluetoothCodecChoiceRules.Trigger.USER) }
            !now.enabled && before.enabled -> restoreAll("switch turned off")
            now.enabled && now.order != before.order -> devices.forEach { request(it, BluetoothCodecChoiceRules.Trigger.USER) }
        }
    }

    private fun request(address: String, trigger: BluetoothCodecChoiceRules.Trigger) {
        if (!BluetoothCodecChoiceConfig.ENABLED) return
        if (trigger != BluetoothCodecChoiceRules.Trigger.READ && !env.prefs().enabled) return
        val delay = synchronized(lock) {
            // A switch outranks a plain read for the same device.
            val existing = wanted[address]
            if (existing == null || existing == BluetoothCodecChoiceRules.Trigger.READ) wanted[address] = trigger
            if (trigger == BluetoothCodecChoiceRules.Trigger.USER) wanted[address] = trigger
            if (scheduled) return
            scheduled = true
            untilAllowed()
        }
        scheduler.schedule(delay) { run() }
    }

    private fun untilAllowed(): Long {
        val last = lastSwitchStartMs ?: return 0L
        return maxOf(0L, last + BluetoothCodecChoiceConfig.MIN_INTERVAL_MS - env.nowMs())
    }

    /** Scheduler thread (public for the unit tests). */
    fun run() {
        val batch = synchronized(lock) {
            val wait = untilAllowed()
            if (wait > 0L) {
                scheduler.schedule(wait) { run() }
                return
            }
            scheduled = false
            LinkedHashMap(wanted).also { wanted.clear() }
        }
        for ((address, trigger) in batch) {
            try {
                process(address, trigger)
            } catch (error: Exception) {
                env.log("bluetooth codec: unexpected failure: $error")
            }
        }
    }

    /** One headset: decide and, when a switch is due, carry it out. Returns the plan (for the tests). */
    fun process(address: String, trigger: BluetoothCodecChoiceRules.Trigger): BluetoothCodecChoiceRules.Plan {
        val prefs = env.prefs()
        val source = env.source()
        fun plan(choices: BluetoothCodecChoices?) = BluetoothCodecChoiceRules.plan(
            BluetoothCodecChoiceRules.Input(
                killSwitchCode = BluetoothCodecChoiceConfig.ENABLED,
                settingEnabled = prefs.enabled,
                androidSupported = env.supported(),
                breakerTripped = breaker.tripped,
                source = source,
                stored = prefs.order,
                choices = choices,
                trigger = trigger,
            ),
        )
        var current = plan(env.choices(address))
        if (current is BluetoothCodecChoiceRules.Plan.NeedOffers) {
            val read = readChoices(source, address)
            if (read == null) {
                // The stack's own announcement may still arrive (the broadcast is not sticky); wait for it once.
                if (trigger == BluetoothCodecChoiceRules.Trigger.CONNECT) {
                    synchronized(lock) { waitingForAnnouncement.add(address) }
                }
                return current
            }
            current = plan(read)
        }
        when (current) {
            is BluetoothCodecChoiceRules.Plan.Skip -> env.log("bluetooth codec: skipped, ${current.reason.wire}")
            is BluetoothCodecChoiceRules.Plan.AlreadyOk -> {
                env.log("bluetooth codec: ${current.key} is already in use")
                env.reportFailure(address, null)
            }
            is BluetoothCodecChoiceRules.Plan.Switch -> switchTo(source, address, current.key)
            BluetoothCodecChoiceRules.Plan.NeedOffers -> Unit
        }
        return current
    }

    private fun readChoices(source: MixerRateRules.Source, address: String): BluetoothCodecChoices? {
        val backend = env.backend(source) ?: return null
        val result = call(backend, BluetoothRateRules.HelperRequest.Codecs(address))
        if (result is BluetoothRateRules.HelperResult.Codecs) {
            val choices = BluetoothCodecChoices(
                result.activeKey,
                BluetoothCodecChoiceRules.deviceOrder(result.selectable),
                BluetoothCodecChoiceRules.deviceOrder(result.local),
            )
            env.storeChoices(address, choices)
            return choices
        }
        synchronized(lock) { readFailures[address] = (readFailures[address] ?: 0) + 1 }
        env.log("bluetooth codec: could not read the codecs, ${describe(result)}")
        return null
    }

    private fun switchTo(source: MixerRateRules.Source, address: String, key: String) {
        val backend = env.backend(source) ?: return
        synchronized(lock) { lastSwitchStartMs = env.nowMs() }
        val result = call(backend, BluetoothRateRules.HelperRequest.SetCodec(address, key))
        if (result is BluetoothRateRules.HelperResult.CodecSwitched) {
            env.reportFailure(address, null)
            if (!result.changed) {
                env.log("bluetooth codec: $key is already in use")
                return
            }
            val previous = store.all()[address]
            val original = previous?.original ?: result.fromKey
            if (original != null) store.put(address, BluetoothCodecChoiceRules.SavedCodec(original, key))
            val heard = env.awaitCodec(address, key, BluetoothCodecChoiceConfig.WAIT_CHANGE_MS)
            env.log(
                "bluetooth codec: ${result.fromKey} -> $key, " +
                    if (heard) "confirmed by the codec broadcast" else "not confirmed within ${BluetoothCodecChoiceConfig.WAIT_CHANGE_MS} ms",
            )
        } else {
            env.log("bluetooth codec: failed, ${describe(result)}")
            env.reportFailure(address, FAILURE_TEXT)
        }
    }

    private fun describe(result: BluetoothRateRules.HelperResult): String = when (result) {
        is BluetoothRateRules.HelperResult.Failed -> result.reason
        BluetoothRateRules.HelperResult.TimedOut -> "the helper timed out"
        else -> "unexpected answer"
    }

    /** Runs the helper once with the breaker around it. */
    private fun call(backend: BluetoothRateBackend, helper: BluetoothRateRules.HelperRequest): BluetoothRateRules.HelperResult {
        synchronized(breaker) { breaker.switchStarted() }
        val result = try {
            backend.run(helper, BluetoothCodecChoiceConfig.COMMAND_TIMEOUT_MS)
        } catch (error: Exception) {
            BluetoothRateRules.HelperResult.Failed(error.toString())
        }
        val ok = result is BluetoothRateRules.HelperResult.CodecSwitched || result is BluetoothRateRules.HelperResult.Codecs
        synchronized(breaker) {
            if (PrivilegedRunnerErrors.isRunnerBug(result)) breaker.switchIgnored() else breaker.switchEnded(env.nowMs(), ok)
        }
        if (breaker.tripped) env.log("bluetooth codec: disabled for this process, ${breaker.reason}")
        return result
    }

    /** An audio-server symptom (a sink or player error, any thread). */
    fun symptom(what: String) {
        val tripped = synchronized(breaker) { breaker.symptom(env.nowMs(), what) }
        if (tripped) env.log("bluetooth codec: disabled for this process, ${breaker.reason}")
    }

    /** Put the original codecs back on the controller's thread (the switch was turned off). */
    fun restoreAll(why: String) {
        scheduler.schedule(0L) { restoreNow(why) }
    }

    /** Scheduler thread (public for the unit tests). A codec the user changed meanwhile is not overwritten. */
    fun restoreNow(why: String) {
        val saved = store.all()
        if (saved.isEmpty() || breaker.tripped) return
        val source = env.source()
        val backend = env.backend(source) ?: return
        for ((address, codecs) in saved) {
            var active = env.choices(address)?.activeKey
            if (active == null) active = readChoices(source, address)?.activeKey
            if (!BluetoothCodecChoiceRules.shouldRestore(codecs, active)) {
                env.log("bluetooth codec: $why, $address is no longer on the codec we set, nothing to restore")
                store.remove(address)
                continue
            }
            val result = call(backend, BluetoothRateRules.HelperRequest.SetCodec(address, codecs.original))
            if (result is BluetoothRateRules.HelperResult.CodecSwitched) {
                store.remove(address)
                if (result.changed) env.awaitCodec(address, codecs.original, BluetoothCodecChoiceConfig.WAIT_CHANGE_MS)
                env.log("bluetooth codec: $why, $address back to ${codecs.original}")
            } else {
                env.log("bluetooth codec: $why, could not restore $address")
            }
        }
    }

    companion object {
        const val FAILURE_TEXT = "無法切換藍牙編碼"

        /** Process-wide: a tripped breaker outlives a restarted playback service. */
        val sharedBreaker = BluetoothRateBreaker(BluetoothCodecChoiceConfig.BREAKER_WINDOW_MS, BluetoothCodecChoiceConfig.FAILURES_TO_TRIP)

        val breakerTripped: Boolean get() = sharedBreaker.tripped
    }
}
