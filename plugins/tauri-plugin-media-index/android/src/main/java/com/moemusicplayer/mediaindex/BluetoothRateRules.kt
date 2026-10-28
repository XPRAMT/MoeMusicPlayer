package com.moemusicplayer.mediaindex

/**
 * Constants of the Bluetooth sample-rate follow (ADR 0051). Every value that decides how often and whether the
 * privileged call runs lives here, so the owner can change it in one place.
 */
internal object BluetoothRateConfig {
    /** Kill switch in code: false makes the feature do nothing whatever the setting says. */
    const val ENABLED = true

    /** Two switches start at least this far apart; requests in between are coalesced into the newest one. */
    const val MIN_INTERVAL_MS = 5_000L

    /** After a switch, wait this long for the Bluetooth stack's codec broadcast that shows the new rate. */
    const val WAIT_CHANGE_MS = 3_000L

    /** One privileged helper process is given this long. */
    const val COMMAND_TIMEOUT_MS = 8_000L

    /** This many failed calls in a row stop the feature for the rest of the process. */
    const val FAILURES_TO_TRIP = 3

    /** An audio-server symptom this soon after a switch stops the feature for the rest of the process. */
    const val BREAKER_WINDOW_MS = 8_000L
}

/** The codec of an A2DP device as the Bluetooth stack announced it, in plain values. */
internal data class BluetoothCodecState(
    val codecType: Int,
    /** `BluetoothCodecConfig.SAMPLE_RATE_*` of the rate in use (one bit). */
    val rateFlag: Int,
    /** The union of `SAMPLE_RATE_*` the codec offers (`codecsSelectableCapabilities` entries of the same codec). */
    val selectableMask: Int,
)

/** Pure rules of the Bluetooth sample-rate follow (ADR 0051): which rate, whether to switch, and the helper protocol. */
internal object BluetoothRateRules {
    // BluetoothCodecConfig.SAMPLE_RATE_* (one bit per rate).
    const val RATE_44100 = 1
    const val RATE_48000 = 2
    const val RATE_88200 = 4
    const val RATE_96000 = 8
    const val RATE_176400 = 16
    const val RATE_192000 = 32

    private val RATES = listOf(
        RATE_44100 to 44_100,
        RATE_48000 to 48_000,
        RATE_88200 to 88_200,
        RATE_96000 to 96_000,
        RATE_176400 to 176_400,
        RATE_192000 to 192_000,
    )

    fun flagOfHz(hz: Int): Int? = RATES.firstOrNull { it.second == hz }?.first

    fun hzOfFlag(flag: Int): Int? = RATES.firstOrNull { it.first == flag }?.second

    /** The rates a mask offers, ascending, as (flag, Hz). */
    fun ratesIn(mask: Int): List<Pair<Int, Int>> = RATES.filter { mask and it.first != 0 }

    /** 44.1 kHz family (11025 × n) or 48 kHz family (8000 × n): converting inside a family is an integer-ratio step. */
    private fun family(hz: Int): Int? = when {
        hz <= 0 -> null
        hz % 11_025 == 0 -> 44_100
        hz % 8_000 == 0 -> 48_000
        else -> null
    }

    /**
     * The `SAMPLE_RATE_*` flag to switch the codec to for a track of [trackHz]: the exact rate when the codec offers it,
     * else the nearest offered rate of the track's family (44.1 kHz or 48 kHz), else the nearest offered rate; a tie
     * goes to the lower rate. Null when the codec offers none or the track rate is unknown.
     */
    fun pickRate(trackHz: Int, selectableMask: Int): Int? {
        if (trackHz <= 0) return null
        val offered = ratesIn(selectableMask)
        if (offered.isEmpty()) return null
        offered.firstOrNull { it.second == trackHz }?.let { return it.first }
        val wanted = family(trackHz)
        val pool = if (wanted == null) offered else offered.filter { family(it.second) == wanted }.ifEmpty { offered }
        return pool.minWith(compareBy({ kotlin.math.abs(it.second - trackHz) }, { it.second })).first
    }

