package com.moemusicplayer.mediaindex

/**
 * Audio files other apps hand to this one with `ACTION_VIEW` (the system's 「開啟方式」 chooser, ADR 0054). These are the
 * pure rules: which intents count, what a `file:` or document URI says about the real path, and the one request the
 * front end takes. Everything that needs a `ContentResolver` lives in [MediaIndexPlugin].
 */
internal object ExternalOpenRules {
    const val ACTION_VIEW = "android.intent.action.VIEW"

    /** `Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`: the task was brought back from Recents, not started by a sender. */
    const val FLAG_LAUNCHED_FROM_HISTORY = 0x00100000

    /** What the sender gave: the data URI text and the MIME type it claimed (absent or `application/octet-stream` too). */
    data class Incoming(val uri: String, val mimeType: String?)

    /**
     * The request an intent carries, or null when it is not one: a launcher start, a share, a URI of another scheme, or an
     * intent the system replays when the task comes back from Recents (it would play the file again).
     */
    fun incomingOf(action: String?, data: String?, type: String?, flags: Int): Incoming? {
        if (action != ACTION_VIEW) return null
        if (flags and FLAG_LAUNCHED_FROM_HISTORY != 0) return null
        val uri = data?.takeIf { it.isNotBlank() } ?: return null
        val scheme = schemeOf(uri) ?: return null
        if (scheme != "file" && scheme != "content") return null
        return Incoming(uri, type?.takeIf { it.isNotBlank() })
    }

    fun schemeOf(uri: String): String? {
        val colon = uri.indexOf(':')
        if (colon <= 0) return null
        val scheme = uri.substring(0, colon)
        if (!scheme.all { it.isLetterOrDigit() || it == '+' || it == '-' || it == '.' }) return null
        return scheme.lowercase()
    }

    /** `file:///storage/emulated/0/My%20Music/a.flac` -> `/storage/emulated/0/My Music/a.flac`. */
    fun pathOfFileUri(uri: String): String? {
        if (schemeOf(uri) != "file") return null
        var rest = uri.substring(uri.indexOf(':') + 1)
        // `file://host/path` and `file:///path`: the path starts at the first slash after the authority.
        if (rest.startsWith("//")) {
            val slash = rest.indexOf('/', 2)
            rest = if (slash < 0) return null else rest.substring(slash)
        }
        val end = rest.indexOfFirst { it == '?' || it == '#' }
        val path = percentDecode(if (end >= 0) rest.substring(0, end) else rest) ?: return null
        if (!path.startsWith("/") || path.split('/').any { it == ".." }) return null
        return path
    }

    /** The `<id>` of a `content://<authority>/[tree/<tree>/]document/<id>` URI, decoded; null for any other shape. */
    fun documentIdOf(uri: String): String? {
        if (schemeOf(uri) != "content") return null
        val marker = "/document/"
        val at = uri.lastIndexOf(marker)
        if (at < 0) return null
        val raw = uri.substring(at + marker.length)
        val end = raw.indexOfFirst { it == '?' || it == '#' }
        return percentDecode(if (end >= 0) raw.substring(0, end) else raw)?.takeIf { it.isNotEmpty() }
    }

    fun authorityOf(uri: String): String? {
        val start = uri.indexOf("://")
        if (start < 0) return null
        val rest = uri.substring(start + 3)
        val end = rest.indexOfFirst { it == '/' || it == '?' || it == '#' }
        return (if (end >= 0) rest.substring(0, end) else rest).takeIf { it.isNotEmpty() }
    }

    /**
     * The real path a document URI names without asking its provider: the external storage provider's
     * `<volume>:<relative>` ids, and the downloads provider's `raw:<absolute path>` ids. Anything else needs a query.
     */
    fun pathOfDocument(authority: String?, documentId: String?): String? {
        if (authority == null || documentId == null) return null
        return when (authority) {
            "com.android.externalstorage.documents" -> MediaIndexPlugin.externalStoragePathFor(documentId)
            "com.android.providers.downloads.documents" ->
                documentId.removePrefix("raw:").takeIf { documentId.startsWith("raw:/") && !it.split('/').contains("..") }
            else -> null
        }
    }

