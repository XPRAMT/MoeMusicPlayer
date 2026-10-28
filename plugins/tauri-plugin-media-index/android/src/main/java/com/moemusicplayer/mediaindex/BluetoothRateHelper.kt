package com.moemusicplayer.mediaindex

import android.bluetooth.BluetoothA2dp
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothCodecConfig
import android.bluetooth.BluetoothCodecStatus
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.AttributionSource
import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import android.os.Looper
import android.os.Process

/**
 * The privileged half of the Bluetooth sample-rate follow (ADR 0051). `BluetoothA2dp.setCodecConfigPreference` is a
 * system API (BLUETOOTH_PRIVILEGED), so an app cannot call it; this class is started as its own process by root
 * (`su -c`) or by Shizuku (the shell user) with
 * `CLASSPATH=<this APK> app_process /system/bin com.moemusicplayer.mediaindex.BluetoothRateHelper <command>`
 * and prints one `RESULT …` line ([BluetoothRateRules.parseResult]). It lives in the app's own APK, runs for under a
 * second per call, and only changes the sample rate of the current codec configuration (codec, priority, bits,
 * channel mode and codec-specific values are copied from the current one).
 *
 * A bare `app_process` never ran `ActivityThread.main`, so the Bluetooth module's service manager was never handed to
 * `BluetoothFrameworkInitializer` and `BluetoothAdapter.getDefaultAdapter()` returns null; [adapterOf] hands it over
 * itself and builds the adapter with the shell/root attribution. The main looper is prepared first because the
 * profile proxy calls back on it. Every step names itself in the error text.
 *
 * Not verified on a device: the whole path above, and that the shell/root user passes the Bluetooth service's
 * permission checks; any exception text is returned in the `ERR` line.
 */
@android.annotation.SuppressLint("NewApi", "PrivateApi", "DiscouragedPrivateApi") // only ever run by app_process on Android 13+ (carryOut checks), never loaded by the app
object BluetoothRateHelper {
    private const val WATCHDOG_MS = 6_000L

    /** The step now running, for the timeout and error texts. */
    @Volatile
    private var step = "start"

    private class StepFailure(step: String, cause: Throwable) :
        Exception(BluetoothRateRules.stepError(step, describe(cause)), cause)

    private fun <T> inStep(name: String, block: () -> T): T {
        step = name
        try {
            return block()
        } catch (error: StepFailure) {
            throw error
        } catch (error: Throwable) {
            throw StepFailure(name, error)
        }
    }

    @JvmStatic
    fun main(args: Array<String>) {
        val request = BluetoothRateRules.parseRequest(args.toList())
        if (request == null) {
            finish(BluetoothRateRules.HelperResult.Failed("usage: follow|set|state <address> <number>"))
            return
        }
        Thread {
            Thread.sleep(WATCHDOG_MS)
            finish(BluetoothRateRules.HelperResult.Failed("timeout at step $step"))
        }.apply { isDaemon = true }.start()
        try {
            inStep("looper") {
                try {
                    Looper.prepareMainLooper()
                } catch (_: IllegalStateException) {
                    // already prepared
                }
            }
            val looper = Looper.getMainLooper()
            val context = inStep("system-context") { shellContext() }
            val adapter = adapterOf(context)
            val address = when (request) {
                is BluetoothRateRules.HelperRequest.Follow -> request.address
                is BluetoothRateRules.HelperRequest.Set -> request.address
                is BluetoothRateRules.HelperRequest.State -> request.address
            }
            val ok = inStep("profile-proxy") {
                adapter.getProfileProxy(context, object : BluetoothProfile.ServiceListener {
                    override fun onServiceConnected(profile: Int, proxy: BluetoothProfile) {
                        val result = try {
                            carryOut(adapter, proxy as BluetoothA2dp, request, address)
                        } catch (error: Throwable) {
                            BluetoothRateRules.HelperResult.Failed(describe(error))
                        }
                        finish(result)
                        looper.quit()
                    }

                    override fun onServiceDisconnected(profile: Int) = Unit
                }, BluetoothProfile.A2DP)
            }
            if (!ok) return finish(BluetoothRateRules.HelperResult.Failed("profile-proxy: getProfileProxy refused"))
            step = "profile-proxy-wait"
            Looper.loop()
        } catch (error: Throwable) {
            finish(BluetoothRateRules.HelperResult.Failed(describe(error)))
        }
    }

    private fun describe(error: Throwable): String {
        if (error is StepFailure) return error.message ?: "unknown"
        val cause = (error as? java.lang.reflect.InvocationTargetException)?.targetException ?: error.cause ?: error
        return "${cause.javaClass.simpleName}: ${cause.message}"
    }

    @Synchronized
    private fun finish(result: BluetoothRateRules.HelperResult) {
        if (result is BluetoothRateRules.HelperResult.State) {
            println(BluetoothRateRules.describeState(result.codecType, result.rateFlag, result.selectableMask, result.specifics))
        }
        println(BluetoothRateRules.formatResult(result))
        System.out.flush()
        Runtime.getRuntime().halt(0)
    }

    private fun attribution(): AttributionSource =
        AttributionSource.Builder(Process.myUid())
            .setPackageName(BluetoothRateRules.packageForUid(Process.myUid()))
            .build()

