package com.moemusicplayer.mediaindex

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.hardware.input.InputManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.util.TypedValue
import android.view.Choreographer
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import java.text.BreakIterator
import java.util.Locale

import org.json.JSONObject

/**
 * Android floating lyrics (ADR 0032): a TYPE_APPLICATION_OVERLAY window with native text. The page pushes the look
 * settings and the current track's timed lines; the line is picked every 100 ms from the playback service's own
 * player position, so it keeps going while the WebView is in the background. Move mode: dragged by touch, with a lock
 * button. Fixed mode: FLAG_NOT_TOUCHABLE (touches pass through, so the window is drawn at most 80% opaque as Android
 * 12+ requires) and unlocked from the player's settings only. The position and the lock state live on the phone
 * (SharedPreferences). The user grants "display over other apps" in the system settings page.
 * Main thread only.
 */
object FloatingLyricsOverlay {
    /**
     * What the playback service reports for the overlay. One instance is refilled every frame (no allocation per
     * frame); `durationMs` is [FloatingLyricsRules.NO_TIME] while the track length is unknown.
     */
    class Sample {
        var trackId: String = ""
        var positionMs = 0L
        var durationMs = FloatingLyricsRules.NO_TIME
        var active = false
        var playing = false

        fun set(trackId: String, positionMs: Long, durationMs: Long, active: Boolean, playing: Boolean) {
            this.trackId = trackId
            this.positionMs = positionMs
            this.durationMs = durationMs
            this.active = active
            this.playing = playing
        }
    }

    /**
     * A lyric line; for a word-timed one (ADR 0041) its words' times (lyrics ms) and UTF-16 ranges `(start, end)` in
     * the trimmed `text`.
     */
    internal class Line(
        val startMs: Long,
        val text: String,
        val translation: String,
        val wordTimes: List<Pair<Double, Double>>? = null,
        val wordRanges: List<Pair<Int, Int>>? = null,
    ) {
        /** Trimmed once here, not every frame. */
        val trimmedText: String = text.trim()
        val trimmedTranslation: String = translation.trim()
    }

    /** The page's `data` (track id, offset, lines) parsed off the main thread; applied with [applyParsed]. */
    internal class ParsedData(
        val trackId: String?,
        val offsetMs: Long,
        val durationMs: Long,
        val lines: List<Line>,
        val starts: LongArray,
    )

    /** The theme accent (karaoke in the none mode, ADR 0041), pushed by the page as `accentColor`. */
    private var accentColor = 0xffa9c3fe.toInt()

    private const val PREFS = "moe_floating_lyrics"
    private const val TICK_MS = 100L

    /**
     * Hidden developer switch (system property, read when the overlay window is created): `views` forces the Views
     * renderer, `gl` the OpenGL one. Without it the GL renderer is used, and the Views one after GL failed (ADR 0032).
     */
    private const val RENDERER_PROPERTY = "debug.moe.lyrics.renderer"
    private const val TAG = "MoeMediaIndex"

    /** The GL renderer's panel while the window is a GL one; null for the Views renderer. */
    private var glPanel: GlLyricsPanel? = null
    /** GL failed once in this process: the Views renderer is used from now on (unless the property forces `gl`). */
    private var glFailed = false
    /** The next window is a Views one whatever the property says (set while falling back from a GL failure). */
    private var viewsNext = false
    /** Some line has a translation: with the switch on, the GL window reserves the translation row for the whole track. */
    private var linesHaveTranslation = false


    private val main = Handler(Looper.getMainLooper())
    private var appContext: Context? = null

    private var enabled = false
    private var align = "center"
    private var fontSizePx = 28
    private var translationScalePercent = 80
    /** Weight of the original text (this tab's own `fontWeight`); the translation row is 50 lighter. */
    private var fontWeight = FloatingLyricsRules.DEFAULT_FONT_WEIGHT
    private var useCustomColor = false
    private var customColor = "#00e1ff"
    private var showTranslation = true
    private var textEffect = "shadow"
    /** "none": one line at a time; "continuous": all lines in one strip that scrolls with playback (ADR 0032). */
    private var animation = "none"
    // The strip of the continuous animation, rebuilt when the lyrics or the look change (see stripFor).
    private class Strip(
        val starts: LongArray,
        val texts: List<String>,
        val translations: List<String>,
        val cells: FloatArray,
        val widths: FloatArray,
        val mainWidths: FloatArray,
        val translationWidths: FloatArray,
        val lastGlyphCenters: FloatArray,
        val firstGlyphCenters: FloatArray,
        /** Word-timed lyrics (ADR 0041 D10): the word glide's knots (lyrics ms, strip px); null for line-timed lyrics (the line glide). */
        val wordKnots: WordLyricsRules.WordKnots?,
        /** Grapheme count of the last line (the terminal glide's duration estimate), counted once. */
        val lastGraphemes: Int,
        val hasTranslation: Boolean,
    ) {
        /** The line glide's knots for the terminal arrival `knotsEnd` they were built with (rebuilt only if that changes). */
        var knots: FloatingLyricsRules.StripKnots? = null
        var knotsEnd = FloatingLyricsRules.NO_TIME
        var knotsBuilt = false
    }
    private var strip: Strip? = null
    // What the strip was built from (primitives; a mismatch rebuilds it): the lines, the translation switch and the two
    // rows' font measures. `stripKeyValid` is also true for "no usable lines" (strip stays null).
    private var stripKeyValid = false
    private var stripKeyLines = 0
    private var stripKeyTranslation = false
    private var stripKeyMain = 0
    private var stripKeySecond = 0
    private var linesVersion = 0
    // What the neighbour images of the none mode were last prepared for.
    private var prefetchValid = false
    private var prefetchIndex = 0
    private var prefetchLinesVersion = 0
    private var prefetchEpoch = 0
    private var prefetchTranslation = false
    private var prefetchLocked = false
    private val prefetchMain = ArrayList<String>()
    private val prefetchAccent = ArrayList<String>()
    private val prefetchSecond = ArrayList<String>()
    private val sample = Sample()
    /** The floating settings' own pair: "none" draws the line at main; "continuous" feeds both into the static mask. */
    private var mainOpacityPercent = FloatingLyricsRules.DEFAULT_MAIN_OPACITY_PERCENT
    private var secondaryOpacityPercent = FloatingLyricsRules.DEFAULT_SECONDARY_OPACITY_PERCENT
    private var widthDp = 800
    private var landscapeWidthDp = 800
    private var rotationWatch: android.content.ComponentCallbacks? = null
    // The position carried on between ExoPlayer's coarse reports (FloatingLyricsRules.smoothPositionMs).
    // Times are in (fractional) ms of the display frame's own clock (Choreographer's frame time, System.nanoTime base).
    private var smoothedMs = 0.0
    private var hasSmoothed = false
    private var smoothedAt = 0.0
    private var lastReportedMs = Long.MIN_VALUE
    private var lastReportedAt = 0.0
    private var lastReportedTrack: String? = null
    private var shownLandscape: Boolean? = null

