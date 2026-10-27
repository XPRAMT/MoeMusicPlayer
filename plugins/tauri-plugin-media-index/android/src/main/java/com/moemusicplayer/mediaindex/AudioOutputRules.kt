package com.moemusicplayer.mediaindex

import org.json.JSONArray
import org.json.JSONObject

/** One output device as `AudioDeviceInfo` describes it, in plain values so the rules run on the JVM (ADR 0044). */
internal data class OutputDeviceFacts(
    val id: Int,
    val type: Int,
    val productName: String,
    val address: String,
    /** Empty: the device takes any rate (the platform resamples). The same for the other two lists. */
    val sampleRates: List<Int>,
    val channelCounts: List<Int>,
    val encodings: List<Int>,
)

/**
 * The output device the user chose, as `settings.json` keeps it (`androidOutputDevice`): its kind and product name, and
 * for Bluetooth also its address. `AudioDeviceInfo` ids change on every plug, so they are never stored.
 */
internal data class OutputDevicePreference(val kind: String, val productName: String, val address: String?)

/** What the Rust side answered about the choice (`nativeOutputDevicePreference`). */
internal sealed class PreferenceReading {
    /** The settings have not been read yet in this process: keep what is applied. */
    object Unknown : PreferenceReading()
    data class Known(val preference: OutputDevicePreference?) : PreferenceReading()
}

/** Pure rules of the Android output device choice (ADR 0044): which devices are listed, identity and matching. */
internal object AudioOutputRules {
    // AudioDeviceInfo.TYPE_* (platform constants; copied so the rules need no android.jar at test time).
    const val TYPE_BUILTIN_EARPIECE = 1
    const val TYPE_BUILTIN_SPEAKER = 2
    const val TYPE_WIRED_HEADSET = 3
    const val TYPE_WIRED_HEADPHONES = 4
    const val TYPE_LINE_ANALOG = 5
    const val TYPE_LINE_DIGITAL = 6
    const val TYPE_BLUETOOTH_SCO = 7
    const val TYPE_BLUETOOTH_A2DP = 8
    const val TYPE_HDMI = 9
    const val TYPE_HDMI_ARC = 10
    const val TYPE_USB_DEVICE = 11
    const val TYPE_USB_ACCESSORY = 12
    const val TYPE_DOCK = 13
    const val TYPE_FM = 14
    const val TYPE_TELEPHONY = 18
    const val TYPE_AUX_LINE = 19
    const val TYPE_IP = 20
    const val TYPE_BUS = 21
    const val TYPE_USB_HEADSET = 22
    const val TYPE_HEARING_AID = 23
    const val TYPE_BUILTIN_SPEAKER_SAFE = 24
    const val TYPE_REMOTE_SUBMIX = 25
    const val TYPE_BLE_HEADSET = 26
    const val TYPE_BLE_SPEAKER = 27
    const val TYPE_HDMI_EARC = 29
    const val TYPE_BLE_BROADCAST = 30
    const val TYPE_DOCK_ANALOG = 31

    /** The kinds, in the order the list shows them (the speaker of the phone last). */
    private val KIND_ORDER = listOf("usb", "wired", "bluetooth", "bleAudio", "hearingAid", "hdmi", "line", "dock", "speaker")

    /**
     * The kind of a device type, or null for a type that is not a place to play music: the earpiece, telephony, SCO
     * (calls), the "safe" speaker for alarms, buses, remote submix, FM, IP and the BLE broadcast (which duplicates a LE
     * Audio headset). Wired headset and headphones share a kind (the same jack reports either, depending on the mic),
     * as do the USB types (one DAC can report itself as a device or a headset).
     */
    fun kindOf(type: Int): String? = when (type) {
        TYPE_BUILTIN_SPEAKER -> "speaker"
        TYPE_WIRED_HEADSET, TYPE_WIRED_HEADPHONES -> "wired"
        TYPE_USB_DEVICE, TYPE_USB_ACCESSORY, TYPE_USB_HEADSET -> "usb"
        TYPE_BLUETOOTH_A2DP -> "bluetooth"
        TYPE_BLE_HEADSET, TYPE_BLE_SPEAKER -> "bleAudio"
        TYPE_HEARING_AID -> "hearingAid"
        TYPE_HDMI, TYPE_HDMI_ARC, TYPE_HDMI_EARC -> "hdmi"
        TYPE_LINE_ANALOG, TYPE_LINE_DIGITAL, TYPE_AUX_LINE -> "line"
        TYPE_DOCK, TYPE_DOCK_ANALOG -> "dock"
        else -> null
    }

    /** Kinds whose identity includes the address: two headsets of the same model are different devices. */
    fun usesAddress(kind: String): Boolean = kind == "bluetooth" || kind == "bleAudio" || kind == "hearingAid"

    /** The identity a device is chosen and found again by, or null for a device that is not listed. */
    fun identityOf(device: OutputDeviceFacts): OutputDevicePreference? {
        val kind = kindOf(device.type) ?: return null
        val address = device.address.trim().takeIf { usesAddress(kind) && it.isNotEmpty() }
        return OutputDevicePreference(kind, device.productName.trim(), address)
    }

