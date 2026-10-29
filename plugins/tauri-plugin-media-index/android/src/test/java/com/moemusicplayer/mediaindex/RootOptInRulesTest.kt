package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RootOptInRulesTest {
    @Test
    fun settingsJsonWinsAndTheOldValueOnlyFillsAGap() {
        assertTrue(RootOptInRules.effective(RootOptInRules.ON, false, null))
        assertFalse(RootOptInRules.effective(RootOptInRules.OFF, true, null))
        assertTrue(RootOptInRules.effective(RootOptInRules.UNSET, true, null))
        assertTrue(RootOptInRules.effective(RootOptInRules.NOT_READ, true, null))
        assertFalse(RootOptInRules.effective(RootOptInRules.UNSET, null, null))
        assertFalse(RootOptInRules.effective(RootOptInRules.NOT_READ, null, null))
        // An answer Rust has not taken yet wins over everything.
        assertFalse(RootOptInRules.effective(RootOptInRules.ON, true, false))
        assertTrue(RootOptInRules.effective(RootOptInRules.NOT_READ, null, true))
    }

    @Test
    fun theOldValueIsCarriedOverOnlyWhenSettingsJsonHasNone() {
        assertEquals(RootOptInRules.LegacyStep.NONE, RootOptInRules.legacyStep(RootOptInRules.UNSET, null))
        assertEquals(RootOptInRules.LegacyStep.WAIT, RootOptInRules.legacyStep(RootOptInRules.NOT_READ, true))
        assertEquals(RootOptInRules.LegacyStep.IMPORT, RootOptInRules.legacyStep(RootOptInRules.UNSET, false))
        assertEquals(RootOptInRules.LegacyStep.DROP, RootOptInRules.legacyStep(RootOptInRules.ON, false))
        assertEquals(RootOptInRules.LegacyStep.DROP, RootOptInRules.legacyStep(RootOptInRules.OFF, true))
    }
}