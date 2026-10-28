package com.moemusicplayer.mediaindex

import android.Manifest
import android.bluetooth.BluetoothCodecConfig
import android.bluetooth.BluetoothCodecStatus
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.annotation.ChecksSdkIntAtLeast
import androidx.annotation.RequiresApi
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

/**
 * The Bluetooth codec each A2DP device streams with (ADR 0044). The public SDK has no call that reads the current
 * codec (`BluetoothA2dp.getCodecStatus` is a system API), but the Bluetooth stack broadcasts the codec status with
 * the public `BluetoothCodecStatus.EXTRA_CODEC_STATUS` whenever it is set (on connection and on every change). This
 * keeps the last one per device address, so it is known only after such a broadcast while the process runs. Needs
 * Android 13+ (the codec classes) and BLUETOOTH_CONNECT (the broadcast is sent to holders of it).
 */
internal object BluetoothCodecWatch {
    private const val TAG = "MoeMediaIndex"
    /** The broadcast the Bluetooth stack sends with the codec status (an action string, not an API call). */
    private const val ACTION_CODEC_CONFIG_CHANGED = "android.bluetooth.a2dp.profile.action.CODEC_CONFIG_CHANGED"

    private val codecs = ConcurrentHashMap<String, BluetoothCodecFacts>()
    /** LDAC's `codecSpecific1` per address, read by the privileged helper (ADR 0051); dropped at every codec announcement. */
    private val ldacModes = ConcurrentHashMap<String, Long>()
    /** Called (on the broadcast thread, memory only) with the address and codec type after each announcement. */
    @Volatile var onAnnounced: ((address: String, codecType: Int) -> Unit)? = null
    /** Called (memory only) with the address after each announcement; the codec priority list uses it (ADR 0052). */
    @Volatile var onChoicesAnnounced: ((address: String) -> Unit)? = null
    /** The same announcement in plain values, with the rates the codec offers (the sample-rate follow, ADR 0051). */
    private val states = ConcurrentHashMap<String, BluetoothCodecState>()
    /** Per device: the codec in use, the codecs it can switch to and the codecs this phone can encode (ADR 0052). */
    private val choices = ConcurrentHashMap<String, BluetoothCodecChoices>()
    /** A short failure text of the last codec switch per device, with the time it happened. */
    private val codecErrors = ConcurrentHashMap<String, Pair<String, Long>>()
    /** The phone's own codecs, as last seen (restored from an earlier run by [restoreLocal]); empty until known. */
    @Volatile var knownLocal: List<CodecOffer> = emptyList()
        private set
    /** Called (memory only) when the stack reported phone codecs different from [knownLocal], so they can be kept. */
    @Volatile var onLocalSeen: ((List<CodecOffer>) -> Unit)? = null
    private val changed = Object()
    @Volatile private var registered = false

    /** Whether the receiver is registered (an announcement can be heard at all). */
    fun listening(): Boolean = registered

    /** Called when a codec change was not announced in time, so the codec should be read again. */
    @Volatile var onNeedRefresh: ((address: String) -> Unit)? = null

    /** Whether the codec of the address is known (announced by the stack or read through the helper). */
    fun known(address: String): Boolean = BluetoothCodecRules.addressKey(address)?.let { codecs.containsKey(it) } ?: false

    /** Keeps a codec the helper read (`state`); a codec the stack announced meanwhile is not overwritten. */
    fun setReadState(address: String, state: BluetoothRateRules.HelperResult.State) {
        val key = BluetoothCodecRules.addressKey(address) ?: return
        val facts = BluetoothCodecFacts(state.codecType, state.rateFlag, 0, 0, null)
        if (BluetoothCodecRules.codecJson(facts) == null) return
        codecs[key] = facts
        states[key] = BluetoothCodecState(state.codecType, state.rateFlag, state.selectableMask)
        if (state.codecType == BluetoothCodecRules.CODEC_LDAC) state.specifics.firstOrNull()?.let { ldacModes[key] = it }
        synchronized(changed) { changed.notifyAll() }
    }

    /** Restores the phone's codec list kept from an earlier run. */
    fun restoreLocal(offers: List<CodecOffer>) {
        if (knownLocal.isEmpty()) knownLocal = offers
    }

    /** What is known of the codecs of the address (announced by the stack or read by the helper), or null. */
    fun choices(address: String): BluetoothCodecChoices? = BluetoothCodecRules.addressKey(address)?.let { choices[it] }