    /** Why the feature does nothing for a track. */
    enum class SkipReason(val wire: String) {
        DISABLED("the setting or the code switch is off"),
        NO_SOURCE("neither root nor an authorized Shizuku"),
        BREAKER("the circuit breaker stopped the feature"),
        NOT_BLUETOOTH("the track does not go to an A2DP device"),
        USB_EXCLUSIVE("the USB exclusive output plays the track"),
        DSD("DSD track"),
        UNKNOWN_RATE("the track's sample rate is unknown"),
        NO_RATE("the codec offers no usable rate"),
        UNSUPPORTED("needs Android 13 or newer"),
    }

    sealed class Plan {
        data class Skip(val reason: SkipReason) : Plan()
        /** The codec already runs at the rate the track wants. */
        data class AlreadyOk(val rateFlag: Int) : Plan()
        /** Switch to [rateFlag] (the codec state is known). */
        data class Switch(val rateFlag: Int) : Plan()
        /** The codec state is not known here: the privileged helper reads it and decides with [pickRate] itself. */
        object Ask : Plan()
    }

    /** What the plan depends on, all read by the caller. */
    data class Input(
        val killSwitchCode: Boolean,
        val settingEnabled: Boolean,
        val androidSupported: Boolean,
        val source: MixerRateRules.Source,
        val breakerTripped: Boolean,
        /** The A2DP device's address the track goes to; null: not an A2DP route. */
        val address: String?,
        val usbExclusive: Boolean,
        val dsd: Boolean,
        val trackHz: Int?,
        val state: BluetoothCodecState?,
    )

    fun plan(input: Input): Plan = when {
        !input.killSwitchCode || !input.settingEnabled -> Plan.Skip(SkipReason.DISABLED)
        input.breakerTripped -> Plan.Skip(SkipReason.BREAKER)
        !input.androidSupported -> Plan.Skip(SkipReason.UNSUPPORTED)
        input.address == null -> Plan.Skip(SkipReason.NOT_BLUETOOTH)
        input.usbExclusive -> Plan.Skip(SkipReason.USB_EXCLUSIVE)
        input.dsd -> Plan.Skip(SkipReason.DSD)
        input.trackHz == null || input.trackHz <= 0 -> Plan.Skip(SkipReason.UNKNOWN_RATE)
        input.source == MixerRateRules.Source.PLATFORM -> Plan.Skip(SkipReason.NO_SOURCE)
        input.state == null -> Plan.Ask
        else -> {
            val target = pickRate(input.trackHz, input.state.selectableMask)
            when {
                target == null -> Plan.Skip(SkipReason.NO_RATE)
                target == input.state.rateFlag -> Plan.AlreadyOk(target)
                else -> Plan.Switch(target)
            }
        }
    }

    /** Whether a saved original should be put back now: only when the codec still runs at the rate we set. */
    fun shouldRestore(saved: SavedRate, current: BluetoothCodecState?): Boolean =
        saved.original != saved.applied && (current == null || current.rateFlag == saved.applied)

    /** What a switch left behind for an address: the rate before our first switch and the rate we set last. */
    data class SavedRate(val original: Int, val applied: Int)

    // ---- identity and diagnostics of the helper -------------------------------------------------------------------

    /**
     * The package name the Bluetooth service's package-vs-uid check accepts for the helper's uid: root is `root`, the
     * system uid is `android`, everything else (the shell, 2000) is `com.android.shell`.
     */
    fun packageForUid(uid: Int): String = when (uid) {
        0 -> "root"
        1000 -> "android"
        else -> "com.android.shell"
    }

    /** A failure text that names the step of the helper that failed, so the log shows where it stopped. */
    fun stepError(step: String, detail: String): String = "$step: $detail"

    /** A name for a `BluetoothCodecConfig.CODEC_TYPE_*` value (SBC 0, AAC 1, aptX 2, aptX HD 3, LDAC 4, Opus 6). */
    fun codecName(codecType: Int): String = when (codecType) {
        0 -> "SBC"
        1 -> "AAC"
        2 -> "aptX"
        3 -> "aptX HD"
        4 -> "LDAC"
        5 -> "aptX Adaptive"
        6 -> "Opus"
        else -> "codec $codecType"
    }

