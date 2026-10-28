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
import androidx.media3.extractor.ExtractorsFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.analytics.AnalyticsListener
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.AudioTrackAudioOutputProvider
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
    // The covers of remote tracks wait on a server, so they have a worker of their own: a local track chosen next must
    // not queue its artwork behind a server that does not answer.
    private val remoteCoverExecutor = ThreadPoolExecutor(
        1,
        1,
        0L,
        TimeUnit.MILLISECONDS,
        ArrayBlockingQueue(1),
        { runnable -> Thread(runnable, "moe-playback-remote-cover").apply { isDaemon = true } },
    )
    private lateinit var pollThread: HandlerThread
    private lateinit var pollHandler: Handler
    private var exoPlayer: ExoPlayer? = null
    /** The chosen output device and what the AudioTrack really uses (ADR 0044). */
    @Volatile private var outputRouter: AudioOutputRouter? = null
    /** Media3's output provider with the USB exclusive output in front (ADR 0047). */
    @Volatile private var usbOutputProvider: UsbAudioOutputProvider? = null
    private var session: MediaSession? = null
    private var sessionPlayer: Player? = null
    private val commandInFlight = AtomicBoolean(false)
    private var queueCapabilities = QueueCapabilities(canNext = false, canPrevious = false)
    private var stopped = false
    private var endedEventLoadInstance: Long? = null
    @Volatile private var pendingLoadAck: JSONObject? = null
    private var expectedLoadCommandId: String? = null
    private var loadInstance = 0L
    private var lastSnapshotSentAt = 0L
    @Volatile private var serviceDestroyed = false
    private var serviceGeneration = 0L
    private val loadFence = PlaybackLoadFence()
    @Volatile private var artworkPreparationCommandId: String? = null
    private var artworkPreparationFuture: java.util.concurrent.Future<*>? = null
    // A new track is switched to only after its file has been opened and recognised (ADR 0036). The check runs on a
    // worker of its own (a file on a server can take seconds, and one that never answers must not block the next
    // check), while the track that plays goes on playing.
    private val preflightExecutor = ThreadPoolExecutor(
        0,
        8,
        20L,
        TimeUnit.SECONDS,
        java.util.concurrent.SynchronousQueue(),
        { runnable -> Thread(runnable, "moe-playback-preflight").apply { isDaemon = true } },
    )
    private val loadGate = LoadPreflightGate()
    private var mediaPreflight: MediaPreflight? = null
    @Volatile private var preflightCommandId: String? = null
    private var preflightRequestId = ""
    private var preflightRemote = false
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
    private val preflightTimeout = Runnable {
        val commandId = preflightCommandId ?: return@Runnable
        preflightCommandId = null
        // Whatever the check answers later is dropped; the choice is refused now.
        loadGate.cancel(commandId)
        rejectLoad(
            commandId,
            preflightRequestId,
            if (preflightRemote) REMOTE_LINK_LOST_MESSAGE else "Opening the file took too long",
        )
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
        // DSD (.dsf), WavPack (.wv) and FLAC (.flac) tracks arrive as moedsd://, moewv:// and moeflac:// URIs (remote
        // ones as moeremote://); they are served as 32-bit float WAV by the Rust converters (ADR 0016, ADR 0025,
        // ADR 0043), so float PCM output is enabled for them.
        val dataSources = DsdAwareDataSourceFactory(DefaultDataSource.Factory(this))
        mediaPreflight = MediaPreflight(dataSources, extractors)
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
                    // Native DSD (ADR 0049): the raw slots of a DSD track must not meet the float converter.
                    .setAudioProcessorChain(FloatAwareAudioProcessorChain { UsbExclusiveEngine.source.nativeDsd })
                    // The default AudioTrack provider, watched so the settings page can show where the track is
                    // really routed (ADR 0044), behind the USB exclusive output, which takes a track only when the
                    // switch is on and the chosen USB device can play it (ADR 0047).
                    .setAudioOutputProvider(
                        UsbAudioOutputProvider(
                            context,
                            RouteObservingOutputProvider(AudioTrackAudioOutputProvider.Builder(context).build()) { track ->
                                outputRouter?.observeTrack(track)
                            },
                        ) { outputRouter?.currentPreference() }.also { usbOutputProvider = it },
                    )
                    .build()
        }.setEnableAudioFloatOutput(true)
            // For the formats Media3 still decodes with MediaCodec (MP3, AAC, Vorbis, Opus…), a decoder that fails to
            // initialise falls back to the next one the device offers.
            .setEnableDecoderFallback(true)
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
                // A track loaded as native DSD whose USB output cannot be used goes on through the PCM converter.
                if (retryDsdThroughPcm(player)) return
                pendingLoadAck?.let { ack ->
                    pendingLoadAck = null
                    loadFence.finish(expectedLoadCommandId)
                    expectedLoadCommandId = null
                    mainHandler.removeCallbacks(loadAckTimeout)
                    ack.put("error", remoteLinkLostMessage(error) ?: error.message ?: "Android audio decoder failed")
                    sendAck(ack)
                }
                reportSnapshot()
            }
        })
        player.addListener(clockListener)
        exoPlayer = player
        val router = AudioOutputRouter(this, mainHandler, { device -> player.setPreferredAudioDevice(device) }, ::readRealMixerRate)
        outputRouter = router
        player.addAnalyticsListener(object : AnalyticsListener {
            override fun onAudioTrackInitialized(
                eventTime: AnalyticsListener.EventTime,
                audioTrackConfig: AudioSink.AudioTrackConfig,
            ) = router.onTrackInitialized(audioTrackConfig)

            override fun onAudioTrackReleased(
                eventTime: AnalyticsListener.EventTime,
                audioTrackConfig: AudioSink.AudioTrackConfig,
            ) = router.onTrackReleased()

            // An audio sink or player error soon after a privileged dump trips the lookup's circuit breaker (ADR 0048).
            override fun onAudioSinkError(eventTime: AnalyticsListener.EventTime, audioSinkError: Exception) =
                router.mixerLookup.symptom("audio sink error")

            override fun onPlayerError(eventTime: AnalyticsListener.EventTime, error: androidx.media3.common.PlaybackException) =
                router.mixerLookup.symptom("player error")
        })
        // The choice lives in settings.json (read by Rust); it applies with no Activity or WebView around.
        router.setPreference(readOutputPreference())
        readUsbExclusive()?.let(UsbExclusiveEngine::setEnabled)
        lastUsbDevices = readUsbDevices()
        router.onDevicesChanged = ::onDevicesChanged
        router.start()
        // A USB audio device already attached and not permitted is asked for as soon as the service is up (foreground only).
        autoAskUsbPermission("service-start")
        // Listen for the Bluetooth codec from the start, so a headset that connects while the app runs is known.
        BluetoothCodecWatch.ensure(this)
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

    /**
     * The playback clock for the GL lyrics overlay (ADR 0032): every event that changes the position's course publishes a
     * [ClockSnapshot] (and wakes the render thread); while the overlay wants it, a poll re-anchors on every step of the
     * player's coarse position. Main thread (the player's thread) only.
     */
    private val clockListener = object : Player.Listener {
        override fun onIsPlayingChanged(isPlaying: Boolean) {
            publishClock(true)
            ensureClockPolling()
        }

        override fun onPlaybackStateChanged(playbackState: Int) = publishClock(true)

        // A pause while buffering changes no `isPlaying`: 「暫停時隱藏」 needs it too.
        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) = publishClock(true)

        override fun onMediaItemTransition(mediaItem: androidx.media3.common.MediaItem?, reason: Int) = publishClock(true)

        override fun onPositionDiscontinuity(
            oldPosition: Player.PositionInfo,
            newPosition: Player.PositionInfo,
            reason: Int,
        ) = publishClock(true)

        override fun onPlaybackParametersChanged(playbackParameters: androidx.media3.common.PlaybackParameters) = publishClock(true)
    }
    private var clockPolling = false
    private val clockPoll = object : Runnable {
        override fun run() {
            if (serviceDestroyed || LyricsClock.demand.get() <= 0 || !playbackActive) {
                clockPolling = false
                return
            }
            publishClock(false)
            mainHandler.postDelayed(this, CLOCK_POLL_MS)
        }
    }

    internal fun ensureClockPolling() {
        if (clockPolling || serviceDestroyed || LyricsClock.demand.get() <= 0 || !playbackActive) return
        clockPolling = true
        mainHandler.postDelayed(clockPoll, CLOCK_POLL_MS)
    }

    internal fun publishClock(wake: Boolean) {
        val player = exoPlayer
        val item = player?.currentMediaItem
        if (player == null || item == null) {
            if (wake) LyricsClock.clear()
            return
        }
        val state = player.playbackState
        val active = state == Player.STATE_READY || state == Player.STATE_BUFFERING
        LyricsClock.publish(
            trackId = item.mediaId,
            positionMs = player.currentPosition,
            playing = player.isPlaying,
            speed = player.playbackParameters.speed,
            active = active,
            durationMs = player.duration.takeIf { it > 0 } ?: FloatingLyricsRules.NO_TIME,
            nowNanos = System.nanoTime(),
            wake = wake,
            // A stepped position was reached somewhere in the last poll interval: anchor in the middle of it.
            anchorBackNanos = if (wake) 0L else CLOCK_POLL_MS * 1_000_000L / 2,
            wantsPlay = player.playWhenReady,
            buffering = state == Player.STATE_BUFFERING,
        )
    }

    override fun onDestroy() {
        if (activeInstance === this) activeInstance = null
        outputRouter?.shutdown()
        mainHandler.removeCallbacks(clockPoll)
        clockPolling = false
        LyricsClock.clear()
        serviceDestroyed = true
        playbackActive = false
        mainHandler.removeCallbacks(loadAckTimeout)
        mainHandler.removeCallbacks(artworkPreparationTimeout)
        artworkPreparationFuture?.cancel(true)
        artworkExecutor.shutdownNow()
        remoteCoverExecutor.shutdownNow()
        preflightExecutor.shutdownNow()
        mainHandler.removeCallbacks(preflightTimeout)
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
        outputRouter?.stop()
        outputRouter = null
        UsbExclusiveEngine.release("播放服務結束")
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
            val acquired = commandInFlight.compareAndSet(false, true)
            // A load that is still being awaited (a slow server) does not hold the next command back: a newer choice of
            // track takes over from it (Rust gave up waiting for the old one).
            if (acquired || PlaybackCommandGate.mayPreempt(
                    pendingLoadAck != null,
                    artworkPreparationCommandId != null,
                    preflightCommandId != null,
                )
            ) {
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
                } else if (acquired) {
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
                            LOCAL_CONVERTED_SCHEMES.any { uri.startsWith("$it:///") } ||
                            uri.startsWith("$REMOTE_URI_SCHEME://"),
                    ) {
                        "Android playback requires a file://, content://, " +
                            LOCAL_CONVERTED_SCHEMES.joinToString { "$it://" } + " or $REMOTE_URI_SCHEME:// URI"
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
                    // A service that started before the settings were read picks the choice up here, before any sound.
                    outputRouter?.setPreference(readOutputPreference())
                    readUsbExclusive()?.let(UsbExclusiveEngine::setEnabled)
                    // A DSD track is played natively (raw slots to the USB device, ADR 0049) or converted to PCM; the
                    // decision is made here, where the route, the permission and the device's descriptors are known.
                    val isDsd = metadata.optBoolean("dsd", false)
                    val dsdDecision = DsdPassthrough.decide(
                        this,
                        readDsdPassthrough() ?: false,
                        uri,
                        isDsd,
                        outputRouter?.currentPreference(),
                        precomputedRawPlans.remove(commandId),
                    )
                    val loadUri = (dsdDecision as? DsdPassthroughRules.Decision.Raw)
                        ?.let { DsdPassthroughRules.rawUriOf(uri) } ?: uri
                    val nativeDsd = loadUri != uri
                    val mediaItemBuilder = MediaItem.Builder()
                        .setMediaId(trackId)
                        .setTag(commandId)
                        .setUri(Uri.parse(loadUri))
                        .setMediaMetadata(mediaMetadata)
                    // A converted track is served as a WAV stream, so the extractor is chosen by MIME type, not by the
                    // extension. Rust says which loads are converted (`player_formats::android_converter`).
                    if (command.optBoolean("converted", false)) {
                        mediaItemBuilder.setMimeType("audio/wav")
                    }
                    val mediaItem = mediaItemBuilder.build()
                    // The same track through the PCM converter, for when the native output cannot be used.
                    dsdPcmFallbackItem = if (nativeDsd) mediaItemBuilder.setUri(Uri.parse(uri)).build() else null
                    // Each track gets its own chance on the USB exclusive output, with its own bit depth (ADR 0047).
                    UsbExclusiveEngine.newTrack(
                        UsbExclusiveRules.sourceOf(
                            metadata.optInt("bitsPerSample", 0),
                            isDsd,
                            nativeDsd,
                        ),
                        (dsdDecision as? DsdPassthroughRules.Decision.Pcm)?.reason,
                    )
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
                "cancel_load" -> {
                    // Rust stopped waiting for a load: if its file is still being checked, it is not switched to.
                    loadGate.cancel(command.optString("targetCommandId"))
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
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
                    // Rust already clamps and turns a seek to the end into the natural end (ADR 0036); keep it in range here too.
                    val duration = player.duration
                    val target = command.optLong("positionMs", 0L).coerceAtLeast(0L)
                    player.seekTo(if (duration != C.TIME_UNSET && duration > 0L) target.coerceAtMost(duration) else target)
                    ack.put("snapshot", snapshot(player))
                    sendAck(ack)
                }
                "stop" -> {
                    player.stop()
                    player.clearMediaItems()
                    UsbExclusiveEngine.release("已停止播放")
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

    /** The PCM-converter form of the DSD track loaded natively, while that is the one playing (ADR 0049). */
    private var dsdPcmFallbackItem: MediaItem? = null

    /**
     * The native DSD output failed (no USB output could be made for the raw slots: the device went away, the mode was
     * switched, the rate was refused): the same track goes on through the DSD-to-PCM converter from where it was. True
     * when it did, so the error is not reported.
     */
    private fun retryDsdThroughPcm(player: ExoPlayer): Boolean {
        val item = dsdPcmFallbackItem ?: return false
        if (!UsbExclusiveEngine.source.nativeDsd) return false
        val reason = UsbExclusiveEngine.trackFallback ?: "USB 獨佔輸出無法使用"
        dsdPcmFallbackItem = null
        UsbExclusiveEngine.dsdPassthroughFailed(reason)
        android.util.Log.w("MoeMediaIndex", "DSD passthrough: reloading the track through the PCM converter: $reason")
        player.setMediaItem(item, player.currentPosition.coerceAtLeast(0L))
        player.prepare()
        return true
    }

    private val preparedArtworkBytes = HashMap<String, ByteArray>()

    /**
     * The file plan of a DSD load (`nativeRawPlan`, ADR 0049), by command id: read on the preflight thread, where a
     * remote header may take a round trip, and used by the decision on the main thread.
     */
    private val precomputedRawPlans = HashMap<String, String>()

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
        loadGate.begin(commandId)
        // A DSD load that may play natively has its file header read on the preflight thread (the setting and the
        // exclusive switch are read the way the load will read them).
        val planUri = Uri.parse(contentUri)
        val wantsPlan = DsdPassthroughRules.wantsFilePlan(
            readDsdPassthrough() ?: false,
            metadata.optBoolean("dsd", false),
            planUri.scheme,
            readUsbExclusive() ?: UsbExclusiveEngine.enabled,
        )
        preflightLoad(commandId, requestId, contentUri, wantsPlan) { rawPlan ->
            precomputedRawPlans.clear()
            if (rawPlan != null) precomputedRawPlans[commandId] = rawPlan
            continueLoad(raw, commandId, requestId, contentUri, command, metadata)
        }
    }

    /**
     * Check that [contentUri] opens (and its container is recognised) before anything of the player is touched; then
     * [onPassed] goes on to switch tracks. A refused load is answered with an error that says the track that plays was
     * left alone ([rejectLoad]); a load that a newer one replaced, or Rust gave up on, is dropped without an answer.
     */
    private fun preflightLoad(
        commandId: String,
        requestId: String,
        contentUri: String,
        wantsPlan: Boolean,
        onPassed: (rawPlan: String?) -> Unit,
    ) {
        val preflight = mediaPreflight
        if (preflight == null) {
            onPassed(null)
            return
        }
        preflightCommandId = commandId
        preflightRequestId = requestId
        preflightRemote = contentUri.startsWith("$REMOTE_URI_SCHEME:")
        mainHandler.removeCallbacks(preflightTimeout)
        mainHandler.postDelayed(preflightTimeout, PREFLIGHT_TIMEOUT_MS)
        try {
            preflightExecutor.execute {
                val result = try {
                    preflight.check(Uri.parse(contentUri))
                } catch (error: Throwable) {
                    PreflightResult.Failed(error.message ?: "Cannot open the file", serverUnreachable = false)
                }
                // The file is known to open: its header decides whether it can be carried natively. A failure here is
                // not a failed load; the track then plays through the PCM converter, as the decision's reason says.
                val rawPlan = if (wantsPlan && result is PreflightResult.Passed) {
                    try {
                        DsdPassthrough.readFilePlan(Uri.parse(contentUri))
                    } catch (error: Throwable) {
                        android.util.Log.w("MoeMediaIndex", "DSD passthrough: reading the file header failed: $error")
                        null
                    }
                } else {
                    null
                }
                mainHandler.post {
                    if (serviceDestroyed) return@post
                    if (preflightCommandId == commandId) {
                        preflightCommandId = null
                        mainHandler.removeCallbacks(preflightTimeout)
                    }
                    val wanted = loadGate.mayCommit(commandId)
                    loadGate.finish(commandId)
                    if (!wanted) return@post
                    when (result) {
                        is PreflightResult.Passed -> onPassed(rawPlan)
                        is PreflightResult.Failed -> rejectLoad(
                            commandId,
                            requestId,
                            if (result.serverUnreachable) REMOTE_LINK_LOST_MESSAGE else result.message,
                        )
                    }
                }
            }
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            preflightCommandId = null
            mainHandler.removeCallbacks(preflightTimeout)
            rejectLoad(commandId, requestId, "Android file check is busy")
        }
    }

    /** Answer a load that was refused before the player was touched: `retained` says the track that plays is untouched. */
    private fun rejectLoad(commandId: String, requestId: String, message: String) {
        reportEvent(JSONObject().apply {
            put("v", 1)
            put("kind", "ack")
            put("commandId", commandId)
            put("requestId", requestId)
            put("serviceGeneration", serviceGeneration)
            put("error", message)
            put("retained", true)
            put("snapshot", snapshot(exoPlayer))
        })
        commandInFlight.set(false)
    }

    private fun continueLoad(
        raw: String,
        commandId: String,
        requestId: String,
        contentUri: String,
        command: JSONObject,
        metadata: JSONObject,
    ) {
        if (contentUri.startsWith("$REMOTE_URI_SCHEME:")) {
            // A track on a server starts at once; its cover is fetched afterwards (a round trip or several) and put on the
            // item when it arrives, so a slow server never delays the load.
            executeCommand(raw)
            fetchRemoteArtwork(contentUri, command.optString("trackId"))
            return
        }
        // A load whose artwork is still being prepared is replaced by this one.
        artworkPreparationFuture?.cancel(true)
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
            remoteCoverExecutor.submit {
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
     * The cover for the notification: [coverFromRust] (read by the one embedded-cover reader, every container, local or
     * remote), else the picture the platform retriever finds in the file, else the image at [artworkUri].
     */
    private fun loadArtwork(loadUri: String, artworkUri: String?, coverFromRust: ByteArray? = null): ByteArray? {
        // A DSD, WavPack or FLAC track is loaded as moedsd://, moewv:// or moeflac://; the file (and embedded picture)
        // is the same path as file://.
        // The cover Rust read with the one embedded-cover reader (every container, local or remote) comes first; the
        // platform retriever is only the fallback for a file that reader could not give a picture.
        if (coverFromRust != null && coverFromRust.size <= MAX_NOTIFICATION_SOURCE_BYTES) {
            createNotificationArtwork(coverFromRust)?.let { return it }
        }
        val contentUri = fileUriOfConverted(loadUri)
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

    /** The output device choice the Rust side keeps (ADR 0044). */
    private fun readOutputPreference(): PreferenceReading = try {
        AudioOutputRules.parsePreference(nativeOutputDevicePreference())
    } catch (_: UnsatisfiedLinkError) {
        PreferenceReading.Unknown
    } catch (_: RuntimeException) {
        PreferenceReading.Unknown
    }

    /**
     * Main thread. The USB exclusive switch or the chosen output device changes to [after] ([apply] stores it); the
     * change reaches the track that is playing at once (ADR 0047): the output Media3 plays through is retired, so
     * Media3 re-creates it through the provider (position and play/pause state kept), and a device the app holds that
     * is no longer wanted is handed back first.
     */
    private fun changeOutput(after: UsbExclusiveRules.Wish, what: String, apply: () -> Unit) {
        val before = UsbExclusiveRules.Wish(UsbExclusiveEngine.enabled, outputRouter?.currentPreference())
        // The choice is made again for this track: an earlier fallback reason no longer holds.
        if (before != after) UsbExclusiveEngine.trackFallback = null
        // Stored first, so the output Media3 opens next is already chosen by the new state.
        apply()
        if (before != after) applyRoute(after, "$what changed (${describeWish(before)} → ${describeWish(after)})")
    }

    /** The attached USB audio devices as last seen, to tell an attach or an unplug from other device changes. */
    private var lastUsbDevices: List<UsbExclusiveRules.UsbDeviceFacts> = emptyList()

    private fun readUsbDevices(): List<UsbExclusiveRules.UsbDeviceFacts> =
        UsbExclusiveEngine.usbDevices(
            getSystemService(android.content.Context.USB_SERVICE) as? android.hardware.usb.UsbManager,
        )

    /**
     * Main thread. The attached devices changed (the router's callback, or the app took or gave back its device): in
     * the exclusive mode a USB audio device that was attached or unplugged changes where the playing track goes at
     * once (attached → exclusive on it now; unplugged → shared on the system default).
     */
    private fun onDevicesChanged() {
        val before = lastUsbDevices
        val after = readUsbDevices()
        lastUsbDevices = after
        val wish = UsbExclusiveRules.Wish(UsbExclusiveEngine.enabled, outputRouter?.currentPreference())
        if (!UsbExclusiveRules.usbDevicesChanged(wish, before, after)) {
            autoAskUsbPermission("attach")
            return
        }
        // Another device: the track gets a new chance on it.
        UsbExclusiveEngine.trackFallback = null
        applyRoute(wish, "USB audio devices changed (${before.map { it.key }} → ${after.map { it.key }})")
        autoAskUsbPermission("attach")
    }

    /**
     * Main thread. In the exclusive mode, the USB audio device the choice plays on is attached and not permitted: Android's
     * permission dialog is requested at once (ADR 0047, [UsbPermissionAsk]); a device the user refused is not asked again
     * until it is attached again, and nothing is asked while the app is in the background.
     */
    private fun autoAskUsbPermission(trigger: String) {
        UsbPermissionAsk.check(this, UsbExclusiveRules.Wish(UsbExclusiveEngine.enabled, outputRouter?.currentPreference()), trigger)
    }

    /** Re-create the output and hand the held device back as the routing table says for [wish] now (ADR 0047). */
    private fun applyRoute(wish: UsbExclusiveRules.Wish, what: String) {
        val provider = usbOutputProvider
        val devices = readUsbDevices()
        lastUsbDevices = devices
        val route = UsbExclusiveRules.route(wish, devices)
        val target = (route as? UsbExclusiveRules.Route.Exclusive)?.device?.key
        val change = UsbExclusiveRules.outputChange(
            liveOn = provider?.liveOn,
            claimed = UsbExclusiveEngine.claimed,
            target = target,
            hasOutput = provider?.hasLiveOutput == true,
        )
        if (change.newOutput) provider?.retireCurrent()
        if (change.handBack) UsbExclusiveEngine.release("已變更播放裝置或輸出模式")
        android.util.Log.i(
            "MoeMediaIndex",
            "USB exclusive: $what; route ${describeRoute(route)}; " +
                (if (change.newOutput) "re-creating the output now" else "the output stays") +
                (if (change.handBack) ", device handed back" else ""),
        )
    }

    private fun describeWish(wish: UsbExclusiveRules.Wish): String =
        (if (wish.enabled) "exclusive" else "shared") + " " +
            (wish.chosen?.let { "${it.kind}:${it.productName}" } ?: "system default")

    private fun describeRoute(route: UsbExclusiveRules.Route): String = when (route) {
        is UsbExclusiveRules.Route.Exclusive -> "exclusive on ${route.device.key} (${route.device.productName})"
        is UsbExclusiveRules.Route.Shared -> if (route.noUsbDevice) "shared (no USB audio device)" else "shared"
    }

    /** The USB exclusive switch (ADR 0047), or null while the settings have not been read yet. */
    private fun readUsbExclusive(): Boolean? = try {
        when (nativeUsbExclusiveEnabled()) {
            1 -> true
            0 -> false
            else -> null
        }
    } catch (_: UnsatisfiedLinkError) {
        null
    }

    /** The DSD native passthrough switch (`dsd.nativePassthrough`, ADR 0049), or null while the settings have not been read yet. */
    private fun readDsdPassthrough(): Boolean? = try {
        when (nativeDsdPassthroughEnabled()) {
            1 -> true
            0 -> false
            else -> null
        }
    } catch (_: UnsatisfiedLinkError) {
        null
    }

    private external fun nativePollPlaybackCommand(): String?
    private external fun nativeOutputDevicePreference(): String?
    private external fun nativeUsbExclusiveEnabled(): Int
    private external fun nativeDsdPassthroughEnabled(): Int

    /** The hidden `androidRealMixerRate` switch (ADR 0048): -1 not read yet, 0 off, 1 on. */
    private external fun nativeRealMixerRateEnabled(): Int

    /** On unless settings.json turns it off; before the settings are read it is on (nothing runs without a granted source). */
    private fun readRealMixerRate(): Boolean = try {
        nativeRealMixerRateEnabled() != 0
    } catch (_: UnsatisfiedLinkError) {
        true
    }
    private external fun nativeOnPlaybackEvent(json: String)

    private fun JSONObject.optStringOrNull(key: String): String? =
        optString(key).takeIf { it.isNotBlank() && it != "null" }

    companion object {
        @Volatile private var activeInstance: PlaybackService? = null

        /** Main thread. The GL overlay was created: publish the clock now and keep it fresh while playing. */
        internal fun startClockFeed() {
            val service = activeInstance ?: return
            service.publishClock(true)
            service.ensureClockPolling()
        }

        /** Main thread. The user chose another output device (ADR 0044); a service that is not running reads it when it starts. */
        internal fun applyOutputPreference(preference: OutputDevicePreference?) {
            val service = activeInstance ?: return
            service.changeOutput(UsbExclusiveRules.Wish(UsbExclusiveEngine.enabled, preference), "chosen output device") {
                service.outputRouter?.setPreference(PreferenceReading.Known(preference))
            }
            service.autoAskUsbPermission("preference")
        }

        /** Main thread. The app came to the foreground (or a trigger fired): ask for a pending USB permission now (ADR 0047). */
        internal fun requestUsbPermissionIfNeeded(trigger: String) {
            activeInstance?.autoAskUsbPermission(trigger)
        }

        /**
         * Main thread. The user switched the output mode (ADR 0047, true: 獨佔); it applies to the track that is
         * playing at once, by the routing table: exclusive → the output is re-created on the USB device the choice
         * plays on; shared → a device the app holds is handed back at once and the track goes on through Android.
         */
        internal fun applyUsbExclusive(enabled: Boolean) {
            val service = activeInstance
            if (service == null) {
                UsbExclusiveEngine.setEnabled(enabled)
                return
            }
            val chosen = service.outputRouter?.currentPreference()
            service.changeOutput(UsbExclusiveRules.Wish(enabled, chosen), "output mode") {
                UsbExclusiveEngine.setEnabled(enabled)
            }
            service.autoAskUsbPermission("mode")
        }

        /** The panel asked to check the privileged sources again: the current track is looked up again (rate-limited). */
        internal fun recheckMixerRate() {
            activeInstance?.outputRouter?.mixerLookup?.recheck()
        }

        /** Main thread. USB permission was just granted: the playing track tries the exclusive output again now. */
        internal fun usbPermissionGranted() {
            val service = activeInstance ?: return
            UsbExclusiveEngine.trackFallback = null
            service.applyRoute(
                UsbExclusiveRules.Wish(UsbExclusiveEngine.enabled, service.outputRouter?.currentPreference()),
                "USB permission granted",
            )
        }

        /** Main thread. The output devices for the settings page; [saved] is the choice in settings.json. */
        internal fun audioOutputStatus(context: android.content.Context, saved: OutputDevicePreference?): JSONObject {
            val status = activeInstance?.outputRouter?.status(saved) ?: AudioOutputRouter.idleStatus(context, saved)
            // The Bluetooth codec of each A2DP device, when the stack has told us (needs BLUETOOTH_CONNECT).
            BluetoothCodecWatch.ensure(context)
            status.put("bluetoothPermission", BluetoothCodecWatch.permissionState(context))
            status.put("bluetoothCodecs", JSONObject(BluetoothCodecWatch.snapshot() as Map<*, *>))
            // Where the exclusive mode plays the saved choice (the USB device, or none attached; ADR 0047).
            status.optJSONObject("exclusive")?.put("route", UsbExclusiveEngine.routeJson(context, saved))
            return status
        }

        private const val CLOCK_POLL_MS = 40L

        private const val ACTIVE_POLL_INTERVAL_MS = 50L
        private const val IDLE_POLL_INTERVAL_MS = 250L
        private const val SNAPSHOT_SAMPLE_INTERVAL_MS = 250L
        private const val SNAPSHOT_MIN_INTERVAL_MS = 200L
        private const val LOAD_ACK_TIMEOUT_MS = 8_000L
        // Shorter than the wait of the Rust side (15 s), so the refusal arrives before Rust gives up.
        private const val PREFLIGHT_TIMEOUT_MS = 12_000L
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
