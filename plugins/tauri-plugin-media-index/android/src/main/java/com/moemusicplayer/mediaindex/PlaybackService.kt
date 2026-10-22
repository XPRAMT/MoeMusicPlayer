package com.moemusicplayer.mediaindex

import android.app.PendingIntent
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.extractor.DefaultExtractorsFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicBoolean
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

/**
 * The Android-owned single-track playback endpoint. Queue traversal remains in Rust; this
 * service only executes Rust commands and reports MediaSession transport requests.
 */
@UnstableApi
class PlaybackService : MediaSessionService() {
    private val mainHandler = Handler(Looper.getMainLooper())
    private val artworkExecutor = ThreadPoolExecutor(
        1,
        1,
        0L,
        TimeUnit.MILLISECONDS,
        ArrayBlockingQueue(1),
        { runnable -> Thread(runnable, "moe-playback-artwork").apply { isDaemon = true } },
    )
    private lateinit var pollThread: HandlerThread
    private lateinit var pollHandler: Handler
    private var exoPlayer: ExoPlayer? = null
    private var session: MediaSession? = null
    private var sessionPlayer: Player? = null
    private val commandInFlight = AtomicBoolean(false)
    private var queueCapabilities = QueueCapabilities(canNext = false, canPrevious = false)
    private var stopped = false
    private var endedEventLoadInstance: Long? = null
    private var pendingLoadAck: JSONObject? = null
    private var expectedLoadCommandId: String? = null
    private var loadInstance = 0L
    private var lastSnapshotSentAt = 0L
    @Volatile private var serviceDestroyed = false
    private var serviceGeneration = 0L
    private val loadFence = PlaybackLoadFence()
    private var artworkPreparationCommandId: String? = null
    private var artworkPreparationFuture: java.util.concurrent.Future<*>? = null
    @Volatile private var playbackActive = false
    private val loadAckTimeout = Runnable {
        val ack = pendingLoadAck ?: return@Runnable
        if (!loadFence.cancel(expectedLoadCommandId)) return@Runnable
        pendingLoadAck = null
        expectedLoadCommandId = null
        val player = exoPlayer
        player?.stop()
        player?.clearMediaItems()
        stopped = true
        ack.put("error", "Android media source did not become ready within ${LOAD_ACK_TIMEOUT_MS}ms")
        sendAck(ack)
    }
    private val artworkPreparationTimeout = Runnable {
        val commandId = artworkPreparationCommandId ?: return@Runnable
        artworkPreparationCommandId = null
        artworkPreparationFuture?.cancel(true)
        artworkPreparationFuture = null
        reportEvent(JSONObject().apply {
            put("kind", "ack")
            put("commandId", commandId)
            put("requestId", artworkPreparationRequestId)
            put("serviceGeneration", serviceGeneration)
            put("error", "Timed out preparing Android artwork before media load")
            put("snapshot", snapshot(exoPlayer))
        })
        commandInFlight.set(false)
    }
    private var artworkPreparationRequestId = ""

