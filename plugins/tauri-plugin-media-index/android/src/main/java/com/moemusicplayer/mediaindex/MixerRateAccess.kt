package com.moemusicplayer.mediaindex

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.util.Log
import rikka.shizuku.Shizuku
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** What a bounded process run produced. */
internal class ProcessResult(val stdout: String, val stderr: String, val exitCode: Int?, val timedOut: Boolean)

/** Runs a process with a hard time limit and a size limit on what is kept (the rest is read and dropped). */
internal object BoundedProcess {
    fun read(process: Process, timeoutMs: Long, maxBytes: Int): ProcessResult {
        val out = Capture(process.inputStream, maxBytes)
        val err = Capture(process.errorStream, 16 * 1024)
        val done = CountDownLatch(2)
        for ((name, capture) in listOf("moe-proc-out" to out, "moe-proc-err" to err)) {
            Thread {
                try {
                    capture.run()
                } finally {
                    done.countDown()
                }
            }.apply {
                this.name = name
                isDaemon = true
            }.start()
        }
        val finished = done.await(timeoutMs, TimeUnit.MILLISECONDS)
        if (!finished) {
            try {
                process.destroy()
            } catch (_: RuntimeException) {
            }
        }
        var exit: Int? = null
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(500)
        while (exit == null && System.nanoTime() < deadline) {
            exit = try {
                process.exitValue()
            } catch (_: IllegalThreadStateException) {
                Thread.sleep(20)
                null
            }
        }
        return ProcessResult(out.text(), err.text(), exit, !finished)
    }

    private class Capture(private val stream: InputStream, private val max: Int) {
        private val buffer = ByteArrayOutputStream()

        fun run() {
            val chunk = ByteArray(8192)
            try {
                while (true) {
                    val n = stream.read(chunk)
                    if (n < 0) break
                    synchronized(buffer) {
                        val room = max - buffer.size()
                        if (room > 0) buffer.write(chunk, 0, minOf(room, n))
                    }
                }
            } catch (_: java.io.IOException) {
                // The process was destroyed or closed its end.
            }
        }

        fun text(): String = synchronized(buffer) { buffer.toString(Charsets.UTF_8.name()) }
    }
}

/** The small amount of state the lookup keeps across runs of the app. */
internal object MixerRatePrefs {
    private const val FILE = "moe_mixer_rate"

    fun rootOptIn(context: Context): Boolean =
        context.getSharedPreferences(FILE, Context.MODE_PRIVATE).getBoolean("rootOptIn", false)

    fun setRootOptIn(context: Context, value: Boolean) {
        context.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().putBoolean("rootOptIn", value).apply()
    }
}

/**
 * Root through `su` (ADR 0048). Nothing runs until the user asked in the explanation panel ([MixerRatePrefs.rootOptIn]):
 * the first `su` call of an app makes the root manager show its own grant dialog, which the app must not provoke.
 */
internal object RootAccess {
    @Volatile var state = MixerRateRules.RootState.UNKNOWN
        private set

    /** `su -c id`; AVAILABLE when it exits with 0 and prints `uid=0`. A refusal, a timeout or a missing `su` is UNAVAILABLE. */
    fun probe(timeoutMs: Long): MixerRateRules.RootState {
        val result = try {
            BoundedProcess.read(ProcessBuilder("su", "-c", "id").start(), timeoutMs, 4096)
        } catch (error: Exception) {
            Log.i(TAG, "su is not available: $error")
            state = MixerRateRules.RootState.UNAVAILABLE
            return state
        }
        state = if (!result.timedOut && result.exitCode == 0 && result.stdout.contains("uid=0")) {
            MixerRateRules.RootState.AVAILABLE
        } else {
            MixerRateRules.RootState.UNAVAILABLE
        }
        Log.i(TAG, "root probe: ${state.name}, exit ${result.exitCode}, timedOut ${result.timedOut}")
        return state
    }

    /** Forget the answer so the next lookup asks again (after a failed run). */
    fun forget() {
        state = MixerRateRules.RootState.UNKNOWN
    }

    fun dumpsys(service: String, timeoutMs: Long, maxBytes: Int): ProcessResult =
        BoundedProcess.read(ProcessBuilder("su", "-c", "dumpsys $service").start(), timeoutMs, maxBytes)

    private const val TAG = "MoeMediaIndex"
}

/**
 * Shizuku (ADR 0048). Every call is guarded: Shizuku throws when its binder is not there. The command runs through
 * `Shizuku.newProcess` (private in API 13.1.5, reached by reflection as RedMagicX does, kept by the consumer rules):
 * a user service would need an AIDL interface, a second process and a bind lifecycle for one `dumpsys` call.
 */
internal object ShizukuAccess {
    const val PACKAGE = "moe.shizuku.privileged.api"
    private const val TAG = "MoeMediaIndex"
    private const val REQUEST_CODE = 4801

    fun installed(context: Context): Boolean = try {
        context.packageManager.getPackageInfo(PACKAGE, 0)
        true
    } catch (_: PackageManager.NameNotFoundException) {
        false
    } catch (_: RuntimeException) {
        false
    }

    fun binderAlive(): Boolean = try {
        Shizuku.pingBinder()
    } catch (_: Throwable) {
        false
    }

    fun state(context: Context): MixerRateRules.ShizukuState {
        val alive = binderAlive()
        var preV11 = false
        var granted = false
        var rationale = false
        if (alive) {
            try {
                preV11 = Shizuku.isPreV11()
                granted = !preV11 && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
                rationale = !preV11 && !granted && Shizuku.shouldShowRequestPermissionRationale()
            } catch (error: Throwable) {
                Log.i(TAG, "Shizuku state could not be read: $error")
                return MixerRateRules.ShizukuState.NOT_RUNNING
            }
        }
        return MixerRateRules.shizukuState(installed(context), alive, preV11, granted, rationale)
    }

    /** Shows Shizuku's own dialog; [onResult] gets the state afterwards (main thread), also when nothing answers in time. */
    fun requestPermission(context: Context, onResult: (MixerRateRules.ShizukuState) -> Unit) {
        val main = Handler(Looper.getMainLooper())
        var delivered = false
        lateinit var listener: Shizuku.OnRequestPermissionResultListener
        fun deliver() {
            if (delivered) return
            delivered = true
            try {
                Shizuku.removeRequestPermissionResultListener(listener)
            } catch (_: Throwable) {
            }
            onResult(state(context))
        }
        listener = Shizuku.OnRequestPermissionResultListener { requestCode, _ ->
            if (requestCode == REQUEST_CODE) main.post { deliver() }
        }
        try {
            Shizuku.addRequestPermissionResultListener(listener)
            Shizuku.requestPermission(REQUEST_CODE)
            main.postDelayed({ deliver() }, 60_000L)
        } catch (error: Throwable) {
            Log.i(TAG, "Shizuku permission request failed: $error")
            deliver()
        }
    }

    fun launchIntent(context: Context): Intent? =
        context.packageManager.getLaunchIntentForPackage(PACKAGE)?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    fun dumpsys(service: String, timeoutMs: Long, maxBytes: Int): ProcessResult {
        val method = Shizuku::class.java.getDeclaredMethod(
            "newProcess",
            Array<String>::class.java,
            Array<String>::class.java,
            String::class.java,
        )
        method.isAccessible = true
        val process = method.invoke(null, arrayOf("dumpsys", service), null, null) as Process
        return BoundedProcess.read(process, timeoutMs, maxBytes)
    }
}
