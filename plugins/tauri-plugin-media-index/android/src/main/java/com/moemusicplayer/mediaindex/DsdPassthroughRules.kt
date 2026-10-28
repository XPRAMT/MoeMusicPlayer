package com.moemusicplayer.mediaindex

/**
 * Pure rules of native DSD passthrough (ADR 0049), plain values only so they run on the JVM: whether a DSD track is
 * loaded as raw slots for the USB exclusive output or converted to PCM as before, and what the sink does for such a
 * track.
 */
internal object DsdPassthroughRules {
    /** The raw slots reach Media3 as 16-bit PCM with this many channels (8 bytes per frame, two 32-bit slots). */
    const val CARRIER_CHANNELS = 4
    const val CARRIER_ENCODING = UsbExclusiveRules.ENCODING_PCM_16BIT

    /** Media3 drops this much of the start of a new output's data, which its volume ramp scaled (mirrors Rust `dsd.rs`). */
    const val RAMP_DISCARD_MS = 25

    /** The reason shown when a track cannot be carried, whatever the cause is a file the carrier cannot take. */
    const val UNSUPPORTED_FILE = "只支援雙聲道 DSD64、DSD128、DSD256 的 DSF 與 WavPack-DSD"

    /** The load schemes of a track that may play natively: a local DSF, a local WavPack file, a remote file (local or remote alike). */
    val PASSTHROUGH_SCHEMES = setOf(DSD_URI_SCHEME, WAVPACK_URI_SCHEME, REMOTE_URI_SCHEME)

    /** What the data source reported about the file (`nativeRawPlan`): [reason] is null when it can be carried at [rate]. */
    data class FilePlan(val rate: Int, val reason: String?)

    sealed class Decision {
        /** The setting is off or the track is not DSD: nothing to say. */
        object Off : Decision()

        /** The track plays as DSD-to-PCM; [reason] is shown on the output card (「DSD 直通無法使用：…」). */
        data class Pcm(val reason: String) : Decision()

        /** The track is loaded as raw slots at [rate] frames per second (DSD bit rate ÷ 32). */
        data class Raw(val rate: Int) : Decision()
    }

    /**
     * Whether the file's header must be read for the decision (which may mean a round trip to a server, so the caller
     * does it on a worker thread, before [decide]): the cheap conditions that come before the file in [decide].
     */
    fun wantsFilePlan(featureOn: Boolean, isDsd: Boolean, scheme: String?, exclusive: Boolean): Boolean =
        featureOn && isDsd && scheme in PASSTHROUGH_SCHEMES && exclusive

    /**
     * The decision at load time. The checks run in this order and the lambdas only when everything before them passed
     * (they read the file and the USB descriptors): the setting, a DSD track, a load scheme that can be carried (a
     * local or remote DSF or WavPack-DSD file; whether the file itself is stereo DSD64/128/256 is the file plan's
     * answer), the exclusive mode, a USB device the mode plays on ([routedToUsb]), its permission, the file, the device
     * (the rate, the RAW_DATA alt).
     */
    fun decide(
        featureOn: Boolean,
        isDsd: Boolean,
        scheme: String?,
        exclusive: Boolean,
        routedToUsb: Boolean,
        permitted: Boolean,
        filePlan: () -> FilePlan,
        deviceReason: (Int) -> String?,
    ): Decision {
        if (!featureOn || !isDsd) return Decision.Off
        if (scheme !in PASSTHROUGH_SCHEMES) return Decision.Pcm(UNSUPPORTED_FILE)
        if (!exclusive) return Decision.Pcm("不是獨佔模式")
        if (!routedToUsb) return Decision.Pcm("沒有 USB 音訊裝置")
        if (!permitted) return Decision.Pcm("未取得 USB 權限")
        val plan = filePlan()
        plan.reason?.let { return Decision.Pcm(it) }
        deviceReason(plan.rate)?.let { return Decision.Pcm(it) }
        return Decision.Raw(plan.rate)
    }

    /**
     * The raw-slots form of a load URI, or null for anything else: `moedsd:///…` becomes `moedsdraw:///…`, `moewv:///…`
     * becomes `moewvraw:///…` and `moeremote://…` becomes `moeremoteraw://…` (the rest unchanged).
     */
    fun rawUriOf(uri: String): String? = when {
        uri.startsWith("$DSD_URI_SCHEME:///") -> "$DSD_RAW_URI_SCHEME:" + uri.removePrefix("$DSD_URI_SCHEME:")
        uri.startsWith("$WAVPACK_URI_SCHEME:///") -> "$WAVPACK_RAW_URI_SCHEME:" + uri.removePrefix("$WAVPACK_URI_SCHEME:")
        uri.startsWith("$REMOTE_URI_SCHEME://") -> "$REMOTE_RAW_URI_SCHEME:" + uri.removePrefix("$REMOTE_URI_SCHEME:")
        else -> null
    }

    /** Reads the answer of `nativeRawPlan`: [FilePlan] with a null reason, or the reason (also for an unreadable answer). */
    fun parseFilePlan(json: String?): FilePlan {
        val answer = try {
            json?.let { org.json.JSONObject(it) }
        } catch (_: org.json.JSONException) {
            null
        }
        if (answer == null) return FilePlan(0, "無法讀取 DSD 檔案")
        if (answer.optBoolean("ok", false)) return FilePlan(answer.optInt("rate", 0), null)
        return FilePlan(0, answer.optString("reason").takeIf { it.isNotBlank() } ?: "不支援直通")
    }

    /** The format Media3 gets for a raw-slots track: 16-bit PCM, four channels (the carrier). */
    fun isCarrier(encoding: Int, channels: Int): Boolean = encoding == CARRIER_ENCODING && channels == CARRIER_CHANNELS

    /**
     * What the output does with the app volume. Native DSD ignores it, the audio-focus ducking (which also arrives as a
     * volume) and `setVolume` alike: scaling the slots would be noise. Returns the gain to apply, or null for "ignore".
     */
    fun gainFor(nativeDsd: Boolean, volume: Float): Float? = if (nativeDsd) null else volume

    /**
     * The output must be the USB exclusive one: a raw-slots track never goes to an AudioTrack. [usb] is the provider's
     * choice for it; when it is not USB the sink fails the track so the service reloads it through the PCM converter.
     */
    fun mustFail(nativeDsd: Boolean, usb: Boolean): Boolean = nativeDsd && !usb

    /**
     * Why the provider did not choose the USB output for a track, in words for the card: the exclusive mode is off, the
     * chosen device is not USB, no USB audio device is attached, or the reason the choice itself gave. Null when it chose
     * USB (a failure to start is reported by the engine instead).
     */
    fun notUsbReason(facts: UsbExclusiveRules.Facts, choice: UsbExclusiveRules.Choice): String? = when {
        choice == UsbExclusiveRules.Choice.Usb -> null
        !facts.enabled -> "不是獨佔模式"
        !UsbExclusiveRules.usbChoice(facts.chosenKind) -> "播放裝置不是 USB"
        !facts.deviceAttached -> "沒有 USB 音訊裝置"
        else -> (choice as? UsbExclusiveRules.Choice.AudioTrack)?.reason
    }

    /** Frames of the start of a new output that are dropped, for the ramp (same arithmetic as Rust `dsd::discard_frames`). */
    fun discardFrames(rate: Int): Int = ((rate.toLong() * RAMP_DISCARD_MS + 999) / 1000).toInt()
}
