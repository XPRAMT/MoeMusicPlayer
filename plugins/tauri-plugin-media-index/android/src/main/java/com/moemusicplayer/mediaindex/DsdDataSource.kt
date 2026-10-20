package com.moemusicplayer.mediaindex

import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSourceException
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.TransferListener
import java.io.IOException

/**
 * Natives of the Rust DSD and WavPack converters (ADR 0016, ADR 0025), exported from `moemusicplayer_lib`
 * (`src-tauri/src/android_dsd.rs`). A handle is a DSF or WavPack file served as a WAV file of 32-bit float PCM,
 * converted on demand; the Rust side picks the converter from the file extension.
 */
object DsdBridge {
    /** Opens a DSF file; 0 means it could not be opened or is not supported. */
    @JvmStatic external fun nativeOpen(path: String): Long

    /** Length in bytes of the virtual WAV file, or -1 for an unknown handle. */
    @JvmStatic external fun nativeLength(handle: Long): Long

    /** Copies up to [length] bytes starting at [position] into [buffer]; 0 is the end, -1 an error. */
    @JvmStatic external fun nativeRead(handle: Long, position: Long, buffer: ByteArray, offset: Int, length: Int): Int

    @JvmStatic external fun nativeClose(handle: Long)

    /** The cover picture bytes of a `moeremote://` track, or null. Blocks on the server: call it from a worker thread. */
    @JvmStatic external fun nativeRemoteCover(uri: String): ByteArray?
}

const val DSD_URI_SCHEME = "moedsd"

/** Scheme of a WavPack (`.wv`) track: `moewv:///path/to/file.wv`, served by the same data source as DSD. */
const val WAVPACK_URI_SCHEME = "moewv"

/**
 * Scheme of a track on a remote server (ADR 0026): `moeremote://<source id>/<path>`. Rust reads it through the shared remote
 * provider: DSF and WavPack are converted to float WAV like local files, every other format is passed through byte for
 * byte for Media3 to decode.
 */
const val REMOTE_URI_SCHEME = "moeremote"

/** Whether a track served by Rust is a WAV stream (converted) and not the file's own bytes. */
fun isConvertedUri(uri: String): Boolean {
    val lower = uri.lowercase()
    return lower.startsWith("$DSD_URI_SCHEME:") || lower.startsWith("$WAVPACK_URI_SCHEME:") ||
        (lower.startsWith("$REMOTE_URI_SCHEME:") && (lower.endsWith(".dsf") || lower.endsWith(".wv")))
}

/** Reads a `moedsd:///path/to/file.dsf`, `moewv:///path/to/file.wv` or `moeremote://…` URI served by Rust. */
@UnstableApi
class DsdDataSource : BaseDataSource(/* isNetwork = */ false) {
    private var uri: Uri? = null
    private var handle = 0L
    private var position = 0L
    private var bytesRemaining = 0L
    private var opened = false

    override fun open(dataSpec: DataSpec): Long {
        uri = dataSpec.uri
        transferInitializing(dataSpec)
        // A remote location is handed over whole (the source id is its host); the others are plain paths.
        val path = if (dataSpec.uri.scheme == REMOTE_URI_SCHEME) {
            dataSpec.uri.toString()
        } else {
            dataSpec.uri.path ?: throw IOException("Converted-media URI has no path")
        }
        handle = DsdBridge.nativeOpen(path)
        if (handle == 0L) throw IOException("Cannot open DSD file: $path")
        val total = DsdBridge.nativeLength(handle)
        if (total < 0L) throw IOException("DSD stream is gone")
        position = dataSpec.position
        if (position > total) {
            close()
            throw DataSourceException(PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE)
        }
        bytesRemaining = if (dataSpec.length != C.LENGTH_UNSET.toLong()) {
            minOf(dataSpec.length, total - position)
        } else {
            total - position
        }
        opened = true
        transferStarted(dataSpec)
        return bytesRemaining
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (length == 0) return 0
        if (bytesRemaining == 0L) return C.RESULT_END_OF_INPUT
        val wanted = minOf(length.toLong(), bytesRemaining).toInt()
        val read = DsdBridge.nativeRead(handle, position, buffer, offset, wanted)
        if (read < 0) throw IOException("DSD read failed")
        if (read == 0) return C.RESULT_END_OF_INPUT
        position += read
        bytesRemaining -= read
        bytesTransferred(read)
        return read
    }

    override fun getUri(): Uri? = uri

    override fun close() {
        uri = null
        if (handle != 0L) {
            DsdBridge.nativeClose(handle)
            handle = 0L
        }
        if (opened) {
            opened = false
            transferEnded()
        }
    }
}

/** Sends `moedsd:` and `moewv:` URIs to [DsdDataSource] and everything else to the regular data source. */
@UnstableApi
class DsdAwareDataSourceFactory(private val base: DataSource.Factory) : DataSource.Factory {
    override fun createDataSource(): DataSource = RoutingDataSource(base.createDataSource(), DsdDataSource())
}

@UnstableApi
private class RoutingDataSource(private val base: DataSource, private val dsd: DataSource) : DataSource {
    private var active: DataSource = base

    override fun addTransferListener(transferListener: TransferListener) {
        base.addTransferListener(transferListener)
        dsd.addTransferListener(transferListener)
    }

    override fun open(dataSpec: DataSpec): Long {
        val scheme = dataSpec.uri.scheme
        active = if (scheme == DSD_URI_SCHEME || scheme == WAVPACK_URI_SCHEME || scheme == REMOTE_URI_SCHEME) dsd else base
        return active.open(dataSpec)
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int = active.read(buffer, offset, length)

    override fun getUri(): Uri? = active.uri

    override fun getResponseHeaders(): Map<String, List<String>> = active.responseHeaders

    override fun close() {
        active.close()
    }
}