    /** The human-readable line the helper prints before the `RESULT STATE` line of the `state` command. */
    fun describeState(codecType: Int, rateFlag: Int, selectableMask: Int): String {
        val current = hzOfFlag(rateFlag)?.toString() ?: "flag $rateFlag"
        val offered = ratesIn(selectableMask).joinToString(",") { it.second.toString() }.ifEmpty { "none" }
        return "INFO codec=${codecName(codecType)}($codecType) sampleRate=$current selectable=$offered"
    }

    // ---- the helper process (app_process, shell or root) ---------------------------------------------------------

    private val ADDRESS = Regex("^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$")

    fun validAddress(address: String): Boolean = ADDRESS.matches(address)

    /** The commands of [BluetoothRateHelper]. */
    sealed class HelperRequest {
        /** Read the codec, pick the rate for a track of [trackHz] and switch when it differs. */
        data class Follow(val address: String, val trackHz: Int) : HelperRequest()
        /** Switch to [rateFlag] (a restore). */
        data class Set(val address: String, val rateFlag: Int) : HelperRequest()
        data class State(val address: String) : HelperRequest()
    }

    fun parseRequest(args: List<String>): HelperRequest? {
        if (args.size != 3) return null
        val address = args[1].uppercase()
        if (!validAddress(address)) return null
        val number = args[2].toIntOrNull() ?: return null
        return when (args[0]) {
            "follow" -> if (number in 1..1_000_000) HelperRequest.Follow(address, number) else null
            "set" -> if (hzOfFlag(number) != null) HelperRequest.Set(address, number) else null
            "state" -> if (number == 0) HelperRequest.State(address) else null
            else -> null
        }
    }

    fun requestArgs(request: HelperRequest): List<String> = when (request) {
        is HelperRequest.Follow -> listOf("follow", request.address, request.trackHz.toString())
        is HelperRequest.Set -> listOf("set", request.address, request.rateFlag.toString())
        is HelperRequest.State -> listOf("state", request.address, "0")
    }

    /** The helper's one result line. */
    sealed class HelperResult {
        data class Switched(val codecType: Int, val from: Int, val to: Int, val changed: Boolean) : HelperResult()
        data class State(val codecType: Int, val rateFlag: Int, val selectableMask: Int) : HelperResult()
        data class Failed(val reason: String) : HelperResult()
        object TimedOut : HelperResult()
    }

    fun formatResult(result: HelperResult): String = when (result) {
        is HelperResult.Switched -> "RESULT OK ${result.codecType} ${result.from} ${result.to} ${if (result.changed) 1 else 0}"
        is HelperResult.State -> "RESULT STATE ${result.codecType} ${result.rateFlag} ${result.selectableMask}"
        is HelperResult.Failed -> "RESULT ERR ${result.reason.replace('\n', ' ').take(300)}"
        HelperResult.TimedOut -> "RESULT ERR timeout"
    }

    /** The last `RESULT` line of the helper's output, or a failure when there is none. */
    fun parseResult(output: String): HelperResult {
        val line = output.lineSequence().map { it.trim() }.lastOrNull { it.startsWith("RESULT ") }
            ?: return HelperResult.Failed("no result: ${output.trim().take(200)}")
        val parts = line.split(' ')
        fun number(index: Int): Int? = parts.getOrNull(index)?.toIntOrNull()
        return when (parts.getOrNull(1)) {
            "OK" -> {
                val values = (2..5).map(::number)
                if (values.any { it == null }) HelperResult.Failed("unreadable result: $line")
                else HelperResult.Switched(values[0]!!, values[1]!!, values[2]!!, values[3]!! != 0)
            }
            "STATE" -> {
                val values = (2..4).map(::number)
                if (values.any { it == null }) HelperResult.Failed("unreadable result: $line")
                else HelperResult.State(values[0]!!, values[1]!!, values[2]!!)
            }
            "ERR" -> HelperResult.Failed(line.removePrefix("RESULT ERR").trim().ifEmpty { "unknown" })
            else -> HelperResult.Failed("unreadable result: $line")
        }
    }

