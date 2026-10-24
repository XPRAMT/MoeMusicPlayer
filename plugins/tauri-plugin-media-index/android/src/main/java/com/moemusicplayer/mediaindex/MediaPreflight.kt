package com.moemusicplayer.mediaindex

import android.net.Uri
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import androidx.media3.extractor.DefaultExtractorInput
import androidx.media3.extractor.ExtractorsFactory
import java.io.EOFException
import java.io.IOException

/** The result of [MediaPreflight.check]. */
sealed class PreflightResult {
    /** The file opens and its container is recognised: the player may switch to it. */
    object Passed : PreflightResult()

    /** The file does not open. [serverUnreachable]: it is on a server that could not be reached. */
    data class Failed(val message: String, val serverUnreachable: Boolean) : PreflightResult()
}

/**
 * Opens a track the way the player will (through the same data sources, so a `file://`, `content://`, `moedsd://`,
 * `moewv://` or `moeremote://` location is read by the same code) and lets the extractors recognise its container,
 * without touching the player (ADR 0036). A new track is switched to only after this passed, so a file that is missing,
 * unreadable, on a server that does not answer, or in a format no extractor knows never takes the playing track down.
 *
 * Blocks while a server answers: call it from a worker thread.
 */
@UnstableApi
class MediaPreflight(
    private val dataSources: DataSource.Factory,
    private val extractors: ExtractorsFactory,
) {
    fun check(uri: Uri): PreflightResult {
        val dataSource = dataSources.createDataSource()
        try {
            val length = dataSource.open(DataSpec(uri))
            val input = DefaultExtractorInput(dataSource, 0L, length)
            if (!recognised(input, uri)) {
                return PreflightResult.Failed("Unsupported audio format", serverUnreachable = false)
            }
            return PreflightResult.Passed
        } catch (error: RemoteLinkLostException) {
            return PreflightResult.Failed(REMOTE_LINK_LOST_MESSAGE, serverUnreachable = true)
        } catch (error: IOException) {
            return PreflightResult.Failed(
                remoteLinkLostMessage(error) ?: (error.message ?: "Cannot open the file"),
                serverUnreachable = remoteLinkLostMessage(error) != null,
            )
        } catch (error: Exception) {
            return PreflightResult.Failed(error.message ?: "Cannot open the file", serverUnreachable = false)
        } finally {
            try {
                dataSource.close()
            } catch (_: Exception) {
                // Nothing to give back.
            }
        }
    }

    /** The way ExoPlayer chooses an extractor: the first one whose `sniff` accepts the start of the file. */
    private fun recognised(input: DefaultExtractorInput, uri: Uri): Boolean {
        for (extractor in extractors.createExtractors(uri, emptyMap())) {
            try {
                if (extractor.sniff(input)) return true
            } catch (_: EOFException) {
                // Too short for this container: the next one may know it.
            } finally {
                input.resetPeekPosition()
                extractor.release()
            }
        }
        return false
    }
}