    /** Percent-decoding of UTF-8 text (no `+` handling: this is a path); null for a broken escape. */
    fun percentDecode(text: String): String? {
        if ('%' !in text) return text
        val bytes = java.io.ByteArrayOutputStream(text.length)
        var index = 0
        while (index < text.length) {
            if (text[index] == '%') {
                if (index + 3 > text.length) return null
                val digits = text.substring(index + 1, index + 3)
                if (!digits.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) return null
                val value = digits.toInt(16)
                bytes.write(value)
                index += 3
            } else {
                // A literal character (a surrogate pair is one code point): written as UTF-8.
                val length = Character.charCount(text.codePointAt(index))
                bytes.write(text.substring(index, index + length).toByteArray(Charsets.UTF_8))
                index += length
            }
        }
        return String(bytes.toByteArray(), Charsets.UTF_8)
    }

    /**
     * MIME type -> file extension for the copy made when a file has no real path. Mirrors
     * `player_formats::extension_for_mime` for the types the manifest declares; the test
     * `plugins/tauri-plugin-media-index/tests/manifest_open_with.rs` fails when they drift.
     */
    internal val MIME_EXTENSIONS: Map<String, String> = mapOf(
        "audio/mpeg" to "mp3",
        "audio/mp3" to "mp3",
        "audio/mp4" to "m4a",
        "audio/x-m4a" to "m4a",
        "audio/m4a" to "m4a",
        "audio/aac" to "m4a",
        "audio/mp4a-latm" to "m4a",
        "audio/flac" to "flac",
        "audio/x-flac" to "flac",
        "audio/wav" to "wav",
        "audio/x-wav" to "wav",
        "audio/vnd.wave" to "wav",
        "audio/wave" to "wav",
        "audio/aiff" to "aiff",
        "audio/x-aiff" to "aiff",
        "audio/ogg" to "ogg",
        "application/ogg" to "ogg",
        "audio/vorbis" to "ogg",
        "audio/opus" to "opus",
        "audio/x-matroska" to "mka",
        "audio/matroska" to "mka",
        "audio/x-wavpack" to "wv",
        "audio/x-dsf" to "dsf",
        "audio/dsf" to "dsf",
        "audio/ac3" to "ac3",
        "audio/amr" to "amr",
    )

    /** Extensions the player opens (mirrors `AUDIO_EXTENSIONS` of the plugin). */
    fun hasAudioExtension(name: String): Boolean =
        name.substringAfterLast('.', "").lowercase() in MediaIndexPlugin.AUDIO_EXTENSIONS

    /**
     * The name for the private copy of a file without a real path: a safe base name that keeps the sender's extension when
     * it is an audio one, else the one the MIME type names, else the name as it is (the player then refuses it).
     */
    fun copyName(displayName: String?, mimeType: String?): String {
        val cleaned = (displayName ?: "").substringAfterLast('/').replace(Regex("[\\u0000-\\u001f\\\\:*?\"<>|]"), "_").trim()
        val base = cleaned.ifEmpty { "audio" }
        if (hasAudioExtension(base)) return base
        val extension = mimeType?.lowercase()?.let { MIME_EXTENSIONS[it] }
        return if (extension != null) "$base.$extension" else base
    }
}

/** The one request waiting for the front end: set by the launch intent or by a new `ACTION_VIEW`, taken once. */
internal class ExternalOpenInbox {
    private var pending: ExternalOpenRules.Incoming? = null

    @Synchronized
    fun offer(incoming: ExternalOpenRules.Incoming?) {
        // A newer request replaces an older one that nobody took: the user's last choice wins.
        if (incoming != null) pending = incoming
    }

    @Synchronized
    fun hasPending(): Boolean = pending != null

    @Synchronized
    fun take(): ExternalOpenRules.Incoming? {
        val taken = pending
        pending = null
        return taken
    }
}
