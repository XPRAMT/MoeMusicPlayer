package com.moemusicplayer.mediaindex

import android.util.Log
import org.json.JSONObject

/**
 * The floating lyrics overlay's lock state and positions (ADR 0032). settings.json keeps them (`floatingLyrics.locked`,
 * `position` = portrait, `landscapePosition`) and Rust is the only writer (`android_floating.rs`): this object reads
 * Rust's copy over JNI and reports every change back over JNI, which never waits for the disk. Until Rust has read the
 * settings ([nativeState] is null) the values here are the ones in memory (nothing saved yet: unlocked, centred); it
 * asks again on the next read, and changes made meanwhile are sent once it answers. Thread-safe.
 */
internal object FloatingLyricsStore {
    private const val TAG = "MoeMediaIndex"

    /** Rust's copy as JSON (`{"locked":…,"position":{x,y}|null,"landscapePosition":…}`), or null until it is known. */
    private external fun nativeState(): String?

    /** A change (only its keys change; `null` clears a position); false when Rust cannot take it yet. */
    private external fun nativeSave(change: String): Boolean

    /** The one-time carry-over of the old SharedPreferences values; writes the file before it returns (not on main). */
    private external fun nativeImportLegacy(values: String): Boolean

    private var loaded = false
    private var placement = FloatingLyricsRules.SavedPlacement()
    /** Changes Rust did not take yet (sent again, in one piece, once it answers). */
    private var pending = JSONObject()

    /** The current values: Rust's once it answered, until then the ones in memory. */
    @Synchronized
    fun current(): FloatingLyricsRules.SavedPlacement {
        if (!loaded) load()
        return placement
    }

    /** Reads Rust's copy again (after the carry-over); keeps the values in memory while it has none. */
    @Synchronized
    fun reload() {
        loaded = false
        load()
    }

    @Synchronized
    fun setLocked(locked: Boolean) {
        placement = placement.copy(locked = locked)
        send(JSONObject().put("locked", locked))
    }

    /** Saves (or with null clears) the position of one orientation. */
    @Synchronized
    fun setPosition(landscape: Boolean, ratio: FloatingLyricsRules.Ratio?) {
        placement = placement.withPosition(landscape, ratio)
        send(JSONObject().put(FloatingLyricsRules.positionKey(landscape), ratioJson(ratio)))
    }

    /** See [nativeImportLegacy]; false when Rust cannot take it yet (try again later). Not on the main thread. */
    fun importLegacy(values: JSONObject): Boolean = try {
        nativeImportLegacy(values.toString())
    } catch (error: Throwable) {
        Log.w(TAG, "floating lyrics: the old values could not be carried over", error)
        false
    }

    private fun load() {
        val json = try {
            nativeState()
        } catch (error: Throwable) {
            null
        } ?: return
        val stored = parseState(json) ?: return
        loaded = true
        placement = applyChange(stored, pending)
        if (pending.length() > 0 && saveNative(pending)) pending = JSONObject()
    }

    private fun send(change: JSONObject) {
        if (loaded && pending.length() == 0 && saveNative(change)) return
        // Not known to Rust yet (or an earlier change is still waiting): keep it, newest value per key.
        for (key in change.keys()) pending.put(key, change.get(key))
        if (loaded && saveNative(pending)) pending = JSONObject()
    }

    private fun saveNative(change: JSONObject): Boolean = try {
        nativeSave(change.toString())
    } catch (error: Throwable) {
        false
    }

    /** JSON of a position (`JSONObject.NULL` for none). */
    fun ratioJson(ratio: FloatingLyricsRules.Ratio?): Any =
        if (ratio == null) JSONObject.NULL else JSONObject().put("x", ratio.x).put("y", ratio.y)

    /** A position from JSON; null for none or anything invalid. */
    fun parseRatio(value: Any?): FloatingLyricsRules.Ratio? {
        val item = value as? JSONObject ?: return null
        if (!item.has("x") || !item.has("y")) return null
        val ratio = FloatingLyricsRules.Ratio(item.optDouble("x"), item.optDouble("y"))
        return ratio.takeIf { it.isValid() }
    }

    /** Rust's state JSON; null when it cannot be read. */
    fun parseState(json: String): FloatingLyricsRules.SavedPlacement? = try {
        val item = JSONObject(json)
        FloatingLyricsRules.SavedPlacement(
            item.optBoolean("locked", false),
            parseRatio(item.opt("position")),
            parseRatio(item.opt("landscapePosition")),
        )
    } catch (error: Exception) {
        null
    }

    /** `state` with the keys of `change` applied (the same rule as Rust's `merge`). */
    fun applyChange(state: FloatingLyricsRules.SavedPlacement, change: JSONObject): FloatingLyricsRules.SavedPlacement {
        var next = state
        if (change.has("locked")) next = next.copy(locked = change.optBoolean("locked", next.locked))
        for (landscape in listOf(false, true)) {
            val key = FloatingLyricsRules.positionKey(landscape)
            if (change.has(key)) next = next.withPosition(landscape, parseRatio(change.opt(key)))
        }
        return next
    }

    /** The carry-over payload of the old values (only the ones that existed). */
    fun legacyJson(locked: Boolean?, portrait: FloatingLyricsRules.Ratio?, landscape: FloatingLyricsRules.Ratio?): JSONObject {
        val values = JSONObject()
        if (locked != null) values.put("locked", locked)
        if (portrait != null) values.put(FloatingLyricsRules.positionKey(false), ratioJson(portrait))
        if (landscape != null) values.put(FloatingLyricsRules.positionKey(true), ratioJson(landscape))
        return values
    }
}
