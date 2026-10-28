package com.moemusicplayer.mediaindex

import android.content.Context
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbManager
import android.os.Process
import android.util.Log
import org.json.JSONObject
import java.nio.ByteBuffer

/** The USB exclusive output's native side (ADR 0047), exported from `moemusicplayer_lib` (`src-tauri/src/android_usb_output.rs`). */
internal object UsbExclusiveNative {
    /** Install a claim of the device behind [fd]; false while one is alive. Its loop runs in [nativeLoop]. */
    @JvmStatic external fun nativeStart(fd: Int, rawDescriptors: ByteArray): Boolean
    /** Blocks until the claim ended (the device handed back); the last status as JSON. */
    @JvmStatic external fun nativeLoop(): String?
    @JvmStatic external fun nativeAlive(): Boolean
    /**
     * A ring of [capacityFrames] frames of [encoding] (`ENCODING_PCM_*`); 0 when the encoding is not PCM. A [dsdRate] above
     * 0 makes it the ring of native DSD at that many frames per second (ADR 0049): 16-bit, 4 channels, and the start of
     * what is written is replaced by idle fill.
     */
    @JvmStatic external fun nativeCreateRing(capacityFrames: Int, encoding: Int, channels: Int, dsdRate: Int): Long
    @JvmStatic external fun nativeDestroyRing(ring: Long)
    /** Copy what fits of the direct [buffer] from [offset], [length] bytes; the bytes taken (never blocks). */
    @JvmStatic external fun nativeWrite(ring: Long, buffer: ByteBuffer, offset: Int, length: Int): Int
    /** Frames that came back from the bus: the output's position. */
    @JvmStatic external fun nativePlayedFrames(ring: Long): Long
    @JvmStatic external fun nativeRingEnd(ring: Long)
    @JvmStatic external fun nativeRingFailed(ring: Long): Boolean
    @JvmStatic external fun nativeRingUnderruns(ring: Long): Long
    /** `{ ok, rate, channels, alt, subslotBytes, bits }` or `{ ok: false, reason }`. */
    @JvmStatic external fun nativeAttach(ring: Long, rate: Int, sourceBits: Int, dsd: Boolean, nativeDsd: Boolean): String?
    /** Null when it worked, else the reason. The same for the next three. */
    @JvmStatic external fun nativePlay(ring: Long): String?
    @JvmStatic external fun nativePause(ring: Long): String?
    @JvmStatic external fun nativeDetach(ring: Long): String?
    @JvmStatic external fun nativeRelease(reason: String): String?
    @JvmStatic external fun nativeSetGain(gain: Float)
    @JvmStatic external fun nativeStatus(): String?
    /** Null when the device of [rawDescriptors] can take the format, else why not. */
    @JvmStatic external fun nativeSupports(rawDescriptors: ByteArray, rate: Int, channels: Int, sourceBits: Int, dsd: Boolean): String?
    /** Null when the device of [rawDescriptors] has the RAW_DATA alt for native DSD at [rate] frames per second, else why not. */
    @JvmStatic external fun nativeSupportsDsd(rawDescriptors: ByteArray, rate: Int): String?
}

/**
 * The USB device the exclusive output drives (ADR 0047): the output mode, which USB device the mode plays on, its
 * `UsbDeviceConnection` while a claim runs, and the claim's thread. The thread (`moe-usb-audio`, URGENT_AUDIO) runs
 * the Rust device loop and closes the connection only after Rust has given the device back to the kernel driver.
 * While a claim runs, [held] is the output device as Android listed it before it disappeared (for the router).
 */
internal object UsbExclusiveEngine {
    private const val TAG = "MoeMediaIndex"
    private const val JOIN_MS = 2_500L

    /** The output mode `androidUsbExclusive` (true: 獨佔), as the service last read it. */
    @Volatile var enabled = false
        private set

    /** The chosen output device while the app holds it; read by the router. */
    @Volatile var held: OutputDeviceFacts? = null
        private set

