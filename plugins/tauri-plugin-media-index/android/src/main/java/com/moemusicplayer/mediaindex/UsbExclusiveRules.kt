package com.moemusicplayer.mediaindex

/**
 * Pure rules of the USB exclusive output (ADR 0047), plain values only so they run on the JVM: which output Media3
 * gets for a track, which USB device is the chosen one, and how the router treats the device while the app holds it.
 */
internal object UsbExclusiveRules {
    // AudioFormat.ENCODING_PCM_* (the same numbers as Media3's C.ENCODING_PCM_*).
    const val ENCODING_PCM_16BIT = 2
    const val ENCODING_PCM_FLOAT = 4
    const val ENCODING_PCM_24BIT_PACKED = 21
    const val ENCODING_PCM_32BIT = 22

    /** The ring between Media3 and the device thread holds this much audio. */
    const val RING_MS = 250

    /** What the track is, as the load command told the service. */
    data class Source(val bits: Int, val dsd: Boolean)

    /** Everything the choice depends on, read by the provider when Media3 configures an output. */
    data class Facts(
        /** The output mode (`androidUsbExclusive`): true for 獨佔, false for 共享. */
        val enabled: Boolean,
        /** The chosen output device's kind (`usb`, …), null for the system default. */
        val chosenKind: String?,
        /** The USB audio device the mode plays on for this choice is attached (the app may be holding it). */
        val deviceAttached: Boolean,
        val permitted: Boolean,
        val encoding: Int,
        /** Why this track already failed on the USB output (it then plays through Android until the next track). */
        val trackFallback: String?,
        /** What the device's descriptors say about the format: null when it can take it, else why not. */
        val unsupported: String?,
        // Only for the decision log: the format and the source the choice was made for.
        val rate: Int = 0,
        val channels: Int = 0,
        val source: Source = Source(0, false),
        /** Only for the decision log: the USB device ([UsbDeviceFacts.key]) the choice was made for. */
        val device: String? = null,
    )

    sealed class Choice {
        object Usb : Choice()
        /** The AudioTrack path; [reason] is shown when the user expected the USB output and it is not used. */
        data class AudioTrack(val reason: String?) : Choice()
    }

    fun isPcm(encoding: Int): Boolean = encoding == ENCODING_PCM_16BIT || encoding == ENCODING_PCM_FLOAT ||
        encoding == ENCODING_PCM_24BIT_PACKED || encoding == ENCODING_PCM_32BIT

    /** Whether the exclusive mode plays a choice on a USB device: the system default or a USB device. */
    fun usbChoice(chosenKind: String?): Boolean = chosenKind == null || chosenKind == "usb"

    /**
     * The output for a track. Only in the exclusive mode, with the system default or a USB device chosen and a USB
     * audio device attached ([route]), is the USB output taken; anything else is Android's (shared).
     */
    fun choose(facts: Facts): Choice = when {
        !facts.enabled || !usbChoice(facts.chosenKind) || !facts.deviceAttached -> Choice.AudioTrack(null)
        !facts.permitted -> Choice.AudioTrack("未取得 USB 權限")
        facts.trackFallback != null -> Choice.AudioTrack(facts.trackFallback)
        !isPcm(facts.encoding) -> Choice.AudioTrack("不是 PCM 音訊")
        facts.unsupported != null -> Choice.AudioTrack(facts.unsupported)
        else -> Choice.Usb
    }

    /** Why [choose] decides as it does, in the same order, for the decision log. */
    fun explain(facts: Facts): String = when {
        !facts.enabled -> "shared mode"
        !usbChoice(facts.chosenKind) -> "chosen output is not USB (${facts.chosenKind}), shared"
        !facts.deviceAttached -> "no USB audio device attached for ${facts.chosenKind ?: "system default"}, shared"
        !facts.permitted -> "no USB permission"
        facts.trackFallback != null -> "this track already fell back"
        !isPcm(facts.encoding) -> "encoding ${facts.encoding} is not linear PCM"
        facts.unsupported != null -> "the device cannot take the format"
        else -> "every check passed"
    }

