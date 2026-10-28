package com.moemusicplayer.mediaindex

import android.Manifest
import android.bluetooth.BluetoothCodecStatus
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.annotation.ChecksSdkIntAtLeast
import androidx.annotation.RequiresApi
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

/**
 * The Bluetooth codec each A2DP device streams with (ADR 0044). The public SDK has no call that reads the current
 * codec (`BluetoothA2dp.getCodecStatus` is a system API), but the Bluetooth stack broadcasts the codec status with
 * the public `BluetoothCodecStatus.EXTRA_CODEC_STATUS` whenever it is set (on connection and on every change). This
 * keeps the last one per device address, so it is known only after such a broadcast while the process runs. Needs
 * Android 13+ (the codec classes) and BLUETOOTH_CONNECT (the broadcast is sent to holders of it).
 */
internal object BluetoothCodecWatch {
    private const val TAG = "MoeMediaIndex"
    /** The broadcast the Bluetooth stack sends with the codec status (an action string, not an API call). */
    private const val ACTION_CODEC_CONFIG_CHANGED = "android.bluetooth.a2dp.profile.action.CODEC_CONFIG_CHANGED"

    private val codecs = ConcurrentHashMap<String, BluetoothCodecFacts>()
    /** LDAC's `codecSpecific1` per address, read by the privileged helper (ADR 0051); dropped at every codec announcement. */
    private val ldacModes = ConcurrentHashMap<String, Long>()
    /** Called (on the broadcast thread, memory only) with the address and codec type after each announcement. */
    @Volatile var onAnnounced: ((address: String, codecType: Int) -> Unit)? = null
    /** The same announcement in plain values, with the rates the codec offers (the sample-rate follow, ADR 0051). */
    private val states = ConcurrentHashMap<String, BluetoothCodecState>()
    private val changed = Object()
    @Volatile private var registered = false

    /** Whether the receiver is registered (an announcement can be heard at all). */
    fun listening(): Boolean = registered

    /** The last codec state announced for the address, or null. */
    fun state(address: String): BluetoothCodecState? = BluetoothCodecRules.addressKey(address)?.let { states[it] }

    /**
     * Waits up to [timeoutMs] until the announced rate of the address is [rateFlag]; true when it is. Never call it on
     * the main thread.
     */
    fun awaitRate(address: String, rateFlag: Int, timeoutMs: Long): Boolean {
        val key = BluetoothCodecRules.addressKey(address) ?: return false
        val deadline = System.nanoTime() + timeoutMs * 1_000_000L
        synchronized(changed) {
            while (true) {
                if (states[key]?.rateFlag == rateFlag) return true
                val remainingMs = (deadline - System.nanoTime()) / 1_000_000L
                if (remainingMs <= 0L) return false
                try {
                    changed.wait(remainingMs)
                } catch (_: InterruptedException) {
                    return false
                }
            }
        }
    }

    /** Whether the codec can be read here at all (Android 13+). */
    @ChecksSdkIntAtLeast(api = Build.VERSION_CODES.TIRAMISU)
    fun supported(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

    fun permissionGranted(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
            context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

    /** `granted`, `needed` (ask for BLUETOOTH_CONNECT) or `unavailable` (before Android 13: nothing to read). */
    fun permissionState(context: Context): String = when {
        !supported() -> "unavailable"
        permissionGranted(context) -> "granted"
        else -> "needed"
    }

    /** Start listening once the permission is held; safe to call again (the service and the settings page do). */
    @Synchronized
    fun ensure(context: Context) {
        if (registered || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU || !permissionGranted(context)) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                if (intent != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) remember(intent)
            }
        }
        try {
            // The Bluetooth stack is another process: the receiver is exported (the action is a protected broadcast).
            context.applicationContext.registerReceiver(
                receiver,
                IntentFilter(ACTION_CODEC_CONFIG_CHANGED),
                Context.RECEIVER_EXPORTED,
            )
            registered = true
        } catch (error: RuntimeException) {
            Log.w(TAG, "Could not listen for the Bluetooth codec", error)
        }
    }

    /** The mode the helper read for the address since the last announcement, or null. */
    fun ldacMode(address: String): Long? = BluetoothCodecRules.addressKey(address)?.let { ldacModes[it] }

    /** Keeps what the helper read for the address. */
    fun setLdacMode(address: String, codecSpecific1: Long) {
        BluetoothCodecRules.addressKey(address)?.let { ldacModes[it] = codecSpecific1 }
    }

    /** The last codec of each device, by upper-case address; LDAC carries the mode the helper read, if it did. */
    fun snapshot(): Map<String, JSONObject> {
        val out = HashMap<String, JSONObject>()
        for ((key, facts) in codecs) {
            val withMode = if (facts.codecType == BluetoothCodecRules.CODEC_LDAC) facts.copy(codecSpecific1 = ldacModes[key]) else facts
            BluetoothCodecRules.codecJson(withMode)?.let { out[key] = it }
        }
        return out
    }

    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun remember(intent: Intent) {
        try {
            val device = intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
            val status = intent.getParcelableExtra(BluetoothCodecStatus.EXTRA_CODEC_STATUS, BluetoothCodecStatus::class.java)
            val key = BluetoothCodecRules.addressKey(device?.address) ?: return
            val config = status?.codecConfig
            if (config == null) {
                codecs.remove(key)
                states.remove(key)
                ldacModes.remove(key)
                return
            }
            var selectable = config.sampleRate
            for (capability in status?.codecsSelectableCapabilities.orEmpty()) {
                if (capability.codecType == config.codecType) selectable = selectable or capability.sampleRate
            }
            states[key] = BluetoothCodecState(config.codecType, config.sampleRate, selectable)
            synchronized(changed) { changed.notifyAll() }
            val extendedName = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                try {
                    config.extendedCodecType?.codecName
                } catch (_: RuntimeException) {
                    null
                }
            } else {
                null
            }
            ldacModes.remove(key)
            val facts =
                BluetoothCodecFacts(
                    codecType = config.codecType,
                    sampleRateFlag = config.sampleRate,
                    bitsPerSampleFlag = config.bitsPerSample,
                    channelModeFlag = config.channelMode,
                    codecSpecific1 = null,
                    extendedName = extendedName,
                )
            if (BluetoothCodecRules.codecJson(facts) == null) codecs.remove(key) else codecs[key] = facts
            device?.address?.let { address -> onAnnounced?.invoke(address, config.codecType) }
        } catch (error: RuntimeException) {
            Log.w(TAG, "Could not read the Bluetooth codec", error)
        }
    }
}