    /**
     * The shell command that runs [BluetoothRateHelper] from the installed APK. Every argument is validated by
     * [parseRequest] first (an address and numbers only), and the APK path is single-quoted.
     */
    fun shellCommand(apkPath: String, request: HelperRequest): String? {
        if (apkPath.isEmpty() || apkPath.contains('\'') || apkPath.contains('\n')) return null
        val args = requestArgs(request)
        if (parseRequest(args) != request) return null
        return "CLASSPATH='$apkPath' app_process /system/bin ${HELPER_CLASS} ${args.joinToString(" ")}"
    }

    const val HELPER_CLASS = "com.moemusicplayer.mediaindex.BluetoothRateHelper"
}

/**
 * The circuit breaker of the Bluetooth rate follow (ADR 0051): [failuresToTrip] failed calls in a row, or an audio
 * symptom within [windowMs] after a switch, stop the feature for the rest of the process. Pure.
 */
internal class BluetoothRateBreaker(private val windowMs: Long, private val failuresToTrip: Int) {
    var tripped = false
        private set
    var reason: String? = null
        private set
    private var failures = 0
    private var lastSwitchEndMs: Long? = null
    private var switchRunning = false

    fun switchStarted() {
        switchRunning = true
    }

    fun switchEnded(nowMs: Long, ok: Boolean) {
        switchRunning = false
        lastSwitchEndMs = nowMs
        if (ok) {
            failures = 0
        } else if (++failures >= failuresToTrip) {
            trip("$failures failed calls in a row")
        }
    }

    /** A sink error, a player error: true when it tripped the breaker. */
    fun symptom(nowMs: Long, what: String): Boolean {
        if (tripped) return false
        val last = lastSwitchEndMs
        val soon = switchRunning || (last != null && nowMs - last in 0..windowMs)
        if (!soon) return false
        trip("$what within ${windowMs} ms of a switch")
        return true
    }

    private fun trip(why: String) {
        if (tripped) return
        tripped = true
        reason = why
    }
}

/** Where the saved rates live (SharedPreferences on the device). */
internal interface BluetoothRateStore {
    fun all(): Map<String, BluetoothRateRules.SavedRate>
    fun put(address: String, saved: BluetoothRateRules.SavedRate)
    fun remove(address: String)
}

/** One privileged way of running the helper (root or Shizuku). */
internal interface BluetoothRateBackend {
    /** Blocking; runs off every app thread that matters. */
    fun run(request: BluetoothRateRules.HelperRequest, timeoutMs: Long): BluetoothRateRules.HelperResult
}

/** Runs the controller's tasks later, on its own thread. */
internal interface BluetoothRateScheduler {
    fun schedule(delayMs: Long, task: () -> Unit)
}

/** What the controller needs from the device; a fake in the unit tests. */
internal interface BluetoothRateEnvironment {
    fun nowMs(): Long
    fun settingEnabled(): Boolean
    fun androidSupported(): Boolean
    fun source(): MixerRateRules.Source
    fun backend(source: MixerRateRules.Source): BluetoothRateBackend?
    fun codecState(address: String): BluetoothCodecState?
    /** Blocks up to [timeoutMs] until the stack announced [rateFlag] for the address; false when it did not (or cannot be heard). */
    fun awaitRate(address: String, rateFlag: Int, timeoutMs: Long): Boolean
    fun log(message: String)
}

/** What the playback service knows when a track is loaded. */
internal data class BluetoothRateRequest(
    /** The A2DP device's address the track goes to, null when it goes elsewhere. */
    val address: String?,
    val usbExclusive: Boolean,
    val dsd: Boolean,
    val trackHz: Int?,
)

/**
 * The Bluetooth sample-rate follow (ADR 0051). At a track change the service passes a [BluetoothRateRequest]; on the
 * controller's own thread this asks the privileged helper to set the A2DP codec's sample rate to the nearest rate the
 * codec offers, once per track, rate-limited and coalesced, and puts the original rate back when asked ([restoreAll]).
 * Nothing here runs on the main or playback thread, and nothing blocks the load: playback goes on whatever happens.
 */
