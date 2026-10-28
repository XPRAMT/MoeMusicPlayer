package com.moemusicplayer.mediaindex

/** Constants of the proactive read of the Bluetooth codec status (ADR 0051 "Knowing the codec reliably"). */
internal object BluetoothCodecStatusConfig {
    const val ENABLED = true

    /** The wait before the 1st, 2nd, ... read of an unknown codec; after the last one the reading stops until re-armed. */
    val BACKOFF_MS = longArrayOf(500L, 1_000L, 2_000L, 4_000L)

    /** One privileged helper process is given this long. */
    const val COMMAND_TIMEOUT_MS = 8_000L

    const val FAILURES_TO_TRIP = 3
    const val BREAKER_WINDOW_MS = 8_000L
}

/** What the reader needs from the device; a fake in the unit tests. */
internal interface BluetoothCodecStatusEnvironment {
    fun nowMs(): Long
    fun supported(): Boolean
    fun source(): MixerRateRules.Source
    fun backend(source: MixerRateRules.Source): BluetoothRateBackend?

    /** Whether the codec of the address is known (announced by the stack or read before). */
    fun known(address: String): Boolean
    fun store(address: String, state: BluetoothRateRules.HelperResult.State)
    fun log(message: String)
}

/**
 * Makes the codec status of an A2DP device known when root or an authorized Shizuku exists. The stack's codec
 * broadcast is not sticky, so after an app start, a service restart or for a headset that was already connected nothing
 * arrives until the codec changes; this reads it through the helper (`state`) instead: when the service starts with an
 * A2DP route, at every track load, when the output page asks, at a connection or route change, and after a codec
 * change nobody confirmed. A read that fails is retried with a growing wait ([BluetoothCodecStatusConfig.BACKOFF_MS]);
 * a known codec is never read (except a forced refresh). Everything runs on the scheduler's own thread; a failure of
 * the runner itself is not counted against the breaker. Without a source nothing is read and nothing is guessed.
 */
internal class BluetoothCodecStatusReader(
    private val env: BluetoothCodecStatusEnvironment,
    private val scheduler: BluetoothRateScheduler,
    private val breaker: BluetoothRateBreaker = sharedBreaker,
) {
    private val lock = Any()
    private val attempts = HashMap<String, Int>()
    private val pending = HashSet<String>()

    /**
     * Ask for the codec of [address] to be known (any thread; only memory is read here). [rearm] starts the backoff
     * again (a connection, a route change, a track load); [force] reads even when the codec is known (a change that
     * nobody confirmed).
     */
    fun request(address: String, rearm: Boolean = false, force: Boolean = false) {
        if (!BluetoothCodecStatusConfig.ENABLED || !env.supported()) return
        if (!force && env.known(address)) {
            synchronized(lock) { attempts.remove(address) }
            return
        }
        enqueue(address, force, reset = rearm || force)
    }

    private fun enqueue(address: String, force: Boolean, reset: Boolean) {
        val delay = synchronized(lock) {
            if (address in pending) return
            if (reset) attempts[address] = 0
            val attempt = attempts[address] ?: 0
            if (attempt >= BluetoothCodecStatusConfig.BACKOFF_MS.size) return
            pending.add(address)
            BluetoothCodecStatusConfig.BACKOFF_MS[attempt]
        }
        scheduler.schedule(delay) { run(address, force) }
    }

    /** Scheduler thread (public for the unit tests). */
    fun run(address: String, force: Boolean) {
        synchronized(lock) { pending.remove(address) }
        if (!force && env.known(address)) {
            synchronized(lock) { attempts.remove(address) }
            return
        }
        if (breaker.tripped) return
        val source = env.source()
        if (source == MixerRateRules.Source.PLATFORM) return
        val backend = env.backend(source) ?: return
        synchronized(breaker) { breaker.switchStarted() }
        val result = try {
            backend.run(BluetoothRateRules.HelperRequest.State(address), BluetoothCodecStatusConfig.COMMAND_TIMEOUT_MS)
        } catch (error: Exception) {
            BluetoothRateRules.HelperResult.Failed(error.toString())
        }
        val ok = result is BluetoothRateRules.HelperResult.State
        synchronized(breaker) {
            if (PrivilegedRunnerErrors.isRunnerBug(result)) breaker.switchIgnored() else breaker.switchEnded(env.nowMs(), ok)
        }
        if (breaker.tripped) {
            env.log("bluetooth codec status: disabled for this process, ${breaker.reason}")
            return
        }
        if (result is BluetoothRateRules.HelperResult.State) {
            synchronized(lock) { attempts.remove(address) }
            env.store(address, result)
            env.log("bluetooth codec status: read codec ${result.codecType} rate flag ${result.rateFlag}")
            return
        }
        val next = synchronized(lock) { (attempts[address] ?: 0) + 1 }.also { synchronized(lock) { attempts[address] = it } }
        val why = (result as? BluetoothRateRules.HelperResult.Failed)?.reason ?: "no answer"
        env.log("bluetooth codec status: read $next failed, $why")
        // The next try waits longer; a forced refresh that failed is not retried forever either.
        enqueue(address, force, reset = false)
    }

    companion object {
        /** Process-wide, like the other breakers. */
        val sharedBreaker = BluetoothRateBreaker(BluetoothCodecStatusConfig.BREAKER_WINDOW_MS, BluetoothCodecStatusConfig.FAILURES_TO_TRIP)
    }
}