    /** Keeps what the helper read; the phone's codecs go to [onLocalSeen] when they are new. */
    fun setChoices(address: String, value: BluetoothCodecChoices) {
        val key = BluetoothCodecRules.addressKey(address) ?: return
        choices[key] = value
        noteLocal(value.local)
        synchronized(changed) { changed.notifyAll() }
    }

    private fun noteLocal(local: List<CodecOffer>) {
        if (local.isEmpty() || local == knownLocal) return
        knownLocal = local
        onLocalSeen?.invoke(local)
    }

    /** Waits up to [timeoutMs] until the announced codec of the address is [codecKey]; never on the main thread. */
    fun awaitCodec(address: String, codecKey: String, timeoutMs: Long): Boolean {
        val key = BluetoothCodecRules.addressKey(address) ?: return false
        val deadline = System.nanoTime() + timeoutMs * 1_000_000L
        synchronized(changed) {
            while (true) {
                if (choices[key]?.activeKey == codecKey) return true
                val remainingMs = (deadline - System.nanoTime()) / 1_000_000L
                if (remainingMs <= 0L) {
                    // Nobody confirmed the change: the codec is read again (ADR 0051).
                    onNeedRefresh?.invoke(address)
                    return false
                }
                try {
                    changed.wait(remainingMs)
                } catch (_: InterruptedException) {
                    return false
                }
            }
        }
    }

    fun setCodecError(address: String, text: String?, nowMs: Long) {
        val key = BluetoothCodecRules.addressKey(address) ?: return
        if (text == null) codecErrors.remove(key) else codecErrors[key] = text to nowMs
    }

    /**
     * The codec list for the settings page: the phone's codecs (the last known ones when no headset is connected) and,
     * per headset, the codec in use, the keys it supports and a recent failure text.
     */
    fun choicesJson(nowMs: Long, errorVisibleMs: Long): JSONObject {
        val local = org.json.JSONArray()
        for (offer in knownLocal) local.put(JSONObject().put("key", offer.key).put("name", offer.name))
        val devices = JSONObject()
        for ((address, value) in choices) {
            val entry = JSONObject()
            entry.put("active", value.activeKey ?: JSONObject.NULL)
            entry.put("supported", org.json.JSONArray(value.selectable.map { it.key }))
            codecErrors[address]?.let { (text, at) -> if (nowMs - at in 0..errorVisibleMs) entry.put("error", text) }
            devices.put(address, entry)
        }
        return JSONObject().put("local", local).put("devices", devices)
    }

    /** The last codec state announced for the address, or null. */
    fun state(address: String): BluetoothCodecState? = BluetoothCodecRules.addressKey(address)?.let { states[it] }

    /**
     * Waits up to [timeoutMs] until the announced rate of the address is [rateFlag]; true when it is. Never call it on
     * the main thread.
     */
    fun awaitRate(address: String, rateFlag: Int, timeoutMs: Long): Boolean {
        val key = BluetoothCodecRules.addressKey(address) ?: return false
        val deadline = System.nanoTime() + timeoutMs * 1_000_000L
        synchronized(changed) {
            while (true) {
                if (states[key]?.rateFlag == rateFlag) return true
                val remainingMs = (deadline - System.nanoTime()) / 1_000_000L
                if (remainingMs <= 0L) {
                    // Nobody confirmed the change: the codec is read again (ADR 0051).
                    onNeedRefresh?.invoke(address)
                    return false
                }
                try {
                    changed.wait(remainingMs)
                } catch (_: InterruptedException) {
                    return false
                }
            }
        }
    }

    /** Whether the codec can be read here at all (Android 13+). */
    @ChecksSdkIntAtLeast(api = Build.VERSION_CODES.TIRAMISU)
    fun supported(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

    fun permissionGranted(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
            context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

    /** `granted`, `needed` (ask for BLUETOOTH_CONNECT) or `unavailable` (before Android 13: nothing to read). */
    fun permissionState(context: Context): String = when {
        !supported() -> "unavailable"
        permissionGranted(context) -> "granted"
        else -> "needed"
    }

    /** Start listening once the permission is held; safe to call again (the service and the settings page do). */
    @Synchronized
    fun ensure(context: Context) {
        if (registered || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU || !permissionGranted(context)) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                if (intent != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) remember(intent)
            }
        }
        try {
            // The Bluetooth stack is another process: the receiver is exported (the action is a protected broadcast).
            context.applicationContext.registerReceiver(
                receiver,
                IntentFilter(ACTION_CODEC_CONFIG_CHANGED),
                Context.RECEIVER_EXPORTED,
            )
            registered = true
        } catch (error: RuntimeException) {
            Log.w(TAG, "Could not listen for the Bluetooth codec", error)
        }
    }

