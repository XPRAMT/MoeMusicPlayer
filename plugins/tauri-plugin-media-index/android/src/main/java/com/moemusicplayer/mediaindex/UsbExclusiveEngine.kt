package com.moemusicplayer.mediaindex

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
    /** A ring of [capacityFrames] frames of [encoding] (`ENCODING_PCM_*`); 0 when the encoding is not PCM. */
    @JvmStatic external fun nativeCreateRing(capacityFrames: Int, encoding: Int, channels: Int): Long
    @JvmStatic external fun nativeDestroyRing(ring: Long)
    /** Copy what fits of the direct [buffer] from [offset], [length] bytes; the bytes taken (never blocks). */
    @JvmStatic external fun nativeWrite(ring: Long, buffer: ByteBuffer, offset: Int, length: Int): Int
    /** Frames that came back from the bus: the output's position. */
    @JvmStatic external fun nativePlayedFrames(ring: Long): Long
    @JvmStatic external fun nativeRingEnd(ring: Long)
    @JvmStatic external fun nativeRingFailed(ring: Long): Boolean
    @JvmStatic external fun nativeRingUnderruns(ring: Long): Long
    /** `{ ok, rate, channels, alt, subslotBytes, bits }` or `{ ok: false, reason }`. */
    @JvmStatic external fun nativeAttach(ring: Long, rate: Int, sourceBits: Int, dsd: Boolean): String?
    /** Null when it worked, else the reason. The same for the next three. */
    @JvmStatic external fun nativePlay(ring: Long): String?
    @JvmStatic external fun nativePause(ring: Long): String?
    @JvmStatic external fun nativeDetach(ring: Long): String?
    @JvmStatic external fun nativeRelease(reason: String): String?
    @JvmStatic external fun nativeSetGain(gain: Float)
    @JvmStatic external fun nativeStatus(): String?
    /** Null when the device of [rawDescriptors] can take the format, else why not. */
    @JvmStatic external fun nativeSupports(rawDescriptors: ByteArray, rate: Int, channels: Int, sourceBits: Int, dsd: Boolean): String?
}

/**
 * The USB device the exclusive output drives (ADR 0047): the switch, which USB device is the chosen output, its
 * `UsbDeviceConnection` while a claim runs, and the claim's thread. The thread (`moe-usb-audio`, URGENT_AUDIO) runs
 * the Rust device loop and closes the connection only after Rust has given the device back to the kernel driver.
 * While a claim runs, [held] is the output device as Android listed it before it disappeared (for the router).
 */
internal object UsbExclusiveEngine {
    private const val TAG = "MoeMediaIndex"
    private const val JOIN_MS = 2_500L

    /** The `androidUsbExclusive` switch, as the service last read it. */
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

    /** The output Media3 plays through now, when it is ours. */
    @Volatile var current: UsbAudioOutput? = null

    private val lock = Any()
    private var thread: Thread? = null
    private val rawCache = HashMap<String, ByteArray>()

    fun setEnabled(value: Boolean) {
        enabled = value
        if (!value) release("已關閉 USB 獨佔")
    }

    /** A new track is loaded: it gets its own chance on the USB output. */
    fun newTrack(next: UsbExclusiveRules.Source) {
        source = next
        trackFallback = null
    }

    /** The USB device of [preference], or null. */
    fun findDevice(usb: UsbManager, preference: OutputDevicePreference?): UsbDevice? {
        if (preference == null || preference.kind != "usb") return null
        val devices = UsbAudioReader.audioDevices(usb)
        val facts = devices.map { UsbExclusiveRules.UsbDeviceFacts(it.deviceName, safeName(it)) }
        val picked = UsbExclusiveRules.pickDevice(preference.productName, facts) ?: return null
        return devices.firstOrNull { it.deviceName == picked.key }
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
                held = null
                onHeldChanged?.invoke()
                Log.i(TAG, "USB exclusive: connection closed")
            }
        }, "moe-usb-audio").apply { start() }
        true
    }

    /** Give the device back now (stop, switch off, service gone). */
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
            put("engine", engine ?: JSONObject.NULL)
        }
    }
}
