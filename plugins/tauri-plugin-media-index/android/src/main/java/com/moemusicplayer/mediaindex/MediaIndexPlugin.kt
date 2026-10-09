package com.moemusicplayer.mediaindex

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ContentResolver
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
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
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors

@InvokeArg
class OfficialUrlArgs {
    var url: String? = null
}

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
    var maxBytes: Long? = null
    var treeUri: String? = null
}

@InvokeArg
class TreeUriArgs {
    var treeUri: String? = null
}

@InvokeArg
class CacheLeaseArgs {
    var token: String? = null
}

@InvokeArg
class PlaylistExportArgs {
    var suggestedName: String? = null
}

@InvokeArg
class WriteLeaseArgs {
    var token: String? = null
    var destinationUri: String? = null
}

@InvokeArg
class SafLyricsArgs {
    var treeUri: String? = null
    var audioUri: String? = null
    var maxBytes: Long? = null
}

@InvokeArg
class PlaylistResolveArgs {
    var treeUri: String? = null
    var playlistUri: String? = null
    var locator: String? = null
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
    private val appContext = activity.applicationContext
    private val io = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "moe-media-index").apply { isDaemon = true }
    }

    @Command
    fun openOfficialUrl(invoke: Invoke) {
        val value = invoke.parseArgs(OfficialUrlArgs::class.java).url
        val uri = value?.let(Uri::parse)
        // The Rust command owns the exact official-link whitelist; the native
        // bridge additionally rejects non-HTTPS schemes and URL credentials.
        if (uri?.scheme != "https" || uri.host.isNullOrBlank() || uri.userInfo != null) {
            invoke.reject("Only official HTTPS links are supported")
            return
        }
        activity.runOnUiThread {
            try {
                activity.startActivity(Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE))
                invoke.resolve(JSObject())
            } catch (error: Exception) {
                invoke.reject("Could not open browser: ${error.message}")
            }
        }
    }

    @Command
    fun mediaStoreVolumes(invoke: Invoke) {
        async(invoke) { buildMediaStoreVolumes() }
    }

    @Command
    fun ensurePlaybackServiceStarted(invoke: Invoke) {
        try {
            activity.startService(Intent(activity, PlaybackService::class.java))
            invoke.resolve(JSObject())
        } catch (error: Exception) {
            invoke.reject("Could not start Android playback service: ${error.message}")
        }
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
    fun cacheContentUri(invoke: Invoke) {
        val args = invoke.parseArgs(ContentUriArgs::class.java)
        val rawUri = args.contentUri
        val maxBytes = args.maxBytes
        if (rawUri.isNullOrBlank() || maxBytes == null || maxBytes <= 0L || maxBytes > MAX_CONTENT_LEASE_BYTES) {
            invoke.reject("contentUri and maxBytes (1..$MAX_CONTENT_LEASE_BYTES) are required")
            return
        }
        async(invoke) { cacheContentUri(Uri.parse(rawUri), maxBytes) }
    }

    @Command
    fun cacheArtworkBytes(invoke: Invoke) {
        val args = invoke.parseArgs(ContentUriArgs::class.java)
        val rawUri = args.contentUri
        val maxBytes = args.maxBytes ?: MAX_ARTWORK_BYTES
        if (rawUri.isNullOrBlank() || maxBytes <= 0L || maxBytes > MAX_ARTWORK_BYTES) {
            invoke.reject("contentUri and maxBytes (1..$MAX_ARTWORK_BYTES) are required")
            return
        }
        async(invoke) {
            cacheArtworkBytes(Uri.parse(rawUri), args.treeUri?.let { Uri.parse(it) }, maxBytes)
        }
    }

    @Command
    fun createCacheLease(invoke: Invoke) {
        async(invoke) { createCacheLease() }
    }

    @Command
    fun releaseCacheLease(invoke: Invoke) {
        val args = invoke.parseArgs(CacheLeaseArgs::class.java)
        val token = args.token
        if (token.isNullOrBlank()) {
            invoke.reject("token is required")
            return
        }
        releaseCacheLease(token)
        invoke.resolve(JSObject())
    }

    @Command
    fun writeContentCacheLease(invoke: Invoke) {
        val args = invoke.parseArgs(WriteLeaseArgs::class.java)
        val token = args.token
        val destination = args.destinationUri
        if (token.isNullOrBlank() || destination.isNullOrBlank()) {
            invoke.reject("token and destinationUri are required")
            return
        }
        async(invoke) {
            writeContentCacheLease(token, Uri.parse(destination))
            JSObject()
        }
    }

    @Command
    fun pickPlaylistImport(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PREFIX_URI_PERMISSION,
            )
        }
        startActivityForResult(invoke, intent, "onPlaylistImportTreePicked")
    }

    @ActivityCallback
    private fun onPlaylistImportTreePicked(invoke: Invoke, result: ActivityResult) {
        val treeUri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || treeUri == null) {
            invoke.resolve(JSObject().put("playlistUri", JSONObject.NULL).put("treeUri", JSONObject.NULL).put("displayName", JSONObject.NULL))
            return
        }
        val readGrant = result.data?.flags?.and(Intent.FLAG_GRANT_READ_URI_PERMISSION) ?: 0
        if (readGrant == 0) {
            invoke.reject("The document provider did not grant playlist-tree read access")
            return
        }
        try {
            resolver.takePersistableUriPermission(treeUri, readGrant)
            io.execute {
                try {
                    val playlists = listPlaylistDocuments(treeUri)
                    activity.runOnUiThread {
                        when (playlists.size) {
                            0 -> {
                                releasePersistedTreeReadGrant(treeUri)
                                invoke.reject("No M3U or M3U8 playlist was found in the selected folder")
                            }
                            1 -> resolvePlaylistPick(invoke, treeUri, playlists.single())
                            else -> {
                                val labels = playlists.map { (uri, name) ->
                                    val documentId = DocumentsContract.getDocumentId(uri)
                                    "$name — ${documentId.substringAfter(':', documentId)}"
                                }.toTypedArray()
                                AlertDialog.Builder(activity)
                                    .setTitle("Choose a playlist")
                                    .setItems(labels) { _, index -> resolvePlaylistPick(invoke, treeUri, playlists[index]) }
                                    .setOnCancelListener {
                                        releasePersistedTreeReadGrant(treeUri)
                                        invoke.resolve(JSObject().put("playlistUri", JSONObject.NULL).put("treeUri", JSONObject.NULL).put("displayName", JSONObject.NULL))
                                    }
                                    .show()
                            }
                        }
                    }
                } catch (error: Exception) {
                    activity.runOnUiThread {
                        invoke.reject(error.message ?: "Could not enumerate playlists in the selected folder")
                    }
                }
            }
        } catch (error: SecurityException) {
            invoke.reject("Could not preserve playlist-tree access: ${error.message}")
        }
    }

    @Command
    fun resolveSafPlaylistEntry(invoke: Invoke) {
        val args = invoke.parseArgs(PlaylistResolveArgs::class.java)
        val tree = args.treeUri
        val playlist = args.playlistUri
        val locator = args.locator
        if (tree.isNullOrBlank() || playlist.isNullOrBlank() || locator.isNullOrBlank()) {
            invoke.reject("treeUri, playlistUri and locator are required")
            return
        }
        async(invoke) {
            val resolved = resolveSafPlaylistEntry(Uri.parse(tree), Uri.parse(playlist), locator)
            if (resolved == null) {
                JSObject().put("contentUri", JSONObject.NULL)
                    .put("sizeBytes", JSONObject.NULL)
                    .put("modifiedAtUtcMs", JSONObject.NULL)
            } else {
                val (sizeBytes, modifiedAtUtcMs) = queryDocumentFingerprint(resolved)
                JSObject().put("contentUri", resolved.toString())
                    .put("sizeBytes", sizeBytes ?: JSONObject.NULL)
                    .put("modifiedAtUtcMs", modifiedAtUtcMs ?: JSONObject.NULL)
            }
        }
    }

    @Command
    fun pickPlaylistExport(invoke: Invoke) {
        val args = invoke.parseArgs(PlaylistExportArgs::class.java)
        val suggestedName = args.suggestedName?.takeIf(String::isNotBlank) ?: "playlist.m3u8"
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "audio/x-mpegurl"
            putExtra(Intent.EXTRA_TITLE, suggestedName)
            addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        }
        startActivityForResult(invoke, intent, "onPlaylistExportPicked")
    }

    @ActivityCallback
    private fun onPlaylistExportPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject().put("uri", JSONObject.NULL))
            return
        }
        invoke.resolve(JSObject().put("uri", uri.toString()))
    }

    @Command
    fun readSafLyricSibling(invoke: Invoke) {
        val args = invoke.parseArgs(SafLyricsArgs::class.java)
        val tree = args.treeUri
        val audio = args.audioUri
        val maxBytes = args.maxBytes ?: MAX_LYRICS_BYTES
        if (tree.isNullOrBlank() || audio.isNullOrBlank() || maxBytes <= 0L || maxBytes > MAX_LYRICS_BYTES) {
            invoke.reject("treeUri, audioUri and maxBytes (1..$MAX_LYRICS_BYTES) are required")
            return
        }
        async(invoke) { readSafLyricSibling(Uri.parse(tree), Uri.parse(audio), maxBytes) }
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
    fun probeContentUri(invoke: Invoke) {
        val args = invoke.parseArgs(ContentUriArgs::class.java)
        val rawUri = args.contentUri
        if (rawUri.isNullOrBlank()) {
            invoke.reject("contentUri is required")
            return
        }
        async(invoke) {
            val uri = Uri.parse(rawUri)
            if (uri.scheme != ContentResolver.SCHEME_CONTENT || uri.authority.isNullOrBlank()) {
                return@async contentUriProbe(null, null, null)
            }
            val descriptor = try {
                resolver.openFileDescriptor(uri, "r")
            } catch (_: SecurityException) {
                null
            } catch (_: java.io.FileNotFoundException) {
                null
            }
            if (descriptor == null) {
                contentUriProbe(null, null, null)
            } else {
                descriptor.use { file ->
                    val sizeBytes = file.statSize.takeIf { it >= 0L }
                        ?: queryLong(uri, OpenableColumns.SIZE)?.takeIf { it >= 0L }
                    val modifiedAtUtcMs = queryLong(uri, DocumentsContract.Document.COLUMN_LAST_MODIFIED)
                        ?.takeIf { it >= 0L }
                        ?: queryLong(uri, MediaStore.MediaColumns.DATE_MODIFIED)?.let { seconds ->
                            if (seconds >= 0L && seconds <= Long.MAX_VALUE / 1000L) seconds * 1000L else null
                        }
                    contentUriProbe(rawUri, sizeBytes, modifiedAtUtcMs)
                }
            }
        }
    }

    private fun contentUriProbe(uri: String?, sizeBytes: Long?, modifiedAtUtcMs: Long?): JSObject =
        JSObject().put("contentUri", uri ?: JSONObject.NULL)
            .put("sizeBytes", sizeBytes ?: JSONObject.NULL)
            .put("modifiedAtUtcMs", modifiedAtUtcMs ?: JSONObject.NULL)

    private fun queryLong(uri: Uri, column: String): Long? = try {
        resolver.query(uri, arrayOf(column), null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) null else {
                val index = cursor.getColumnIndex(column)
                if (index < 0 || cursor.isNull(index)) null else cursor.getLong(index)
            }
        }
    } catch (_: Exception) {
        null
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
            )
            val cursor = resolver.query(collection, projection.toTypedArray(), null, null, null)
                ?: return scanResult("unavailable", "MediaStore returned no cursor for '$volumeName'")
            cursor.use { rows ->
                val idColumn = rows.getColumnIndex(MediaStore.Audio.Media._ID)
                val sizeColumn = rows.getColumnIndex(MediaStore.Audio.Media.SIZE)
                val modifiedColumn = rows.getColumnIndex(MediaStore.Audio.Media.DATE_MODIFIED)
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
                        val modifiedSeconds = rows.longOrNull(modifiedColumn)
                        val modifiedMillis = modifiedSeconds?.let { seconds ->
                            if (seconds >= 0 && seconds <= Long.MAX_VALUE / 1000L) seconds * 1000L else null
                        }
                        val uri = Uri.withAppendedPath(collection, id.toString())
                        tracks.put(trackRecord(
                            sourceId = sourceId,
                            sourceItemId = "$volumeName:$id",
                            contentUri = uri.toString(),
                            sizeBytes = size,
                            modifiedAtUtcMs = modifiedMillis,
                            metadata = null,
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
                tracks.put(trackRecord(
                            sourceId = sourceId,
                            sourceItemId = "$authority:$documentId",
                            contentUri = documentUri.toString(),
                            sizeBytes = size,
                            modifiedAtUtcMs = rows.longOrNull(modifiedColumn)?.takeIf { it >= 0L },
                    metadata = null,
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
                bitDepth = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_BITS_PER_SAMPLE))
                } else {
                    null
                },
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

    private fun cacheContentUri(uri: Uri, maxBytes: Long): JSObject {
        val input = resolver.openInputStream(uri)
            ?: throw IOException("Could not open authorized content URI")
        return input.use { stream -> cacheStream(stream, maxBytes, resolver.getType(uri)) }
    }

    private fun cacheArtworkBytes(uri: Uri, treeUri: Uri?, maxBytes: Long): JSObject {
        if (resolver.getType(uri)?.startsWith("image/", ignoreCase = true) == true) {
            val directInput = resolver.openInputStream(uri)
                ?: throw IOException("Could not open authorized artwork document")
            return directInput.use { stream -> cacheStream(stream, maxBytes, resolver.getType(uri)) }
        }
        val retriever = android.media.MediaMetadataRetriever()
        val embedded = try {
            retriever.setDataSource(appContext, uri)
            retriever.embeddedPicture
        } finally {
            try {
                retriever.release()
            } catch (_: RuntimeException) {
                // Keep the extraction result/error as the primary outcome.
            }
        }
        if (embedded != null) {
            if (embedded.size.toLong() > maxBytes) throw IOException("ARTWORK_TOO_LARGE")
            return cacheBytes(embedded, artworkMimeType(embedded))
        }
        val sibling = treeUri?.let { findArtworkSibling(it, uri) }
            ?: throw IOException("No embedded or SAF sibling artwork was found")
        val input = resolver.openInputStream(sibling)
            ?: throw IOException("Could not open authorized artwork document")
        return input.use { stream -> cacheStream(stream, maxBytes, resolver.getType(sibling)) }
    }

    private fun createCacheLease(): JSObject {
        cleanupExpiredLeases()
        reserveLeaseBytes(0L)
        val file = newLeaseFile()
        FileOutputStream(file).use { }
        return leaseResponse(file, null)
    }

    private fun cacheStream(input: java.io.InputStream, maxBytes: Long, mimeType: String?): JSObject {
        cleanupExpiredLeases()
        val existingBytes = currentLeaseBytes()
        val file = newLeaseFile()
        try {
            FileOutputStream(file).use { output ->
                val buffer = ByteArray(COPY_BUFFER_SIZE)
                var total = 0L
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    total += count
                    if (total > maxBytes) throw IOException("CONTENT_TOO_LARGE")
                    if (existingBytes + total > MAX_TOTAL_LEASE_BYTES) throw IOException("CACHE_LEASE_LIMIT")
                    output.write(buffer, 0, count)
                }
                output.fd.sync()
            }
            return leaseResponse(file, mimeType)
        } catch (error: Exception) {
            file.delete()
            throw error
        }
    }

    private fun cacheBytes(bytes: ByteArray, mimeType: String?): JSObject {
        cleanupExpiredLeases()
        reserveLeaseBytes(bytes.size.toLong())
        val file = newLeaseFile()
        try {
            FileOutputStream(file).use { output ->
                output.write(bytes)
                output.fd.sync()
            }
            return leaseResponse(file, mimeType)
        } catch (error: Exception) {
            file.delete()
            throw error
        }
    }

    private fun newLeaseFile(): File {
        val directory = leaseDirectory()
        if (!directory.exists() && !directory.mkdirs()) throw IOException("Could not create media cache lease directory")
        repeat(3) {
            val token = UUID.randomUUID().toString()
            val file = File(directory, "$token.bin")
            if (file.createNewFile()) return file
        }
        throw IOException("Could not allocate a unique media cache lease")
    }

    private fun leaseResponse(file: File, mimeType: String?): JSObject {
        if (file.length() > MAX_TOTAL_LEASE_BYTES) {
            file.delete()
            throw IOException("CONTENT_TOO_LARGE")
        }
        return JSObject().apply {
            put("token", file.name.removeSuffix(".bin"))
            put("path", file.absolutePath)
            put("sizeBytes", file.length())
            put("mimeType", mimeType ?: JSONObject.NULL)
        }
    }

    private fun releaseCacheLease(token: String) {
        if (!TOKEN_PATTERN.matches(token)) return
        File(leaseDirectory(), "$token.bin").delete()
    }

    private fun writeContentCacheLease(token: String, destination: Uri) {
        if (!TOKEN_PATTERN.matches(token)) throw IOException("Invalid cache lease token")
        val file = File(leaseDirectory(), "$token.bin")
        if (!file.isFile || file.length() > MAX_PLAYLIST_BYTES) throw IOException("Playlist cache lease is missing or oversized")
        val output = resolver.openOutputStream(destination, "wt")
            ?: throw IOException("Could not open selected playlist destination")
        file.inputStream().use { input -> output.use { input.copyTo(it, COPY_BUFFER_SIZE) } }
    }

    private fun leaseDirectory(): File = File(appContext.cacheDir, "media-index/leases")

    private fun cleanupExpiredLeases() {
        val directory = leaseDirectory()
        val files = directory.listFiles()?.filter(File::isFile).orEmpty()
        val staleBefore = System.currentTimeMillis() - LEASE_TTL_MS
        files.filter { it.lastModified() < staleBefore }.forEach(File::delete)
    }

    private fun currentLeaseBytes(): Long =
        leaseDirectory().listFiles()?.filter(File::isFile)?.sumOf(File::length) ?: 0L

    private fun reserveLeaseBytes(expectedBytes: Long) {
        val currentBytes = currentLeaseBytes()
        if (expectedBytes < 0L || currentBytes > MAX_TOTAL_LEASE_BYTES - expectedBytes) {
            throw IOException("CACHE_LEASE_LIMIT")
        }
    }

    private fun readSafLyricSibling(treeUri: Uri, audioUri: Uri, maxBytes: Long): JSObject {
        val uri = findSafSibling(treeUri, audioUri, ".lrc")
            ?: return JSObject().put("text", JSONObject.NULL)
        val input = resolver.openInputStream(uri) ?: throw IOException("Could not open authorized lyric sidecar")
        val bytes = input.use { readBounded(it, maxBytes) }
        val text = bytes.toString(Charsets.UTF_8)
        if (!text.toByteArray(Charsets.UTF_8).contentEquals(bytes)) {
            throw IOException("Lyric sidecar is not valid UTF-8")
        }
        return JSObject().put("text", text)
    }

    private fun listPlaylistDocuments(treeUri: Uri): List<Pair<Uri, String>> {
        require(DocumentsContract.isTreeUri(treeUri)) { "Selected location is not a SAF document tree" }
        val rootId = DocumentsContract.getTreeDocumentId(treeUri)
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, rootId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        )
        val playlists = mutableListOf<Pair<Uri, String>>()
        resolver.query(children, projection, null, null, null)?.use { cursor ->
            val idColumn = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val nameColumn = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            require(idColumn >= 0 && nameColumn >= 0) { "Document provider omitted playlist entry fields" }
            while (cursor.moveToNext()) {
                val documentId = cursor.getString(idColumn)
                val name = cursor.getString(nameColumn)
                if (name.substringAfterLast('.', "").lowercase() in setOf("m3u", "m3u8")) {
                    playlists.add(DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId) to name)
                    require(playlists.size <= MAX_PLAYLIST_CANDIDATES) {
                        "Selected folder contains more than $MAX_PLAYLIST_CANDIDATES playlists"
                    }
                }
            }
        } ?: throw IOException("Document provider returned no selected folder entries")
        return playlists
    }

    private fun resolvePlaylistPick(invoke: Invoke, treeUri: Uri, playlist: Pair<Uri, String>) {
        invoke.resolve(JSObject().apply {
            put("playlistUri", playlist.first.toString())
            put("treeUri", treeUri.toString())
            put("displayName", playlist.second)
        })
    }

    private fun resolveSafPlaylistEntry(treeUri: Uri, playlistUri: Uri, locator: String): Uri? {
        if (!DocumentsContract.isTreeUri(treeUri) || !DocumentsContract.isTreeUri(playlistUri)) return null
        if (treeUri.authority != playlistUri.authority || !hasPersistedTreeReadGrant(treeUri)) {
            throw SecurityException("Playlist entry is outside the persisted SAF grant")
        }
        val rootId = DocumentsContract.getTreeDocumentId(treeUri)
        val playlistId = DocumentsContract.getDocumentId(playlistUri)
        if (!playlistId.startsWith("$rootId/")) {
            throw SecurityException("Selected playlist is outside its SAF root")
        }
        val playlistRelative = playlistId.removePrefix("$rootId/")
        if (playlistRelative.contains('/')) {
            throw SecurityException("Playlist picker returned a document outside the selected folder")
        }
        val path = try {
            normalizeSafPlaylistLocator(locator)
        } catch (error: IllegalArgumentException) {
            throw SecurityException("M3U relative entry escapes the selected SAF folder", error)
        } ?: return null
        var parentId = rootId
        var resolved: Uri? = null
        for (component in path) {
            resolved = findChildDocument(treeUri, parentId, component) ?: return null
            parentId = DocumentsContract.getDocumentId(resolved)
        }
        return resolved
    }

    private fun hasPersistedTreeReadGrant(uri: Uri): Boolean = resolver.persistedUriPermissions.any {
        it.isReadPermission && it.uri == uri
    }

    private fun releasePersistedTreeReadGrant(uri: Uri) {
        try {
            resolver.releasePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } catch (_: SecurityException) {
            // A revoked or already-released picker result needs no follow-up.
        }
    }

    private fun queryDocumentFingerprint(uri: Uri): Pair<Long?, Long?> {
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
        )
        resolver.query(uri, projection, null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) return null to null
            val sizeColumn = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_SIZE)
            val modifiedColumn = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
            val size = if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) cursor.getLong(sizeColumn).takeIf { it >= 0L } else null
            val modified = if (modifiedColumn >= 0 && !cursor.isNull(modifiedColumn)) cursor.getLong(modifiedColumn).takeIf { it >= 0L } else null
            return size to modified
        }
        return null to null
    }

    private fun findArtworkSibling(treeUri: Uri, audioUri: Uri): Uri? {
        for (name in listOf("cover.jpg", "folder.jpg", "cover.png", "folder.png")) {
            findSafSiblingByName(treeUri, audioUri, name)?.let { return it }
        }
        return null
    }

    private fun findSafSibling(treeUri: Uri, audioUri: Uri, extension: String): Uri? {
        if (!DocumentsContract.isTreeUri(treeUri) || !DocumentsContract.isTreeUri(audioUri)) return null
        return try {
            val treeDocumentId = DocumentsContract.getTreeDocumentId(treeUri)
            if (DocumentsContract.getTreeDocumentId(audioUri) != treeDocumentId) return null
            val audioDocumentId = DocumentsContract.getDocumentId(audioUri)
            val audioName = queryDisplayName(audioUri) ?: return null
            val stem = audioName.substringBeforeLast('.', audioName)
            val targetName = "$stem$extension"
            findChildDocument(treeUri, audioDocumentId.substringBeforeLast('/', ""), targetName)
        } catch (_: Exception) {
            null
        }
    }

    private fun findSafSiblingByName(treeUri: Uri, audioUri: Uri, name: String): Uri? {
        if (!DocumentsContract.isTreeUri(treeUri) || !DocumentsContract.isTreeUri(audioUri)) return null
        return try {
            val treeDocumentId = DocumentsContract.getTreeDocumentId(treeUri)
            if (DocumentsContract.getTreeDocumentId(audioUri) != treeDocumentId) return null
            val audioDocumentId = DocumentsContract.getDocumentId(audioUri)
            findChildDocument(treeUri, audioDocumentId.substringBeforeLast('/', ""), name)
        } catch (_: Exception) {
            null
        }
    }

    private fun findChildDocument(treeUri: Uri, parentDocumentId: String, displayName: String): Uri? {
        if (parentDocumentId.isBlank()) return null
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocumentId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        )
        resolver.query(children, projection, null, null, null)?.use { cursor ->
            val idColumn = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DOCUMENT_ID)
            val nameColumn = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
            if (idColumn < 0 || nameColumn < 0) return null
            while (cursor.moveToNext()) {
                if (cursor.getString(nameColumn) == displayName) {
                    return DocumentsContract.buildDocumentUriUsingTree(treeUri, cursor.getString(idColumn))
                }
            }
        }
        return null
    }

    private fun queryDisplayName(uri: Uri): String? {
        val projection = arrayOf(android.provider.OpenableColumns.DISPLAY_NAME)
        resolver.query(uri, projection, null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val index = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                if (index >= 0) return cursor.getString(index)
            }
        }
        return null
    }

    private fun readBounded(input: java.io.InputStream, maxBytes: Long): ByteArray {
        val output = java.io.ByteArrayOutputStream(minOf(maxBytes, 8192L).toInt())
        val buffer = ByteArray(COPY_BUFFER_SIZE)
        var total = 0L
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            total += count
            if (total > maxBytes) throw IOException("CONTENT_TOO_LARGE")
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }

    private fun artworkMimeType(bytes: ByteArray): String? = when {
        bytes.size >= 3 && bytes[0] == 0xff.toByte() && bytes[1] == 0xd8.toByte() -> "image/jpeg"
        bytes.size >= 8 && bytes.copyOfRange(0, 8).contentEquals(byteArrayOf(137.toByte(), 80, 78, 71, 13, 10, 26, 10)) -> "image/png"
        bytes.size >= 12 && bytes.copyOfRange(0, 4).contentEquals("RIFF".toByteArray()) -> "image/webp"
        else -> null
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
        metadata: JSONObject?,
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
        put("metadata", metadata ?: JSONObject.NULL)
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
        bitDepth: Int? = null,
    ): JSONObject = JSONObject().apply {
        put("title", title?.takeIf { it.isNotBlank() } ?: JSONObject.NULL)
        put("artist", artist ?: JSONObject.NULL)
        put("album", album ?: JSONObject.NULL)
        put("albumArtist", albumArtist ?: JSONObject.NULL)
        put("trackNumber", trackNumber ?: JSONObject.NULL)
        put("discNumber", discNumber ?: JSONObject.NULL)
        put("durationMs", durationMs ?: JSONObject.NULL)
        put("codec", codec ?: JSONObject.NULL)
        put("bitrateBps", bitrateBps ?: JSONObject.NULL)
        put("sampleRateHz", sampleRateHz ?: JSONObject.NULL)
        put("bitDepth", bitDepth ?: JSONObject.NULL)
    }

    private fun Cursor.stringOrNull(column: Int): String? =
        if (column < 0 || isNull(column)) null else getString(column)

    private fun Cursor.longOrNull(column: Int): Long? =
        if (column < 0 || isNull(column)) null else getLong(column)

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

        private const val MAX_CONTENT_LEASE_BYTES = 512L * 1024 * 1024
        private const val MAX_ARTWORK_BYTES = 32L * 1024 * 1024
        private const val MAX_PLAYLIST_BYTES = 32L * 1024 * 1024
        private const val MAX_LYRICS_BYTES = 2L * 1024 * 1024
        private const val MAX_TOTAL_LEASE_BYTES = 512L * 1024 * 1024
        private const val LEASE_TTL_MS = 15 * 60 * 1000L
        private const val COPY_BUFFER_SIZE = 64 * 1024
        private const val MAX_PLAYLIST_CANDIDATES = 256
        private val TOKEN_PATTERN = Regex("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
    }
}

internal enum class AudioCandidate { YES, NO, UNKNOWN }
