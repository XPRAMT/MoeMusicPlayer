package com.moemusicplayer.mediaindex

/**
 * Where the root opt-in (ADR 0048) comes from: settings.json through Rust (`android_root.rs`) is the store; the old
 * SharedPreferences value (`moe_mixer_rate`, key `rootOptIn`) is carried over once. Pure rules for [MixerRatePrefs].
 */
internal object RootOptInRules {
    /** Rust's copy as `nativeRootOptIn` reports it. */
    const val NOT_READ = -1
    const val UNSET = 0
    const val OFF = 1
    const val ON = 2

    /**
     * The value used now: an answer not yet taken by Rust first, then settings.json, then (while settings.json has
     * none or is not read yet) the old SharedPreferences value; otherwise false (never asked: `su` is never run).
     */
    fun effective(stored: Int, legacy: Boolean?, pending: Boolean?): Boolean = pending ?: when (stored) {
        ON -> true
        OFF -> false
        else -> legacy ?: false
    }

    enum class LegacyStep { NONE, WAIT, IMPORT, DROP }

    /** What to do with the old value: wait for Rust, carry it over (settings.json has none), or just drop it. */
    fun legacyStep(stored: Int, legacy: Boolean?): LegacyStep = when {
        legacy == null -> LegacyStep.NONE
        stored == NOT_READ -> LegacyStep.WAIT
        stored == UNSET -> LegacyStep.IMPORT
        else -> LegacyStep.DROP
    }
}