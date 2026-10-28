package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ExternalOpenRulesTest {
    private val view = "android.intent.action.VIEW"

    @Test
    fun aViewIntentWithAFileOrContentUriIsARequest() {
        assertEquals(
            ExternalOpenRules.Incoming("content://media/external/audio/media/42", "audio/flac"),
            ExternalOpenRules.incomingOf(view, "content://media/external/audio/media/42", "audio/flac", 0),
        )
        assertEquals(
            ExternalOpenRules.Incoming("file:///storage/emulated/0/Music/a.flac", null),
            ExternalOpenRules.incomingOf(view, "file:///storage/emulated/0/Music/a.flac", " ", 0),
        )
    }

    @Test
    fun otherIntentsAreNotRequests() {
        assertNull(ExternalOpenRules.incomingOf("android.intent.action.MAIN", null, null, 0))
        assertNull(ExternalOpenRules.incomingOf("android.intent.action.SEND", "content://x/y", "audio/flac", 0))
        assertNull(ExternalOpenRules.incomingOf(view, null, "audio/flac", 0))
        assertNull(ExternalOpenRules.incomingOf(view, "https://example.com/a.flac", "audio/flac", 0))
        assertNull(ExternalOpenRules.incomingOf(view, "smb://nas/a.flac", null, 0))
        assertNull(ExternalOpenRules.incomingOf(view, "", null, 0))
    }

    @Test
    fun anIntentReplayedFromRecentsIsNotARequest() {
        assertNull(
            ExternalOpenRules.incomingOf(view, "content://x/y", "audio/flac", ExternalOpenRules.FLAG_LAUNCHED_FROM_HISTORY),
        )
        assertTrue(
            ExternalOpenRules.incomingOf(view, "content://x/y", "audio/flac", 0x10000000) != null,
        )
    }

    @Test
    fun aFileUriGivesItsDecodedPath() {
        assertEquals(
            "/storage/emulated/0/My Music/a b.flac",
            ExternalOpenRules.pathOfFileUri("file:///storage/emulated/0/My%20Music/a%20b.flac"),
        )
        assertEquals("/sdcard/歌.flac", ExternalOpenRules.pathOfFileUri("file:///sdcard/%E6%AD%8C.flac"))
        assertEquals("/a/b.flac", ExternalOpenRules.pathOfFileUri("file://localhost/a/b.flac?x=1#y"))
        assertEquals("/a/100%.flac", ExternalOpenRules.pathOfFileUri("file:///a/100%25.flac"))
        assertNull(ExternalOpenRules.pathOfFileUri("file:///a/../b.flac"))
        assertNull(ExternalOpenRules.pathOfFileUri("file:///a/%zz.flac"))
        assertNull(ExternalOpenRules.pathOfFileUri("file:///a/%4"))
        assertNull(ExternalOpenRules.pathOfFileUri("content://x/a"))
    }

    @Test
    fun aDocumentUriGivesItsIdAndAuthority() {
        val uri = "content://com.android.externalstorage.documents/document/primary%3AMusic%2FA%20B%2Fa.flac"
        assertEquals("primary:Music/A B/a.flac", ExternalOpenRules.documentIdOf(uri))
        assertEquals("com.android.externalstorage.documents", ExternalOpenRules.authorityOf(uri))
        val tree = "content://com.android.externalstorage.documents/tree/primary%3AMusic/document/primary%3AMusic%2Fa.flac"
        assertEquals("primary:Music/a.flac", ExternalOpenRules.documentIdOf(tree))
        assertNull(ExternalOpenRules.documentIdOf("content://media/external/audio/media/42"))
        assertNull(ExternalOpenRules.documentIdOf("file:///a/document/b"))
    }

    @Test
    fun theStorageAndDownloadsProvidersNameTheirFile() {
        assertEquals(
            "/storage/emulated/0/Music/a.flac",
            ExternalOpenRules.pathOfDocument("com.android.externalstorage.documents", "primary:Music/a.flac"),
        )
        assertEquals(
            "/storage/1A2B-3C4D/Music/a.flac",
            ExternalOpenRules.pathOfDocument("com.android.externalstorage.documents", "1A2B-3C4D:Music/a.flac"),
        )
        assertEquals(
            "/storage/emulated/0/Download/a.flac",
            ExternalOpenRules.pathOfDocument("com.android.providers.downloads.documents", "raw:/storage/emulated/0/Download/a.flac"),
        )
        // A numeric downloads id names nothing without a query; another provider's id is its own business.
        assertNull(ExternalOpenRules.pathOfDocument("com.android.providers.downloads.documents", "1234"))
        assertNull(ExternalOpenRules.pathOfDocument("com.android.providers.downloads.documents", "raw:/a/../b"))
        assertNull(ExternalOpenRules.pathOfDocument("com.example.files", "primary:Music/a.flac"))
        assertNull(ExternalOpenRules.pathOfDocument(null, "primary:a"))
    }

    @Test
    fun theCopyKeepsAnAudioExtensionOrTakesTheOneTheTypeNames() {
        assertEquals("Song.FLAC", ExternalOpenRules.copyName("Song.FLAC", "application/octet-stream"))
        assertEquals("Song.flac", ExternalOpenRules.copyName("Song", "audio/x-flac"))
        assertEquals("a.b.m4a", ExternalOpenRules.copyName("a.b", "audio/mp4"))
        assertEquals("audio.wv", ExternalOpenRules.copyName(null, "audio/x-wavpack"))
        assertEquals("Song.bin", ExternalOpenRules.copyName("Song.bin", null))
        assertEquals("x_y.flac", ExternalOpenRules.copyName("dir/x:y.flac", null))
        assertFalse(ExternalOpenRules.copyName("../../etc/passwd", null).contains("/"))
    }

    @Test
    fun theInboxKeepsTheNewestRequestAndGivesItOnce() {
        val inbox = ExternalOpenInbox()
        assertFalse(inbox.hasPending())
        assertNull(inbox.take())
        val first = ExternalOpenRules.Incoming("content://a/1", null)
        val second = ExternalOpenRules.Incoming("content://a/2", "audio/flac")
        inbox.offer(first)
        inbox.offer(null)
        assertTrue(inbox.hasPending())
        inbox.offer(second)
        assertEquals(second, inbox.take())
        assertFalse(inbox.hasPending())
        assertNull(inbox.take())
    }
}