    /** Called (any thread) when [held] changes. */
    @Volatile var onHeldChanged: (() -> Unit)? = null

    /** Why the current track does not play on the USB output; cleared at every load. */
    @Volatile var trackFallback: String? = null

    /** The source of the track being loaded (bits per sample, DSD), from the load command. */
    @Volatile var source = UsbExclusiveRules.Source(0, false)

    /**
     * Why the DSD track being loaded plays as PCM although native passthrough is on (the card shows it as
     * 「DSD 直通無法使用：…」), null when it plays natively, the setting is off or the track is not DSD (ADR 0049).
     */
    @Volatile var dsdReason: String? = null

    /** The output Media3 plays through now, when it is ours. */
    @Volatile var current: UsbAudioOutput? = null

    /** The USB device ([UsbExclusiveRules.UsbDeviceFacts.key], its `deviceName`) a claim runs on, null when none. */
    @Volatile var claimed: String? = null
        private set

    private val lock = Any()
    private var thread: Thread? = null
    private val rawCache = HashMap<String, ByteArray>()

    fun setEnabled(value: Boolean) {
        enabled = value
        if (!value) release("已切換為共享模式")
    }

    /** A new track is loaded: it gets its own chance on the USB output. */
    fun newTrack(next: UsbExclusiveRules.Source, dsdPcmReason: String? = null) {
        source = next
        trackFallback = null
        dsdReason = dsdPcmReason
    }

    /**
     * The native DSD output could not be used after the load chose it (the device went away, refused the rate, ...): the
     * track is reloaded through the DSD-to-PCM converter and gets its own chance on the USB output as PCM (ADR 0049).
     */
    fun dsdPassthroughFailed(reason: String) {
        source = source.copy(nativeDsd = false)
        trackFallback = null
        dsdReason = reason
    }

    /** The attached USB audio devices, in `UsbManager` order (by device name). */
    fun usbDevices(usb: UsbManager?): List<UsbExclusiveRules.UsbDeviceFacts> = try {
        usb?.let(UsbAudioReader::audioDevices)?.map { UsbExclusiveRules.UsbDeviceFacts(it.deviceName, safeName(it)) }
            ?: emptyList()
    } catch (_: RuntimeException) {
        emptyList()
    }

    /**
     * The USB device the exclusive mode plays [preference] on (ADR 0047): the first USB audio device for the system
     * default (null), the matching one for a USB device, none for anything else.
     */
    fun findDevice(usb: UsbManager, preference: OutputDevicePreference?): UsbDevice? {
        val devices = UsbAudioReader.audioDevices(usb)
        val facts = devices.map { UsbExclusiveRules.UsbDeviceFacts(it.deviceName, safeName(it)) }
        val picked = UsbExclusiveRules.pickExclusiveDevice(preference, facts) ?: return null
        return devices.firstOrNull { it.deviceName == picked.key }
    }

    /**
     * The page's view of the routing table for the saved choice [saved] (ADR 0047): which USB device the exclusive
     * mode plays on (`usbDevice`: its product name and whether the app may open it), whether none is attached
     * (`noUsbDevice`) and how many USB audio devices there are.
     */
    fun routeJson(context: Context, saved: OutputDevicePreference?): JSONObject {
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager
        val devices = usbDevices(usb)
        val route = UsbExclusiveRules.route(UsbExclusiveRules.Wish(true, saved), devices)
        return JSONObject().apply {
            put("usbDevices", devices.size)
            put("noUsbDevice", route is UsbExclusiveRules.Route.Shared && route.noUsbDevice)
            val target = (route as? UsbExclusiveRules.Route.Exclusive)?.device
            val device = target?.let { picked -> usb?.let(UsbAudioReader::audioDevices)?.firstOrNull { it.deviceName == picked.key } }
            put(
                "usbDevice",
                target?.let {
                    JSONObject().apply {
                        put("key", it.key)
                        put("productName", it.productName ?: JSONObject.NULL)
                        put("permitted", device != null && usb?.hasPermission(device) == true)
                    }
                } ?: JSONObject.NULL,
            )
        }
    }