    override fun onCreate() {
        super.onCreate()
        activeInstance = this
        System.loadLibrary("moemusicplayer_lib")
        serviceGeneration = SERVICE_GENERATIONS.incrementAndGet()

        val extractors = DefaultExtractorsFactory().setConstantBitrateSeekingEnabled(true)
        // DSD (.dsf) tracks arrive as moedsd:// URIs and WavPack (.wv) tracks as moewv:// URIs; both are served as
        // 32-bit float WAV by the Rust converters (ADR 0016, ADR 0025), so float PCM output is enabled for them.
        val dataSources = DsdAwareDataSourceFactory(DefaultDataSource.Factory(this))
        // Media3 keeps its existing native float path for 24/32-bit integer and float PCM. The chain converter
        // covers the remaining PCM encodings without bypassing the normal 16-bit playback processing.
        val renderers = object : DefaultRenderersFactory(this) {
            override fun buildAudioSink(
                context: android.content.Context,
                enableFloatOutput: Boolean,
                enableAudioTrackPlaybackParams: Boolean,
            ): androidx.media3.exoplayer.audio.AudioSink =
                androidx.media3.exoplayer.audio.DefaultAudioSink.Builder(context)
                    .setEnableFloatOutput(enableFloatOutput)
                    .setEnableAudioTrackPlaybackParams(enableAudioTrackPlaybackParams)
                    .setAudioProcessorChain(FloatAwareAudioProcessorChain())
                    .build()
        }.setEnableAudioFloatOutput(true)
        val player = ExoPlayer.Builder(this, renderers)
            .setMediaSourceFactory(DefaultMediaSourceFactory(dataSources, extractors))
            .setAudioAttributes(
                AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(),
                true,
            )
            .build()
        player.setHandleAudioBecomingNoisy(true)
        player.addListener(object : Player.Listener {
            override fun onPlaybackStateChanged(playbackState: Int) {
                playbackActive = player.isPlaying
                if (playbackState == Player.STATE_READY) {
                    pendingLoadAck?.let { ack ->
                    val currentItem = player.currentMediaItem
                        if (!loadFence.matchesReady(
                                currentItem?.mediaId,
                                currentItem?.localConfiguration?.tag as? String,
                            )
                        ) return@let
                        loadFence.finish(expectedLoadCommandId)
                        pendingLoadAck = null
                        expectedLoadCommandId = null
                        sendAck(ack)
                    }
                }
                if (playbackState == Player.STATE_ENDED) reportEndedOnce()
                reportSnapshot()
            }

            override fun onIsPlayingChanged(isPlaying: Boolean) {
                playbackActive = isPlaying
                reportSnapshot()
            }

            override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                pendingLoadAck?.let { ack ->
                    pendingLoadAck = null
                    loadFence.finish(expectedLoadCommandId)
                    expectedLoadCommandId = null
                    mainHandler.removeCallbacks(loadAckTimeout)
                    ack.put("error", error.message ?: "Android audio decoder failed")
                    sendAck(ack)
                }
                reportSnapshot()
            }
        })
        exoPlayer = player
        val forwarding = QueueAwarePlayer(player)
        sessionPlayer = forwarding
        val launchIntent = packageManager.getLaunchIntentForPackage(packageName)
        val sessionActivity = launchIntent?.let {
            PendingIntent.getActivity(
                this,
                0,
                it,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
        val builtSession = MediaSession.Builder(this, forwarding)
            .apply { if (sessionActivity != null) setSessionActivity(sessionActivity) }
            .build()
        session = builtSession
        // The service is started with startService and no in-app MediaController ever connects, so
        // Media3 would never learn about the session and never post its media notification. Adding it
        // explicitly lets DefaultMediaNotificationProvider show the MediaStyle notification (cover,
        // title, artist, previous / play-pause / next from the queue-aware commands), promote the
        // service to a mediaPlayback foreground service while playing, and remove it on stop.
        addSession(builtSession)

        pollThread = HandlerThread("moe-playback-command-poll").apply { start() }
        pollHandler = Handler(pollThread.looper)
        scheduleCommandPoll(0L)
        scheduleSnapshotSample()
        reportServiceReady()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    /** Swiping the app away while paused/stopped clears the notification and ends the service. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        val player = exoPlayer
        if (player == null || !player.playWhenReady || player.mediaItemCount == 0) {
            stopSelf()
        }
    }

    /** Main thread (the player's looper): what the floating lyrics overlay needs (ADR 0032). */
    internal fun overlaySample(): FloatingLyricsOverlay.Sample? {
        val player = exoPlayer ?: return null
        val item = player.currentMediaItem ?: return null
        val state = player.playbackState
        val active = state == androidx.media3.common.Player.STATE_READY ||
            state == androidx.media3.common.Player.STATE_BUFFERING
        val duration = player.duration.takeIf { it > 0 }
        // `playing` lets the overlay carry the coarse position on per frame (FloatingLyricsRules.smoothPositionMs).
        return FloatingLyricsOverlay.Sample(item.mediaId, player.currentPosition, duration, active, player.isPlaying)
    }

    override fun onDestroy() {
        if (activeInstance === this) activeInstance = null
        serviceDestroyed = true
        playbackActive = false
        mainHandler.removeCallbacks(loadAckTimeout)
        mainHandler.removeCallbacks(artworkPreparationTimeout)
        artworkPreparationFuture?.cancel(true)
        artworkExecutor.shutdownNow()
        artworkPreparationCommandId?.let { commandId ->
            reportEvent(JSONObject().apply {
                put("kind", "ack")
                put("commandId", commandId)
                put("requestId", artworkPreparationRequestId)
                put("error", "Android playback service stopped during artwork preparation")
                put("snapshot", snapshot(exoPlayer))
            })
        }
        artworkPreparationCommandId = null
        preparedArtworkBytes.clear()
        pendingLoadAck?.let { ack ->
            pendingLoadAck = null
            loadFence.cancel(expectedLoadCommandId)
            expectedLoadCommandId = null
            ack.put("error", "Android playback service stopped while loading")
            sendAck(ack)
        }
        pollHandler.removeCallbacksAndMessages(null)
        pollThread.quitSafely()
        session?.let { removeSession(it) }
        session?.release()
        session = null
        sessionPlayer = null
        exoPlayer?.release()
        exoPlayer = null
        super.onDestroy()
    }

    private fun scheduleCommandPoll(delayMs: Long) {
        pollHandler.postDelayed({
            if (serviceDestroyed) return@postDelayed
        if (commandInFlight.compareAndSet(false, true)) {
                val command = try {
                    nativePollPlaybackCommand()
                } catch (_: UnsatisfiedLinkError) {
                    null
                } catch (_: RuntimeException) {
                    null
                }
                if (command != null) {
                    mainHandler.post {
                        if (serviceDestroyed) commandInFlight.set(false) else dispatchCommand(command)
                    }
                } else {
                    commandInFlight.set(false)
                }
            }
            scheduleCommandPoll(if (playbackActive) ACTIVE_POLL_INTERVAL_MS else IDLE_POLL_INTERVAL_MS)
        }, delayMs)
    }

    private fun scheduleSnapshotSample() {
        mainHandler.postDelayed({
            if (serviceDestroyed) return@postDelayed
            if (playbackActive) reportSnapshot()
            scheduleSnapshotSample()
        }, SNAPSHOT_SAMPLE_INTERVAL_MS)
    }

    private fun executeCommand(raw: String) {
        val player = exoPlayer ?: run {
            commandInFlight.set(false)
            return
        }
        val command = try {
            JSONObject(raw)
        } catch (error: Exception) {
            commandInFlight.set(false)
            reportEvent(JSONObject().put("kind", "bridgeError").put("message", "Invalid playback command: ${error.message}"))
            return
        }
        val commandId = command.optString("commandId")
        val requestId = command.optString("requestId")
        val ack = JSONObject().apply {
            put("v", 1)
            put("kind", "ack")
            put("commandId", commandId)
            put("requestId", requestId)
            put("serviceGeneration", serviceGeneration)
        }
        try {
            when (command.optString("op")) {
                "load" -> {
                    val uri = command.optString("contentUri")
                    val trackId = command.optString("trackId")
                    // Library tracks are real paths under All files access (file://, ADR 0014);
                    // remaining legacy tracks may still be content:// URIs.
                    require(
                        uri.startsWith("file:///") || uri.startsWith("content://") ||
                            uri.startsWith("$DSD_URI_SCHEME:///") || uri.startsWith("$WAVPACK_URI_SCHEME:///") ||
                            uri.startsWith("$REMOTE_URI_SCHEME://"),
                    ) {
                        "Android playback requires a file://, content://, $DSD_URI_SCHEME://, $WAVPACK_URI_SCHEME:// or $REMOTE_URI_SCHEME:// URI"
                    }
                    require(trackId.isNotBlank()) { "trackId is required" }
                    val metadata = command.optJSONObject("metadata") ?: JSONObject()
                    val artworkBytes = preparedArtworkBytes.remove(commandId)
                    val mediaMetadataBuilder = MediaMetadata.Builder()
                        .setTitle(metadata.optStringOrNull("title"))
                        .setArtist(metadata.optStringOrNull("artist"))
                        .setAlbumTitle(metadata.optStringOrNull("album"))
                    if (artworkBytes != null) {
                        mediaMetadataBuilder.setArtworkData(artworkBytes, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
                    } else {
                        metadata.optStringOrNull("artworkUri")?.let { mediaMetadataBuilder.setArtworkUri(Uri.parse(it)) }
                    }
                    val mediaMetadata = mediaMetadataBuilder.build()
                    val mediaItemBuilder = MediaItem.Builder()
                        .setMediaId(trackId)
                        .setTag(commandId)
                        .setUri(Uri.parse(uri))
                        .setMediaMetadata(mediaMetadata)
                    // Both are served as a WAV stream, so the extractor is chosen by MIME type, not by the extension.
                    if (isConvertedUri(uri)) {
                        mediaItemBuilder.setMimeType("audio/wav")
                    }
                    val mediaItem = mediaItemBuilder.build()
                    endedEventLoadInstance = null
                    stopped = false
                    loadInstance = loadFence.begin(commandId, trackId)
                    expectedLoadCommandId = commandId
                    player.setMediaItem(mediaItem)
                    player.playWhenReady = command.optBoolean("playWhenReady", false)
                    player.prepare()
                    val requestedPosition = command.optLong("positionMs", 0L).coerceAtLeast(0L)
                    if (requestedPosition > 0L) player.seekTo(requestedPosition)
                    pendingLoadAck = ack
                    mainHandler.removeCallbacks(loadAckTimeout)
                    mainHandler.postDelayed(loadAckTimeout, LOAD_ACK_TIMEOUT_MS)
                    if (player.playbackState == Player.STATE_READY) {
                        pendingLoadAck = null
                        loadFence.finish(expectedLoadCommandId)
                        expectedLoadCommandId = null
                        mainHandler.removeCallbacks(loadAckTimeout)
                        sendAck(ack)
                    }
                }
                "play" -> {
                    player.play()
                    stopped = false
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                "pause" -> {
                    player.pause()
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                "seek" -> {
                    player.seekTo(command.optLong("positionMs", 0L).coerceAtLeast(0L))
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                "stop" -> {
                    player.stop()
                    player.clearMediaItems()
                    stopped = true
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                "set_volume" -> {
                    val volume = command.optDouble("volume", 1.0).toFloat().coerceIn(0f, 1f)
                    player.volume = volume
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                "set_capabilities" -> {
                    queueCapabilities = QueueCapabilities(
                        canNext = command.optBoolean("canNext", false),
                        canPrevious = command.optBoolean("canPrevious", false),
                    )
                    (sessionPlayer as? QueueAwarePlayer)?.notifyCapabilitiesChanged()
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                else -> error("Unsupported Android playback operation '${command.optString("op")}'")
            }
        } catch (error: Exception) {
            ack.put("error", error.message ?: "Android playback command failed")
            ack.put("snapshot", snapshot(player))
            sendAck(ack)
        }
    }

    private val preparedArtworkBytes = HashMap<String, ByteArray>()

    private fun dispatchCommand(raw: String) {
        val command = try {
            JSONObject(raw)
        } catch (_: Exception) {
            executeCommand(raw)
            return
        }
        if (command.optString("op") != "load") {
            executeCommand(raw)
            return
        }
        val commandId = command.optString("commandId")
        val requestId = command.optString("requestId")
        val contentUri = command.optString("contentUri")
        val metadata = command.optJSONObject("metadata") ?: JSONObject()
        if (commandId.isBlank() || contentUri.isBlank()) {
            executeCommand(raw)
            return
        }
        if (contentUri.startsWith("$REMOTE_URI_SCHEME:")) {
            // A track on a server starts at once; its cover is fetched afterwards (a round trip or several) and put on the
            // item when it arrives, so a slow server never delays the load.
            executeCommand(raw)
            fetchRemoteArtwork(contentUri, command.optString("trackId"))
            return
        }
        artworkPreparationCommandId = commandId
        artworkPreparationRequestId = requestId
        mainHandler.removeCallbacks(artworkPreparationTimeout)
        mainHandler.postDelayed(artworkPreparationTimeout, ARTWORK_PREPARATION_TIMEOUT_MS)
        try {
            artworkPreparationFuture = artworkExecutor.submit {
                val coverFromRust = metadata.optStringOrNull("artworkDataBase64")?.let {
                    try { android.util.Base64.decode(it, android.util.Base64.DEFAULT) } catch (_: IllegalArgumentException) { null }
                }
                val bytes = loadArtwork(contentUri, metadata.optStringOrNull("artworkUri"), coverFromRust)
                mainHandler.post {
                    if (serviceDestroyed || artworkPreparationCommandId != commandId) return@post
                    mainHandler.removeCallbacks(artworkPreparationTimeout)
                    artworkPreparationCommandId = null
                    artworkPreparationFuture = null
                    if (bytes != null) preparedArtworkBytes[commandId] = bytes
                    executeCommand(raw)
                }
            }
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            mainHandler.removeCallbacks(artworkPreparationTimeout)
            artworkPreparationCommandId = null
            reportEvent(JSONObject().apply {
                put("kind", "ack")
                put("commandId", commandId)
                put("requestId", requestId)
                put("error", "Android artwork worker is busy")
                put("snapshot", snapshot(exoPlayer))
            })
            commandInFlight.set(false)
        }
    }

    /** Put the cover of the remote track [trackId] on the playing item once Rust has read it (see [dispatchCommand]). */
    private fun fetchRemoteArtwork(contentUri: String, trackId: String) {
        try {
            artworkExecutor.submit {
                val bytes = try {
                    DsdBridge.nativeRemoteCover(contentUri)?.takeIf { it.size <= MAX_NOTIFICATION_SOURCE_BYTES }
                        ?.let { createNotificationArtwork(it) }
                } catch (_: Throwable) {
                    null
                }
                if (bytes == null) return@submit
                mainHandler.post {
                    if (serviceDestroyed) return@post
                    val player = exoPlayer ?: return@post
                    val item = player.currentMediaItem ?: return@post
                    if (item.mediaId != trackId) return@post
                    val updated = item.buildUpon()
                        .setMediaMetadata(
                            item.mediaMetadata.buildUpon()
                                .setArtworkData(bytes, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
                                .build(),
                        )
                        .build()
                    player.replaceMediaItem(player.currentMediaItemIndex, updated)
                }
            }
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            // The next track's cover will be tried again; this one stays without a picture.
        }
    }

    private fun sendAck(ack: JSONObject) {
        mainHandler.removeCallbacks(loadAckTimeout)
        ack.put("snapshot", snapshot(exoPlayer))
        reportEvent(ack)
        commandInFlight.set(false)
    }

    private fun reportServiceReady() {
        reportEvent(JSONObject().apply {
            put("v", 1)
            put("kind", "service_ready")
            put("serviceGeneration", serviceGeneration)
            put("snapshot", snapshot(exoPlayer))
        })
    }

    /**
     * The cover for the notification: the picture the platform retriever finds in the file, else [coverFromRust] (a
     * cover the Rust side read itself, for formats such as WavPack whose APEv2 cover the retriever cannot see), else
     * the image at [artworkUri].
     */
    private fun loadArtwork(loadUri: String, artworkUri: String?, coverFromRust: ByteArray? = null): ByteArray? {
        // A DSD track is loaded as moedsd:// and a WavPack track as moewv://; the file (and embedded picture) is the
        // same path as file://.
        val contentUri = when {
            loadUri.startsWith("$DSD_URI_SCHEME:") -> "file:" + loadUri.removePrefix("$DSD_URI_SCHEME:")
            loadUri.startsWith("$WAVPACK_URI_SCHEME:") -> "file:" + loadUri.removePrefix("$WAVPACK_URI_SCHEME:")
            else -> loadUri
        }
        val embedded = try {
            val retriever = android.media.MediaMetadataRetriever()
            try {
                retriever.setDataSource(this, Uri.parse(contentUri))
                retriever.embeddedPicture
            } finally {
                retriever.release()
            }
        } catch (_: Exception) {
            null
        }
        if (embedded != null && embedded.size <= MAX_NOTIFICATION_SOURCE_BYTES) {
            createNotificationArtwork(embedded)?.let { return it }
        }
        if (coverFromRust != null && coverFromRust.size <= MAX_NOTIFICATION_SOURCE_BYTES) {
            createNotificationArtwork(coverFromRust)?.let { return it }
        }
        if (artworkUri.isNullOrBlank()) return null
        return try {
            contentResolver.openInputStream(Uri.parse(artworkUri))?.use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(ARTWORK_COPY_BUFFER_SIZE)
                var total = 0
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    total += count
                    if (total > MAX_NOTIFICATION_SOURCE_BYTES) return null
                    output.write(buffer, 0, count)
                }
                createNotificationArtwork(output.toByteArray())
            }
        } catch (_: Exception) {
            null
        }
    }

    private fun createNotificationArtwork(original: ByteArray): ByteArray? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(original, 0, original.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0 ||
            bounds.outWidth.toLong() * bounds.outHeight.toLong() > MAX_ARTWORK_PIXELS
        ) return null
        var sampleSize = 1
        while (maxOf(bounds.outWidth / sampleSize, bounds.outHeight / sampleSize) > MAX_NOTIFICATION_ARTWORK_SIDE) {
            sampleSize *= 2
        }
        while (sampleSize <= 256) {
            val options = BitmapFactory.Options().apply {
                inSampleSize = sampleSize
                inPreferredConfig = Bitmap.Config.ARGB_8888
            }
            val bitmap = BitmapFactory.decodeStream(ByteArrayInputStream(original), null, options) ?: return null
            val output = ByteArrayOutputStream()
            val format = if (bitmap.hasAlpha()) Bitmap.CompressFormat.PNG else Bitmap.CompressFormat.JPEG
            bitmap.compress(format, NOTIFICATION_JPEG_QUALITY, output)
            bitmap.recycle()
            val bytes = output.toByteArray()
            if (bytes.size <= MAX_NOTIFICATION_ARTWORK_BYTES) return bytes
            sampleSize *= 2
        }
        return null
    }

    private fun reportEndedOnce() {
        val trackId = exoPlayer?.currentMediaItem?.mediaId ?: return
        if (loadInstance == 0L || loadInstance == endedEventLoadInstance) return
        endedEventLoadInstance = loadInstance
        reportEvent(JSONObject().apply {
            put("v", 1)
            put("kind", "ended")
            put("eventId", nextEventId())
            put("trackId", trackId)
            put("mediaKey", exoPlayer?.currentMediaItem?.localConfiguration?.tag?.toString())
            put("loadInstance", loadInstance)
        })
    }

    private fun reportSnapshot() {
        val player = exoPlayer ?: return
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastSnapshotSentAt < SNAPSHOT_MIN_INTERVAL_MS && player.playWhenReady) return
        lastSnapshotSentAt = now
        reportEvent(JSONObject().apply {
            put("v", 1)
            put("kind", "snapshot")
            put("eventId", nextEventId())
            put("snapshot", snapshot(player))
        })
    }

    private fun snapshot(player: Player?): JSONObject {
        val current = player ?: return snapshotEmpty()
        val state = playbackStateName(
            hasError = current.playerError != null,
            ended = current.playbackState == Player.STATE_ENDED,
            isPlaying = current.isPlaying,
            playWhenReady = current.playWhenReady,
            buffering = current.playbackState == Player.STATE_BUFFERING,
            stopped = stopped,
            ready = current.playbackState == Player.STATE_READY,
            hasItem = current.currentMediaItem != null,
        )
        return PlaybackEventEnvelope.snapshot(
            serviceGeneration = serviceGeneration,
            loadInstance = loadInstance,
            trackId = current.currentMediaItem?.mediaId,
            mediaKey = current.currentMediaItem?.localConfiguration?.tag?.toString(),
            state = state,
            positionMs = current.currentPosition.coerceAtLeast(0L),
            durationMs = current.duration.takeIf { it >= 0L },
            volume = current.volume.toDouble(),
            error = current.playerError?.let { remoteLinkLostMessage(it) ?: it.message },
        )
    }

    private fun snapshotEmpty(): JSONObject = PlaybackEventEnvelope.snapshot(
        serviceGeneration = serviceGeneration,
        loadInstance = loadInstance,
        trackId = null,
        mediaKey = null,
        state = "empty",
        positionMs = 0L,
        durationMs = null,
        volume = 1.0,
        error = null,
    )

    private fun reportEvent(event: JSONObject) {
        val suppliedEventId = event.optString("eventId").takeIf { event.has("eventId") && it.isNotBlank() }
        PlaybackEventEnvelope.sequence(event, serviceGeneration, suppliedEventId ?: nextEventId())
        try {
            nativeOnPlaybackEvent(event.toString())
        } catch (_: UnsatisfiedLinkError) {
            // A missing Rust bridge is surfaced by command timeout on the Rust side.
        } catch (_: RuntimeException) {
            // Keep the playback service alive if the bridge is being torn down.
        }
    }

    private fun nextEventId(): String = EVENT_IDS.incrementAndGet().toString()

    private inner class QueueAwarePlayer(delegate: Player) : ForwardingPlayer(delegate) {
        private val listeners = LinkedHashSet<Player.Listener>()

        override fun addListener(listener: Player.Listener) {
            listeners.add(listener)
            super.addListener(listener)
        }

        override fun removeListener(listener: Player.Listener) {
            listeners.remove(listener)
            super.removeListener(listener)
        }

        fun notifyCapabilitiesChanged() {
            val availableCommands = getAvailableCommands()
            listeners.toList().forEach { it.onAvailableCommandsChanged(availableCommands) }
        }

        override fun getAvailableCommands(): Player.Commands {
            val commands = Player.Commands.Builder().addAll(super.getAvailableCommands())
                .remove(Player.COMMAND_CHANGE_MEDIA_ITEMS)
                .remove(Player.COMMAND_SET_MEDIA_ITEM)
                .remove(Player.COMMAND_SET_REPEAT_MODE)
                .remove(Player.COMMAND_SET_SHUFFLE_MODE)
                .remove(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
                .remove(Player.COMMAND_SEEK_TO_NEXT)
                .remove(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                .remove(Player.COMMAND_SEEK_TO_PREVIOUS)
            if (queueCapabilities.allowsNext()) commands.add(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
            if (queueCapabilities.allowsNext()) commands.add(Player.COMMAND_SEEK_TO_NEXT)
            if (queueCapabilities.allowsPrevious()) commands.add(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
            if (queueCapabilities.allowsPrevious()) commands.add(Player.COMMAND_SEEK_TO_PREVIOUS)
            return commands.build()
        }

        override fun isCommandAvailable(command: Int): Boolean = when (command) {
            Player.COMMAND_SEEK_TO_NEXT -> queueCapabilities.allowsNext()
            Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> queueCapabilities.allowsNext()
            Player.COMMAND_SEEK_TO_PREVIOUS -> queueCapabilities.allowsPrevious()
            Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> queueCapabilities.allowsPrevious()
            else -> super.isCommandAvailable(command)
        }

        override fun seekToNextMediaItem() {
            if (queueCapabilities.allowsNext()) reportTransport("next")
        }

        override fun seekToPreviousMediaItem() {
            if (queueCapabilities.allowsPrevious()) reportTransport("previous")
        }

        override fun seekToNext() {
            if (queueCapabilities.allowsNext()) reportTransport("next")
        }

        override fun seekToPrevious() {
            if (queueCapabilities.allowsPrevious()) reportTransport("previous")
        }

        override fun play() {
            super.play()
            stopped = false
            reportTransport("play")
        }

        override fun pause() {
            super.pause()
            reportTransport("pause")
        }

        override fun stop() {
            super.stop()
            stopped = true
            reportTransport("stop")
        }

        override fun seekTo(positionMs: Long) {
            super.seekTo(positionMs)
            reportTransport("seek", positionMs)
        }
    }

    private fun reportTransport(action: String, positionMs: Long? = null) {
        reportEvent(JSONObject().apply {
            put("v", 1)
            put("kind", "transport")
            put("eventId", nextEventId())
            put("action", action)
            if (positionMs != null) put("positionMs", positionMs.coerceAtLeast(0L))
            put("trackId", exoPlayer?.currentMediaItem?.mediaId ?: JSONObject.NULL)
            put("mediaKey", exoPlayer?.currentMediaItem?.localConfiguration?.tag?.toString() ?: JSONObject.NULL)
            put("loadInstance", loadInstance)
        })
    }

    private external fun nativePollPlaybackCommand(): String?
    private external fun nativeOnPlaybackEvent(json: String)

    private fun JSONObject.optStringOrNull(key: String): String? =
        optString(key).takeIf { it.isNotBlank() && it != "null" }

    companion object {
        @Volatile private var activeInstance: PlaybackService? = null

        /** Main thread only. Null while no playback service is running. */
        fun floatingLyricsSample(): FloatingLyricsOverlay.Sample? = activeInstance?.overlaySample()

        private const val ACTIVE_POLL_INTERVAL_MS = 50L
        private const val IDLE_POLL_INTERVAL_MS = 250L
        private const val SNAPSHOT_SAMPLE_INTERVAL_MS = 250L
        private const val SNAPSHOT_MIN_INTERVAL_MS = 200L
        private const val LOAD_ACK_TIMEOUT_MS = 8_000L
        private const val MAX_NOTIFICATION_SOURCE_BYTES = 32 * 1024 * 1024
        private const val MAX_NOTIFICATION_ARTWORK_BYTES = 256 * 1024
        private const val MAX_NOTIFICATION_ARTWORK_SIDE = 320
        private const val MAX_ARTWORK_PIXELS = 64L * 1024 * 1024
        private const val ARTWORK_COPY_BUFFER_SIZE = 32 * 1024
        private const val ARTWORK_PREPARATION_TIMEOUT_MS = 4_000L
        private const val NOTIFICATION_JPEG_QUALITY = 82
        private val SERVICE_GENERATIONS = AtomicLong(0L)
        private val EVENT_IDS = AtomicLong(0L)
    }
}

/**
 * The state name a snapshot reports. Told to play but still filling (a seek, a slow remote read) is "buffering",
 * not a pause: the app keeps the controls in "playing" and only stops counting listening time, so a seek no longer
 * flashes the play button.
 */
internal fun playbackStateName(
    hasError: Boolean,
    ended: Boolean,
    isPlaying: Boolean,
    playWhenReady: Boolean,
    buffering: Boolean,
    stopped: Boolean,
    ready: Boolean,
    hasItem: Boolean,
): String = when {
    hasError -> "error"
    ended -> "ended"
    isPlaying -> "playing"
    playWhenReady && buffering && hasItem -> "buffering"
    stopped -> "stopped"
    ready && hasItem -> "paused"
    hasItem -> "ready"
    else -> "empty"
}
