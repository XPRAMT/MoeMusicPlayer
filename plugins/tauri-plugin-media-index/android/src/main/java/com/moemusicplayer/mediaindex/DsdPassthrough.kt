package com.moemusicplayer.mediaindex

import android.content.Context
import android.hardware.usb.UsbManager
import android.net.Uri
import android.util.Log

/**
 * The load-time decision of native DSD passthrough (ADR 0049), with the Android facts it needs: the exclusive routing
 * table, the USB device and its permission, the file's header and the device's descriptors. The rules are
 * [DsdPassthroughRules.decide].
 */
internal object DsdPassthrough {
    private const val TAG = "MoeMediaIndex"

    /**
     * Whether the DSD track at [uri] (the `moedsd://`, `moewv://` or `moeremote://` URI Rust sent) plays as raw slots on
     * the USB device or converted to PCM, with the reason for the card in the second case. [featureOn] is
     * `dsd.nativePassthrough`. [rawPlanJson] is the answer of `DsdBridge.nativeRawPlan` for the file, read earlier on
     * a worker thread (a remote header is a round trip to the server; this runs on the main thread); null when it was
     * not read or failed.
     */
    fun decide(
        context: Context,
        featureOn: Boolean,
        uri: String,
        isDsd: Boolean,
        preference: OutputDevicePreference?,
        rawPlanJson: String?,
    ): DsdPassthroughRules.Decision {
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager
        val parsed = Uri.parse(uri)
        val wish = UsbExclusiveRules.Wish(UsbExclusiveEngine.enabled, preference)
        val route = UsbExclusiveRules.route(wish, UsbExclusiveEngine.usbDevices(usb))
        val device = usb?.let { UsbExclusiveEngine.findDevice(it, preference) }
        val decision = try {
            DsdPassthroughRules.decide(
                featureOn = featureOn,
                isDsd = isDsd,
                scheme = parsed.scheme,
                exclusive = wish.enabled,
                routedToUsb = route is UsbExclusiveRules.Route.Exclusive,
                permitted = usb != null && device != null && usb.hasPermission(device),
                filePlan = { DsdPassthroughRules.parseFilePlan(rawPlanJson) },
                deviceReason = { rate ->
                    val raw = if (usb != null && device != null) UsbExclusiveEngine.descriptorsOf(usb, device) else null
                    if (raw == null) "無法讀取 USB 描述元" else UsbExclusiveNative.nativeSupportsDsd(raw, rate)
                },
            )
        } catch (error: Throwable) {
            Log.w(TAG, "DSD passthrough: the decision failed: $error")
            DsdPassthroughRules.Decision.Pcm(error.message ?: "無法判斷")
        }
        Log.i(TAG, "DSD passthrough: ${describe(decision)} (setting=$featureOn dsd=$isDsd scheme=${parsed.scheme})")
        return decision
    }

    /**
     * The header-based answer of `nativeRawPlan` for the file at [uri]. Blocks while a server answers: call it from a
     * worker thread (see `PlaybackService.preflightLoad`).
     */
    fun readFilePlan(uri: Uri): String? = bridgePathOf(uri)?.let(DsdBridge::nativeRawPlan)

    fun describe(decision: DsdPassthroughRules.Decision): String = when (decision) {
        DsdPassthroughRules.Decision.Off -> "off"
        is DsdPassthroughRules.Decision.Pcm -> "PCM (${decision.reason})"
        is DsdPassthroughRules.Decision.Raw -> "native, ${decision.rate} frames/s"
    }
}