    private var trackId: String? = null
    private var offsetMs = 0L
    private var trackDurationMs = FloatingLyricsRules.NO_TIME
    private var lines: List<Line> = emptyList()
    private var starts = LongArray(0)

    private var root: FrameLayout? = null
    private var params: WindowManager.LayoutParams? = null
    private var mainText: LyricLineView? = null
    private var translationText: LyricLineView? = null
    private var lockButton: ImageButton? = null

    /** Every display frame while a line slides during playback, otherwise every 100 ms (line changes, pauses). */
    private var frameScheduled = false
    private var tickScheduled = false
    private val choreographer: Choreographer by lazy(LazyThreadSafetyMode.NONE) { Choreographer.getInstance() }
    private val frame = Choreographer.FrameCallback { frameTimeNanos ->
        frameScheduled = false
        tickOnce(frameTimeNanos)
    }
    private val tick = Runnable {
        tickScheduled = false
        tickOnce(System.nanoTime())
    }

    /** `nowNanos`: the display frame's time when called from a frame, otherwise the current time (same clock). */
    private fun tickOnce(nowNanos: Long) {
        if (tickScheduled) {
            main.removeCallbacks(tick)
            tickScheduled = false
        }
        if (root == null) return
        val sliding = if (LyricsTrace.on) {
            LyricsTrace.frameBoundary()
            val section = LyricsTrace.begin("moeLyrics.render")
            try {
                render(nowNanos)
            } finally {
                LyricsTrace.end(section, true)
            }
        } else {
            render(nowNanos)
        }
        if (root == null) return
        if (sliding) {
            if (!frameScheduled) {
                frameScheduled = true
                choreographer.postFrameCallback(frame)
            }
        } else {
            tickScheduled = true
            main.postDelayed(tick, TICK_MS)
        }
    }

    private fun stopTicking() {
        main.removeCallbacks(tick)
        tickScheduled = false
        if (frameScheduled) {
            choreographer.removeFrameCallback(frame)
            frameScheduled = false
        }
    }

    fun canDrawOverlays(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.M || Settings.canDrawOverlays(context)

    // The lock state is read from SharedPreferences once (a synchronized read) and then kept here; every write goes
    // through setLockedState. Nothing else writes the "locked" key.
    @Volatile
    private var lockedCache = false
    @Volatile
    private var lockedLoaded = false

    fun isLocked(context: Context): Boolean {
        if (!lockedLoaded) {
            lockedCache = prefs(context).getBoolean("locked", false)
            lockedLoaded = true
        }
        return lockedCache
    }

    private fun setLockedState(context: Context, locked: Boolean) {
        lockedCache = locked
        lockedLoaded = true
        prefs(context).edit().putBoolean("locked", locked).apply()
    }

    fun status(context: Context): JSONObject = JSONObject()
        .put("canDrawOverlays", canDrawOverlays(context))
        .put("locked", isLocked(context))
        .put("screenWidthDp", Math.round(screenSize(context).first / context.resources.displayMetrics.density))

    /**
     * The page's `data` turned into the overlay's model. Heavy for long lyrics, so the plugin command calls it on its
     * own thread and hands the result to [update]; nothing here touches the views.
     */
    internal fun parseData(data: JSONObject): ParsedData {
        val started = if (LyricsTrace.on) LyricsTrace.begin("moeLyrics.applyData") else 0L
        try {
            val trackId = if (data.isNull("trackId")) null else data.optString("trackId")
            val durationMs = if (data.isNull("durationMs")) FloatingLyricsRules.NO_TIME else data.optLong("durationMs").takeIf { it > 0 } ?: FloatingLyricsRules.NO_TIME
            val array = data.optJSONArray("lines")
            val parsed = ArrayList<Line>()
            if (array != null) {
                for (index in 0 until array.length()) {
                    val item = array.optJSONObject(index) ?: continue
                    val text = item.optString("text", "")
                    var times: List<Pair<Double, Double>>? = null
                    var ranges: List<Pair<Int, Int>>? = null
                    item.optJSONArray("words")?.let { words ->
                        // The words spell the untrimmed line; `lead` UTF-16 units were trimmed off its front.
                        val lead = item.optInt("lead", 0)
                        var at = 0
                        val t = ArrayList<Pair<Double, Double>>()
                        val r = ArrayList<Pair<Int, Int>>()
                        for (w in 0 until words.length()) {
                            val word = words.optJSONObject(w) ?: continue
                            val len = word.optString("t", "").length
                            t.add(word.optDouble("s") to word.optDouble("e"))
                            r.add((at - lead).coerceIn(0, text.length) to (at + len - lead).coerceIn(0, text.length))
                            at += len
                        }
                        if (t.isNotEmpty()) {
                            times = t
                            ranges = r
                        }
                    }
                    parsed.add(Line(item.optLong("startMs"), text, item.optString("translation", ""), times, ranges))
                }
            }
            parsed.sortBy { it.startMs }
            return ParsedData(trackId, data.optLong("offsetMs", 0L), durationMs, parsed, LongArray(parsed.size) { parsed[it].startMs })
        } finally {
            if (LyricsTrace.on) LyricsTrace.end(started, false)
        }
    }

    /** `{ settings?, data?, setLocked?, resetPosition? }` from the page; `parsed` is `data` already parsed by [parseData]. */
    internal fun update(context: Context, args: JSONObject, parsed: ParsedData? = null) {
        appContext = context.applicationContext
        args.optJSONObject("settings")?.let(::applySettings)
        (parsed ?: args.optJSONObject("data")?.let(::parseData))?.let(::applyParsed)
        if (args.has("accentColor") && !args.isNull("accentColor")) {
            FloatingLyricsRules.parseHexColor(args.optString("accentColor"))?.let {
                accentColor = 0xff000000.toInt() or it
                mainText?.setAccent(accentColor)
            }
        }
        if (args.has("setLocked") && !args.isNull("setLocked")) {
            setLockedState(context, args.optBoolean("setLocked"))
        }
        if (args.optBoolean("moveToStatusBar", false)) moveToStatusBar(context)
        if (args.has("centreAxis") && !args.isNull("centreAxis")) {
            centreAxis(context, args.optString("centreAxis") == "horizontal")
        }
        if (args.has("nudgeX") || args.has("nudgeY")) {
            // A held D-pad arrow repeats with save = false and sends savePosition once released.
            nudge(context, args.optInt("nudgeX", 0), args.optInt("nudgeY", 0), args.optBoolean("save", true))
        }
        if (args.optBoolean("savePosition", false)) params?.let { savePosition(context, it) }
        if (args.optBoolean("resetPosition", false)) {
            // Only the current orientation's position; the other one keeps its own.
            val (keyX, keyY) = positionKeys(context)
            prefs(context).edit().remove(keyX).remove(keyY).apply()
            centre()
        }
        refresh()
    }

    /** Move the overlay to the middle of the screen (the saved position was cleared by the caller). */
    private fun centre() {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val (screenWidth, screenHeight) = screenSize(context)
        // The lyric area (not the lock gutter, not the rooms) at the centre.
        layout.x = FloatingLyricsRules.lyricCentredStart(screenWidth, layout.width, lockPx(context) + 2 * marginPx(context), marginPx(context))
        layout.y = FloatingLyricsRules.centredStart(screenHeight, areaHeightPx(context)) - panelTopRoom
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
    }

    /** Show, restyle or remove the overlay (also after the permission was granted). */
    fun refresh() {
        val context = appContext ?: return
        if (!enabled || !canDrawOverlays(context)) {
            remove()
            return
        }
        if (root == null) create(context)
        applyLook()
        applyMode()
        render()
        pushGl(context)
    }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun applySettings(settings: JSONObject) {
        enabled = settings.optBoolean("enabled", false)
        align = settings.optString("align", "center")
        fontSizePx = settings.optInt("fontSizePx", 28).coerceIn(1, 50)
        translationScalePercent = settings.optInt("translationScalePercent", 80).coerceIn(1, 100)
        fontWeight = FloatingLyricsRules.fontWeight(settings.optInt("fontWeight", FloatingLyricsRules.DEFAULT_FONT_WEIGHT))
        useCustomColor = settings.optBoolean("useCustomColor", false)
        customColor = settings.optString("customColor", "#00e1ff")
        showTranslation = settings.optBoolean("showTranslation", true)
        textEffect = settings.optString("textEffect", "shadow")
        animation = if (settings.optString("animation", "none") == "continuous") "continuous" else "none"
        mainOpacityPercent = FloatingLyricsRules.mainOpacityPercent(settings.optInt("mainOpacityPercent", FloatingLyricsRules.DEFAULT_MAIN_OPACITY_PERCENT))
        secondaryOpacityPercent = FloatingLyricsRules.secondaryOpacityPercent(settings.optInt("secondaryOpacityPercent", FloatingLyricsRules.DEFAULT_SECONDARY_OPACITY_PERCENT))
        widthDp = FloatingLyricsRules.widthDp(settings.optInt("widthPx", 800))
        landscapeWidthDp = FloatingLyricsRules.widthDp(settings.optInt("landscapeWidthPx", 800))
    }

    private fun applyParsed(data: ParsedData) {
        trackId = data.trackId
        offsetMs = data.offsetMs
        trackDurationMs = data.durationMs
        lines = data.lines
        starts = data.starts
        linesHaveTranslation = data.lines.any { it.trimmedTranslation.isNotEmpty() }
        linesVersion++
    }

    private fun dp(context: Context, value: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), context.resources.displayMetrics).toInt()

