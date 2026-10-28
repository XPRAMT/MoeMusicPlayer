package com.moemusicplayer.mediaindex

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * Reads the descriptors of the attached USB audio devices (ADR 0045), read only: USB permission through the system
 * dialog (`UsbManager.requestPermission`, only when the page asks), then `openDevice` → `getRawDescriptors()` → close.
 * It never claims an interface, sets an alternate setting or sends a control transfer, so playback is not touched.
 */
internal object UsbAudioReader {
    private const val TAG = "MoeMediaIndex"
    private const val ACTION_PERMISSION = "com.moemusicplayer.mediaindex.USB_AUDIO_PERMISSION"
    /** A dialog the user leaves open must not keep the page waiting forever. */
    private const val PERMISSION_TIMEOUT_MS = 60_000L
    private const val LOG_CHUNK = 3000

    /** Devices whose permission prompt was answered "no" in this process (UsbManager keeps no refusal). */
    private val refused: MutableSet<String> = java.util.Collections.synchronizedSet(mutableSetOf())

    private fun isAudio(device: UsbDevice): Boolean =
        (0 until device.interfaceCount).any { device.getInterface(it).interfaceClass == UsbConstants.USB_CLASS_AUDIO }

    private fun audioDevices(usb: UsbManager): List<UsbDevice> =
        usb.deviceList.values.filter(::isAudio).sortedBy { it.deviceName }

    /**
     * The attached USB audio devices and their parsed descriptors. With `request`, devices without permission are
     * asked for it first, one system dialog each; `done` is called on the main thread.
     */
    fun read(context: Context, request: Boolean, done: (JSONObject) -> Unit) {
        val main = Handler(Looper.getMainLooper())
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager
        if (usb == null) {
            done(JSONObject().put("supported", false).put("devices", JSONArray()))
            return
        }
        val pending = if (request) audioDevices(usb).filter { !usb.hasPermission(it) } else emptyList()
        askInTurn(context.applicationContext, usb, main, pending) {
            Thread({
                val result = collect(usb)
                main.post { done(result) }
            }, "moe-usb-descriptors").start()
        }
    }

    private fun askInTurn(context: Context, usb: UsbManager, main: Handler, devices: List<UsbDevice>, then: () -> Unit) {
        val device = devices.firstOrNull() ?: return then()
        var answered = false
        lateinit var receiver: BroadcastReceiver
        val finish = { granted: Boolean? ->
            if (!answered) {
                answered = true
                try {
                    context.unregisterReceiver(receiver)
                } catch (_: IllegalArgumentException) {
                }
                if (granted == false) refused += device.deviceName else refused -= device.deviceName
                askInTurn(context, usb, main, devices.drop(1), then)
            }
        }
        receiver = object : BroadcastReceiver() {
            override fun onReceive(receiverContext: Context, intent: Intent) {
                if (intent.action != ACTION_PERMISSION) return
                finish(intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false))
            }
        }
        val filter = IntentFilter(ACTION_PERMISSION)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            context.registerReceiver(receiver, filter)
        }
        // Mutable: the system adds EXTRA_PERMISSION_GRANTED; explicit (own package), as Android 14 requires for that.
        val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
        val intent = PendingIntent.getBroadcast(
            context,
            0,
            Intent(ACTION_PERMISSION).setPackage(context.packageName),
            flags or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        try {
            usb.requestPermission(device, intent)
        } catch (error: RuntimeException) {
            Log.w(TAG, "USB permission request failed for ${device.deviceName}: $error")
            finish(null)
            return
        }
        main.postDelayed({ finish(null) }, PERMISSION_TIMEOUT_MS)
    }

    private fun collect(usb: UsbManager): JSONObject {
        val devices = JSONArray()
        for (device in audioDevices(usb)) {
            val entry = JSONObject()
                .put("name", device.deviceName)
                .put("vendorId", device.vendorId)
                .put("productId", device.productId)
                .put("manufacturer", safe { device.manufacturerName } ?: JSONObject.NULL)
                .put("product", safe { device.productName } ?: JSONObject.NULL)
            val granted = usb.hasPermission(device)
            entry.put("permission", when {
                granted -> "granted"
                device.deviceName in refused -> "denied"
                else -> "notAsked"
            })
            if (granted) {
                val raw = readRaw(usb, device)
                if (raw == null) {
                    entry.put("error", "unreadable")
                } else {
                    val parsed = UsbAudioDescriptors.deviceJson(raw)
                    parsed.keys().forEach { key -> entry.put(key, parsed.get(key)) }
                }
            }
            devices.put(entry)
        }
        val result = JSONObject().put("supported", true).put("devices", devices)
        // logcat cuts a line at about 4 kB: the JSON goes out in numbered parts, read back with `adb logcat -s MoeMediaIndex`.
        val text = result.toString()
        val parts = (text.length + LOG_CHUNK - 1) / LOG_CHUNK
        for (i in 0 until maxOf(parts, 1)) {
            Log.i(TAG, "USB audio descriptors ${i + 1}/$parts: ${text.substring(i * LOG_CHUNK, minOf(text.length, (i + 1) * LOG_CHUNK))}")
        }
        return result
    }

    /** Open, read the raw descriptors and close at once; nothing is claimed or sent. */
    private fun readRaw(usb: UsbManager, device: UsbDevice): ByteArray? {
        val connection = try {
            usb.openDevice(device)
        } catch (error: RuntimeException) {
            Log.w(TAG, "USB openDevice failed for ${device.deviceName}: $error")
            null
        } ?: return null
        return try {
            connection.rawDescriptors
        } catch (error: RuntimeException) {
            Log.w(TAG, "USB getRawDescriptors failed for ${device.deviceName}: $error")
            null
        } finally {
            connection.close()
        }
    }

    private fun <T> safe(read: () -> T?): T? = try {
        read()
    } catch (_: SecurityException) {
        null
    }
}