    /**
     * One decision for the log (tag `MoeMediaIndex`): where it was made ([where]: `getOutputConfig`, `getAudioOutput`,
     * `openUsb`), the result with its reason, and every fact it was made from.
     */
    fun decisionLog(where: String, facts: Facts, choice: Choice): String {
        val result = when (choice) {
            Choice.Usb -> "USB"
            is Choice.AudioTrack -> "AudioTrack" + (choice.reason?.let { " ($it)" } ?: "")
        }
        val mode = if (facts.enabled) "exclusive" else "shared"
        return "USB exclusive: $where → $result: ${explain(facts)}; mode=$mode " +
            "chosen=${facts.chosenKind ?: "system default"} device=${facts.device} " +
            "deviceAttached=${facts.deviceAttached} permitted=${facts.permitted} " +
            "encoding=${facts.encoding} rate=${facts.rate} channels=${facts.channels} " +
            "sourceBits=${facts.source.bits} dsd=${facts.source.dsd} trackFallback=${facts.trackFallback} " +
            "unsupported=${facts.unsupported}"
    }

    /** An output config Media3 asks for that the USB output can play: linear PCM without offload, tunneling or speed. */
    fun takesConfig(encoding: Int, isOffload: Boolean, isTunneling: Boolean, usePlaybackParameters: Boolean): Boolean =
        isPcm(encoding) && !isOffload && !isTunneling && !usePlaybackParameters

    /** What the user asks of the output: the output mode (true: 獨佔) and the chosen output device (null: system default). */
    data class Wish(val enabled: Boolean, val chosen: OutputDevicePreference?) {
        /** The exclusive mode plays this choice on a USB audio device when one is attached ([route]). */
        val usb: Boolean get() = enabled && usbChoice(chosen?.kind)
    }

    /** Where a track plays for a [Wish] and the attached USB audio devices (ADR 0047, the routing table). */
    sealed class Route {
        /** The app drives [device] itself (exclusive). */
        data class Exclusive(val device: UsbDeviceFacts) : Route()

        /**
         * Android plays it (an AudioTrack, shared). [noUsbDevice]: the exclusive mode would have played it on a USB
         * device, but none is attached (the card says so).
         */
        data class Shared(val noUsbDevice: Boolean) : Route()
    }

    /**
     * The USB device the exclusive mode plays the choice [chosen] on: for the system default the first attached USB
     * audio device ([devices] in `UsbManager` order by device name), for a USB device that device ([pickDevice]),
     * for anything else (speaker, Bluetooth, …) none.
     */
    fun pickExclusiveDevice(chosen: OutputDevicePreference?, devices: List<UsbDeviceFacts>): UsbDeviceFacts? = when {
        chosen == null -> devices.firstOrNull()
        chosen.kind == "usb" -> pickDevice(chosen.productName, devices)
        else -> null
    }

    /**
     * The routing table: shared mode → shared; exclusive with a non-USB device chosen → shared on it; exclusive with
     * the system default or a USB device chosen → exclusive on [pickExclusiveDevice], shared (with the hint) when no
     * USB audio device is attached (also once the playing device is unplugged: on to the system default).
     */
    fun route(wish: Wish, devices: List<UsbDeviceFacts>): Route {
        if (!wish.usb) return Route.Shared(noUsbDevice = false)
        val device = pickExclusiveDevice(wish.chosen, devices) ?: return Route.Shared(noUsbDevice = true)
        return Route.Exclusive(device)
    }

    /** The key of the USB device [route] plays on, null when it is shared. */
    fun target(wish: Wish, devices: List<UsbDeviceFacts>): String? =
        (route(wish, devices) as? Route.Exclusive)?.device?.key

