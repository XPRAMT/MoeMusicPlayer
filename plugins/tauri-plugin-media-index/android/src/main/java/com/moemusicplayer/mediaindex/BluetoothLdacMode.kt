package com.moemusicplayer.mediaindex

/** Constants of the LDAC bitrate-mode read (ADR 0051 "LDAC bitrate mode"). */
internal object BluetoothLdacModeConfig {
    /** Two reads start at least this far apart; requests in between are coalesced. */
    const val MIN_INTERVAL_MS = 2_000L

    /** Waits this long after the first request so that a burst of codec announcements is read once. */
    const val DEBOUNCE_MS = 500L

    /** One privileged helper process is given this long. */
    const val COMMAND_TIMEOUT_MS = 8_000L

    /** This many failed reads in a row stop the reading for the rest of the process. */
    const val FAILURES_TO_STOP = 3
}

/** Pure rules of the LDAC bitrate-mode read. */
internal object BluetoothLdacModeRules {
    /**
     * Whether the privileged read is worth doing: the stack announced LDAC for the address, the mode is not known since
     * the last announcement, and a root / Shizuku source exists. Without a source nothing is read and nothing is shown.
     */
    fun wanted(codecType: Int?, known: Boolean, supported: Boolean, hasSource: Boolean, stopped: Boolean): Boolean =
        supported && hasSource && !stopped && !known && codecType == BluetoothCodecRules.CODEC_LDAC

    /** The mode (`codecSpecific1`) of a helper answer, or null when the answer has none. */
    fun modeOf(result: BluetoothRateRules.HelperResult): Long? =
        (result as? BluetoothRateRules.HelperResult.State)
            ?.takeIf { it.codecType == BluetoothCodecRules.CODEC_LDAC }
            ?.specifics?.firstOrNull()
}

/** What the reader needs from the device; a fake in the unit tests. */
internal interface BluetoothLdacModeEnvironment {
    fun nowMs(): Long
    fun supported(): Boolean
    fun source(): MixerRateRules.Source
    fun backend(source: MixerRateRules.Source): BluetoothRateBackend?
    /** The codec type the stack announced for the address, null when none was. */
    fun codecType(address: String): Int?
    /** Whether the mode was read since the last announcement. */
    fun known(address: String): Boolean
    fun store(address: String, mode: Long)
    fun log(message: String)
}

/**
 * Reads LDAC's bitrate mode (`codecSpecific1`) through the privileged helper (root or an authorized Shizuku;
 * `getCodecSpecific1` is not callable by an app). It runs on the scheduler's own thread, never on the main or playback
 * thread, is triggered by a codec announcement or a track change while the codec is LDAC (never polled), is
 * rate-limited and coalesced, and stops for the process after repeated failures.
 */
internal class BluetoothLdacModeReader(
    private val env: BluetoothLdacModeEnvironment,
    private val scheduler: BluetoothRateScheduler,
) {
    private val lock = Any()
    private val pending = LinkedHashSet<String>()
    private var scheduled = false
    private var lastStartMs: Long? = null
    private var failures = 0
    @Volatile private var stopped = false

    /** A codec announcement or a track change (any thread; only memory is read here). */
    fun request(address: String) {
        if (stopped || !env.supported() || env.codecType(address) != BluetoothCodecRules.CODEC_LDAC || env.known(address)) return
        val delay = synchronized(lock) {
            pending.add(address)
            if (scheduled) return
            scheduled = true
            maxOf(BluetoothLdacModeConfig.DEBOUNCE_MS, untilAllowed())
        }
        scheduler.schedule(delay) { run() }
    }

    private fun untilAllowed(): Long {
        val last = lastStartMs ?: return 0L
        return maxOf(0L, last + BluetoothLdacModeConfig.MIN_INTERVAL_MS - env.nowMs())
    }

    /** Scheduler thread (public for the unit tests). */
    fun run() {
        val addresses = synchronized(lock) {
            val wait = untilAllowed()
            if (wait > 0L) {
                scheduler.schedule(wait) { run() }
                return
            }
            scheduled = false
            lastStartMs = env.nowMs()
            ArrayList(pending).also { pending.clear() }
        }
        try {
            val source = env.source()
            val backend = if (source == MixerRateRules.Source.PLATFORM) null else env.backend(source)
            for (address in addresses) {
                val wanted = BluetoothLdacModeRules.wanted(
                    env.codecType(address), env.known(address), env.supported(), backend != null, stopped,
                )
                if (!wanted || backend == null) continue
                read(backend, address)
            }
        } catch (error: Exception) {
            env.log("bluetooth ldac mode: unexpected failure: $error")
        }
    }

    private fun read(backend: BluetoothRateBackend, address: String) {
        val result = try {
            backend.run(BluetoothRateRules.HelperRequest.State(address), BluetoothLdacModeConfig.COMMAND_TIMEOUT_MS)
        } catch (error: Exception) {
            BluetoothRateRules.HelperResult.Failed(error.toString())
        }
        val mode = BluetoothLdacModeRules.modeOf(result)
        if (mode != null) {
            failures = 0
            env.store(address, mode)
            env.log("bluetooth ldac mode: codecSpecific1=$mode")
            return
        }
        // A non-LDAC answer (the headset switched codec meanwhile) is not a failure of the helper.
        if (result is BluetoothRateRules.HelperResult.State && result.codecType != BluetoothCodecRules.CODEC_LDAC) return
        if (PrivilegedRunnerErrors.isRunnerBug(result)) {
            env.log("bluetooth ldac mode: read failed in the runner, not counted")
        } else if (++failures >= BluetoothLdacModeConfig.FAILURES_TO_STOP) {
            stopped = true
            env.log("bluetooth ldac mode: stopped for this process after $failures failed reads")
        } else {
            val why = (result as? BluetoothRateRules.HelperResult.Failed)?.reason ?: "no mode in the answer"
            env.log("bluetooth ldac mode: read failed, $why")
        }
    }
}
