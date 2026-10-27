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

    private val codecs = ConcurrentHashMap<String, JSONObject>()
    @Volatile private var registered = false

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

    /** The last codec of each device, by upper-case address. */
    fun snapshot(): Map<String, JSONObject> = HashMap(codecs)

    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun remember(intent: Intent) {
        try {
            val device = intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
            val status = intent.getParcelableExtra(BluetoothCodecStatus.EXTRA_CODEC_STATUS, BluetoothCodecStatus::class.java)
            val key = BluetoothCodecRules.addressKey(device?.address) ?: return
            val config = status?.codecConfig
            if (config == null) {
                codecs.remove(key)
                return
            }
            val extendedName = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                try {
                    config.extendedCodecType?.codecName
                } catch (_: RuntimeException) {
                    null
                }
            } else {
                null
            }
            val json = BluetoothCodecRules.codecJson(
                BluetoothCodecFacts(
                    codecType = config.codecType,
                    sampleRateFlag = config.sampleRate,
                    bitsPerSampleFlag = config.bitsPerSample,
                    channelModeFlag = config.channelMode,
                    codecSpecific1 = config.codecSpecific1,
                    extendedName = extendedName,
                ),
            )
            if (json == null) codecs.remove(key) else codecs[key] = json
        } catch (error: RuntimeException) {
            Log.w(TAG, "Could not read the Bluetooth codec", error)
        }
    }
}
