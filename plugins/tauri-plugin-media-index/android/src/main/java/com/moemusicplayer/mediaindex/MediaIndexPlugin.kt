package com.moemusicplayer.mediaindex

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import androidx.activity.result.ActivityResult
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray
import org.json.JSONObject
import java.util.ArrayDeque
import java.util.concurrent.Executors

@InvokeArg
class MediaStoreScanArgs {
    var sourceId: String? = null
    var volumeName: String? = null
}

@InvokeArg
class SafScanArgs {
    var sourceId: String? = null
    var treeUri: String? = null
}

@InvokeArg
class ContentUriArgs {
    var contentUri: String? = null
}

@InvokeArg
class TreeUriArgs {
    var treeUri: String? = null
}

@TauriPlugin(
    permissions = [
        Permission(
            strings = [Manifest.permission.READ_EXTERNAL_STORAGE],
            alias = "readExternalStorage",
        ),
        Permission(
            strings = [Manifest.permission.READ_MEDIA_AUDIO],
            alias = "readMediaAudio",
        ),
    ],
)
class MediaIndexPlugin(private val activity: Activity) : Plugin(activity) {
    private val resolver = activity.contentResolver
    private val io = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "moe-media-index").apply { isDaemon = true }
    }

    @Command
    fun mediaStoreVolumes(invoke: Invoke) {
        async(invoke) { buildMediaStoreVolumes() }
    }

    @Command
    fun requestMediaReadPermission(invoke: Invoke) {
        val alias = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            "readMediaAudio"
        } else {
            "readExternalStorage"
        }
        requestPermissionForAlias(alias, invoke, "onMediaPermissionResult")
    }

    @PermissionCallback
    private fun onMediaPermissionResult(invoke: Invoke) {
        val response = JSObject()
        response.put("granted", hasMediaReadPermission())
        invoke.resolve(response)
    }

    @Command
    fun scanMediaStore(invoke: Invoke) {
        val args = invoke.parseArgs(MediaStoreScanArgs::class.java)
        val sourceId = args.sourceId
        val volumeName = args.volumeName
        if (sourceId.isNullOrBlank() || volumeName.isNullOrBlank()) {
            invoke.reject("sourceId and volumeName are required")
            return
        }
        async(invoke) { scanMediaStore(sourceId, volumeName) }
    }

    @Command
    fun scanSafTree(invoke: Invoke) {
        val args = invoke.parseArgs(SafScanArgs::class.java)
        val sourceId = args.sourceId
        val rawTreeUri = args.treeUri
        if (sourceId.isNullOrBlank() || rawTreeUri.isNullOrBlank()) {
            invoke.reject("sourceId and treeUri are required")
            return
        }
        async(invoke) { scanSafTree(sourceId, Uri.parse(rawTreeUri)) }
    }

    @Command
    fun readMetadata(invoke: Invoke) {
        val args = invoke.parseArgs(ContentUriArgs::class.java)
        val rawUri = args.contentUri
        if (rawUri.isNullOrBlank()) {
            invoke.reject("contentUri is required")
            return
        }
        async(invoke) { readMetadata(Uri.parse(rawUri)) }
    }

    @Command
    fun pickSafTree(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PREFIX_URI_PERMISSION,
            )
        }
        startActivityForResult(invoke, intent, "onSafTreePicked")
    }

    @ActivityCallback
    private fun onSafTreePicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            val response = JSObject()
            response.put("uri", JSONObject.NULL)
            invoke.resolve(response)
            return
        }

        val readGrant = result.data?.flags?.and(Intent.FLAG_GRANT_READ_URI_PERMISSION) ?: 0
        if (readGrant == 0) {
            invoke.reject("The document provider did not grant read access")
            return
        }
        try {
            resolver.takePersistableUriPermission(uri, readGrant)
            val response = JSObject()
            response.put("uri", uri.toString())
            invoke.resolve(response)
        } catch (error: SecurityException) {
            invoke.reject("Could not persist document-tree access: ${error.message}")
        }
    }

    @Command
    fun hasSafPermission(invoke: Invoke) {
        val args = invoke.parseArgs(TreeUriArgs::class.java)
        val rawUri = args.treeUri
        if (rawUri.isNullOrBlank()) {
            invoke.reject("treeUri is required")
            return
        }
        val uri = Uri.parse(rawUri)
        val granted = resolver.persistedUriPermissions.any {
            it.isReadPermission && it.uri == uri
        }
        val response = JSObject()
        response.put("granted", granted)
        invoke.resolve(response)
    }

    @Command
    fun releaseSafTree(invoke: Invoke) {
        val args = invoke.parseArgs(TreeUriArgs::class.java)
        val rawUri = args.treeUri
        if (rawUri.isNullOrBlank()) {
            invoke.reject("treeUri is required")
            return
        }
        val uri = Uri.parse(rawUri)
        try {
            resolver.releasePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } catch (_: SecurityException) {
            // Releasing a removed or already-revoked source is intentionally idempotent.
        }
        invoke.resolve(JSObject())
    }

    private fun scanMediaStore(sourceId: String, volumeName: String): JSObject {
        if (!hasMediaReadPermission()) {
            return scanResult("permissionRevoked", "Audio media permission is not granted")
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val currentVolumes = MediaStore.getExternalVolumeNames(activity)
            if (volumeName !in currentVolumes) {
                return scanResult("unavailable", "MediaStore volume '$volumeName' is unavailable")
            }
        } else if (volumeName != "external" || !isLegacyExternalStorageMounted()) {
            return scanResult("unavailable", "External MediaStore volume is unavailable")
        }

        val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Audio.Media.getContentUri(volumeName)
        } else {
            MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
        }
        val versionBefore = getStoreVersion(volumeName)
        val generationBefore = getStoreGeneration(volumeName)
        val tracks = JSONArray()
        val errors = JSONArray()
        var incomplete = false

        try {
            val projection = mutableListOf(
                MediaStore.Audio.Media._ID,
                MediaStore.Audio.Media.SIZE,
                MediaStore.Audio.Media.DATE_MODIFIED,
                MediaStore.Audio.Media.TITLE,
                MediaStore.Audio.Media.ARTIST,
                MediaStore.Audio.Media.ALBUM,
                MediaStore.Audio.Media.ALBUM_ARTIST,
                MediaStore.Audio.Media.TRACK,
                MediaStore.Audio.Media.DURATION,
                MediaStore.Audio.Media.SAMPLERATE,
                MediaStore.MediaColumns.MIME_TYPE,
            )
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                projection.add(MediaStore.MediaColumns.BITRATE)
            }
            val cursor = resolver.query(collection, projection.toTypedArray(), null, null, null)
                ?: return scanResult("unavailable", "MediaStore returned no cursor for '$volumeName'")
            cursor.use { rows ->
                val idColumn = rows.getColumnIndex(MediaStore.Audio.Media._ID)
                val sizeColumn = rows.getColumnIndex(MediaStore.Audio.Media.SIZE)
                val modifiedColumn = rows.getColumnIndex(MediaStore.Audio.Media.DATE_MODIFIED)
                val titleColumn = rows.getColumnIndex(MediaStore.Audio.Media.TITLE)
                val artistColumn = rows.getColumnIndex(MediaStore.Audio.Media.ARTIST)
                val albumColumn = rows.getColumnIndex(MediaStore.Audio.Media.ALBUM)
                val albumArtistColumn = rows.getColumnIndex(MediaStore.Audio.Media.ALBUM_ARTIST)
                val trackColumn = rows.getColumnIndex(MediaStore.Audio.Media.TRACK)
                val durationColumn = rows.getColumnIndex(MediaStore.Audio.Media.DURATION)
                val bitrateColumn = rows.getColumnIndex(MediaStore.MediaColumns.BITRATE)
                val sampleRateColumn = rows.getColumnIndex(MediaStore.Audio.Media.SAMPLERATE)
                val mimeColumn = rows.getColumnIndex(MediaStore.MediaColumns.MIME_TYPE)

                if (idColumn < 0 || sizeColumn < 0) {
                    return scanResult("incomplete", "MediaStore omitted required identity or size columns")
                }

                while (rows.moveToNext()) {
                    try {
                        val id = rows.getLong(idColumn)
                        val size = rows.longOrNull(sizeColumn)
                        if (size == null || size < 0L) {
                            incomplete = true
                            errors.put(sourceError("$volumeName:$id", "MediaStore row has no valid size"))
                            continue
                        }
                        val rawTrack = rows.longOrNull(trackColumn)
                        val (disc, track) = splitTrackNumber(rawTrack)
                        val modifiedSeconds = rows.longOrNull(modifiedColumn)
                        val modifiedMillis = modifiedSeconds?.let { seconds ->
                            if (seconds >= 0 && seconds <= Long.MAX_VALUE / 1000L) seconds * 1000L else null
                        }
                        val uri = Uri.withAppendedPath(collection, id.toString())
                        val metadata = metadataObject(
                            title = rows.stringOrNull(titleColumn),
                            artist = rows.stringOrNull(artistColumn),
                            album = rows.stringOrNull(albumColumn),
                            albumArtist = rows.stringOrNull(albumArtistColumn),
                            trackNumber = track,
                            discNumber = disc,
                            durationMs = rows.longOrNull(durationColumn)?.takeIf { it >= 0 },
                            codec = rows.stringOrNull(mimeColumn),
                            bitrateBps = rows.longOrNull(bitrateColumn)?.takeIf { it in 0..Int.MAX_VALUE }?.toInt(),
                            sampleRateHz = rows.longOrNull(sampleRateColumn)?.takeIf { it in 0..Int.MAX_VALUE }?.toInt(),
                        )
                        tracks.put(trackRecord(
                            sourceId = sourceId,
                            sourceItemId = "$volumeName:$id",
                            contentUri = uri.toString(),
                            sizeBytes = size,
                            modifiedAtUtcMs = modifiedMillis,
                            metadata = metadata,
                        ))
                    } catch (error: Exception) {
                        incomplete = true
                        errors.put(sourceError(null, "Could not read a MediaStore row: ${error.message}"))
                    }
                }
            }
        } catch (error: SecurityException) {
            return scanResult(
                "permissionRevoked",
                "MediaStore permission was revoked during the query: ${error.message}",
                tracks,
                errors,
            )
        } catch (error: Exception) {
            return scanResult(
                "incomplete",
                "MediaStore query did not finish: ${error.message}",
                tracks,
                errors,
            )
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && volumeName !in MediaStore.getExternalVolumeNames(activity)) {
            return scanResult("unavailable", "MediaStore volume '$volumeName' disappeared during the query", tracks, errors)
        }
        val versionAfter = getStoreVersion(volumeName)
        val generationAfter = getStoreGeneration(volumeName)
        if ((versionBefore != null && versionAfter != versionBefore) ||
            (generationBefore != null && generationAfter != generationBefore)
        ) {
            incomplete = true
            errors.put(sourceError(null, "MediaStore changed while the full scan was running"))
        }
        return scanResult(if (incomplete) "incomplete" else "complete", null, tracks, errors)
    }

    private fun scanSafTree(sourceId: String, treeUri: Uri): JSObject {
        if (!DocumentsContract.isTreeUri(treeUri)) {
            return scanResult("unavailable", "SAF source is not a document-tree URI")
        }
        if (resolver.persistedUriPermissions.none { it.uri == treeUri && it.isReadPermission }) {
            return scanResult("permissionRevoked", "The persisted SAF read grant is missing")
        }

        val rootDocumentId = try {
            DocumentsContract.getTreeDocumentId(treeUri)
        } catch (error: Exception) {
            return scanResult("unavailable", "SAF tree URI cannot be opened: ${error.message}")
        }
        val queue = ArrayDeque<String>()
        val visitedDirectories = mutableSetOf<String>()
        val tracks = JSONArray()
        val errors = JSONArray()
        var incomplete = false
        queue.addLast(rootDocumentId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )

        while (queue.isNotEmpty()) {
            val parentId = queue.removeFirst()
            if (!visitedDirectories.add(parentId)) {
                incomplete = true
                errors.put(sourceError(parentId, "SAF provider returned a repeated directory identity"))
                continue
            }
            val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentId)
            try {
                val cursor = resolver.query(childrenUri, projection, null, null, null)
                if (cursor == null) {
                    incomplete = true
                    errors.put(sourceError(parentId, "SAF provider returned no child-document cursor"))
                    continue
                }
                cursor.use { rows ->
                    val idColumn = rows.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
                    val mimeColumn = rows.getColumnIndex(DocumentsContract.Document.COLUMN_MIME_TYPE)
                    val nameColumn = rows.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                    val sizeColumn = rows.getColumnIndex(DocumentsContract.Document.COLUMN_SIZE)
                    val modifiedColumn = rows.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
                    if (idColumn < 0 || mimeColumn < 0) {
                        incomplete = true
                        errors.put(sourceError(parentId, "SAF provider omitted document identity or MIME columns"))
                        continue
                    }
                    while (rows.moveToNext()) {
                        val documentId = rows.stringOrNull(idColumn)
                        val displayName = rows.stringOrNull(nameColumn)
                        val mimeType = rows.stringOrNull(mimeColumn)
                        if (documentId == null || mimeType == null) {
                            incomplete = true
                            errors.put(sourceError(documentId, "SAF row has no document ID or MIME type"))
                            continue
                        }
                        if (mimeType == DocumentsContract.Document.MIME_TYPE_DIR) {
                            queue.addLast(documentId)
                            continue
                        }
                        when (audioCandidate(mimeType, displayName)) {
                            AudioCandidate.NO -> continue
                            AudioCandidate.UNKNOWN -> {
                                incomplete = true
                                errors.put(sourceError(documentId, "SAF document type could not be identified"))
                                continue
                            }
                            AudioCandidate.YES -> Unit
                        }

                        val size = rows.longOrNull(sizeColumn)
                        if (size == null || size < 0L || displayName == null) {
                            incomplete = true
                            errors.put(sourceError(documentId, "SAF audio document has incomplete size or name metadata"))
                            continue
                        }
                        val documentUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId)
                        val authority = treeUri.authority ?: "unknown"
                        val metadata = metadataObject(title = titleFromDisplayName(displayName))
                        tracks.put(trackRecord(
                            sourceId = sourceId,
                            sourceItemId = "$authority:$documentId",
                            contentUri = documentUri.toString(),
                            sizeBytes = size,
                            modifiedAtUtcMs = rows.longOrNull(modifiedColumn)?.takeIf { it >= 0L },
                            metadata = metadata,
                        ))
                    }
                }
            } catch (error: SecurityException) {
                return scanResult(
                    "permissionRevoked",
                    "The SAF read grant was revoked during traversal: ${error.message}",
                    tracks,
                    errors,
                )
            } catch (error: Exception) {
                incomplete = true
                errors.put(sourceError(parentId, "Could not enumerate SAF directory: ${error.message}"))
            }
        }
        return scanResult(if (incomplete) "incomplete" else "complete", null, tracks, errors)
    }

    private fun readMetadata(uri: Uri): JSObject {
        val response = JSObject()
        val retriever = android.media.MediaMetadataRetriever()
        try {
            retriever.setDataSource(activity, uri)
            val metadata = metadataObject(
                title = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_TITLE),
                artist = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_ARTIST),
                album = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_ALBUM),
                albumArtist = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_ALBUMARTIST),
                trackNumber = parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_CD_TRACK_NUMBER)),
                discNumber = parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_DISC_NUMBER)),
                durationMs = parsePositiveLong(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_DURATION)),
                codec = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_MIMETYPE),
                bitrateBps = parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_BITRATE)),
                sampleRateHz = parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_SAMPLERATE)),
            )
            response.put("metadata", metadata)
            response.put("error", JSONObject.NULL)
        } catch (error: Exception) {
            response.put("metadata", JSONObject.NULL)
            response.put("error", error.message ?: "Could not read metadata from content URI")
        } finally {
            try {
                retriever.release()
            } catch (_: RuntimeException) {
                // The metadata error above is the useful failure to report.
            }
        }
        return response
    }

    private fun buildMediaStoreVolumes(): JSObject {
        val apiLevel = Build.VERSION.SDK_INT
        val names = if (apiLevel >= Build.VERSION_CODES.Q) {
            MediaStore.getExternalVolumeNames(activity).sorted()
        } else {
            listOf("external")
        }
        val volumes = JSONArray()
        for (name in names) {
            val item = JSONObject()
            item.put("volumeName", name)
            item.put("version", getStoreVersion(name) ?: JSONObject.NULL)
            item.put("generation", getStoreGeneration(name) ?: JSONObject.NULL)
            volumes.put(item)
        }
        val response = JSObject()
        response.put("apiLevel", apiLevel)
        response.put("volumes", volumes)
        return response
    }

    private fun getStoreVersion(volumeName: String): String? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) MediaStore.getVersion(activity, volumeName) else null

    private fun getStoreGeneration(volumeName: String): Long? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) MediaStore.getGeneration(activity, volumeName) else null

    private fun hasMediaReadPermission(): Boolean {
        val permission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.READ_MEDIA_AUDIO
        } else {
            Manifest.permission.READ_EXTERNAL_STORAGE
        }
        return activity.checkSelfPermission(permission) == android.content.pm.PackageManager.PERMISSION_GRANTED
    }

    private fun isLegacyExternalStorageMounted(): Boolean {
        val state = Environment.getExternalStorageState()
        return state == Environment.MEDIA_MOUNTED || state == Environment.MEDIA_MOUNTED_READ_ONLY
    }

    private fun async(invoke: Invoke, operation: () -> JSObject) {
        io.execute {
            try {
                invoke.resolve(operation())
            } catch (error: Exception) {
                invoke.reject(error.message ?: error.javaClass.simpleName)
            }
        }
    }

    private fun scanResult(
        kind: String,
        reason: String?,
        tracks: JSONArray = JSONArray(),
        errors: JSONArray = JSONArray(),
    ): JSObject {
        val state = JSONObject().put("kind", kind)
        if (reason != null) state.put("reason", reason)
        return JSObject().apply {
            put("state", state)
            put("tracks", tracks)
            put("errors", errors)
        }
    }

    private fun sourceError(itemId: String?, message: String): JSONObject = JSONObject().apply {
        put("sourceItemId", itemId ?: JSONObject.NULL)
        put("message", message)
    }

    private fun trackRecord(
        sourceId: String,
        sourceItemId: String,
        contentUri: String,
        sizeBytes: Long,
        modifiedAtUtcMs: Long?,
        metadata: JSONObject,
    ): JSONObject = JSONObject().apply {
        put("identity", JSONObject().apply {
            put("sourceId", sourceId)
            put("sourceItemId", sourceItemId)
            put("locatorKey", JSONObject.NULL)
        })
        put("locator", contentUri)
        put("fingerprint", JSONObject().apply {
            put("sizeBytes", sizeBytes)
            put("modifiedAtUtcMs", modifiedAtUtcMs ?: JSONObject.NULL)
        })
        put("metadata", metadata)
    }

    private fun metadataObject(
        title: String? = null,
        artist: String? = null,
        album: String? = null,
        albumArtist: String? = null,
        trackNumber: Int? = null,
        discNumber: Int? = null,
        durationMs: Long? = null,
        codec: String? = null,
        bitrateBps: Int? = null,
        sampleRateHz: Int? = null,
    ): JSONObject = JSONObject().apply {
        put("title", title ?: JSONObject.NULL)
        put("artist", artist ?: JSONObject.NULL)
        put("album", album ?: JSONObject.NULL)
        put("albumArtist", albumArtist ?: JSONObject.NULL)
        put("trackNumber", trackNumber ?: JSONObject.NULL)
        put("discNumber", discNumber ?: JSONObject.NULL)
        put("durationMs", durationMs ?: JSONObject.NULL)
        put("codec", codec ?: JSONObject.NULL)
        put("bitrateBps", bitrateBps ?: JSONObject.NULL)
        put("sampleRateHz", sampleRateHz ?: JSONObject.NULL)
    }

    private fun Cursor.stringOrNull(column: Int): String? =
        if (column < 0 || isNull(column)) null else getString(column)

    private fun Cursor.longOrNull(column: Int): Long? =
        if (column < 0 || isNull(column)) null else getLong(column)

    private fun titleFromDisplayName(displayName: String): String {
        val dot = displayName.lastIndexOf('.')
        return if (dot > 0) displayName.substring(0, dot) else displayName
    }

    private fun parsePositiveInt(value: String?): Int? = value?.toIntOrNull()?.takeIf { it >= 0 }
    private fun parsePositiveLong(value: String?): Long? = value?.toLongOrNull()?.takeIf { it >= 0L }

    companion object {
        internal fun splitTrackNumber(encoded: Long?): Pair<Int?, Int?> {
            if (encoded == null || encoded < 0L) return null to null
            val disc = (encoded / 1000L).takeIf { it > 0L && it <= Int.MAX_VALUE }?.toInt()
            val track = (encoded % 1000L).takeIf { it > 0L && it <= Int.MAX_VALUE }?.toInt()
            return disc to track
        }

        internal fun audioCandidate(mimeType: String?, displayName: String?): AudioCandidate {
            if (mimeType?.startsWith("audio/", ignoreCase = true) == true) return AudioCandidate.YES
            if (mimeType == null || mimeType == "application/octet-stream") {
                val extension = displayName?.substringAfterLast('.', "")?.lowercase()
                return if (extension in AUDIO_EXTENSIONS) AudioCandidate.YES else AudioCandidate.UNKNOWN
            }
            return AudioCandidate.NO
        }

        private val AUDIO_EXTENSIONS = setOf(
            "aac", "ac3", "aif", "aiff", "alac", "amr", "ape", "dff", "dsf", "flac",
            "m4a", "m4b", "mid", "midi", "mp3", "mp4", "oga", "ogg", "opus", "spx",
            "wav", "wave", "wma",
        )
    }
}

internal enum class AudioCandidate { YES, NO, UNKNOWN }
