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
        val enabled: Boolean,
        /** The chosen output device's kind (`usb`, …), null for the system default. */
        val chosenKind: String?,
        /** A USB audio device matching the choice is attached (the app may be holding it). */
        val deviceAttached: Boolean,
        val permitted: Boolean,
        val encoding: Int,
        /** Why this track already failed on the USB output (it then plays through Android until the next track). */
        val trackFallback: String?,
        /** What the device's descriptors say about the format: null when it can take it, else why not. */
        val unsupported: String?,
    )

    sealed class Choice {
        object Usb : Choice()
        /** The AudioTrack path; [reason] is shown when the user expected the USB output and it is not used. */
        data class AudioTrack(val reason: String?) : Choice()
    }

    fun isPcm(encoding: Int): Boolean = encoding == ENCODING_PCM_16BIT || encoding == ENCODING_PCM_FLOAT ||
        encoding == ENCODING_PCM_24BIT_PACKED || encoding == ENCODING_PCM_32BIT

    /** The output for a track. Only the chosen USB device with the switch on is ever taken; anything else is Android's. */
    fun choose(facts: Facts): Choice = when {
        !facts.enabled || facts.chosenKind != "usb" || !facts.deviceAttached -> Choice.AudioTrack(null)
        !facts.permitted -> Choice.AudioTrack("未取得 USB 權限")
        facts.trackFallback != null -> Choice.AudioTrack(facts.trackFallback)
        !isPcm(facts.encoding) -> Choice.AudioTrack("不是 PCM 音訊")
        facts.unsupported != null -> Choice.AudioTrack(facts.unsupported)
        else -> Choice.Usb
    }

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
