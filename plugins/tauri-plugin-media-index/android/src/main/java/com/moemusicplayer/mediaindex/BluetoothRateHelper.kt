package com.moemusicplayer.mediaindex

import android.bluetooth.BluetoothA2dp
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothCodecConfig
import android.bluetooth.BluetoothCodecStatus
import android.bluetooth.BluetoothDevice
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
 * Not verified on a device: the system-context trick below (as scrcpy's FakeContext does) and that the shell user
 * passes the Bluetooth service's permission checks; any exception text is returned in the `ERR` line.
 */
@android.annotation.SuppressLint("NewApi") // only ever run by app_process on Android 13+ (carryOut checks), never loaded by the app
object BluetoothRateHelper {
    private const val WATCHDOG_MS = 6_000L

    @JvmStatic
    fun main(args: Array<String>) {
        val request = BluetoothRateRules.parseRequest(args.toList())
        if (request == null) {
            finish(BluetoothRateRules.HelperResult.Failed("usage: follow|set|state <address> <number>"))
            return
        }
        Thread {
            Thread.sleep(WATCHDOG_MS)
            finish(BluetoothRateRules.HelperResult.TimedOut)
        }.apply { isDaemon = true }.start()
        try {
            Looper.prepare()
            val looper = Looper.myLooper()!!
            val adapter = BluetoothAdapter.getDefaultAdapter()
                ?: return finish(BluetoothRateRules.HelperResult.Failed("no Bluetooth adapter"))
            val context = shellContext()
            val ok = adapter.getProfileProxy(context, object : BluetoothProfile.ServiceListener {
                override fun onServiceConnected(profile: Int, proxy: BluetoothProfile) {
                    val result = try {
                        carryOut(adapter, proxy as BluetoothA2dp, request)
                    } catch (error: Throwable) {
                        BluetoothRateRules.HelperResult.Failed(describe(error))
                    }
                    finish(result)
                    looper.quit()
                }

                override fun onServiceDisconnected(profile: Int) = Unit
            }, BluetoothProfile.A2DP)
            if (!ok) return finish(BluetoothRateRules.HelperResult.Failed("getProfileProxy refused"))
            Looper.loop()
        } catch (error: Throwable) {
            finish(BluetoothRateRules.HelperResult.Failed(describe(error)))
        }
    }

    private fun describe(error: Throwable): String {
        val cause = error.cause ?: error
        return "${cause.javaClass.simpleName}: ${cause.message}"
    }

    @Synchronized
    private fun finish(result: BluetoothRateRules.HelperResult) {
        println(BluetoothRateRules.formatResult(result))
        System.out.flush()
        Runtime.getRuntime().halt(0)
    }

    /** The system context of a bare `app_process`, speaking as the shell package so the Bluetooth service's package checks pass. */
    private fun shellContext(): Context {
        val activityThread = Class.forName("android.app.ActivityThread")
        val thread = activityThread.getMethod("systemMain").invoke(null)
        val system = activityThread.getMethod("getSystemContext").invoke(thread) as Context
        return object : ContextWrapper(system) {
            override fun getPackageName(): String = "com.android.shell"
            override fun getOpPackageName(): String = "com.android.shell"
            override fun getApplicationContext(): Context = this
            override fun getAttributionSource(): AttributionSource =
                AttributionSource.Builder(Process.myUid()).setPackageName("com.android.shell").build()
        }
    }

    private fun statusOf(proxy: BluetoothA2dp, device: BluetoothDevice): BluetoothCodecStatus =
        proxy.javaClass.getMethod("getCodecStatus", BluetoothDevice::class.java).invoke(proxy, device) as? BluetoothCodecStatus
            ?: throw IllegalStateException("no codec status for ${device.address}")

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
    ): BluetoothRateRules.HelperResult {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return BluetoothRateRules.HelperResult.Failed("needs Android 13 or newer")
        }
        val address = when (request) {
            is BluetoothRateRules.HelperRequest.Follow -> request.address
            is BluetoothRateRules.HelperRequest.Set -> request.address
            is BluetoothRateRules.HelperRequest.State -> request.address
        }
        val device = adapter.getRemoteDevice(address)
        val status = statusOf(proxy, device)
        val current = status.codecConfig ?: return BluetoothRateRules.HelperResult.Failed("no current codec")
        val mask = maskOf(status, current)
        val target = when (request) {
            is BluetoothRateRules.HelperRequest.State ->
                return BluetoothRateRules.HelperResult.State(current.codecType, current.sampleRate, mask)
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
        proxy.javaClass.getMethod("setCodecConfigPreference", BluetoothDevice::class.java, BluetoothCodecConfig::class.java)
            .invoke(proxy, device, builder.build())
        return BluetoothRateRules.HelperResult.Switched(current.codecType, current.sampleRate, target, true)
    }
}