internal class BluetoothRateController(
    private val env: BluetoothRateEnvironment,
    private val store: BluetoothRateStore,
    private val scheduler: BluetoothRateScheduler,
    private val breaker: BluetoothRateBreaker = sharedBreaker,
) {
    private val lock = Any()
    private var latest: BluetoothRateRequest? = null
    private var scheduled = false
    private var lastSwitchStartMs: Long? = null
    private val triedGone = HashSet<String>()

    /** A track is loaded (main thread; only memory is touched). */
    fun request(request: BluetoothRateRequest) {
        if (!BluetoothRateConfig.ENABLED || request.address == null) return
        val delay = synchronized(lock) {
            latest = request
            if (scheduled) return
            scheduled = true
            delayUntilAllowed()
        }
        scheduler.schedule(delay) { run() }
    }

    private fun delayUntilAllowed(): Long {
        val last = lastSwitchStartMs ?: return 0L
        return maxOf(0L, last + BluetoothRateConfig.MIN_INTERVAL_MS - env.nowMs())
    }

    /** Scheduler thread. */
    private fun run() {
        val request = synchronized(lock) {
            val wait = delayUntilAllowed()
            if (wait > 0L) {
                // A switch started in between: keep the minimum spacing.
                scheduler.schedule(wait) { run() }
                return
            }
            scheduled = false
            latest.also { latest = null }
        } ?: return
        try {
            process(request)
        } catch (error: Exception) {
            env.log("bluetooth rate: unexpected failure: $error")
        }
    }

    /** One track: decide and, when a switch is due, carry it out. Returns the plan (for the tests). */
    fun process(request: BluetoothRateRequest): BluetoothRateRules.Plan {
        val address = request.address
        val source = env.source()
        val plan = BluetoothRateRules.plan(
            BluetoothRateRules.Input(
                killSwitchCode = BluetoothRateConfig.ENABLED,
                settingEnabled = env.settingEnabled(),
                androidSupported = env.androidSupported(),
                source = source,
                breakerTripped = breaker.tripped,
                address = address,
                usbExclusive = request.usbExclusive,
                dsd = request.dsd,
                trackHz = request.trackHz,
                state = address?.let(env::codecState),
            ),
        )
        if (address == null || request.trackHz == null) return plan
        when (plan) {
            is BluetoothRateRules.Plan.Skip -> env.log("bluetooth rate: skipped, ${plan.reason.wire}")
            is BluetoothRateRules.Plan.AlreadyOk ->
                env.log("bluetooth rate: already at ${BluetoothRateRules.hzOfFlag(plan.rateFlag)} Hz")
            is BluetoothRateRules.Plan.Switch ->
                switchTo(source, BluetoothRateRules.HelperRequest.Set(address, plan.rateFlag), address)
            BluetoothRateRules.Plan.Ask ->
                switchTo(source, BluetoothRateRules.HelperRequest.Follow(address, request.trackHz), address)
        }
        return plan
    }

    private fun switchTo(
        source: MixerRateRules.Source,
        helper: BluetoothRateRules.HelperRequest,
        address: String,
    ) {
        val backend = env.backend(source) ?: return
        synchronized(lock) { lastSwitchStartMs = env.nowMs() }
        val result = call(backend, helper)
        when (result) {
            is BluetoothRateRules.HelperResult.Switched -> {
                if (!result.changed) {
                    env.log("bluetooth rate: already at ${BluetoothRateRules.hzOfFlag(result.to)} Hz")
                    return
                }
                val previous = store.all()[address]
                store.put(address, BluetoothRateRules.SavedRate(previous?.original ?: result.from, result.to))
                val heard = env.awaitRate(address, result.to, BluetoothRateConfig.WAIT_CHANGE_MS)
                env.log(
                    "bluetooth rate: ${BluetoothRateRules.hzOfFlag(result.from)} -> ${BluetoothRateRules.hzOfFlag(result.to)} Hz, " +
                        if (heard) "confirmed by the codec broadcast" else "not confirmed within ${BluetoothRateConfig.WAIT_CHANGE_MS} ms",
                )
            }
            is BluetoothRateRules.HelperResult.Failed -> env.log("bluetooth rate: failed, ${result.reason}")
            BluetoothRateRules.HelperResult.TimedOut -> env.log("bluetooth rate: the helper timed out")
            is BluetoothRateRules.HelperResult.State -> env.log("bluetooth rate: unexpected state answer")
        }
    }

    /** Runs the helper once with the breaker around it. */
    private fun call(
        backend: BluetoothRateBackend,
        helper: BluetoothRateRules.HelperRequest,
    ): BluetoothRateRules.HelperResult {
        synchronized(breaker) { breaker.switchStarted() }
        val result = try {
            backend.run(helper, BluetoothRateConfig.COMMAND_TIMEOUT_MS)
        } catch (error: Exception) {
            BluetoothRateRules.HelperResult.Failed(error.toString())
        }
        val ok = result is BluetoothRateRules.HelperResult.Switched || result is BluetoothRateRules.HelperResult.State
        synchronized(breaker) { breaker.switchEnded(env.nowMs(), ok) }
        if (breaker.tripped) env.log("bluetooth rate: disabled for this process, ${breaker.reason}")
        return result
    }

    /** An audio-server symptom (a sink or player error, any thread). */
    fun symptom(what: String) {
        val tripped = synchronized(breaker) { breaker.symptom(env.nowMs(), what) }
        if (tripped) env.log("bluetooth rate: disabled for this process, ${breaker.reason}")
    }

    /**
     * Put the original rate back on the controller's thread (the follow was turned off, the service ends). Only an
     * address whose codec still runs at the rate we set is touched; one that fails stays saved for the next time.
     */
    fun restoreAll(why: String, only: Set<String>? = null) {
        scheduler.schedule(0L) { restoreNow(why, only) }
    }

    /** Scheduler thread (public for the unit tests). [only]: the addresses to look at, null for every saved one. */
    fun restoreNow(why: String, only: Set<String>?) {
        val saved = store.all()
        if (saved.isEmpty() || breaker.tripped) return
        val source = env.source()
        val backend = env.backend(source) ?: return
        for ((address, rates) in saved) {
            if (only != null && address !in only) continue
            val state = env.codecState(address)
            if (!BluetoothRateRules.shouldRestore(rates, state)) {
                env.log("bluetooth rate: $why, $address is no longer at the rate we set, nothing to restore")
                store.remove(address)
                continue
            }
            val result = call(backend, BluetoothRateRules.HelperRequest.Set(address, rates.original))
            if (result is BluetoothRateRules.HelperResult.Switched) {
                store.remove(address)
                env.log("bluetooth rate: $why, $address back to ${BluetoothRateRules.hzOfFlag(rates.original)} Hz")
            } else {
                env.log("bluetooth rate: $why, could not restore $address")
            }
        }
    }

    /**
     * The attached A2DP devices changed (main thread; only memory is read): a device with a saved rate that is gone
     * gets one restore attempt (usually too late, the stack forgets it), and when the follow is off a device that is
     * back gets its original rate.
     */
    fun devicesChanged(attached: Set<String>) {
        val saved = store.all()
        if (saved.isEmpty()) {
            synchronized(lock) { triedGone.clear() }
            return
        }
        val settingOn = env.settingEnabled()
        val gone = HashSet<String>()
        synchronized(lock) {
            triedGone.retainAll(saved.keys - attached)
            for (address in saved.keys) {
                if (address !in attached && triedGone.add(address)) gone.add(address)
            }
        }
        if (gone.isNotEmpty()) restoreAll("device disconnected", gone)
        val back = saved.keys.filterTo(HashSet()) { it in attached }
        if (!settingOn && back.isNotEmpty()) restoreAll("follow is off", back)
    }

    companion object {
        /** Process-wide: a tripped breaker outlives a restarted playback service. */
        val sharedBreaker = BluetoothRateBreaker(BluetoothRateConfig.BREAKER_WINDOW_MS, BluetoothRateConfig.FAILURES_TO_TRIP)

        /** For the settings page: whether the breaker stopped the feature. */
        val breakerTripped: Boolean get() = sharedBreaker.tripped
    }
}
