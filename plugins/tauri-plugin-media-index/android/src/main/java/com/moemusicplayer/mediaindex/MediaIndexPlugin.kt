package com.moemusicplayer.mediaindex

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.app.WallpaperManager
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.database.Cursor
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.Drawable
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.FileProvider
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
class OpenDocumentArgs {
    var mimeTypes: Array<String>? = null
}

@InvokeArg
class CreateDocumentArgs {
    var suggestedName: String? = null
    var mimeType: String? = null
}

@InvokeArg
class ContentUriArgs {
    var contentUri: String? = null
    var maxBytes: Long? = null
    var treeUri: String? = null
}

@InvokeArg
class WallpaperBackdropArgs {
    var enabled: Boolean? = null
    var blurPx: Int? = null
}

@InvokeArg
class FilePathArgs {
    var path: String? = null
}

@InvokeArg
class UriListArgs {
    var uris: List<String>? = null
}

@InvokeArg
class CacheLeaseArgs {
    var token: String? = null
}

@InvokeArg
class WriteLeaseArgs {
    var token: String? = null
    var destinationUri: String? = null
}

@InvokeArg
class CopyFileToUriArgs {
    var sourcePath: String? = null
    var destinationUri: String? = null
}

@InvokeArg
class CopyUriToFileArgs {
    var sourceUri: String? = null
    var destinationPath: String? = null
    var maxBytes: Long = 0
}

