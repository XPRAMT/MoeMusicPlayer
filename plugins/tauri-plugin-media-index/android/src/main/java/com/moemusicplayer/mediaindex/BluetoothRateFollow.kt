package com.moemusicplayer.mediaindex

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.util.Log
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** Runs the helper through `su` (ADR 0051). */
internal class RootBluetoothRateBackend(private val apkPath: String) : BluetoothRateBackend {
    override fun run(request: BluetoothRateRules.HelperRequest, timeoutMs: Long): BluetoothRateRules.HelperResult {
        val command = BluetoothRateRules.shellCommand(apkPath, request)
            ?: return BluetoothRateRules.HelperResult.Failed("the command could not be built")
        val result = RootAccess.exec(command, timeoutMs, OUTPUT_LIMIT)
        if (result.timedOut) return BluetoothRateRules.HelperResult.TimedOut
        return BluetoothRateRules.parseResult(result.stdout + "\n" + result.stderr)
    }
}

/** Runs the helper through Shizuku (the shell user; ADR 0051). */
internal class ShizukuBluetoothRateBackend(private val apkPath: String) : BluetoothRateBackend {
    override fun run(request: BluetoothRateRules.HelperRequest, timeoutMs: Long): BluetoothRateRules.HelperResult {
        val command = BluetoothRateRules.shellCommand(apkPath, request)
            ?: return BluetoothRateRules.HelperResult.Failed("the command could not be built")
        val result = ShizukuAccess.exec(command, timeoutMs, OUTPUT_LIMIT)
        if (result.timedOut) return BluetoothRateRules.HelperResult.TimedOut
        return BluetoothRateRules.parseResult(result.stdout + "\n" + result.stderr)
    }
}

private const val OUTPUT_LIMIT = 16 * 1024

/** The saved original rates, in SharedPreferences (`moe_bt_rate`, key: address, value: `original,applied`). */
internal class PrefsBluetoothRateStore(context: Context) : BluetoothRateStore {
    private val prefs = context.applicationContext.getSharedPreferences("moe_bt_rate", Context.MODE_PRIVATE)

    override fun all(): Map<String, BluetoothRateRules.SavedRate> {
        val out = LinkedHashMap<String, BluetoothRateRules.SavedRate>()
        for ((key, value) in prefs.all) {
            val parts = (value as? String)?.split(',') ?: continue
            val original = parts.getOrNull(0)?.toIntOrNull() ?: continue
            val applied = parts.getOrNull(1)?.toIntOrNull() ?: continue
            out[key] = BluetoothRateRules.SavedRate(original, applied)
        }
        return out
    }

    override fun put(address: String, saved: BluetoothRateRules.SavedRate) {
        prefs.edit().putString(address, "${saved.original},${saved.applied}").apply()
    }

    override fun remove(address: String) {
        prefs.edit().remove(address).apply()
    }
}

private class ExecutorScheduler : BluetoothRateScheduler {
    private val executor = ScheduledThreadPoolExecutor(1) { runnable ->
        Thread(runnable, "moe-bt-rate").apply { isDaemon = true }
    }.apply { removeOnCancelPolicy = true }

    override fun schedule(delayMs: Long, task: () -> Unit) {
        try {
            executor.schedule({ task() }, delayMs, TimeUnit.MILLISECONDS)
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            // The service is gone.
        }
    }

    /** Lets the queued tasks (the restore) finish, then ends the thread. */
    fun finish() = executor.shutdown()
}

/**
 * Wires the Bluetooth sample-rate follow (ADR 0051) to the device: the setting, root / Shizuku, the codec broadcast
 * ([BluetoothCodecWatch]) and the saved rates. One per playback service; a service-less restore (the setting turned
 * off while the app plays nothing) uses [restoreWithoutService].
 */
internal class BluetoothRateFollow private constructor(context: Context) {
    private val appContext = context.applicationContext
    private val scheduler = ExecutorScheduler()
    private val store = PrefsBluetoothRateStore(appContext)
    @Volatile private var enabled = false
    private val controller = BluetoothRateController(Environment(), store, scheduler)

    private inner class Environment : BluetoothRateEnvironment {
        override fun nowMs() = SystemClock.elapsedRealtime()
        override fun settingEnabled() = enabled
        override fun androidSupported() = BluetoothCodecWatch.supported()

        override fun source(): MixerRateRules.Source {
            if (RootAccess.state == MixerRateRules.RootState.UNKNOWN && MixerRatePrefs.rootOptIn(appContext)) {
                RootAccess.probe(MixerRateConfig.ROOT_SILENT_TIMEOUT_MS)
            }
            return MixerRateRules.selectSource(
                RootAccess.state, ShizukuAccess.state(appContext), true, true, false,
            )
        }

        override fun backend(source: MixerRateRules.Source): BluetoothRateBackend? {
            val apk = appContext.applicationInfo.sourceDir ?: return null
            return when (source) {
                MixerRateRules.Source.ROOT -> RootBluetoothRateBackend(apk)
                MixerRateRules.Source.SHIZUKU -> ShizukuBluetoothRateBackend(apk)
                MixerRateRules.Source.PLATFORM -> null
            }
        }

        override fun codecState(address: String) = BluetoothCodecWatch.state(address)

        override fun awaitRate(address: String, rateFlag: Int, timeoutMs: Long): Boolean =
            BluetoothCodecWatch.listening() && BluetoothCodecWatch.awaitRate(address, rateFlag, timeoutMs)

        override fun log(message: String) {
            Log.i(TAG, message)
        }
    }

    /** The setting (any thread). Turning it off puts the original rates back. */
    fun setEnabled(value: Boolean) {
        val was = enabled
        enabled = value
        if (was && !value) controller.restoreAll("follow turned off")
        if (value) BluetoothCodecWatch.ensure(appContext)
    }

    /** A track is loaded (main thread). */
    fun onLoad(request: BluetoothRateRequest) {
        if (enabled) controller.request(request)
    }

    /** The attached A2DP devices' addresses changed (main thread). */
    fun devicesChanged(attachedA2dp: Set<String>) = controller.devicesChanged(attachedA2dp)

    /** An audio-server symptom (any thread). */
    fun symptom(what: String) = controller.symptom(what)

    /** The service ends: the original rates are put back on the worker thread, which then stops. */
    fun shutdown() {
        controller.restoreAll("service ended")
        scheduler.finish()
    }

    /** At service start: saved rates of an earlier run are put back when the follow is off. */
    fun restorePendingIfOff() {
        if (!enabled && store.all().isNotEmpty()) controller.restoreAll("follow is off")
    }

    companion object {
        private const val TAG = "MoeMediaIndex"

        fun create(context: Context): BluetoothRateFollow = BluetoothRateFollow(context)

        /** The setting was turned off while no playback service runs: a short-lived worker puts the rates back. */
        fun restoreWithoutService(context: Context) {
            val follow = BluetoothRateFollow(context)
            if (follow.store.all().isEmpty() || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                follow.scheduler.finish()
                return
            }
            follow.controller.restoreAll("follow turned off")
            follow.scheduler.finish()
        }
    }
}