    /**
     * Whether [device] is the chosen one: the same kind and product name, and the same address when both sides know
     * one (a Bluetooth address can be hidden, and is then not held against the match).
     */
    fun matches(preference: OutputDevicePreference, device: OutputDeviceFacts): Boolean {
        val identity = identityOf(device) ?: return false
        if (identity.kind != preference.kind || identity.productName != preference.productName.trim()) return false
        val wanted = preference.address?.trim()?.takeIf { it.isNotEmpty() } ?: return true
        val actual = identity.address ?: return true
        return sameAddress(wanted, actual)
    }

    /**
     * Whether two Bluetooth addresses are one device. Without BLUETOOTH_CONNECT, Android hides all but the last two
     * bytes (`XX:XX:XX:XX:AB:CD`); a choice saved that way still matches the full address once the permission is
     * granted (and the other way round), by those last two bytes.
     */
    fun sameAddress(a: String, b: String): Boolean {
        val left = a.trim().uppercase()
        val right = b.trim().uppercase()
        if (left == right) return true
        if (!isAnonymized(left) && !isAnonymized(right)) return false
        return left.length >= 5 && right.length >= 5 && left.takeLast(5) == right.takeLast(5)
    }

    private fun isAnonymized(address: String): Boolean = address.startsWith("XX:")

    /** The attached device to pin the player to, or null for the system's choice. */
    fun find(preference: OutputDevicePreference?, devices: List<OutputDeviceFacts>): OutputDeviceFacts? =
        preference?.let { wanted -> userDevices(devices).firstOrNull { matches(wanted, it) } }

    /**
     * The devices the list shows: playable kinds only, one row per identity (a device the platform reports twice, e.g.
     * as USB device and USB headset, is one row whose capabilities are the union), sorted by kind.
     */
    fun userDevices(all: List<OutputDeviceFacts>): List<OutputDeviceFacts> {
        val merged = LinkedHashMap<OutputDevicePreference, OutputDeviceFacts>()
        for (device in all) {
            val identity = identityOf(device) ?: continue
            val previous = merged[identity]
            merged[identity] = if (previous == null) device else previous.copy(
                sampleRates = union(previous.sampleRates, device.sampleRates),
                channelCounts = union(previous.channelCounts, device.channelCounts),
                encodings = union(previous.encodings, device.encodings),
            )
        }
        return merged.entries
            .sortedBy { KIND_ORDER.indexOf(it.key.kind) }
            .map { it.value }
    }

    /** Two capability lists as one; an empty list means "any", so it stays empty. */
    private fun union(a: List<Int>, b: List<Int>): List<Int> =
        if (a.isEmpty() || b.isEmpty()) emptyList() else (a + b).distinct().sorted()

    /** The channel count of an AudioTrack channel mask (`AudioFormat.CHANNEL_OUT_*`): one bit per channel. */
    fun channelCountOfMask(mask: Int): Int = Integer.bitCount(mask)

    /** Read the choice the Rust side keeps: a Java null is "not known yet", `null` the system default. */
    fun parsePreference(json: String?): PreferenceReading {
        if (json == null) return PreferenceReading.Unknown
        val trimmed = json.trim()
        if (trimmed.isEmpty() || trimmed == "null") return PreferenceReading.Known(null)
        return try {
            PreferenceReading.Known(preferenceOf(JSONObject(trimmed)))
        } catch (_: Exception) {
            PreferenceReading.Known(null)
        }
    }

    /** The choice in a JSON object (`kind`, `productName`, `address`), or null when it is not a usable choice. */
    fun preferenceOf(json: JSONObject?): OutputDevicePreference? {
        if (json == null) return null
        val kind = json.optString("kind").trim()
        val name = json.optString("productName").trim()
        val address = if (json.isNull("address")) null else json.optString("address").trim().takeIf { it.isNotEmpty() }
        // A device is told apart by its name or (a Bluetooth device whose name is hidden) its address.
        if (kind.isEmpty() || (name.isEmpty() && address == null)) return null
        return OutputDevicePreference(kind, name, address)
    }

    fun deviceJson(device: OutputDeviceFacts): JSONObject = JSONObject().apply {
        put("id", device.id)
        put("type", device.type)
        put("kind", kindOf(device.type) ?: "other")
        put("productName", device.productName.trim())
        put("address", device.address.trim())
        put("sampleRates", JSONArray(device.sampleRates))
        put("channelCounts", JSONArray(device.channelCounts))
        put("encodings", JSONArray(device.encodings))
        // What the page stores when this device is chosen: the identity rule lives here only.
        put("identity", identityOf(device)?.let(::preferenceJson) ?: JSONObject.NULL)
    }

    fun preferenceJson(preference: OutputDevicePreference): JSONObject = JSONObject().apply {
        put("kind", preference.kind)
        put("productName", preference.productName)
        if (preference.address != null) put("address", preference.address)
    }
}
