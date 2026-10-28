package com.moemusicplayer.mediaindex

import android.content.Context
import android.hardware.usb.UsbManager
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.util.Log
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean

/** Native of the USB exclusive test (ADR 0046), exported from `moemusicplayer_lib` (`src-tauri/src/android_usb_exclusive.rs`). */
object UsbExclusiveBridge {
    /**
     * Takes the USB audio device behind [fd] from the kernel driver, plays [path] (a 16-bit stereo FLAC) for at most
     * 30 s straight over usbfs and hands the device back; blocks until all of it is done. Answers the report as JSON.
     */
    @JvmStatic external fun nativeRun(fd: Int, rawDescriptors: ByteArray, path: String): String?
}

/**
 * The USB exclusive test, development only (ADR 0046, P0): USB permission through the same system dialog as the
 * descriptor reader (ADR 0045), `openDevice`, then the Rust run on its own thread with the connection's file
 * descriptor. The connection stays open until Rust has released the interfaces and given them back to the kernel
 * driver, and is closed only then. The first attached USB audio device is used.
 */
internal object UsbExclusiveTest {
    private const val TAG = "MoeMediaIndex"
    private val running = AtomicBoolean(false)

    private fun answer(started: Boolean, reason: String? = null): JSONObject =
        JSONObject().put("started", started).put("reason", reason ?: JSONObject.NULL)

    /** Start the test on [path]; `done` (main thread) says whether the run started, or why not. */
    fun start(context: Context, path: String, done: (JSONObject) -> Unit) {
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager
            ?: return done(answer(false, "此裝置不支援 USB 主機模式"))
        val device = UsbAudioReader.audioDevices(usb).firstOrNull()
            ?: return done(answer(false, "沒有找到 USB 音訊裝置"))
        if (running.get()) return done(answer(false, "USB 獨佔試播正在進行"))
        val main = Handler(Looper.getMainLooper())
        val pending = if (usb.hasPermission(device)) emptyList() else listOf(device)
        UsbAudioReader.askInTurn(context.applicationContext, usb, main, pending) {
            if (!usb.hasPermission(device)) return@askInTurn done(answer(false, "未取得 USB 權限"))
            val connection = try {
                usb.openDevice(device)
            } catch (error: RuntimeException) {
                Log.w(TAG, "USB exclusive: openDevice failed: $error")
                null
            } ?: return@askInTurn done(answer(false, "無法開啟 USB 裝置"))
            val raw = try {
                connection.rawDescriptors
            } catch (error: RuntimeException) {
                null
            }
            if (raw == null) {
                connection.close()
                return@askInTurn done(answer(false, "無法讀取 USB 描述元"))
            }
            if (!running.compareAndSet(false, true)) {
                connection.close()
                return@askInTurn done(answer(false, "USB 獨佔試播正在進行"))
            }
            Log.i(TAG, "USB exclusive: start on ${device.deviceName} (${device.vendorId}:${device.productId}) fd ${connection.fileDescriptor}")
            Thread({
                Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
                try {
                    val report = UsbExclusiveBridge.nativeRun(connection.fileDescriptor, raw, path) ?: "null"
                    UsbAudioReader.logInParts("USB exclusive:", report)
                } catch (error: Throwable) {
                    Log.e(TAG, "USB exclusive: run failed", error)
                } finally {
                    // Only now: Rust has released the interfaces and reattached the kernel driver.
                    connection.close()
                    running.set(false)
                    Log.i(TAG, "USB exclusive: connection closed")
                }
            }, "moe-usb-exclusive").start()
            done(answer(true))
        }
    }
}