    /** The system context of a bare `app_process`, speaking as the shell/root package so the Bluetooth service's package checks pass. */
    private fun shellContext(): Context {
        val activityThread = Class.forName("android.app.ActivityThread")
        val thread = activityThread.getMethod("systemMain").invoke(null)
        val system = activityThread.getMethod("getSystemContext").invoke(thread) as Context
        val packageName = BluetoothRateRules.packageForUid(Process.myUid())
        return object : ContextWrapper(system) {
            override fun getPackageName(): String = packageName
            override fun getOpPackageName(): String = packageName
            override fun getApplicationContext(): Context = this
            override fun getAttributionSource(): AttributionSource = attribution()
        }
    }

    /**
     * Gives the Bluetooth module the service manager an `ActivityThread.main` would have (null in a bare
     * `app_process`, which is why the default adapter is null there). Both classes are module system APIs.
     */
    private fun ensureServiceManager() {
        val initializer = Class.forName("android.bluetooth.BluetoothFrameworkInitializer")
        if (initializer.getMethod("getBluetoothServiceManager").invoke(null) != null) return
        val manager = Class.forName("android.os.BluetoothServiceManager").getConstructor().newInstance()
        initializer.getMethod("setBluetoothServiceManager", manager.javaClass).invoke(null, manager)
    }

    /** The adapter, by the most direct way first; the texts of the ways that failed are kept for the error. */
    private fun adapterOf(context: Context): BluetoothAdapter {
        step = "adapter"
        val failures = ArrayList<String>()
        fun attempt(name: String, block: () -> BluetoothAdapter?): BluetoothAdapter? = try {
            block() ?: run { failures.add("$name: null"); null }
        } catch (error: Throwable) {
            failures.add("$name: ${describe(error)}")
            null
        }
        attempt("service-manager") { ensureServiceManager(); null }
        attempt("createAdapter") {
            BluetoothAdapter::class.java.getDeclaredMethod("createAdapter", AttributionSource::class.java)
                .apply { isAccessible = true }
                .invoke(null, attribution()) as? BluetoothAdapter
        }?.let { return it }
        attempt("BluetoothManager") {
            (context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager)?.adapter
        }?.let { return it }
        attempt("getDefaultAdapter") { BluetoothAdapter.getDefaultAdapter() }?.let { return it }
        throw StepFailure("adapter", IllegalStateException("no Bluetooth adapter (${failures.joinToString("; ")})"))
    }

    private fun statusOf(proxy: BluetoothA2dp, device: BluetoothDevice): BluetoothCodecStatus =
        proxy.javaClass.getMethod("getCodecStatus", BluetoothDevice::class.java).invoke(proxy, device) as? BluetoothCodecStatus
            ?: throw IllegalStateException("no codec status for ${device.address} (not connected as A2DP?)")

    private fun maskOf(status: BluetoothCodecStatus, current: BluetoothCodecConfig): Int {
        var mask = current.sampleRate
        for (capability in status.codecsSelectableCapabilities.orEmpty()) {
            if (capability.codecType == current.codecType) mask = mask or capability.sampleRate
        }
        return mask
    }

    private fun carryOut(
        adapter: BluetoothAdapter,
        proxy: BluetoothA2dp,
        request: BluetoothRateRules.HelperRequest,
        address: String,
    ): BluetoothRateRules.HelperResult {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return BluetoothRateRules.HelperResult.Failed("needs Android 13 or newer")
        }
        val device = inStep("remote-device") { adapter.getRemoteDevice(address) }
        val status = inStep("codec-status") { statusOf(proxy, device) }
        val current = status.codecConfig ?: return BluetoothRateRules.HelperResult.Failed("codec-status: no current codec")
        val mask = maskOf(status, current)
        val target = when (request) {
            is BluetoothRateRules.HelperRequest.State ->
                return BluetoothRateRules.HelperResult.State(
                    current.codecType,
                    current.sampleRate,
                    mask,
                    listOf(current.codecSpecific1, current.codecSpecific2, current.codecSpecific3, current.codecSpecific4),
                )
            is BluetoothRateRules.HelperRequest.Follow ->
                BluetoothRateRules.pickRate(request.trackHz, mask)
                    ?: return BluetoothRateRules.HelperResult.Failed("the codec offers no rate for ${request.trackHz} Hz")
            is BluetoothRateRules.HelperRequest.Set -> {
                if (mask and request.rateFlag == 0) {
                    return BluetoothRateRules.HelperResult.Failed("the codec does not offer rate flag ${request.rateFlag}")
                }
                request.rateFlag
            }
        }
        if (target == current.sampleRate) {
            return BluetoothRateRules.HelperResult.Switched(current.codecType, current.sampleRate, target, false)
        }
        val builder = BluetoothCodecConfig.Builder()
            .setCodecType(current.codecType)
            .setCodecPriority(current.codecPriority)
            .setSampleRate(target)
            .setBitsPerSample(current.bitsPerSample)
            .setChannelMode(current.channelMode)
            .setCodecSpecific1(current.codecSpecific1)
            .setCodecSpecific2(current.codecSpecific2)
            .setCodecSpecific3(current.codecSpecific3)
            .setCodecSpecific4(current.codecSpecific4)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            current.extendedCodecType?.let { builder.setExtendedCodecType(it) }
        }
        inStep("set-codec-preference") {
            proxy.javaClass.getMethod("setCodecConfigPreference", BluetoothDevice::class.java, BluetoothCodecConfig::class.java)
                .invoke(proxy, device, builder.build())
        }
        return BluetoothRateRules.HelperResult.Switched(current.codecType, current.sampleRate, target, true)
    }
}