    /** The whole display (status bar and navigation bar included): the overlay may sit over the status bar. */
    private fun screenSize(context: Context): Pair<Int, Int> {
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bounds = windowManager.maximumWindowMetrics.bounds
            return Pair(bounds.width(), bounds.height())
        }
        val metrics = android.util.DisplayMetrics()
        @Suppress("DEPRECATION") windowManager.defaultDisplay.getRealMetrics(metrics)
        return Pair(metrics.widthPixels, metrics.heightPixels)
    }

    private fun statusBarHeight(context: Context): Int {
        val id = context.resources.getIdentifier("status_bar_height", "dimen", "android")
        return if (id > 0) context.resources.getDimensionPixelSize(id) else dp(context, 24)
    }

    /** Top cutout (camera) as left..right in px, when the display reports one. */
    private fun topCutout(): Pair<Int, Int>? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) return null
        val rect = root?.rootWindowInsets?.displayCutout?.boundingRects?.firstOrNull { it.top <= 0 } ?: return null
        return Pair(rect.left, rect.right)
    }

    /**
     * 「移動到狀態列」: only a move, never a different look. The font, translation line and panel sizing stay the user's;
     * the panel goes where the lyric area's vertical centre (the panel's, as the area is centred in it) is the status
     * bar's, beside the cutout. Depends only on the current size, so every tap lands the same. Saved per orientation.
     */
    private fun moveToStatusBar(context: Context) {
        val view = root ?: return
        view.post {
            val layout = params ?: return@post
            if (root !== view) return@post
            val (screenWidth, _) = screenSize(context)
            layout.x = FloatingLyricsRules.statusBarX(screenWidth, layout.width, topCutout(), dp(context, 4))
            layout.y = FloatingLyricsRules.centredOnBar(statusBarHeight(context), areaHeightPx(context)) - panelTopRoom
            savePosition(context, layout)
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
            } catch (_: Exception) {
            }
        }
    }

    /** The 面板 arrows: one px in any direction, saved for the current orientation (status bar area included). */
    private fun nudge(context: Context, dx: Int, dy: Int, save: Boolean) {
        val view = root ?: return
        val layout = params ?: return
        if (dx == 0 && dy == 0) return
        val (screenWidth, screenHeight) = screenSize(context)
        val (x, y) = clampToText(
            layout.x + dx.coerceIn(-1, 1), layout.y + dy.coerceIn(-1, 1), layout.width, screenWidth, screenHeight,
        )
        layout.x = x
        layout.y = y
        if (save) savePosition(context, layout)
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
    }

    /** 「水平置中」 / 「垂直置中」: centre on one axis of the screen, keep the other; saved for this orientation. */
    private fun centreAxis(context: Context, horizontal: Boolean) {
        val view = root ?: return
        val layout = params ?: return
        val (screenWidth, screenHeight) = screenSize(context)
        if (horizontal) layout.x = FloatingLyricsRules.lyricCentredStart(screenWidth, layout.width, lockPx(context) + 2 * marginPx(context), marginPx(context))
        else layout.y = FloatingLyricsRules.centredStart(screenHeight, areaHeightPx(context)) - panelTopRoom
        savePosition(context, layout)
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
    }

    /** `FloatingLyricsRules.clampWindowToArea` for this panel: only the lyric area must stay on screen. */
    private fun clampToText(x: Int, y: Int, width: Int, screenWidth: Int, screenHeight: Int): Pair<Int, Int> {
        val context = appContext ?: return Pair(x, y)
        return FloatingLyricsRules.clampWindowToArea(x, y, width, areaHeightPx(context), panelTopRoom, panelMargin, screenWidth, screenHeight)
    }

    /**
     * The margin and the room above the lyric area the window position was last converted with (the window is the
     * lyric area's top-left less these). A change of font or size moves the window by the difference, so the lyric
     * area, which is what is saved, stays where it is.
     */
    private var panelMargin = 0
    private var panelTopRoom = 0

    /** Room above the lyric area: from the main row only (the translation row never moves the first row). */
    private fun topRoomPx(context: Context): Int = FloatingLyricsRules.panelTopRoomPx(mainRowHeightPx(context))

    private fun mainRowHeightPx(context: Context): Int {
        val metrics = Paint().apply { typeface = lyricTypeface(fontWeight); textSize = mainFontPx(context) }.fontMetrics
        return maxOf(1, FloatingLyricsRules.lineHeightPx(metrics.ascent, metrics.descent))
    }

    /** The lyric area's height now: GL, the layout's (translation row reserved for the track); Views, the column's. */
    private fun areaHeightPx(context: Context): Int {
        val layout = params
        if (glPanel != null && layout != null) return glLayout(context, layout.width).areaH
        val column = fadeLayout
        if (column != null && column.height > 0) return column.height
        return mainRowHeightPx(context)
    }

    /** Brings the window position in line with the margin and top room of the current font (the lyric area stays put). */
    private fun syncPanelOrigin(context: Context) {
        val layout = params ?: return
        val margin = marginPx(context)
        val top = topRoomPx(context)
        if (margin == panelMargin && top == panelTopRoom) return
        layout.x += panelMargin - margin
        layout.y += panelTopRoom - top
        panelMargin = margin
        panelTopRoom = top
        val view = root ?: return
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
    }

    /** Keeps the lock centred on the lyric area (the window's top room is fixed, the area's height may change). */
    private fun placeLock(context: Context, areaPx: Int) {
        val lock = lockButton ?: return
        val params = lock.layoutParams as? FrameLayout.LayoutParams ?: return
        val top = FloatingLyricsRules.lockTop(panelTopRoom, areaPx, lockPx(context))
        if (params.topMargin != top) {
            params.topMargin = top
            lock.layoutParams = params
        }
    }

    /** Saves the lyric area's top-left (not the window's) for this orientation. */
    private fun savePosition(context: Context, layout: WindowManager.LayoutParams) {
        val (keyX, keyY) = positionKeys(context)
        val (areaX, areaY) = FloatingLyricsRules.areaOrigin(layout.x, layout.y, panelMargin, panelTopRoom)
        prefs(context).edit().putInt(keyX, areaX).putInt(keyY, areaY).apply()
    }

    private fun isLandscape(context: Context): Boolean {
        val (width, height) = screenSize(context)
        return width > height
    }

    /** Portrait and landscape each keep their own position (ADR 0032). */
    private fun positionKeys(context: Context): Pair<String, String> {
        val suffix = FloatingLyricsRules.orientationKey(isLandscape(context))
        return Pair("x_$suffix", "y_$suffix")
    }

    /** The lyric area (narrowed to the screen) plus the lock gutter. */
    private fun overlayWidth(context: Context): Int = FloatingLyricsRules.overlayWidth(
        dp(context, if (isLandscape(context)) landscapeWidthDp else widthDp),
        screenSize(context).first,
        lockPx(context) + 2 * marginPx(context),
    )

    /** Transparent side margin for the shadow / glow, left of the lyric area and right of the lock. */
    private fun marginPx(context: Context): Int = FloatingLyricsRules.sideMarginPx(mainFontPx(context))

    /** Main line font size in px: always the user's. */
    private fun mainFontPx(context: Context): Float {
        return TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, fontSizePx.toFloat(), context.resources.displayMetrics)
    }

    /**
     * The lock button, and the transparent gutter right of the lyric area that holds it: as tall as the main line
     * so it scales with the font and never props the overlay open.
     */
    private fun lockPx(context: Context): Int {
        val metrics = android.graphics.Paint().apply { textSize = mainFontPx(context) }.fontMetrics
        return maxOf(1, FloatingLyricsRules.lineHeightPx(metrics.ascent, metrics.descent))
    }

    /** After a rotation: that orientation's width and saved position, or the centre when it has none. */
    /** Set while `applyOrientation` restyles, so a width change from the rotation is not taken for the slider. */
    private var rotating = false

    private fun applyOrientation() {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val landscape = isLandscape(context)
        if (shownLandscape == landscape) return
        shownLandscape = landscape
        rotating = true
        try {
            applyLook()
        } finally {
            rotating = false
        }
        layout.width = overlayWidth(context)
        val prefs = prefs(context)
        val (keyX, keyY) = positionKeys(context)
        if (prefs.contains(keyX) && prefs.contains(keyY)) {
            val (screenWidth, screenHeight) = screenSize(context)
            val (windowX, windowY) = FloatingLyricsRules.windowOrigin(prefs.getInt(keyX, 0), prefs.getInt(keyY, 0), panelMargin, panelTopRoom)
            val (x, y) = clampToText(windowX, windowY, layout.width, screenWidth, screenHeight)
            layout.x = x
            layout.y = y
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
            } catch (_: Exception) {
            }
        } else {
            centre()
        }
        pushGl(context)
    }

    /** The window: the GL renderer unless the switch, an earlier failure or a failure to set it up says Views. */
    private fun create(context: Context) {
        val property = LyricsTrace.systemProperty(RENDERER_PROPERTY)
        val gl = RendererChoice.useGl(property, glFailed, viewsNext)
        viewsNext = false
        if (gl) {
            try {
                createGl(context)
                return
            } catch (error: Throwable) {
                Log.w(TAG, "lyrics gl: the GL overlay could not be created, using the Views renderer", error)
                glFailed = true
                remove()
            }
        }
        createViews(context)
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun createViews(context: Context) {
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val (screenWidth, screenHeight) = screenSize(context)
        val button = lockPx(context)
        panelMargin = marginPx(context)
        panelTopRoom = topRoomPx(context)
        // The shadow / glow may draw past the lyric area into the transparent room around it.
        val frame = FrameLayout(context).apply { clipChildren = false; clipToPadding = false }
        val column = EdgeFadeLayout(context).apply { orientation = LinearLayout.VERTICAL; clipChildren = false; clipToPadding = false }
        // The room above the area is fixed (from the main row); the room below tracks the area's height (the
        // translation line comes and goes), and the lock stays centred on the area.
        column.addOnLayoutChangeListener { _, _, top, _, bottom, _, _, _, _ ->
            val topRoom = panelTopRoom
            val bottomRoom = FloatingLyricsRules.panelRoomPx(bottom - top)
            if (frame.paddingTop != topRoom || frame.paddingBottom != bottomRoom) {
                frame.post { frame.setPadding(frame.paddingLeft, topRoom, frame.paddingRight, bottomRoom) }
            }
            placeLock(context, bottom - top)
        }
        fun line(): LyricLineView = LyricLineView(context)
        val first = line()
        val second = line()
        column.addView(first, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        column.addView(second, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        frame.addView(column, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP).apply {
            marginStart = 0
            marginEnd = button
        })
        val lock = ImageButton(context).apply {
            setImageResource(R.drawable.floating_lyrics_lock_open)
            setColorFilter(Color.WHITE)
            background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(0x8C000000.toInt()) }
            contentDescription = "Lock"
            setOnClickListener { setLockedFromOverlay() }
        }
        frame.addView(lock, FrameLayout.LayoutParams(button, button, Gravity.END or Gravity.TOP))
        val width = overlayWidth(context)
        val prefs = prefs(context)
        val layout = WindowManager.LayoutParams(
            width,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
            else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE,
            // LAYOUT_IN_SCREEN / NO_LIMITS: y = 0 is the top of the display, so the overlay can be dragged over the
            // status bar instead of being pushed below it.
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            }
            // The saved position is the lyric area's top-left; the window is that less the margin and the top room.
            x = if (prefs.contains(positionKeys(context).first)) prefs.getInt(positionKeys(context).first, 0) - panelMargin
            else FloatingLyricsRules.lyricCentredStart(screenWidth, width, button + 2 * marginPx(context), marginPx(context))
            y = prefs.getInt(positionKeys(context).second, (screenHeight * 0.15f).toInt()) - panelTopRoom
        }
        frame.setOnTouchListener(DragListener(context))
        windowManager.addView(frame, layout)
        shownLandscape = isLandscape(context)
        val (_, keyY) = positionKeys(context)
        // No saved position for this orientation yet: the centre of the screen.
        if (!prefs.contains(keyY)) frame.post { if (root === frame && !prefs.contains(keyY)) centre() }
        val watch = object : android.content.ComponentCallbacks {
            override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
                // Post so the new display size is in place first.
                main.post { applyOrientation() }
            }

            @Deprecated("Deprecated in Java")
            override fun onLowMemory() {}
        }
        context.applicationContext.registerComponentCallbacks(watch)
        rotationWatch = watch
        root = frame
        params = layout
        mainText = first
        translationText = second
        lockButton = lock
        fadeLayout = column
        appliedLocked = null
        stripKeyValid = false
        prefetchValid = false
        stopTicking()
        tickScheduled = true
        main.postDelayed(tick, TICK_MS)
        LyricsTrace.configure(context)
    }

    // ---- OpenGL renderer (ADR 0032): the window holds a SurfaceView the render thread draws into, plus the real lock
    // button above it; the main thread only lays the window out and forwards the inputs.

    /** The panel's geometry for a window `windowWidth` px wide, derived from the settings and the lyrics (see [LyricsLayout]). */
    private fun glLayout(context: Context, windowWidth: Int): LyricsLayout {
        val mainPx = mainFontPx(context)
        val secondPx = mainPx * translationScalePercent / 100f
        val main = Paint().apply { typeface = lyricTypeface(fontWeight); textSize = mainPx }.fontMetrics
        val second = Paint().apply { typeface = lyricTypeface(FloatingLyricsRules.auxiliaryFontWeight(fontWeight)); textSize = secondPx }.fontMetrics
        val margin = marginPx(context)
        val lock = lockPx(context)
        return LyricsLayout(
            margin, lock, maxOf(1, windowWidth - lock - 2 * margin), main.ascent, main.descent, second.ascent, second.descent,
            showTranslation && linesHaveTranslation,
        )
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun createGl(context: Context) {
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val (screenWidth, screenHeight) = screenSize(context)
        val button = lockPx(context)
        panelMargin = marginPx(context)
        panelTopRoom = topRoomPx(context)
        val panel = GlLyricsPanel(context) { reason -> onGlFatal(reason) }
        glPanel = panel
        // The surface fills the window (the shadow room and the margins are drawn by the renderer); the lock button is a
        // real view above it, in the gutter right of the lyric area.
        val frame = FrameLayout(context).apply { clipChildren = false; clipToPadding = false }
        frame.addView(panel.surfaceView, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        val lock = ImageButton(context).apply {
            setImageResource(R.drawable.floating_lyrics_lock_open)
            setColorFilter(Color.WHITE)
            background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(0x8C000000.toInt()) }
            contentDescription = "Lock"
            setOnClickListener { setLockedFromOverlay() }
        }
        frame.addView(lock, FrameLayout.LayoutParams(button, button, Gravity.END or Gravity.TOP).apply {
            marginEnd = marginPx(context)
            topMargin = FloatingLyricsRules.lockTop(panelTopRoom, glLayout(context, width).areaH, button)
        })
        val width = overlayWidth(context)
        val prefs = prefs(context)
        val layout = WindowManager.LayoutParams(
            width,
            glLayout(context, width).surfaceH,
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
            else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            // The SurfaceView is TRANSLUCENT too: the lyrics blend over whatever is behind the window.
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            }
            // The saved position is the lyric area's top-left; the window is that less the margin and the top room.
            x = if (prefs.contains(positionKeys(context).first)) prefs.getInt(positionKeys(context).first, 0) - panelMargin
            else FloatingLyricsRules.lyricCentredStart(screenWidth, width, button + 2 * marginPx(context), marginPx(context))
            y = prefs.getInt(positionKeys(context).second, (screenHeight * 0.15f).toInt()) - panelTopRoom
        }
        frame.setOnTouchListener(DragListener(context))
        windowManager.addView(frame, layout)
        root = frame
        params = layout
        lockButton = lock
        mainText = null
        translationText = null
        fadeLayout = null
        shownLandscape = isLandscape(context)
        val (_, keyY) = positionKeys(context)
        // No saved position for this orientation yet: the centre of the screen.
        if (!prefs.contains(keyY)) frame.post { if (root === frame && !prefs.contains(keyY)) centre() }
        val watch = object : android.content.ComponentCallbacks {
            override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
                // Post so the new display size is in place first.
                main.post { applyOrientation() }
            }

            @Deprecated("Deprecated in Java")
            override fun onLowMemory() {}
        }
        context.applicationContext.registerComponentCallbacks(watch)
        rotationWatch = watch
        appliedLocked = null
        stripKeyValid = false
        prefetchValid = false
        stopTicking()
        LyricsTrace.configure(context)
        PlaybackService.startClockFeed()
    }

    /** The window height follows the font and the reserved translation row (a fixed height: the surface has no content size). */
    private fun applyGlLook(context: Context) {
        val view = root ?: return
        val layout = params ?: return
        val info = glLayout(context, layout.width)
        lockButton?.let { lock ->
            (lock.layoutParams as? FrameLayout.LayoutParams)?.let {
                if (it.marginEnd != info.marginPx) {
                    it.marginEnd = info.marginPx
                    lock.layoutParams = it
                }
            }
        }
        placeLock(context, info.areaH)
        if (layout.height != info.surfaceH) {
            layout.height = info.surfaceH
            val (screenWidth, screenHeight) = screenSize(context)
            val (x, y) = clampToText(layout.x, layout.y, layout.width, screenWidth, screenHeight)
            layout.x = x
            layout.y = y
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
            } catch (_: Exception) {
            }
        }
    }

    /** Hands the current settings and lyrics to the GL builder thread (the render thread picks the result up). */
    private fun pushGl(context: Context) {
        val panel = glPanel ?: return
        val layout = params ?: return
        val color = if (useCustomColor) FloatingLyricsRules.parseHexColor(customColor) ?: 0xffffff else 0xffffff
        val mainPx = mainFontPx(context)
        panel.submit(
            RenderInputs(
                linesVersion, trackId, offsetMs, trackDurationMs, lines,
                if (animation == "continuous") ANIMATION_CONTINUOUS else ANIMATION_NONE,
                align, textEffect, color, accentColor, mainPx, mainPx * translationScalePercent / 100f, fontWeight,
                FloatingLyricsRules.auxiliaryFontWeight(fontWeight), showTranslation,
                FloatingLyricsRules.mainOpacityPercent(mainOpacityPercent) / 100f,
                FloatingLyricsRules.secondaryOpacityPercent(secondaryOpacityPercent) / 100f,
                isLocked(context), context.resources.displayMetrics.density, glLayout(context, layout.width),
                dp(context, 10).toFloat(),
            ),
        )
    }

    /** The render thread gave up (main thread): take the GL window down and bring the Views renderer up in its place. */
    private fun onGlFatal(reason: String) {
        if (glPanel == null) return
        Log.w(TAG, "lyrics gl: falling back to the Views renderer ($reason)")
        glFailed = true
        remove()
        viewsNext = true
        refresh()
    }

    private fun remove() {
        stopTicking()
        glPanel?.destroy()
        glPanel = null
        val view = root ?: return
        val context = appContext
        rotationWatch?.let { context?.applicationContext?.unregisterComponentCallbacks(it) }
        rotationWatch = null
        shownLandscape = null
        root = null
        params = null
        mainText = null
        translationText = null
        lockButton = null
        fadeLayout = null
        appliedLocked = null
        stripKeyValid = false
        strip = null
        prefetchValid = false
        LyricsTrace.stop()
        if (context != null) {
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).removeView(view)
            } catch (_: Exception) {
            }
        }
    }

    private fun setLockedFromOverlay() {
        val context = appContext ?: return
        setLockedState(context, true)
        applyMode()
        pushGl(context)
    }

    private fun applyLook() {
        val context = appContext ?: return
        val view = root
        val layout = params
        if (view != null && layout != null) {
            syncPanelOrigin(context)
            val width = overlayWidth(context)
            if (layout.width != width && rotating) {
                // A rotation: that orientation's own width; `applyOrientation` places it.
                layout.width = width
            } else if (layout.width != width) {
                // The 寬度 slider: grow or shrink around the lyric area's centre, then keep the text on screen; saved.
                val margin = marginPx(context)
                val x = FloatingLyricsRules.resizeAroundCentre(layout.x, layout.width, width, margin, lockPx(context) + margin)
                layout.width = width
                val (screenWidth, screenHeight) = screenSize(context)
                val (cx, cy) = clampToText(x, layout.y, width, screenWidth, screenHeight)
                layout.x = cx
                layout.y = cy
                savePosition(context, layout)
                try {
                    (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
                } catch (_: Exception) {
                }
            }
        }
        val color = if (useCustomColor) FloatingLyricsRules.parseHexColor(customColor) ?: 0xffffff else 0xffffff
        val effect = FloatingLyricsRules.effectRgb(color)

        // The lyric area (text height × 1.1) sits in a panel text height × 1.5 tall with transparent side margins;
        // the lock gutter right of it is as wide as the main line is tall and stays transparent.
        val button = lockPx(context)
        lockButton?.let { lock ->
            val inset = maxOf(0, button / 6)
            lock.setPadding(inset, inset, inset, inset)
            lock.scaleType = android.widget.ImageView.ScaleType.FIT_CENTER
            (lock.layoutParams as? FrameLayout.LayoutParams)?.let { it.width = button; it.height = button; lock.layoutParams = it }
            if (glPanel == null) placeLock(context, areaHeightPx(context))
        }
        (mainText?.parent as? View)?.let { column ->
            (column.layoutParams as? FrameLayout.LayoutParams)?.let {
                it.marginStart = 0
                it.marginEnd = button
                column.layoutParams = it
            }
        }
        // Side margins; the room above and below follows the lyric area's height (the column's layout listener).
        if (glPanel == null) root?.let { frame -> frame.setPadding(marginPx(context), panelTopRoom, marginPx(context), frame.paddingBottom) }
        for ((view, scale) in listOf(mainText to 100, translationText to translationScalePercent)) {
            view ?: continue
            val px = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, fontSizePx * scale / 100f, context.resources.displayMetrics)
            val effectiveEffect = FloatingLyricsRules.effectiveTextEffect(animation, textEffect)
            val radius = if (effectiveEffect == "shadow") FloatingLyricsRules.shadowLayerRadius(px) else 0f
            val shadow = if (effectiveEffect == "shadow") (FloatingLyricsRules.SHADOW_ALPHA shl 24) or effect else 0
            val strokeWidth = if (effectiveEffect == "stroke") maxOf(1f, px * 0.08f) else 0f
            val strokeColor = if (effectiveEffect == "stroke") 0x80000000.toInt() or effect else 0
            val weight = if (view === mainText) fontWeight else FloatingLyricsRules.auxiliaryFontWeight(fontWeight)
            view.setLook(px, 0xff000000.toInt() or color, align, radius, shadow, strokeWidth, strokeColor, weight)
            if (view === mainText) view.setAccent(accentColor)
            // Animation "none": the line at the main opacity; "continuous": 1, the static mask carries main and secondary.
            view.alpha = FloatingLyricsRules.lineViewAlpha(animation, mainOpacityPercent)
            view.setInactiveFactor(1f)
        }
        if (glPanel != null) applyGlLook(context)
    }

    /** The lock state the window (flags, alpha, background, lock button) was last set for; null before the first apply. */
    private var appliedLocked: Boolean? = null
    private var fadeLayout: EdgeFadeLayout? = null

    private fun applyMode() {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val locked = isLocked(context)
        val flags = if (locked) {
            layout.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        } else {
            layout.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
        }
        // The window itself is opaque (the lines carry the main and secondary opacity). Android 12+ only lets touches
        // pass through an overlay up to the system's maximum obscuring opacity (normally 0.8), so a locked overlay is
        // capped at that limit; the drawn values are not compensated.
        val systemLimit = if (locked && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (context.getSystemService(Context.INPUT_SERVICE) as InputManager).maximumObscuringOpacityForTouch
        } else {
            null
        }
        val alpha = FloatingLyricsRules.windowAlpha(systemLimit)
        // Nothing changed since the last apply (every page update calls this): no new window layout pass.
        if (appliedLocked == locked && layout.flags == flags && layout.alpha == alpha) return
        appliedLocked = locked
        layout.flags = flags
        layout.alpha = alpha
        // Move mode: only the lyric area gets the background, never the lock gutter.
        view.background = null
        (mainText?.parent as? View)?.background = if (locked) null else GradientDrawable().apply {
            cornerRadius = dp(context, 10).toFloat()
            setColor(0x47000000)
        }
        lockButton?.visibility = if (locked) View.GONE else View.VISIBLE
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
    }

    /**
     * Draw the current line; true while a line slides during playback (then the next frame is wanted). `nowNanos` is the
     * display frame's time (Choreographer), or the current time outside a frame; the position is carried on in it.
     */
    private fun render(nowNanos: Long = System.nanoTime()): Boolean {
        if (glPanel != null) return false
        val view = root ?: return false
        val context = appContext ?: return false
        val have = PlaybackService.floatingLyricsSample(sample)
        if (have) {
            val now = nowNanos / 1_000_000.0
            if (sample.trackId != lastReportedTrack || sample.positionMs != lastReportedMs) {
                if (sample.trackId != lastReportedTrack) hasSmoothed = false
                lastReportedTrack = sample.trackId
                lastReportedMs = sample.positionMs
                lastReportedAt = now
            }
            val smoothed = FloatingLyricsRules.smoothPosition(
                smoothedMs, hasSmoothed, smoothedAt, sample.positionMs, lastReportedAt, now, sample.playing,
            )
            smoothedMs = smoothed
            hasSmoothed = true
            smoothedAt = now
            sample.positionMs = kotlin.math.round(smoothed).toLong()
        }
        val index = if (have && sample.active && sample.trackId == trackId) {
            FloatingLyricsRules.activeIndex(starts, sample.positionMs, offsetMs)
        } else {
            -1
        }
        if (animation == "continuous" && renderStrip(view, have)) {
            prefetchValid = false
            return have && sample.playing && view.visibility == View.VISIBLE
        }
        val line = lines.getOrNull(index)
        val text = line?.trimmedText.orEmpty()
        val translation = if (showTranslation) line?.trimmedTranslation.orEmpty() else ""
        val locked = isLocked(context)
        val shown = if (text.isEmpty() && !locked) "♪" else text
        view.visibility = if (shown.isEmpty()) View.INVISIBLE else View.VISIBLE
        val progress = if (line != null && have) {
            val duration = FloatingLyricsRules.lineDurationMs(
                starts, index, if (sample.durationMs != FloatingLyricsRules.NO_TIME) sample.durationMs else trackDurationMs, offsetMs,
            )
            FloatingLyricsRules.scrollProgress(sample.positionMs + offsetMs - line.startMs, duration)
        } else {
            0f
        }

        var sliding = false
        fadeLayout?.fadeEnabled = true
        mainText?.let {
            it.setWordMask(0f)
            it.setLine(shown, progress)
            sliding = it.overflows()
            // Word-timed line: karaoke, one smooth accent edge over the line (ADR 0041 D10).
            val times = line?.wordTimes
            val ranges = line?.wordRanges
            if (times != null && ranges != null && shown == text && have) {
                val lyricsMs = (sample.positionMs + offsetMs).toDouble()
                it.setKaraoke(ranges, times, lyricsMs)
                if (sample.playing && it.karaokeMoving(lyricsMs)) sliding = true
            } else {
                it.setKaraoke(null, null, 0.0)
            }
        }
        translationText?.setWordMask(0f)
        translationText?.let {
            it.visibility = if (translation.isEmpty()) View.GONE else View.VISIBLE
            it.setLine(translation, progress)
            if (translation.isNotEmpty() && it.overflows()) sliding = true
        }
        prefetchNeighbours(index, locked)
        return sliding && have && sample.active && view.visibility == View.VISIBLE
    }

    /**
     * The none mode: have the images of the shown line's neighbours (and the karaoke copies and translations) made in
     * the background, so the next line change finds them ready. Runs when the line, the lyrics, the translation switch or
     * a look changed (the views' raster epochs), not every frame.
     */
    private fun prefetchNeighbours(index: Int, locked: Boolean) {
        val first = mainText ?: return
        val second = translationText ?: return
        val epoch = first.rasterEpoch() xor (second.rasterEpoch() * 17)
        if (prefetchValid && prefetchIndex == index && prefetchLinesVersion == linesVersion && prefetchEpoch == epoch &&
            prefetchTranslation == showTranslation && prefetchLocked == locked
        ) {
            return
        }
        prefetchValid = true
        prefetchIndex = index
        prefetchLinesVersion = linesVersion
        prefetchEpoch = epoch
        prefetchTranslation = showTranslation
        prefetchLocked = locked
        prefetchMain.clear()
        prefetchAccent.clear()
        prefetchSecond.clear()
        // The shown line first, then the next, the previous and the one after the next.
        val order = if (index >= 0) intArrayOf(index, index + 1, index - 1, index + 2) else intArrayOf(0, 1)
        for (at in order) {
            val line = lines.getOrNull(at) ?: continue
            if (line.trimmedText.isNotEmpty()) {
                prefetchMain.add(line.trimmedText)
                if (line.wordRanges != null) prefetchAccent.add(line.trimmedText)
            }
            if (showTranslation && line.trimmedTranslation.isNotEmpty()) prefetchSecond.add(line.trimmedTranslation)
        }
        if (!locked) prefetchMain.add("♪")
        first.prefetchLines(prefetchMain, prefetchAccent)
        second.prefetchLines(prefetchSecond, emptyList())
    }

    /**
     * The strip for the current lyrics and look: lines with some text (main or translation), the translation strip
     * using the same cells (each cell as wide as the wider text, one U+3000 of the main line apart). Rebuilt only when
     * the lyrics, the translation switch or either row's font measures changed (compared as numbers, not per frame).
     */
    private fun stripFor(main: LyricLineView, second: LyricLineView?): Strip? {
        main.setKaraoke(null, null, 0.0)
        val secondVersion = second?.measureVersion ?: -1
        if (stripKeyValid && stripKeyLines == linesVersion && stripKeyTranslation == showTranslation &&
            stripKeyMain == main.measureVersion && stripKeySecond == secondVersion
        ) {
            return strip
        }
        stripKeyValid = true
        stripKeyLines = linesVersion
        stripKeyTranslation = showTranslation
        stripKeyMain = main.measureVersion
        stripKeySecond = secondVersion
        strip = null
        val keptLines = lines.filter { line ->
            line.trimmedText.isNotEmpty() || (showTranslation && line.trimmedTranslation.isNotEmpty())
        }
        if (keptLines.isEmpty()) return null
        val texts = keptLines.map { it.trimmedText }
        val translations = keptLines.map { if (showTranslation) it.trimmedTranslation else "" }
        val mainWidths = FloatArray(keptLines.size) { main.textWidth(texts[it]) }
        val translationWidths = FloatArray(keptLines.size) { second?.textWidth(translations[it]) ?: 0f }
        val lastGlyphCenters = FloatArray(keptLines.size) { index -> main.lastGlyphCenter(texts[index]) }
        val firstGlyphCenters = FloatArray(keptLines.size) { index -> main.firstGlyphCenter(texts[index]) }
        val (cells, widths) = FloatingLyricsRules.stripLayout(mainWidths, translationWidths, main.textWidth("　"))
        // Word-timed lyrics (ADR 0041 D10): knots at the line starts (first glyph centre) and the last words' ends (last
        // glyph centre); individual words never shape the glide. The translation strip shares the main offset.
        val wordKnots = if (keptLines.any { it.wordRanges != null }) {
            WordLyricsRules.WordKnots.of(
                WordLyricsRules.wordStripKnots(keptLines.mapIndexed { i, line ->
                    val times = line.wordTimes
                    val end = if (line.wordRanges != null && times != null) times.lastOrNull()?.second else null
                    WordLyricsRules.KnotLine(line.startMs.toDouble(), cells[i].toDouble(), firstGlyphCenters[i].toDouble(), end, lastGlyphCenters[i].toDouble())
                }),
            )
        } else {
            null
        }
        return Strip(
            LongArray(keptLines.size) { keptLines[it].startMs }, texts, translations, cells, widths, mainWidths, translationWidths,
            lastGlyphCenters, firstGlyphCenters, wordKnots, FloatingLyricsRules.graphemeCount(texts.last()),
            translations.any { it.isNotEmpty() },
        ).also { strip = it }
    }

    /** Continuous animation: draw both strips at the current place; false when there is nothing to draw as a strip. */
    private fun renderStrip(view: View, have: Boolean): Boolean {
        val main = mainText ?: return false
        if (!have || !sample.active || sample.trackId != trackId) return false
        val current = stripFor(main, translationText) ?: return false
        val lyricsMs = sample.positionMs + offsetMs
        val duration = if (sample.durationMs != FloatingLyricsRules.NO_TIME) sample.durationMs else trackDurationMs
        val trackEndAtLyricsMs = if (duration != FloatingLyricsRules.NO_TIME) duration + offsetMs else FloatingLyricsRules.NO_TIME
        val wordKnots = current.wordKnots
        val offset = if (wordKnots != null) {
            (main.availableWidth() / 2.0 - WordLyricsRules.knotValue(wordKnots, lyricsMs.toDouble())).toFloat()
        } else {
            val arrival = FloatingLyricsRules.finalLineArrivalFromGraphemes(current.starts[current.starts.size - 1], current.lastGraphemes, trackEndAtLyricsMs)
            if (!current.knotsBuilt || current.knotsEnd != arrival) {
                current.knots = FloatingLyricsRules.stripKnots(
                    current.starts, current.cells, current.widths, FloatingLyricsRules.STRIP_ALIGN, arrival,
                    current.mainWidths, current.lastGlyphCenters, current.firstGlyphCenters,
                )
                current.knotsEnd = arrival
                current.knotsBuilt = true
            }
            val knots = current.knots
            if (knots == null) 0f else FloatingLyricsRules.stripKnotsOffset(
                knots, lyricsMs, FloatingLyricsRules.stripTarget(main.availableWidth(), FloatingLyricsRules.STRIP_TARGET),
            )
        }
        // Line- and word-timed: the static mask at the main row's em with the main and secondary opacity on both
        // rows, instead of the edge fade and the line fades.
        val em = main.textSizePx()
        val mainOpacity = FloatingLyricsRules.mainOpacityPercent(mainOpacityPercent) / 100f
        val secondaryOpacity = FloatingLyricsRules.secondaryOpacityPercent(secondaryOpacityPercent) / 100f
        fadeLayout?.fadeEnabled = false
        main.setWordMask(em, mainOpacity, secondaryOpacity)
        translationText?.setWordMask(em, mainOpacity, secondaryOpacity)
        view.visibility = View.VISIBLE
        main.setStrip(current.texts, current.cells, current.widths, current.mainWidths, offset, current.starts, lyricsMs)
        translationText?.let {
            it.visibility = if (current.hasTranslation) View.VISIBLE else View.GONE
            if (current.hasTranslation) it.setStrip(current.translations, current.cells, current.widths, current.translationWidths, offset, current.starts, lyricsMs)
        }
        return true
    }

    private class DragListener(private val context: Context) : View.OnTouchListener {
        private var startX = 0
        private var startY = 0
        private var touchX = 0f
        private var touchY = 0f

        @SuppressLint("ClickableViewAccessibility")
        override fun onTouch(view: View, event: MotionEvent): Boolean {
            val layout = params ?: return false
            if (isLocked(context)) return false
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    startX = layout.x
                    startY = layout.y
                    touchX = event.rawX
                    touchY = event.rawY
                }
                MotionEvent.ACTION_MOVE -> {
                    val (screenWidth, screenHeight) = screenSize(context)
                    val (x, y) = clampToText(
                        startX + (event.rawX - touchX).toInt(),
                        startY + (event.rawY - touchY).toInt(),
                        view.width,
                        screenWidth,
                        screenHeight,
                    )
                    layout.x = x
                    layout.y = y
                    try {
                        (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
                    } catch (_: Exception) {
                    }
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                    savePosition(context, layout)
                }
            }
            return true
        }
    }
}