    /**
     * What a change does at once: [handBack] the device the app holds (it is no longer the wanted one), [newOutput]
     * retire the output Media3 plays through now, so Media3 asks the provider again and the new choice applies to the
     * track that is playing (not only to the next one).
     */
    data class OutputChange(val handBack: Boolean, val newOutput: Boolean)

    /**
     * A change of the mode, of the chosen device or of the attached USB devices. [liveOn]: the USB device the output
     * Media3 plays through now drives (null: an AudioTrack); [claimed]: the USB device the app holds (null: none);
     * [target]: where the track should play now ([target]); [hasOutput]: Media3 has an output open (a track is loaded).
     * Shared to shared (speaker, Bluetooth, another Android device) changes nothing here: the router re-pins the
     * AudioTrack.
     */
    fun outputChange(liveOn: String?, claimed: String?, target: String?, hasOutput: Boolean): OutputChange =
        OutputChange(handBack = claimed != null && claimed != target, newOutput = hasOutput && liveOn != target)

    /**
     * The attached USB audio devices changed ([before] → [after]): whether it matters (the exclusive mode wants USB
     * for the choice and the device it plays on changed: attached → exclusive now, unplugged → shared). A device the
     * app claims stays listed by `UsbManager`, so a claim itself is no change.
     */
    fun usbDevicesChanged(wish: Wish, before: List<UsbDeviceFacts>, after: List<UsbDeviceFacts>): Boolean =
        wish.usb && target(wish, before) != target(wish, after)

    /**
     * Everything the automatic USB permission request depends on (ADR 0047): the [wish], the attached [devices], the keys
     * of the devices the app may already open ([permitted]), those whose prompt was answered "no" since they were attached
     * ([refused]), whether the app is in the foreground and whether a prompt is on screen now ([inFlight]).
     */
    data class AskFacts(
        val wish: Wish,
        val devices: List<UsbDeviceFacts>,
        val permitted: Set<String>,
        val refused: Set<String>,
        val foreground: Boolean,
        val inFlight: Boolean,
    )

    /** Why [decideAsk] does not ask now. */
    enum class SkipReason(val text: String) {
        BACKGROUND("background"),
        IN_FLIGHT("inFlight"),
        NO_TARGET("noTarget"),
        PERMITTED("permitted"),
        REFUSED("refused"),
    }

    /** The outcome of [decideAsk]: ask the permission of the device [Ask.key], or [Skip] for a [Skip.reason]. */
    sealed class AskDecision {
        data class Ask(val key: String) : AskDecision()
        data class Skip(val reason: SkipReason) : AskDecision()
    }

    /**
     * Whether to ask Android's permission dialog now (ADR 0047): only in the foreground, with no prompt open, for the
     * device the routing table plays on ([target]) when it is not permitted and was not refused since it was attached.
     * In the background the request waits for the foreground (the caller checks again then).
     */
    fun decideAsk(facts: AskFacts): AskDecision {
        if (!facts.foreground) return AskDecision.Skip(SkipReason.BACKGROUND)
        if (facts.inFlight) return AskDecision.Skip(SkipReason.IN_FLIGHT)
        val key = target(facts.wish, facts.devices) ?: return AskDecision.Skip(SkipReason.NO_TARGET)
        if (key in facts.permitted) return AskDecision.Skip(SkipReason.PERMITTED)
        if (key in facts.refused) return AskDecision.Skip(SkipReason.REFUSED)
        return AskDecision.Ask(key)
    }

    /** The device to ask for now, or null: [decideAsk] as a plain key. */
    fun autoRequestTarget(facts: AskFacts): String? = (decideAsk(facts) as? AskDecision.Ask)?.key

    /** `RunningAppProcessInfo.IMPORTANCE_FOREGROUND`: the process has the activity the user is interacting with. */
    const val IMPORTANCE_FOREGROUND = 100

