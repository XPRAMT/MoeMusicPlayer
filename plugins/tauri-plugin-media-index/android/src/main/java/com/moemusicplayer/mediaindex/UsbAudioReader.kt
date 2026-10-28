package com.moemusicplayer.mediaindex

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.AudioMixerAttributes
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

    internal fun audioDevices(usb: UsbManager): List<UsbDevice> =
        usb.deviceList.values.filter(::isAudio).sortedBy { it.deviceName }

    /**
     * The attached USB audio devices and their parsed descriptors. With `request`, devices without permission are
     * asked for it first, one system dialog each; `done` is called on the main thread. The answer's `mixerAttributes`
     * holds the system mixer formats of the output device `deviceId` (every USB output when null or not attached).
     */
    fun read(context: Context, request: Boolean, deviceId: Int?, done: (JSONObject) -> Unit) {
        val main = Handler(Looper.getMainLooper())
        val audio = context.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager
        if (usb == null) {
            val result = JSONObject().put("supported", false).put("devices", JSONArray())
            done(result.put("mixerAttributes", mixerAttributes(audio, deviceId)))
            return
        }
        val pending = if (request) audioDevices(usb).filter { !usb.hasPermission(it) } else emptyList()
        askInTurn(context.applicationContext, usb, main, pending) {
            Thread({
                val result = collect(usb, audio, deviceId)
                main.post { done(result) }
            }, "moe-usb-descriptors").start()
        }
    }

    private val USB_OUTPUT_TYPES = setOf(
        AudioOutputRules.TYPE_USB_DEVICE,
        AudioOutputRules.TYPE_USB_HEADSET,
        AudioOutputRules.TYPE_USB_ACCESSORY,
    )

    /**
     * The system mixer formats (Android 14+) of the card's output device, or of every USB output when the card's
     * device is not given or no longer attached. Read only: only the two queries; nothing is set, cleared or opened.
     */
    private fun mixerAttributes(audio: AudioManager?, deviceId: Int?): JSONObject {
        val sdk = Build.VERSION.SDK_INT
        if (sdk < MixerAttributesRules.MIN_SDK) return MixerAttributesRules.resultJson(sdk, emptyList())
        val outputs = try {
            audio?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)?.toList() ?: emptyList()
        } catch (error: RuntimeException) {
            Log.w(TAG, "listing the output devices failed: $error")
            emptyList()
        }
        val usbOutputs = outputs.filter { it.type in USB_OUTPUT_TYPES }
        val chosen = usbOutputs.filter { it.id == deviceId }.ifEmpty { usbOutputs }
        return MixerAttributesRules.resultJson(sdk, chosen.map { device -> mixerAttributesOf(audio!!, device) })
    }

    @androidx.annotation.RequiresApi(MixerAttributesRules.MIN_SDK)
    private fun mixerAttributesOf(audio: AudioManager, device: AudioDeviceInfo): JSONObject {
        val name = device.productName?.toString() ?: ""
        return try {
            val supported = audio.getSupportedMixerAttributes(device).map(::entryOf)
            val media = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build()
            val preferred = audio.getPreferredMixerAttributes(media, device)?.let(::entryOf)
            MixerAttributesRules.deviceJson(device.id, device.type, name, supported, preferred)
        } catch (error: Exception) {
            Log.w(TAG, "mixer attributes failed for device ${device.id}: $error")
            MixerAttributesRules.deviceJson(device.id, device.type, name, emptyList(), null, error.toString())
        }
    }

    @androidx.annotation.RequiresApi(MixerAttributesRules.MIN_SDK)
    private fun entryOf(attributes: AudioMixerAttributes): MixerAttributesRules.Entry {
        val format = attributes.format
        return MixerAttributesRules.Entry(
            encoding = format.encoding,
            sampleRate = format.sampleRate,
            channelMask = format.channelMask,
            channelCount = format.channelCount,
            behavior = attributes.mixerBehavior,
        )
    }

    internal fun askInTurn(context: Context, usb: UsbManager, main: Handler, devices: List<UsbDevice>, then: () -> Unit) {
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

    private fun collect(usb: UsbManager, audio: AudioManager?, deviceId: Int?): JSONObject {
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
        logInParts("USB audio descriptors", result.toString())
        val mixer = mixerAttributes(audio, deviceId)
        logInParts("USB mixer attributes", mixer.toString())
        return result.put("mixerAttributes", mixer)
    }

    /** logcat cuts a line at about 4 kB: the JSON goes out in numbered parts, read back with `adb logcat -s MoeMediaIndex`. */
    internal fun logInParts(prefix: String, text: String) {
        val parts = maxOf((text.length + LOG_CHUNK - 1) / LOG_CHUNK, 1)
        for (i in 0 until parts) {
            Log.i(TAG, "$prefix ${i + 1}/$parts: ${text.substring(i * LOG_CHUNK, minOf(text.length, (i + 1) * LOG_CHUNK))}")
        }
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
