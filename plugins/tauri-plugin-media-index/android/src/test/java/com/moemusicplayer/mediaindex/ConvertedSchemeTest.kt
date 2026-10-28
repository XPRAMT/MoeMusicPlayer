package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConvertedSchemeTest {
    @Test
    fun convertedLocalAndRemoteSchemesAreReadThroughTheRustDataSource() {
        for (scheme in listOf("moedsd", "moedsdraw", "moewv", "moewvraw", "moeflac", "moeremote", "moeremoteraw")) {
            assertTrue(scheme, isRustServedScheme(scheme))
        }
        for (scheme in listOf("file", "content", "http", "moeflacx", "moedsdrawx", "moeremoterawx", null)) {
            assertFalse("$scheme", isRustServedScheme(scheme))
        }
    }

    @Test
    fun aConvertedLocalTrackMapsBackToItsFileUri() {
        assertEquals("file:///x/a.flac", fileUriOfConverted("moeflac:///x/a.flac"))
        assertEquals("file:///x/a.dsf", fileUriOfConverted("moedsd:///x/a.dsf"))
        assertEquals("file:///x/a.dsf", fileUriOfConverted("moedsdraw:///x/a.dsf"))
        assertEquals("file:///x/A%20B.wv", fileUriOfConverted("moewv:///x/A%20B.wv"))
        assertEquals("file:///x/a.mp3", fileUriOfConverted("file:///x/a.mp3"))
        assertEquals("moeremote://id/a.flac", fileUriOfConverted("moeremote://id/a.flac"))
    }
}