    /**
     * Whether the app is in the foreground for a dialog: the system's process importance is foreground (an activity of
     * ours on top). A process that only runs the playback service is `IMPORTANCE_FOREGROUND_SERVICE` (125) and a visible
     * but not top one `IMPORTANCE_VISIBLE` (200): the user cannot answer a dialog we start then, so neither counts.
     */
    fun isForegroundImportance(importance: Int): Boolean = importance <= IMPORTANCE_FOREGROUND

    /** The one decision log line (`USB exclusive: auto-ask check ...`), the facts and the outcome. */
    fun decisionLine(trigger: String, facts: AskFacts, decision: AskDecision): String {
        val mode = if (facts.wish.enabled) "exclusive" else "shared"
        val key = target(facts.wish, facts.devices)
        val chosen = facts.wish.chosen?.let { "${it.kind}/${it.productName}" } ?: "systemDefault"
        val outcome = when (decision) {
            is AskDecision.Ask -> "ask"
            is AskDecision.Skip -> "skip (${decision.reason.text})"
        }
        return "USB exclusive: auto-ask check (trigger=$trigger) mode=$mode chosen=$chosen target=${key ?: "none"} " +
            "permitted=${key != null && key in facts.permitted} refused=${key != null && key in facts.refused} " +
            "foreground=${facts.foreground} inFlight=${facts.inFlight} -> $outcome"
    }

    /** A refusal is remembered only while its device stays attached: unplugged and plugged in again, it is asked again. */
    fun pruneRefused(refused: Set<String>, devices: List<UsbDeviceFacts>): Set<String> =
        refused.filterTo(HashSet()) { key -> devices.any { it.key == key } }

    /** The refusals after the user answered the prompt for [key]: a grant forgets it, a "no" remembers it. */
    fun refusedAfterAnswer(refused: Set<String>, key: String, granted: Boolean): Set<String> =
        if (granted) refused - key else refused + key

    /** One attached USB device as `UsbManager` lists it. */
    data class UsbDeviceFacts(val key: String, val productName: String?)

    /**
     * The USB device of the chosen output: the one whose USB product name is the chosen device's product name (Android
     * names a USB output after it), else the only USB audio device attached; null when it cannot be told.
     */
    fun pickDevice(productName: String, devices: List<UsbDeviceFacts>): UsbDeviceFacts? {
        val wanted = productName.trim()
        devices.firstOrNull { it.productName?.trim()?.equals(wanted, ignoreCase = true) == true }?.let { return it }
        return devices.singleOrNull()
    }

    /**
     * The devices the router lists while the app holds [held]: Android has removed it from its outputs (the kernel
     * driver let go), but it still plays, so it stays in the list as it was when it was taken.
     */
    fun withHeld(attached: List<OutputDeviceFacts>, held: OutputDeviceFacts?): List<OutputDeviceFacts> {
        if (held == null) return attached
        val identity = AudioOutputRules.identityOf(held) ?: return attached
        return if (attached.any { AudioOutputRules.identityOf(it) == identity }) attached else attached + held
    }

    /**
     * While the app holds the chosen device, its disappearance must not move the player to the system default (nor
     * show 「目前由…播放」): the pin is left as it is until the device is handed back.
     */
    fun keepsPin(preference: OutputDevicePreference?, held: OutputDeviceFacts?): Boolean =
        preference != null && held != null && AudioOutputRules.matches(preference, held)

    /** Frames the ring holds at [sampleRate]. */
    fun ringFrames(sampleRate: Int): Int = maxOf(sampleRate.toLong() * RING_MS / 1000, 1L).toInt()

    /** Microseconds of [frames] at [sampleRate] (rounded down), the output's position. */
    fun framesToUs(frames: Long, sampleRate: Int): Long =
        if (sampleRate <= 0) 0L else (frames.toBigInteger() * 1_000_000.toBigInteger() / sampleRate.toBigInteger()).toLong()

    /** The source as the load command carries it (`metadata.bitsPerSample`, `metadata.dsd`). */
    fun sourceOf(bits: Int?, dsd: Boolean): Source = Source(bits?.takeIf { it in 8..32 } ?: 0, dsd)
}
