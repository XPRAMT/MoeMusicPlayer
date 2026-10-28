package com.moemusicplayer.mediaindex

import android.content.Context
import android.os.SystemClock
import android.util.Log

/** The saved original codecs and the phone's last known codec list, in SharedPreferences (`moe_bt_codec`). */
internal class PrefsBluetoothCodecStore(context: Context) : BluetoothCodecStore {
    private val prefs = context.applicationContext.getSharedPreferences("moe_bt_codec", Context.MODE_PRIVATE)

    override fun all(): Map<String, BluetoothCodecChoiceRules.SavedCodec> {
        val out = LinkedHashMap<String, BluetoothCodecChoiceRules.SavedCodec>()
        for ((key, value) in prefs.all) {
            if (!key.startsWith(SAVED)) continue
            val parts = (value as? String)?.split(',') ?: continue
            val original = parts.getOrNull(0)?.takeIf(BluetoothCodecChoiceRules::validKey) ?: continue
            val applied = parts.getOrNull(1)?.takeIf(BluetoothCodecChoiceRules::validKey) ?: continue
            out[key.removePrefix(SAVED)] = BluetoothCodecChoiceRules.SavedCodec(original, applied)
        }
        return out
    }

    override fun put(address: String, saved: BluetoothCodecChoiceRules.SavedCodec) {
        prefs.edit().putString(SAVED + address, "${saved.original},${saved.applied}").apply()
    }

    override fun remove(address: String) {
        prefs.edit().remove(SAVED + address).apply()
    }

    fun local(): List<CodecOffer> = BluetoothCodecChoiceRules.parseOffers(prefs.getString(LOCAL, null)) ?: emptyList()

    fun saveLocal(offers: List<CodecOffer>) {
        prefs.edit().putString(LOCAL, BluetoothCodecChoiceRules.formatOffers(offers)).apply()
    }

    private companion object {
        const val SAVED = "saved:"
        const val LOCAL = "local"
    }
}

/**
 * Wires the Bluetooth codec priority list (ADR 0052) to the device: the prefs the page saved, root / Shizuku, the codec
 * broadcast ([BluetoothCodecWatch]) and the saved original codecs. One per playback service; switching the list off while
 * no service runs uses [restoreWithoutService].
 */
internal class BluetoothCodecChoiceFollow private constructor(context: Context) {
    private val appContext = context.applicationContext
    private val scheduler = ExecutorScheduler()
    private val store = PrefsBluetoothCodecStore(appContext)
    @Volatile private var prefs = BluetoothCodecPrefs.OFF
    private val controller = BluetoothCodecChoiceController(Environment(), store, scheduler)

    private inner class Environment : BluetoothCodecChoiceEnvironment {
        override fun nowMs() = SystemClock.elapsedRealtime()
        override fun supported() = BluetoothCodecWatch.supported()
        override fun source() = BluetoothRateFollow.privilegedSource(appContext)
        override fun backend(source: MixerRateRules.Source) = BluetoothRateFollow.privilegedBackend(appContext, source)
        override fun prefs() = prefs
        override fun choices(address: String) = BluetoothCodecWatch.choices(address)
        override fun storeChoices(address: String, choices: BluetoothCodecChoices) = BluetoothCodecWatch.setChoices(address, choices)

        override fun awaitCodec(address: String, key: String, timeoutMs: Long): Boolean =
            BluetoothCodecWatch.listening() && BluetoothCodecWatch.awaitCodec(address, key, timeoutMs)

        override fun reportFailure(address: String, text: String?) =
            BluetoothCodecWatch.setCodecError(address, text, SystemClock.elapsedRealtime())

        override fun log(message: String) {
            Log.i(TAG, message)
        }
    }

    /** The service starts (main thread): the stored list, the prefs, and the attached headsets as connections. */
    fun start(prefsJson: String?, attachedA2dp: Set<String>) {
        prefs = BluetoothCodecPrefs.parse(prefsJson)
        BluetoothCodecWatch.restoreLocal(store.local())
        BluetoothCodecWatch.onLocalSeen = { store.saveLocal(it) }
        BluetoothCodecWatch.onChoicesAnnounced = { controller.announced(it.uppercase()) }
        BluetoothCodecWatch.ensure(appContext)
        controller.start(attachedA2dp)
    }

    /** The page saved the switch or the order (main thread). */
    fun prefsChanged(prefsJson: String?) {
        prefs = BluetoothCodecPrefs.parse(prefsJson)
        controller.prefsChanged()
    }

    fun devicesChanged(attachedA2dp: Set<String>) = controller.devicesChanged(attachedA2dp)

    /** The settings page is showing the list: read the codecs of headsets that have none known (main thread). */
    fun ensureChoices(attachedA2dp: Set<String>) {
        for (address in attachedA2dp) controller.ensureChoices(address)
    }

    fun symptom(what: String) = controller.symptom(what)

    /** The service ends. The codec we set stays (that is the point of the list); only the worker stops. */
    fun shutdown() {
        BluetoothCodecWatch.onChoicesAnnounced = null
        BluetoothCodecWatch.onLocalSeen = null
        scheduler.finish()
    }

    companion object {
        private const val TAG = "MoeMediaIndex"

        fun create(context: Context): BluetoothCodecChoiceFollow = BluetoothCodecChoiceFollow(context)

        /** The switch was turned off while no playback service runs: a short-lived worker puts the codecs back. */
        fun restoreWithoutService(context: Context) {
            val follow = BluetoothCodecChoiceFollow(context)
            if (follow.store.all().isEmpty() || !BluetoothCodecWatch.supported()) {
                follow.scheduler.finish()
                return
            }
            follow.controller.restoreAll("switch turned off")
            follow.scheduler.finish()
        }
    }
}
