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
 * Natives of the Rust DSD, WavPack and FLAC converters (ADR 0016, ADR 0025, ADR 0043), exported from
 * `moemusicplayer_lib` (`src-tauri/src/android_dsd.rs`). A handle is a DSF, WavPack or FLAC file served as a WAV file
 * of 32-bit float PCM, converted on demand; the Rust side picks the converter (`player_formats::android_converter`).
 */
object DsdBridge {
    /** Opens a converted file; 0 means it could not be opened or is not supported, -2 an unreachable server. */
    @JvmStatic external fun nativeOpen(path: String): Long

    /**
     * Opens a local DSF as the raw 32-bit DSD slots of native passthrough (`moedsdraw://`, ADR 0049); 0 when it cannot
     * be carried (not stereo DSD64/128/256).
     */
    @JvmStatic external fun nativeOpenRaw(path: String): Long

    /** Whether the DSF at [path] can be played natively: JSON `{ ok, rate, name }` or `{ ok: false, reason }` (header only). */
    @JvmStatic external fun nativeRawPlan(path: String): String?

    /** Length in bytes of the virtual WAV file, or -1 for an unknown handle. */
    @JvmStatic external fun nativeLength(handle: Long): Long

    /**
     * Copies up to [length] bytes starting at [position] into [buffer]; 0 is the end, -1 an error, -2 a remote track
     * whose server could not be reached (the Rust stream retried for minutes and gave up).
     */
    @JvmStatic external fun nativeRead(handle: Long, position: Long, buffer: ByteArray, offset: Int, length: Int): Int

    @JvmStatic external fun nativeClose(handle: Long)

    /** The cover picture bytes of a `moeremote://` track, or null. Blocks on the server: call it from a worker thread. */
    @JvmStatic external fun nativeRemoteCover(uri: String): ByteArray?
}

const val DSD_URI_SCHEME = "moedsd"

/**
 * Scheme of a local DSF served as the raw DSD slots of native passthrough to the USB exclusive output (ADR 0049):
 * `moedsdraw:///path/to/file.dsf`. Rust never hands it over; `DsdPassthrough` swaps it in for a `moedsd://` load when
 * the track can play natively. It is a WAV (16-bit, 4 channels) whose data is not audio.
 */
const val DSD_RAW_URI_SCHEME = "moedsdraw"

/** Message shown when a remote track stopped because its server could not be reached (Rust `REMOTE_LINK_LOST`). */
const val REMOTE_LINK_LOST_MESSAGE = "伺服器無法連線"

/**
 * A remote track's server could not be reached and the Rust stream already retried for minutes (ADR 0026). A subclass of
 * [java.io.FileNotFoundException] because Media3's default load error policy does not retry those: playback stops with
 * this error instead of trying again for minutes per retry, and nothing moves on to the next track.
 */
class RemoteLinkLostException(message: String) : java.io.FileNotFoundException(message)

/** The "server could not be reached" message if [error] (or one of its causes) is a [RemoteLinkLostException]. */
fun remoteLinkLostMessage(error: Throwable?): String? {
    var cause = error
    var depth = 0
    while (cause != null && depth < 10) {
        if (cause is RemoteLinkLostException) return cause.message
        cause = cause.cause
        depth++
    }
    return null
}

/** Scheme of a WavPack (`.wv`) track: `moewv:///path/to/file.wv`, served by the same data source as DSD. */
const val WAVPACK_URI_SCHEME = "moewv"

/**
 * Scheme of a FLAC (`.flac`) track: `moeflac:///path/to/file.flac`, served by the same data source as DSD (ADR 0043:
 * some devices offer only a vendor FLAC decoder that fails on 24-bit files).
 */
const val FLAC_URI_SCHEME = "moeflac"

/** Schemes of local files that Rust converts; which formats these are is decided on the Rust side. */
val LOCAL_CONVERTED_SCHEMES = setOf(DSD_URI_SCHEME, DSD_RAW_URI_SCHEME, WAVPACK_URI_SCHEME, FLAC_URI_SCHEME)

/** Whether [scheme] is read through [DsdDataSource] (a converted local file or a remote track). */
fun isRustServedScheme(scheme: String?): Boolean = scheme in LOCAL_CONVERTED_SCHEMES || scheme == REMOTE_URI_SCHEME

/** The `file:` URI of a converted local track (the same path), or [uri] itself. */
fun fileUriOfConverted(uri: String): String {
    for (scheme in LOCAL_CONVERTED_SCHEMES) {
        if (uri.startsWith("$scheme:")) return "file:" + uri.removePrefix("$scheme:")
    }
    return uri
}

/**
 * Scheme of a track on a remote server (ADR 0026): `moeremote://<source id>/<path>`. Rust reads it through the shared remote
 * provider: DSF, WavPack and FLAC are converted to float WAV like local files, every other format is passed through byte
 * for byte for Media3 to decode. Whether a load is a converted WAV stream comes with the load command (`converted`).
 */
const val REMOTE_URI_SCHEME = "moeremote"

/** Reads a `moedsd:///…dsf`, `moedsdraw:///…dsf`, `moewv:///…wv`, `moeflac:///…flac` or `moeremote://…` URI served by Rust. */
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
        handle = if (dataSpec.uri.scheme == DSD_RAW_URI_SCHEME) DsdBridge.nativeOpenRaw(path) else DsdBridge.nativeOpen(path)
        if (handle == -2L) {
            // The server of a remote track cannot be reached: stop at once (this is not retried, see RemoteLinkLostException).
            handle = 0L
            throw RemoteLinkLostException(REMOTE_LINK_LOST_MESSAGE)
        }
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
        if (read == -2) throw RemoteLinkLostException(REMOTE_LINK_LOST_MESSAGE)
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

/** Sends the URIs Rust serves ([isRustServedScheme]) to [DsdDataSource] and everything else to the regular data source. */
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
        active = if (isRustServedScheme(dataSpec.uri.scheme)) dsd else base
        return active.open(dataSpec)
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int = active.read(buffer, offset, length)

    override fun getUri(): Uri? = active.uri

    override fun getResponseHeaders(): Map<String, List<String>> = active.responseHeaders

    override fun close() {
        active.close()
    }
}