@TauriPlugin(
    permissions = [
        // Android 10 and older only (maxSdkVersion 29). Android 11+ uses All files access (ADR 0014).
        Permission(
            strings = [Manifest.permission.READ_EXTERNAL_STORAGE, Manifest.permission.WRITE_EXTERNAL_STORAGE],
            alias = "legacyStorage",
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

    /**
     * Whether the clipboard currently holds an image. Only the clip description is read, so this
     * neither copies data nor triggers Android 12+'s "pasted from" notice. Android WebView rejects
     * navigator.clipboard.read() for images, so the cover paste goes through ClipboardManager.
     * Android 10+ only exposes the clip while this app has window focus, so `readable` is false when
     * the description could not be read (the UI keeps 貼上 enabled and reads on click).
     */
    @Command
    fun clipboardImageStatus(invoke: Invoke) {
        activity.runOnUiThread {
            val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            val description = try { clipboard?.primaryClipDescription } catch (_: Exception) { null }
            val types = description?.let { value -> (0 until value.mimeTypeCount).map(value::getMimeType) }
            val hasImage = description != null && (description.hasMimeType("image/*") ||
                description.hasMimeType(ClipDescription.MIMETYPE_TEXT_URILIST))
            Log.i(TAG, "clipboard status: focus=${activity.hasWindowFocus()} hasClip=${clipboard?.hasPrimaryClip()} types=$types hasImage=$hasImage")
            invoke.resolve(JSObject().put("hasImage", hasImage).put("readable", description != null))
        }
    }

    /**
     * Copy the first image item of the clipboard into the app cache and return its path (the
     * clip's temporary URI grant ends with the clip). Accepts content:// URIs (galleries, browsers,
     * this app's own copy) and file:// URIs, from the item URI or a text/uri-list item; the bytes
     * must sniff as JPEG / PNG / WebP / GIF / BMP. `path` is null with `reason` `noClip` (nothing
     * readable, e.g. the app is not focused) or `noImage`.
     */
    @Command
    fun readClipboardImage(invoke: Invoke) {
        activity.runOnUiThread {
            val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            val clip = try { clipboard?.primaryClip } catch (error: Exception) {
                Log.w(TAG, "clipboard read failed", error)
                null
            }
            if (clip == null) {
                Log.i(TAG, "clipboard read: no clip (focus=${activity.hasWindowFocus()} hasClip=${clipboard?.hasPrimaryClip()})")
                invoke.resolve(JSObject().put("path", JSONObject.NULL).put("reason", "noClip"))
                return@runOnUiThread
            }
            val types = (0 until clip.description.mimeTypeCount).map(clip.description::getMimeType)
            val uris = (0 until clip.itemCount).flatMap { index ->
                val item = clip.getItemAt(index)
                listOfNotNull(item.uri) + (item.text?.toString().orEmpty().lineSequence()
                    .map(String::trim)
                    .filter { it.startsWith("content://") || it.startsWith("file://") }
                    .map(Uri::parse)
                    .toList())
            }
            Log.i(TAG, "clipboard read: items=${clip.itemCount} types=$types uris=${uris.map { it.scheme + "://" + it.authority }}")
            io.execute {
                val directory = File(appContext.cacheDir, "clipboard").apply { mkdirs() }
                directory.listFiles()?.forEach { it.delete() }
                for (uri in uris) {
                    try {
                        val bytes = openClipboardUri(uri)?.use { readBounded(it, MAX_ARTWORK_BYTES) } ?: continue
                        val mimeType = imageMimeType(bytes)
                        if (mimeType == null) {
                            Log.i(TAG, "clipboard item is not an image: ${uri.scheme}://${uri.authority} type=${typeOf(uri)}")
                            continue
                        }
                        val target = File(directory, "clipboard-${UUID.randomUUID()}")
                        FileOutputStream(target).use { it.write(bytes) }
                        invoke.resolve(JSObject().put("path", target.absolutePath).put("mimeType", mimeType))
                        return@execute
                    } catch (error: Exception) {
                        Log.w(TAG, "clipboard item unreadable: ${uri.scheme}://${uri.authority}: ${error.message}")
                    }
                }
                invoke.resolve(JSObject().put("path", JSONObject.NULL).put("reason", "noImage"))
            }
        }
    }

    private fun openClipboardUri(uri: Uri): java.io.InputStream? = when (uri.scheme) {
        ContentResolver.SCHEME_CONTENT -> resolver.openInputStream(uri)
        ContentResolver.SCHEME_FILE -> uri.path?.let { File(it).inputStream() }
        else -> null
    }

    private fun typeOf(uri: Uri): String? = try { resolver.getType(uri) } catch (_: Exception) { null }

    private fun imageMimeType(bytes: ByteArray): String? = artworkMimeType(bytes) ?: when {
        bytes.size >= 6 && String(bytes, 0, 6, Charsets.US_ASCII).let { it == "GIF87a" || it == "GIF89a" } -> "image/gif"
        bytes.size >= 2 && bytes[0] == 'B'.code.toByte() && bytes[1] == 'M'.code.toByte() -> "image/bmp"
        else -> null
    }

    /** Private cache folder whose files can be put on the clipboard (FileProvider `clipboard-share/`). */
    @Command
    fun clipboardShareDirectory(invoke: Invoke) {
        val directory = File(appContext.cacheDir, CLIPBOARD_SHARE_DIRECTORY).apply { mkdirs() }
        invoke.resolve(JSObject().put("path", directory.absolutePath))
    }

    /**
     * Put an image file from the clipboard-share cache on the system clipboard as a FileProvider
     * content URI with its exact image MIME type, so this app and other apps can paste it (the
     * clipboard service grants readers temporary access). Older shared files are removed.
     */
    @Command
    fun writeClipboardImage(invoke: Invoke) {
        val path = invoke.parseArgs(FilePathArgs::class.java).path
        val file = path?.let(::File)
        val share = File(appContext.cacheDir, CLIPBOARD_SHARE_DIRECTORY).canonicalFile
        if (file == null || !file.isFile || !file.canonicalPath.startsWith(share.path + File.separator)) {
            invoke.reject("Clipboard image is missing")
            return
        }
        val head = file.inputStream().use { input -> ByteArray(16).let { buffer -> buffer.copyOf(input.read(buffer).coerceAtLeast(0)) } }
        val mimeType = imageMimeType(head)
        if (mimeType == null) {
            invoke.reject("Not an image")
            return
        }
        share.listFiles()?.filter { it.canonicalPath != file.canonicalPath }?.forEach { it.delete() }
        activity.runOnUiThread {
            try {
                val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.moe.updates", file)
                val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                clipboard.setPrimaryClip(ClipData(ClipDescription("cover", arrayOf(mimeType)), ClipData.Item(uri)))
                Log.i(TAG, "clipboard write: $mimeType ${file.length()} bytes")
                invoke.resolve(JSObject().put("mimeType", mimeType))
            } catch (error: Exception) {
                invoke.reject("Could not copy the image: ${error.message}")
            }
        }
    }

    /**
     * Best-effort "open containing folder" in the system Files app (DocumentsUI) via the folder's
     * ExternalStorageProvider document URI. No URI grant flag: this app holds no permission on that
     * provider, so FLAG_GRANT_READ_URI_PERMISSION makes startActivity throw SecurityException
     * (DocumentsUI has its own access). Every form is tried with startActivity + catch rather than a
     * resolveActivity pre-check (package visibility hides results on Android 11+).
     */
    @Command
    fun openFolderInFileManager(invoke: Invoke) {
        val path = invoke.parseArgs(FilePathArgs::class.java).path
        val documentId = path?.let(::externalStorageDocumentIdFor)
        if (documentId == null) {
            invoke.reject("This folder is not on device storage")
            return
        }
        val uri = DocumentsContract.buildDocumentUri(EXTERNAL_STORAGE_AUTHORITY, documentId)
        val view = { type: String? ->
            Intent(Intent.ACTION_VIEW).apply { if (type == null) data = uri else setDataAndType(uri, type) }
        }
        val candidates = listOf(
            view(DocumentsContract.Document.MIME_TYPE_DIR),
            view(DocumentsContract.Document.MIME_TYPE_DIR).setPackage("com.google.android.documentsui"),
            view(DocumentsContract.Document.MIME_TYPE_DIR).setPackage("com.android.documentsui"),
            view("resource/folder"),
            view(null),
        ).map { intent -> intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) }
        activity.runOnUiThread {
            for ((index, intent) in candidates.withIndex()) {
                try {
                    activity.startActivity(intent)
                    Log.i(TAG, "open folder: started form $index")
                    invoke.resolve(JSObject().put("opened", true))
                    return@runOnUiThread
                } catch (error: Exception) {
                    Log.w(TAG, "open folder form $index failed: ${error.javaClass.simpleName}: ${error.message}")
                }
            }
            invoke.resolve(JSObject().put("opened", false))
        }
    }

    /** Private cache folder for downloaded update APKs (shared with the FileProvider paths). */
    @Command
    fun updateDownloadDirectory(invoke: Invoke) {
        val directory = File(appContext.cacheDir, "updates").apply { mkdirs() }
        invoke.resolve(JSObject().put("path", directory.absolutePath))
    }

    /**
     * Hand a verified update APK to the system package installer. Without "install unknown apps"
     * for this app (Android 8+), open that settings page instead and report `permission`; the user
     * allows it there and retries. The system installer always asks the user to confirm.
     */
    @Command
    fun installApk(invoke: Invoke) {
        val path = invoke.parseArgs(FilePathArgs::class.java).path
        val file = path?.let(::File)
        val updates = File(appContext.cacheDir, "updates").canonicalFile
        if (file == null || !file.isFile || !file.canonicalPath.startsWith(updates.path + File.separator)) {
            invoke.reject("Update APK is missing")
            return
        }
        activity.runOnUiThread {
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
                    activity.startActivity(
                        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES)
                            .setData(Uri.parse("package:${activity.packageName}")),
                    )
                    invoke.resolve(JSObject().put("status", "permission"))
                    return@runOnUiThread
                }
                val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.moe.updates", file)
                activity.startActivity(
                    Intent(Intent.ACTION_VIEW)
                        .setDataAndType(uri, "application/vnd.android.package-archive")
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK),
                )
                invoke.resolve(JSObject().put("status", "started"))
            } catch (error: Exception) {
                invoke.reject("Could not open the system installer: ${error.message}")
            }
        }
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

    @PermissionCallback
    private fun onLegacyStoragePermissionResult(invoke: Invoke) {
        invoke.resolve(JSObject().put("launched", false).put("granted", hasStorageAccess()))
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

    /** Streams a file of the app's cache folder into a document the user picked (backup export; no size cap). */
    @Command
    fun copyFileToUri(invoke: Invoke) {
        val args = invoke.parseArgs(CopyFileToUriArgs::class.java)
        val sourcePath = args.sourcePath
        val destination = args.destinationUri
        if (sourcePath.isNullOrBlank() || destination.isNullOrBlank()) {
            invoke.reject("sourcePath and destinationUri are required")
            return
        }
        async(invoke) {
            val source = cacheFile(sourcePath)
            if (!source.isFile) throw IOException("The backup file is missing")
            val output = resolver.openOutputStream(Uri.parse(destination), "wt")
                ?: throw IOException("Could not open the selected destination")
            source.inputStream().use { input -> output.use { input.copyTo(it, COPY_BUFFER_SIZE) } }
            JSObject()
        }
    }

    /** Copies a document the user picked into the app's cache folder (backup import), at most `maxBytes`. */
    @Command
    fun copyUriToFile(invoke: Invoke) {
        val args = invoke.parseArgs(CopyUriToFileArgs::class.java)
        val sourceUri = args.sourceUri
        val destinationPath = args.destinationPath
        if (sourceUri.isNullOrBlank() || destinationPath.isNullOrBlank() || args.maxBytes <= 0L) {
            invoke.reject("sourceUri, destinationPath and maxBytes are required")
            return
        }
        async(invoke) {
            val destination = cacheFile(destinationPath)
            destination.parentFile?.mkdirs()
            var copied = 0L
            try {
                val input = resolver.openInputStream(Uri.parse(sourceUri))
                    ?: throw IOException("Could not open the selected file")
                input.use { source ->
                    destination.outputStream().use { target ->
                        val buffer = ByteArray(COPY_BUFFER_SIZE)
                        while (true) {
                            val read = source.read(buffer)
                            if (read < 0) break
                            copied += read
                            if (copied > args.maxBytes) throw IOException("The selected file is too large")
                            target.write(buffer, 0, read)
                        }
                    }
                }
            } catch (error: Exception) {
                destination.delete()
                throw error
            }
            JSObject().apply { put("size", copied) }
        }
    }

    /** A path that must lie inside the app's cache folder (the plugin never touches other private files). */
    private fun cacheFile(path: String): File {
        val root = appContext.cacheDir.canonicalFile
        val file = File(path).canonicalFile
        if (!file.path.startsWith(root.path + File.separator)) throw IOException("Path is outside the cache folder")
        return file
    }

    @Command
    fun cacheContentUriNamed(invoke: Invoke) {
        val args = invoke.parseArgs(ContentUriArgs::class.java)
        val rawUri = args.contentUri
        val maxBytes = args.maxBytes
        if (rawUri.isNullOrBlank() || maxBytes == null || maxBytes <= 0L || maxBytes > MAX_CONTENT_LEASE_BYTES) {
            invoke.reject("contentUri and maxBytes (1..$MAX_CONTENT_LEASE_BYTES) are required")
            return
        }
        async(invoke) {
            val uri = Uri.parse(rawUri)
            val response = cacheContentUri(uri, maxBytes)
            response.put("displayName", queryDisplayName(uri) ?: JSONObject.NULL)
            response
        }
    }

    @Command
    fun pickOpenDocument(invoke: Invoke) {
        val args = invoke.parseArgs(OpenDocumentArgs::class.java)
        val mimeTypes = args.mimeTypes?.filter(String::isNotBlank)?.toTypedArray().orEmpty()
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = if (mimeTypes.size == 1) mimeTypes[0] else "*/*"
            if (mimeTypes.size > 1) putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        startActivityForResult(invoke, intent, "onOpenDocumentPicked")
    }

    /**
     * Single image via the Android Photo Picker (PickVisualMedia, ImageOnly). If the picker is not
     * available, fall back to the SAF document picker limited to the given image MIME types. The
     * grant is temporary; callers copy the bytes right away.
     */
    @Command
    fun pickVisualImage(invoke: Invoke) {
        val args = invoke.parseArgs(OpenDocumentArgs::class.java)
        val intent = if (ActivityResultContracts.PickVisualMedia.isPhotoPickerAvailable(activity)) {
            ActivityResultContracts.PickVisualMedia().createIntent(
                activity,
                PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly),
            )
        } else {
            val mimeTypes = args.mimeTypes?.filter(String::isNotBlank)?.toTypedArray()?.takeIf { it.isNotEmpty() }
                ?: arrayOf("image/*")
            Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = if (mimeTypes.size == 1) mimeTypes[0] else "image/*"
                if (mimeTypes.size > 1) putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        }
        startActivityForResult(invoke, intent, "onOpenDocumentPicked")
    }

    @ActivityCallback
    private fun onOpenDocumentPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject().put("uri", JSONObject.NULL).put("displayName", JSONObject.NULL))
            return
        }
        io.execute {
            val name = try { queryDisplayName(uri) } catch (_: Exception) { null }
            invoke.resolve(JSObject().put("uri", uri.toString()).put("displayName", name ?: JSONObject.NULL))
        }
    }

    @Command
    fun pickCreateDocument(invoke: Invoke) {
        val args = invoke.parseArgs(CreateDocumentArgs::class.java)
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = args.mimeType?.takeIf(String::isNotBlank) ?: "application/octet-stream"
            putExtra(Intent.EXTRA_TITLE, args.suggestedName?.takeIf(String::isNotBlank) ?: "export")
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

    private fun allFilesAccessGranted(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && Environment.isExternalStorageManager()

    /**
     * Storage access used for shared-music scanning and direct edits (ADR 0014): Android 11+ needs
     * All files access; Android 10 and older use legacy READ/WRITE_EXTERNAL_STORAGE.
     */
    private fun hasStorageAccess(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            activity.checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) ==
                android.content.pm.PackageManager.PERMISSION_GRANTED
        }

    /** Storage access state; `legacy` = Android 10 or older (runtime permission instead of the settings page). */
    @Command
    fun allFilesAccessStatus(invoke: Invoke) {
        invoke.resolve(
            JSObject()
                .put("supported", true)
                .put("legacy", Build.VERSION.SDK_INT < Build.VERSION_CODES.R)
                .put("granted", hasStorageAccess()),
        )
    }

    /** Open the system "All files access" page for this app; the user toggles it there. */
    @Command
    fun requestAllFilesAccess(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            requestPermissionForAlias("legacyStorage", invoke, "onLegacyStoragePermissionResult")
            return
        }
        activity.runOnUiThread {
            try {
                try {
                    activity.startActivity(
                        Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
                            .setData(Uri.parse("package:${activity.packageName}")),
                    )
                } catch (_: Exception) {
                    activity.startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
                }
                invoke.resolve(JSObject().put("launched", true).put("granted", hasStorageAccess()))
            } catch (error: Exception) {
                invoke.reject("Could not open the all files access settings: ${error.message}")
            }
        }
    }

    /**
     * Folder picker used only to choose a location (ADR 0014): the tree URI is converted to its
     * real path and no persisted grant is taken. Scanning then uses All files access.
     */
    @Command
    fun pickDirectoryPath(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                putExtra(
                    DocumentsContract.EXTRA_INITIAL_URI,
                    DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", "primary:Music"),
                )
            }
        }
        startActivityForResult(invoke, intent, "onDirectoryPathPicked")
    }

    @ActivityCallback
    private fun onDirectoryPathPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject().put("path", JSONObject.NULL))
            return
        }
        val path = try {
            if (uri.authority == "com.android.externalstorage.documents") {
                externalStoragePathFor(DocumentsContract.getTreeDocumentId(uri))
            } else {
                null
            }
        } catch (_: Exception) {
            null
        }
        if (path == null || !File(path).isDirectory) {
            invoke.reject("Please choose a folder on the device storage")
            return
        }
        invoke.resolve(JSObject().put("path", File(path).absolutePath))
    }

    /** Document picker used only to choose a file; returns its real path (no persisted grant). */
    @Command
    fun pickDocumentPath(invoke: Invoke) {
        val args = invoke.parseArgs(OpenDocumentArgs::class.java)
        val mimeTypes = args.mimeTypes?.filter(String::isNotBlank)?.toTypedArray().orEmpty()
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = if (mimeTypes.size == 1) mimeTypes[0] else "*/*"
            if (mimeTypes.size > 1) putExtra(Intent.EXTRA_MIME_TYPES, mimeTypes)
        }
        startActivityForResult(invoke, intent, "onDocumentPathPicked")
    }

    /** "Save as" picker used only to name a new file; returns its real path (no persisted grant). */
    @Command
    fun pickCreateDocumentPath(invoke: Invoke) {
        val args = invoke.parseArgs(CreateDocumentArgs::class.java)
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = args.mimeType?.takeIf(String::isNotBlank) ?: "application/octet-stream"
            putExtra(Intent.EXTRA_TITLE, args.suggestedName?.takeIf(String::isNotBlank) ?: "export")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                putExtra(
                    DocumentsContract.EXTRA_INITIAL_URI,
                    DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", "primary:Music"),
                )
            }
        }
        startActivityForResult(invoke, intent, "onDocumentPathPicked")
    }

    @ActivityCallback
    private fun onDocumentPathPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject().put("path", JSONObject.NULL))
            return
        }
        io.execute {
            val path = try { documentPathFor(uri) } catch (_: Exception) { null }
            if (path == null) {
                invoke.reject("Please choose a file on the device storage")
            } else {
                invoke.resolve(JSObject().put("path", path))
            }
        }
    }

    /** Real path for a picked document (ExternalStorageProvider, MediaStore, or media documents). */
    private fun documentPathFor(uri: Uri): String? {
        directPathFor(uri)?.let { return it }
        if (uri.authority == "com.android.providers.media.documents") {
            val (type, id) = DocumentsContract.getDocumentId(uri).split(':', limit = 2).let {
                if (it.size == 2) it[0] to it[1] else return null
            }
            val collection = when (type) {
                "audio" -> MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
                "image" -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI
                "video" -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI
                else -> MediaStore.Files.getContentUri("external")
            }
            val id64 = id.toLongOrNull() ?: return null
            return directPathFor(android.content.ContentUris.withAppendedId(collection, id64))
        }
        return null
    }

    /** Real paths for many MediaStore / ExternalStorageProvider URIs, in order (source migration). */
    @Command
    fun resolveDirectPaths(invoke: Invoke) {
        val args = invoke.parseArgs(UriListArgs::class.java)
        val uris = args.uris.orEmpty()
        io.execute {
            val paths = org.json.JSONArray()
            for (raw in uris) {
                val path = try {
                    if (hasStorageAccess()) directPathFor(Uri.parse(raw)) else null
                } catch (_: Exception) {
                    null
                }
                paths.put(path ?: JSONObject.NULL)
            }
            invoke.resolve(JSObject().put("paths", paths))
        }
    }

    /**
     * Real file path of a MediaStore / ExternalStorageProvider document, only when All files access
     * is granted and the file exists (then Rust reads/writes it directly). Returns null otherwise;
     * there is no per-file write-prompt fallback (ADR 0014).
     */
    @Command
    fun resolveDirectPath(invoke: Invoke) {
        val rawUri = invoke.parseArgs(ContentUriArgs::class.java).contentUri
        if (rawUri.isNullOrBlank()) {
            invoke.reject("contentUri is required")
            return
        }
        io.execute {
            val path = try {
                if (allFilesAccessGranted()) directPathFor(Uri.parse(rawUri)) else null
            } catch (_: Exception) {
                null
            }
            invoke.resolve(JSObject().put("path", path ?: JSONObject.NULL))
        }
    }

    private fun directPathFor(uri: Uri): String? {
        val candidate = when (uri.authority) {
            MediaStore.AUTHORITY -> resolver.query(uri, arrayOf(MediaStore.MediaColumns.DATA), null, null, null)?.use { rows ->
                if (rows.moveToFirst()) rows.stringOrNull(0) else null
            }
            "com.android.externalstorage.documents" -> {
                val documentId = DocumentsContract.getDocumentId(uri)
                externalStoragePathFor(documentId)
            }
            else -> null
        } ?: return null
        val file = File(candidate)
        return if (file.isFile) file.absolutePath else null
    }

    /** Ask MediaStore to re-read a file changed through its direct path. */
    @Command
    fun scanFile(invoke: Invoke) {
        val path = invoke.parseArgs(FilePathArgs::class.java).path
        if (path.isNullOrBlank()) {
            invoke.reject("path is required")
            return
        }
        MediaScannerConnection.scanFile(appContext, arrayOf(path), null, null)
        invoke.resolve(JSObject())
    }

    /**
     * Copy the static system wallpaper (as displayed) into the app cache as JPEG. Android 13+ only
     * allows this with All files access; a live wallpaper has no bitmap. The result has a path, or
     * a reason ("permission" | "live" | "unavailable").
     */
    @Command
    fun readSystemWallpaper(invoke: Invoke) {
        io.execute {
            val response = JSObject()
            try {
                val manager = WallpaperManager.getInstance(appContext)
                val info = manager.wallpaperInfo
                Log.i(LOG_TAG, "wallpaper: sdk=${Build.VERSION.SDK_INT} info=${info?.component} allFiles=${allFilesAccessGranted()} " +
                    "isWallpaperSupported=${manager.isWallpaperSupported} isSetWallpaperAllowed=${manager.isSetWallpaperAllowed} " +
                    "desired=${manager.desiredMinimumWidth}x${manager.desiredMinimumHeight}")
                if (info != null) {
                    Log.i(LOG_TAG, "wallpaper: reported as a live wallpaper, no bitmap is read")
                    invoke.resolve(response.put("path", JSONObject.NULL).put("reason", "live"))
                    return@execute
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && !allFilesAccessGranted()) {
                    Log.i(LOG_TAG, "wallpaper: all files access is not granted")
                    invoke.resolve(response.put("path", JSONObject.NULL).put("reason", "permission"))
                    return@execute
                }
                val drawable = manager.drawable
                Log.i(LOG_TAG, "wallpaper: drawable=${drawable?.javaClass?.name} size=${drawable?.intrinsicWidth}x${drawable?.intrinsicHeight}")
                if (drawable == null) {
                    invoke.resolve(response.put("path", JSONObject.NULL).put("reason", "unavailable"))
                    return@execute
                }
                val bitmap = if (drawable is BitmapDrawable && drawable.bitmap != null) {
                    drawable.bitmap
                } else {
                    val width = drawable.intrinsicWidth.takeIf { it > 0 } ?: 1080
                    val height = drawable.intrinsicHeight.takeIf { it > 0 } ?: 1920
                    Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).also { target ->
                        val canvas = Canvas(target)
                        drawable.setBounds(0, 0, width, height)
                        drawable.draw(canvas)
                    }
                }
                val directory = File(appContext.cacheDir, "wallpaper").apply { mkdirs() }
                val output = File(directory, "system-wallpaper.jpg")
                val temporary = File(directory, "system-wallpaper.jpg.tmp")
                FileOutputStream(temporary).use { stream ->
                    if (!bitmap.compress(Bitmap.CompressFormat.JPEG, 95, stream)) {
                        throw IOException("JPEG encoding failed")
                    }
                }
                if (!temporary.renameTo(output)) {
                    output.delete()
                    if (!temporary.renameTo(output)) throw IOException("Could not store the wallpaper copy")
                }
                invoke.resolve(response.put("path", output.absolutePath).put("reason", JSONObject.NULL))
            } catch (error: SecurityException) {
                Log.i(LOG_TAG, "wallpaper: refused: ${error.message}")
                invoke.resolve(response.put("path", JSONObject.NULL).put("reason", "permission"))
            } catch (error: Exception) {
                Log.i(LOG_TAG, "wallpaper: failed: $error")
                invoke.reject("Could not read the system wallpaper: ${error.message}")
            }
        }
    }

    private var savedWindowBackground: Drawable? = null
    private var savedWindowFormat: Int = PixelFormat.OPAQUE
    private val savedViewBackgrounds = mutableListOf<Pair<View, Drawable?>>()
    private var wallpaperBackdropActive = false

    /**
     * Show the system wallpaper behind the (transparent) WebView via FLAG_SHOW_WALLPAPER. Android 13+ no
     * longer lets ordinary apps read wallpaper pixels (WallpaperManager.getDrawable needs
     * MANAGE_EXTERNAL_STORAGE), but the window manager can still composite it behind our window. Blur uses
     * cross-window background blur when the device supports it.
     */
    @Command
    fun setWallpaperBackdrop(invoke: Invoke) {
        val args = invoke.parseArgs(WallpaperBackdropArgs::class.java)
        val enabled = args.enabled == true
        val blurPx = (args.blurPx ?: 0).coerceIn(0, 150)
        activity.runOnUiThread {
            try {
                val window = activity.window
                var blurApplied = false
                if (enabled) {
                    val webView = findWebView(window.decorView)
                    if (!wallpaperBackdropActive) {
                        savedWindowBackground = window.decorView.background
                        savedWindowFormat = window.attributes.format
                        savedViewBackgrounds.clear()
                        // Every ancestor between the WebView and the decor view must be clear too.
                        var parent = webView?.parent
                        while (parent is View && parent !== window.decorView) {
                            savedViewBackgrounds.add(parent to parent.background)
                            parent = parent.parent
                        }
                    }
                    wallpaperBackdropActive = true
                    // An opaque window surface paints black under transparent content, hiding the
                    // wallpaper even though this window is the wallpaper target.
                    window.setFormat(PixelFormat.TRANSLUCENT)
                    window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WALLPAPER)
                    window.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
                    savedViewBackgrounds.forEach { (view, _) -> view.background = null }
                    webView?.setBackgroundColor(Color.TRANSPARENT)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                        val supported = activity.windowManager.isCrossWindowBlurEnabled
                        window.setBackgroundBlurRadius(if (supported) blurPx else 0)
                        blurApplied = supported && blurPx > 0
                    }
                } else if (wallpaperBackdropActive) {
                    wallpaperBackdropActive = false
                    window.clearFlags(WindowManager.LayoutParams.FLAG_SHOW_WALLPAPER)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setBackgroundBlurRadius(0)
                    window.setBackgroundDrawable(savedWindowBackground ?: ColorDrawable(Color.BLACK))
                    savedViewBackgrounds.forEach { (view, background) -> view.background = background }
                    savedViewBackgrounds.clear()
                    window.setFormat(savedWindowFormat)
                    savedWindowBackground = null
                }
                val blurSupported = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && activity.windowManager.isCrossWindowBlurEnabled
                invoke.resolve(JSObject().put("active", wallpaperBackdropActive).put("blurApplied", blurApplied).put("blurSupported", blurSupported))
            } catch (error: Exception) {
                invoke.reject("Could not show the system wallpaper: ${error.message}")
            }
        }
    }

    private fun findWebView(view: View): WebView? {
        if (view is WebView) return view
        if (view is ViewGroup) {
            for (index in 0 until view.childCount) findWebView(view.getChildAt(index))?.let { return it }
        }
        return null
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

    private fun readMetadata(uri: Uri): JSObject {
        val response = JSObject()
        val retriever = android.media.MediaMetadataRetriever()
        val audioFormat = readAudioFormat(uri)
        var retrieverError: String? = null
        val retrieved = try {
            retriever.setDataSource(activity, uri)
            metadataObject(
                title = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_TITLE),
                artist = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_ARTIST),
                album = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_ALBUM),
                albumArtist = retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_ALBUMARTIST),
                trackNumber = parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_CD_TRACK_NUMBER)),
                discNumber = parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_DISC_NUMBER)),
                durationMs = parsePositiveLong(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_DURATION)),
                codec = audioFormat?.stringOrNull(android.media.MediaFormat.KEY_MIME)
                    ?: retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_MIMETYPE),
                bitrateBps = audioFormat?.intOrNull(android.media.MediaFormat.KEY_BIT_RATE)
                    ?: parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_BITRATE)),
                sampleRateHz = audioFormat?.intOrNull(android.media.MediaFormat.KEY_SAMPLE_RATE)
                    ?: parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_SAMPLERATE)),
                bitDepth = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    parsePositiveInt(retriever.extractMetadata(android.media.MediaMetadataRetriever.METADATA_KEY_BITS_PER_SAMPLE))
                } else {
                    null
                },
            )
        } catch (error: Exception) {
            retrieverError = error.message
            null
        } finally {
            try {
                retriever.release()
            } catch (_: RuntimeException) {
                // The metadata error above is the useful failure to report.
            }
        }
        val metadata = retrieved ?: metadataObject(
            durationMs = audioFormat?.longOrNull(android.media.MediaFormat.KEY_DURATION)?.div(1000L),
            codec = audioFormat?.stringOrNull(android.media.MediaFormat.KEY_MIME),
            bitrateBps = audioFormat?.intOrNull(android.media.MediaFormat.KEY_BIT_RATE),
            sampleRateHz = audioFormat?.intOrNull(android.media.MediaFormat.KEY_SAMPLE_RATE),
        )
        val hasUsefulMetadata = listOf(
            "title", "artist", "album", "durationMs", "codec", "bitrateBps", "sampleRateHz",
        ).any { key -> metadata.opt(key).let { it != null && it != JSONObject.NULL && it != "" } }
        response.put("metadata", if (hasUsefulMetadata) metadata else JSONObject.NULL)
        response.put(
            "error",
            if (hasUsefulMetadata) JSONObject.NULL else retrieverError ?: "Android could not identify an audio track",
        )
        return response
    }

    private fun readAudioFormat(uri: Uri): android.media.MediaFormat? {
        val extractor = android.media.MediaExtractor()
        return try {
            extractor.setDataSource(activity, uri, null)
            (0 until extractor.trackCount)
                .asSequence()
                .map(extractor::getTrackFormat)
                .firstOrNull { it.stringOrNull(android.media.MediaFormat.KEY_MIME)?.startsWith("audio/") == true }
        } catch (_: Exception) {
            null
        } finally {
            try {
                extractor.release()
            } catch (_: RuntimeException) {
                // MetadataRetriever remains the fallback for tags and duration.
            }
        }
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

    private fun getStoreVersion(volumeName: String): String? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) MediaStore.getVersion(activity, volumeName) else null

    private fun getStoreGeneration(volumeName: String): Long? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) MediaStore.getGeneration(activity, volumeName) else null


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
        scanStateReason(kind, reason, errors.optJSONObject(0)?.optString("message"))?.let {
            state.put("reason", it)
        }
        return JSObject().apply {
            put("state", state)
            put("tracks", tracks)
            put("errors", errors)
        }
    }

    private fun android.media.MediaFormat.stringOrNull(key: String): String? =
        if (containsKey(key)) getString(key) else null

    private fun android.media.MediaFormat.intOrNull(key: String): Int? =
        if (containsKey(key)) getInteger(key).takeIf { it > 0 } else null

    private fun android.media.MediaFormat.longOrNull(key: String): Long? =
        if (containsKey(key)) getLong(key).takeIf { it > 0L } else null

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
        private const val LOG_TAG = "MoeMediaIndex"

        internal fun scanStateReason(kind: String, reason: String?, firstErrorMessage: String?): String? {
            if (kind == "complete") return null
            return reason?.takeIf(String::isNotBlank)
                ?: firstErrorMessage?.takeIf(String::isNotBlank)
                ?: "Android source scan did not complete"
        }

        internal fun isHiddenDocument(displayName: String?): Boolean =
            displayName?.startsWith('.') == true

        internal const val MAX_SLIDESHOW_IMAGES = 10_000

        /** "primary:Music/a.flac" -> /storage/emulated/0/Music/a.flac; "1A2B-3C4D:x" -> /storage/1A2B-3C4D/x. */
        private const val EXTERNAL_STORAGE_AUTHORITY = "com.android.externalstorage.documents"

        /** Inverse of [externalStoragePathFor]: `/storage/emulated/0/Music` -> `primary:Music`. */
        internal fun externalStorageDocumentIdFor(path: String): String? {
            val normalized = path.trimEnd('/')
            if (normalized.split('/').any { it == ".." }) return null
            val primary = "/storage/emulated/0"
            if (normalized == primary) return "primary:"
            if (normalized.startsWith("$primary/")) return "primary:" + normalized.removePrefix("$primary/")
            val match = Regex("^/storage/([0-9A-Fa-f]{4}-[0-9A-Fa-f]{4})(/(.*))?$").matchEntire(normalized) ?: return null
            return match.groupValues[1] + ":" + match.groupValues[3]
        }

        internal fun externalStoragePathFor(documentId: String): String? {
            val colon = documentId.indexOf(':')
            if (colon <= 0) return null
            val volume = documentId.substring(0, colon)
            val relative = documentId.substring(colon + 1)
            if (relative.split('/').any { it == ".." }) return null
            val root = if (volume == "primary") "/storage/emulated/0" else "/storage/$volume"
            return if (relative.isEmpty()) root else "$root/$relative"
        }
        private val SLIDESHOW_IMAGE_EXTENSIONS = setOf("png", "jpg", "jpeg", "webp", "gif", "bmp")
        private val SLIDESHOW_IMAGE_MIME_TYPES = setOf("image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "image/x-ms-bmp")

        /** Same formats as the desktop slideshow (main_background.rs is_supported_image). */
        internal fun isSlideshowImage(mimeType: String?, displayName: String?): Boolean {
            val mime = mimeType?.lowercase()
            if (mime != null && mime in SLIDESHOW_IMAGE_MIME_TYPES) return true
            if (mime != null && mime != "application/octet-stream") return false
            val extension = displayName?.substringAfterLast('.', "")?.lowercase().orEmpty()
            return extension in SLIDESHOW_IMAGE_EXTENSIONS
        }

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

        // Mirrors player-formats::android_candidate_extensions(); the test
        // plugins/tauri-plugin-media-index/tests/kotlin_extension_list.rs fails when the two drift.
        private val AUDIO_EXTENSIONS = setOf(
            "aac", "ac3", "aif", "aiff", "alac", "amr", "ape", "dff", "dsf", "flac",
            "m4a", "m4b", "mid", "midi", "mka", "mp3", "mp4", "oga", "ogg", "opus", "spx",
            "wav", "wave", "wma", "wv",
        )

        private const val MAX_CONTENT_LEASE_BYTES = 512L * 1024 * 1024
        private const val MAX_ARTWORK_BYTES = 32L * 1024 * 1024
        private const val TAG = "MoeMediaIndex"
        private const val CLIPBOARD_SHARE_DIRECTORY = "clipboard-share"
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