    /** The mode the helper read for the address since the last announcement, or null. */
    fun ldacMode(address: String): Long? = BluetoothCodecRules.addressKey(address)?.let { ldacModes[it] }

    /** Keeps what the helper read for the address. */
    fun setLdacMode(address: String, codecSpecific1: Long) {
        BluetoothCodecRules.addressKey(address)?.let { ldacModes[it] = codecSpecific1 }
    }

    /** The last codec of each device, by upper-case address; LDAC carries the mode the helper read, if it did. */
    fun snapshot(): Map<String, JSONObject> {
        val out = HashMap<String, JSONObject>()
        for ((key, facts) in codecs) {
            val withMode = if (facts.codecType == BluetoothCodecRules.CODEC_LDAC) facts.copy(codecSpecific1 = ldacModes[key]) else facts
            BluetoothCodecRules.codecJson(withMode)?.let { out[key] = it }
        }
        return out
    }

    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun remember(intent: Intent) {
        try {
            val device = intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
            val status = intent.getParcelableExtra(BluetoothCodecStatus.EXTRA_CODEC_STATUS, BluetoothCodecStatus::class.java)
            val key = BluetoothCodecRules.addressKey(device?.address) ?: return
            val config = status?.codecConfig
            if (config == null) {
                codecs.remove(key)
                choices.remove(key)
                states.remove(key)
                ldacModes.remove(key)
                return
            }
            var selectable = config.sampleRate
            for (capability in status?.codecsSelectableCapabilities.orEmpty()) {
                if (capability.codecType == config.codecType) selectable = selectable or capability.sampleRate
            }
            states[key] = BluetoothCodecState(config.codecType, config.sampleRate, selectable)
            val selectableOffers = status?.codecsSelectableCapabilities.orEmpty().mapNotNull(::codecOfferOf)
            val localOffers = status?.codecsLocalCapabilities.orEmpty().mapNotNull(::codecOfferOf)
            val announced = BluetoothCodecChoices(
                codecKeyOf(config),
                BluetoothCodecChoiceRules.deviceOrder(selectableOffers),
                BluetoothCodecChoiceRules.deviceOrder(localOffers),
            )
            choices[key] = announced
            noteLocal(announced.local)
            synchronized(changed) { changed.notifyAll() }
            val extendedName = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                try {
                    config.extendedCodecType?.codecName
                } catch (_: RuntimeException) {
                    null
                }
            } else {
                null
            }
            ldacModes.remove(key)
            val facts =
                BluetoothCodecFacts(
                    codecType = config.codecType,
                    sampleRateFlag = config.sampleRate,
                    bitsPerSampleFlag = config.bitsPerSample,
                    channelModeFlag = config.channelMode,
                    codecSpecific1 = null,
                    extendedName = extendedName,
                )
            if (BluetoothCodecRules.codecJson(facts) == null) codecs.remove(key) else codecs[key] = facts
            device?.address?.let { address -> onAnnounced?.invoke(address, config.codecType) }
            device?.address?.let { address -> onChoicesAnnounced?.invoke(address) }
        } catch (error: RuntimeException) {
            Log.w(TAG, "Could not read the Bluetooth codec", error)
        }
    }
}

/** The extended codec id of an Android 15+ vendor codec, or null (also when the platform does not say). */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal fun codecExtendedId(config: BluetoothCodecConfig): Long? {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return null
    return try {
        config.extendedCodecType?.codecId
    } catch (_: RuntimeException) {
        null
    }
}

/** The codec's key (`t<type>` or `x<id>`; ADR 0052), or null when it has neither. */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal fun codecKeyOf(config: BluetoothCodecConfig): String? =
    BluetoothCodecChoiceRules.keyOf(config.codecType, codecExtendedId(config))

/** One stack codec in plain values with a readable name, or null when it cannot be told apart. */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
internal fun codecOfferOf(config: BluetoothCodecConfig): CodecOffer? {
    val key = codecKeyOf(config) ?: return null
    val extendedName = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
        try {
            config.extendedCodecType?.codecName
        } catch (_: RuntimeException) {
            null
        }
    } else {
        null
    }
    val name = BluetoothCodecRules.codecName(config.codecType, extendedName) ?: key
    return CodecOffer(key, name, config.codecPriority.toLong(), config.sampleRate, config.bitsPerSample, config.channelMode)
}