    private fun safeName(device: UsbDevice): String? = try {
        device.productName
    } catch (_: SecurityException) {
        null
    }

    private fun cacheKey(device: UsbDevice) = "${device.deviceName}:${device.vendorId}:${device.productId}"

    /** The raw descriptors of [device] (read once per plug of the device; needs permission). */
    fun descriptorsOf(usb: UsbManager, device: UsbDevice): ByteArray? {
        synchronized(lock) { rawCache[cacheKey(device)]?.let { return it } }
        val connection = try {
            usb.openDevice(device)
        } catch (error: RuntimeException) {
            null
        } ?: return null
        return try {
            connection.rawDescriptors?.also { raw -> synchronized(lock) { rawCache[cacheKey(device)] = raw } }
        } catch (_: RuntimeException) {
            null
        } finally {
            connection.close()
        }
    }

    /**
     * Make sure a claim of [device] runs (open it and start the device thread); [facts] is the output device as
     * Android lists it now. False when it cannot be opened.
     */
    fun ensureRunning(usb: UsbManager, device: UsbDevice, facts: OutputDeviceFacts?): Boolean = synchronized(lock) {
        if (UsbExclusiveNative.nativeAlive()) return true
        // A claim that just ended may still be closing its connection.
        thread?.let { old ->
            old.join(JOIN_MS)
            if (old.isAlive) return false
        }
        val connection: UsbDeviceConnection = try {
            usb.openDevice(device)
        } catch (error: RuntimeException) {
            Log.w(TAG, "USB exclusive: openDevice failed: $error")
            null
        } ?: return false
        val raw = try {
            connection.rawDescriptors
        } catch (_: RuntimeException) {
            null
        }
        if (raw == null || !UsbExclusiveNative.nativeStart(connection.fileDescriptor, raw)) {
            connection.close()
            return false
        }
        rawCache[cacheKey(device)] = raw
        claimed = device.deviceName
        if (facts != null) held = facts
        onHeldChanged?.invoke()
        Log.i(TAG, "USB exclusive: claim of ${device.deviceName} (${device.vendorId}:${device.productId})")
        thread = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
            try {
                val status = UsbExclusiveNative.nativeLoop() ?: "null"
                UsbAudioReader.logInParts("USB exclusive:", status)
            } catch (error: Throwable) {
                Log.e(TAG, "USB exclusive: device loop failed", error)
            } finally {
                // Only now: Rust has released the interfaces and reattached the kernel driver.
                connection.close()
                claimed = null
                held = null
                onHeldChanged?.invoke()
                Log.i(TAG, "USB exclusive: connection closed")
            }
        }, "moe-usb-audio").apply { start() }
        true
    }

    /** Give the device back now (stop, shared mode, service gone). */
    fun release(reason: String) {
        if (!UsbExclusiveNative.nativeAlive()) return
        UsbExclusiveNative.nativeRelease(reason)?.let { Log.i(TAG, "USB exclusive: release: $it") }
    }

    /** The status the settings page reads (`android_audio_output_status` → `exclusive`). */
    fun statusJson(): JSONObject {
        val engine = try {
            UsbExclusiveNative.nativeStatus()?.let(::JSONObject)
        } catch (_: Throwable) {
            null
        }
        val output = current
        val state = engine?.optString("state")
        // The track's output is ours (also while the device went back to Android after 10 s paused: play claims again).
        val active = output != null && !output.failed
        return JSONObject().apply {
            put("enabled", enabled)
            put("active", active)
            put("state", state ?: JSONObject.NULL)
            put("fallbackReason", trackFallback ?: JSONObject.NULL)
            // Native DSD (ADR 0049): the card says 「DSD 直通」; or why a DSD track plays as PCM although the setting is on.
            put("dsdNative", active && source.nativeDsd)
            put("dsdFallbackReason", dsdReason ?: JSONObject.NULL)
            put("engine", engine ?: JSONObject.NULL)
        }
    }
}
