package com.moemusicplayer.mediaindex

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.ValueAnimator
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
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageButton

import org.json.JSONObject

/**
 * Android floating lyrics (ADR 0032): a TYPE_APPLICATION_OVERLAY window whose text an OpenGL ES render thread
 * draws ([GlLyricsPanel]); there is no Views renderer and no fallback: when GL cannot be set up the overlay shows
 * nothing (logged with the tag MoeMediaIndex) until it is switched off and on again. The page pushes the look
 * settings and the current track's timed lines; the playback clock comes from the playback service, so it keeps going
 * while the WebView is in the background. Move mode: dragged by touch, with a lock button. Fixed mode:
 * FLAG_NOT_TOUCHABLE (touches pass through, so the window is drawn at most 80% opaque as Android 12+ requires) and unlocked from the player's settings only. The position (per orientation, a
 * ratio of the screen) and the lock state are kept in settings.json through Rust ([FloatingLyricsStore]); a restored
 * position is always clamped so the lyric area stays on screen. The user grants "display over other apps" in the system
 * settings page.
 * Main thread only.
 */
object FloatingLyricsOverlay {
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
        /** The line's own end time (`endMs`: SRT/ASS cue end, yrc/QRC line length, enhanced LRC closing stamp). */
        val endMs: Long? = null,
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
    )

    /** The theme accent (karaoke in the none mode, ADR 0041), pushed by the page as `accentColor`. */
    private var accentColor = 0xffa9c3fe.toInt()

    private const val TAG = "MoeMediaIndex"

    /** The GL renderer's panel while the window exists. */
    private var glPanel: GlLyricsPanel? = null

    /** GL failed in this process: nothing is shown (and nothing retried) until the overlay is switched off and on again. */
    private var glFailed = false

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
    /** The chosen font's upright faces (`fontFaces`, resolved by Rust from `fontFamily`; ADR 0042); empty: the default font. */
    private var fontFaces: List<LyricFontFace> = emptyList()
    private var useCustomColor = false
    private var customColor = "#00e1ff"
    private var showTranslation = true
    private var textEffect = "shadow"
    /** "none": one line at a time; "continuous": all lines in one strip that scrolls with playback (ADR 0032). */
    private var animation = "none"
    private var linesVersion = 0
    /** The floating settings' own pair: "none" draws the line at main; "continuous" feeds both into the static mask. */
    private var mainOpacityPercent = FloatingLyricsRules.DEFAULT_MAIN_OPACITY_PERCENT
    private var secondaryOpacityPercent = FloatingLyricsRules.DEFAULT_SECONDARY_OPACITY_PERCENT
    /** 「暫停時隱藏」 (this tab's own, default on): the window fades out while playback is paused or stopped. */
    private var hideWhenPaused = true
    private var widthDp = 800
    private var landscapeWidthDp = 800
    private var rotationWatch: android.content.ComponentCallbacks? = null
    private var shownLandscape: Boolean? = null

    private var trackId: String? = null
    private var offsetMs = 0L
    private var trackDurationMs = FloatingLyricsRules.NO_TIME
    private var lines: List<Line> = emptyList()

    private var root: FrameLayout? = null
    private var params: WindowManager.LayoutParams? = null
    private var lockButton: ImageButton? = null

    fun canDrawOverlays(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.M || Settings.canDrawOverlays(context)

    // The lock state and the positions come from settings.json through Rust ([FloatingLyricsStore], read once and
    // then kept in memory); every write goes through setLockedState / storePosition.
    @Suppress("UNUSED_PARAMETER")
    fun isLocked(context: Context): Boolean = FloatingLyricsStore.current().locked

    @Suppress("UNUSED_PARAMETER")
    private fun setLockedState(context: Context, locked: Boolean) {
        FloatingLyricsStore.setLocked(locked)
    }

    /** The saved position of the orientation shown now (a ratio of the screen), or null: centred. */
    private fun savedPosition(context: Context): FloatingLyricsRules.Ratio? =
        FloatingLyricsStore.current().position(isLandscape(context))

    /**
     * Where a window `width` px wide with a lyric area `areaPx` tall goes for a saved position: the ratio put back on
     * this screen, the lyric area (and the lock column) kept on it, less the margin and the top room the window is
     * converted with. The same path for a new window, a rotation and the carry-over of the old values.
     */
    private fun restoredWindowOrigin(context: Context, saved: FloatingLyricsRules.Ratio, width: Int, areaPx: Int): Pair<Int, Int> {
        val (screenWidth, screenHeight) = screenSize(context)
        val (areaX, areaY) = FloatingLyricsRules.restoreAreaOrigin(
            saved.x, saved.y, screenWidth, screenHeight, width - 2 * panelMargin, areaPx,
        )
        return FloatingLyricsRules.windowOrigin(areaX, areaY, panelMargin, panelTopRoom)
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
                    val endMs = if (item.has("endMs") && !item.isNull("endMs")) item.optLong("endMs") else null
                    parsed.add(Line(item.optLong("startMs"), text, item.optString("translation", ""), times, ranges, endMs))
                }
            }
            parsed.sortBy { it.startMs }
            return ParsedData(trackId, data.optLong("offsetMs", 0L), durationMs, parsed)
        } finally {
            if (LyricsTrace.on) LyricsTrace.end(started, false)
        }
    }

    /** `{ settings?, data?, setLocked?, resetPosition? }` from the page; `parsed` is `data` already parsed by [parseData]. */
    internal fun update(context: Context, args: JSONObject, parsed: ParsedData? = null) {
        appContext = context.applicationContext
        // Once: the values the overlay used to keep in SharedPreferences go to settings.json (ADR 0032).
        FloatingLyricsLegacyPrefs.migrateOnce(context) { placeSaved() }
        args.optJSONObject("settings")?.let(::applySettings)
        (parsed ?: args.optJSONObject("data")?.let(::parseData))?.let(::applyParsed)
        if (args.has("accentColor") && !args.isNull("accentColor")) {
            FloatingLyricsRules.parseHexColor(args.optString("accentColor"))?.let {
                accentColor = 0xff000000.toInt() or it
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
            FloatingLyricsStore.setPosition(isLandscape(context), null)
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
        if (glFailed) return
        if (root == null) create(context)
        if (root == null) return
        applyLook()
        applyPauseVisibility(animate = true)
        applyMode()
        pushGl(context)
    }

    private fun applySettings(settings: JSONObject) {
        enabled = settings.optBoolean("enabled", false)
        // Switching the overlay off and on again is the one way to try GL again after it failed.
        if (!enabled) glFailed = false
        align = settings.optString("align", "center")
        fontSizePx = settings.optInt("fontSizePx", 28).coerceIn(1, 50)
        translationScalePercent = settings.optInt("translationScalePercent", 80).coerceIn(1, 100)
        fontWeight = FloatingLyricsRules.fontWeight(settings.optInt("fontWeight", FloatingLyricsRules.DEFAULT_FONT_WEIGHT))
        fontFaces = LyricFonts.parse(settings.optJSONArray("fontFaces"))
        useCustomColor = settings.optBoolean("useCustomColor", false)
        customColor = settings.optString("customColor", "#00e1ff")
        showTranslation = settings.optBoolean("showTranslation", true)
        textEffect = settings.optString("textEffect", "shadow")
        animation = if (settings.optString("animation", "none") == "continuous") "continuous" else "none"
        mainOpacityPercent = FloatingLyricsRules.mainOpacityPercent(settings.optInt("mainOpacityPercent", FloatingLyricsRules.DEFAULT_MAIN_OPACITY_PERCENT))
        secondaryOpacityPercent = FloatingLyricsRules.secondaryOpacityPercent(settings.optInt("secondaryOpacityPercent", FloatingLyricsRules.DEFAULT_SECONDARY_OPACITY_PERCENT))
        hideWhenPaused = settings.optBoolean("hideWhenPaused", true)
        widthDp = FloatingLyricsRules.widthDp(settings.optInt("widthPx", 800))
        landscapeWidthDp = FloatingLyricsRules.widthDp(settings.optInt("landscapeWidthPx", 800))
    }

    private fun applyParsed(data: ParsedData) {
        trackId = data.trackId
        offsetMs = data.offsetMs
        trackDurationMs = data.durationMs
        lines = data.lines
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
        val metrics = Paint().apply { typeface = lyricTypeface(fontWeight, fontFaces); textSize = mainFontPx(context) }.fontMetrics
        return maxOf(1, FloatingLyricsRules.lineHeightPx(metrics.ascent, metrics.descent))
    }

    /** The lyric area's height now: the layout's (the translation row is reserved for the whole track when shown). */
    private fun areaHeightPx(context: Context): Int {
        val layout = params ?: return mainRowHeightPx(context)
        return glLayout(context, layout.width).areaH
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

    /** Saves the lyric area's top-left (not the window's) for this orientation, as a ratio of this screen. */
    private fun savePosition(context: Context, layout: WindowManager.LayoutParams) {
        val (areaX, areaY) = FloatingLyricsRules.areaOrigin(layout.x, layout.y, panelMargin, panelTopRoom)
        val (screenWidth, screenHeight) = screenSize(context)
        val ratio = FloatingLyricsRules.ratioOf(areaX, areaY, screenWidth, screenHeight) ?: return
        FloatingLyricsStore.setPosition(screenWidth > screenHeight, ratio)
    }

    /** The saved values changed under the window (the carry-over of the old ones): place it again. */
    private fun placeSaved() {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val saved = savedPosition(context)
        if (saved == null) {
            centre()
        } else {
            val (x, y) = restoredWindowOrigin(context, saved, layout.width, areaHeightPx(context))
            layout.x = x
            layout.y = y
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
            } catch (_: Exception) {
            }
        }
        applyMode()
        pushGl(context)
    }

    private fun isLandscape(context: Context): Boolean {
        val (width, height) = screenSize(context)
        return width > height
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
        val saved = savedPosition(context)
        if (saved != null) {
            val (x, y) = restoredWindowOrigin(context, saved, layout.width, areaHeightPx(context))
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

    /** The window. GL that cannot be set up is logged and leaves the overlay empty: there is no other renderer. */
    private fun create(context: Context) {
        try {
            createGl(context)
        } catch (error: Throwable) {
            Log.w(TAG, "lyrics gl: the overlay could not be created, nothing is shown", error)
            glFailed = true
            remove()
        }
    }

    // ---- OpenGL renderer (ADR 0032): the window holds a SurfaceView the render thread draws into, plus the real lock
    // button above it; the main thread only lays the window out and forwards the inputs.

    /** The panel's geometry for a window `windowWidth` px wide, derived from the settings and the lyrics (see [LyricsLayout]). */
    private fun glLayout(context: Context, windowWidth: Int): LyricsLayout {
        val mainPx = mainFontPx(context)
        val secondPx = mainPx * translationScalePercent / 100f
        val main = Paint().apply { typeface = lyricTypeface(fontWeight, fontFaces); textSize = mainPx }.fontMetrics
        val second = Paint().apply { typeface = lyricTypeface(FloatingLyricsRules.auxiliaryFontWeight(fontWeight), fontFaces); textSize = secondPx }.fontMetrics
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
        val saved = savedPosition(context)
        // Placed like a rotation places it (the ratio put back on this screen, then clamped), or centred.
        val origin = saved?.let { restoredWindowOrigin(context, it, width, glLayout(context, width).areaH) }
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
            x = origin?.first
                ?: FloatingLyricsRules.lyricCentredStart(screenWidth, width, button + 2 * marginPx(context), marginPx(context))
            y = origin?.second ?: ((screenHeight * 0.15f).toInt() - panelTopRoom)
        }
        frame.setOnTouchListener(DragListener(context))
        windowManager.addView(frame, layout)
        root = frame
        params = layout
        lockButton = lock
        shownLandscape = isLandscape(context)
        // No saved position for this orientation yet: the centre of the screen.
        if (saved == null) frame.post { if (root === frame && savedPosition(context) == null) centre() }
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
        LyricsTrace.configure(context)
        // 「暫停時隱藏」: a new window starts hidden until the player is seen playing (no flash while paused).
        pauseGate = PauseGate()
        pauseHidden = null
        LyricsClock.onPlayState = playStateHook
        observePlayState(animate = false)
        PlaybackService.startClockFeed()
    }

    // ---- 「暫停時隱藏」 (ADR 0032): the gate follows the playback service's clock reports; the window fades its alpha
    // between 0 and its normal value (the locked cap included) over 200 ms, takes no touches and shows no lock button
    // while hidden, and the render thread asks for no frames once the fade-out is over.

    private var pauseGate = PauseGate()
    /** What the window was last set to (null: not yet). */
    private var pauseHidden: Boolean? = null
    private var pauseFade: ValueAnimator? = null
    /** Runs on the publishing thread, which is the main thread (the player's). */
    private val playStateHook: () -> Unit = { observePlayState(animate = true) }
    private val pauseTick = Runnable {
        pauseGate.tick(SystemClock.uptimeMillis())
        schedulePauseTick()
        applyPauseVisibility(animate = true)
    }

    /** Main thread: feed the newest clock report to the gate and apply the result. */
    private fun observePlayState(animate: Boolean) {
        if (root == null) return
        pauseGate.observe(PauseHide.activity(LyricsClock.snapshot), SystemClock.uptimeMillis())
        schedulePauseTick()
        applyPauseVisibility(animate)
    }

    private fun schedulePauseTick() {
        main.removeCallbacks(pauseTick)
        val due = pauseGate.dueAt() ?: return
        main.postDelayed(pauseTick, (due - SystemClock.uptimeMillis()).coerceAtLeast(1L))
    }

    /** The window's alpha while shown: 1, or the system's cap for a touch-through (locked) overlay on Android 12+. */
    private fun shownAlpha(context: Context): Float {
        val systemLimit = if (isLocked(context) && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (context.getSystemService(Context.INPUT_SERVICE) as InputManager).maximumObscuringOpacityForTouch
        } else {
            null
        }
        return FloatingLyricsRules.windowAlpha(systemLimit)
    }

    /** Hide or show the window for the gate and the setting; `animate` false sets it at once (a new window). */
    private fun applyPauseVisibility(animate: Boolean) {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val hidden = !pauseGate.visible(hideWhenPaused)
        if (pauseHidden == hidden) return
        pauseHidden = hidden
        pauseFade?.cancel()
        pauseFade = null
        // Showing: frames again (one full redraw) before the alpha rises. The touch flag and the lock button follow
        // at once (applyMode), both ways.
        if (!hidden) glPanel?.setSuspended(false)
        val target = if (hidden) 0f else shownAlpha(context)
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val finish = {
            if (hidden && pauseHidden == true && root === view) glPanel?.setSuspended(true)
        }
        if (!animate || layout.alpha == target) {
            layout.alpha = target
            applyMode(force = true)
            finish()
            return
        }
        // One 200 ms animator (the system's animator duration scale applies: "remove animations" makes it instant).
        val fade = ValueAnimator.ofFloat(layout.alpha, target)
        pauseFade = fade
        applyMode(force = true)
        fade.duration = PauseHide.FADE_MS
        fade.addUpdateListener { animation ->
            if (root === view && pauseFade === animation) {
                layout.alpha = animation.animatedValue as Float
                try {
                    windowManager.updateViewLayout(view, layout)
                } catch (_: Exception) {
                }
            }
        }
        fade.addListener(object : AnimatorListenerAdapter() {
            private var cancelled = false
            override fun onAnimationCancel(animation: Animator) {
                cancelled = true
            }

            override fun onAnimationEnd(animation: Animator) {
                if (pauseFade !== animation) return
                pauseFade = null
                if (!cancelled) finish()
            }
        })
        fade.start()
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
                dp(context, 10).toFloat(), fontFaces,
            ),
        )
    }

    /** The render thread gave up (main thread): log it, take the window down and show nothing until re-enabled. */
    private fun onGlFatal(reason: String) {
        if (glPanel == null) return
        Log.w(TAG, "lyrics gl: the renderer failed, nothing is shown ($reason)")
        glFailed = true
        remove()
    }

    private fun remove() {
        if (LyricsClock.onPlayState === playStateHook) LyricsClock.onPlayState = null
        main.removeCallbacks(pauseTick)
        val fade = pauseFade
        pauseFade = null
        fade?.cancel()
        pauseHidden = null
        glPanel?.destroy()
        glPanel = null
        val view = root ?: return
        val context = appContext
        rotationWatch?.let { context?.applicationContext?.unregisterComponentCallbacks(it) }
        rotationWatch = null
        shownLandscape = null
        root = null
        params = null
        lockButton = null
        appliedLocked = null
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
        // The lyric area (text height × 1.1) sits in a panel text height × 1.5 tall with transparent side margins;
        // the lock gutter right of it is as wide as the main line is tall and stays transparent.
        val button = lockPx(context)
        lockButton?.let { lock ->
            val inset = maxOf(0, button / 6)
            lock.setPadding(inset, inset, inset, inset)
            lock.scaleType = android.widget.ImageView.ScaleType.FIT_CENTER
            (lock.layoutParams as? FrameLayout.LayoutParams)?.let { it.width = button; it.height = button; lock.layoutParams = it }
        }
        applyGlLook(context)
    }

    /** The lock state the window (flags, alpha, background, lock button) was last set for; null before the first apply. */
    private var appliedLocked: Boolean? = null
    private var appliedHidden: Boolean? = null

    private fun applyMode(force: Boolean = false) {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val locked = isLocked(context)
        // Hidden by 「暫停時隱藏」: no touches either (the window is not there for the user).
        val hidden = pauseHidden == true
        val flags = if (locked || hidden) {
            layout.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        } else {
            layout.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
        }
        // The window itself is opaque (the lines carry the main and secondary opacity). Android 12+ only lets touches
        // pass through an overlay up to the system's maximum obscuring opacity (normally 0.8), so a locked overlay is
        // capped at that limit; the drawn values are not compensated. While the pause fade runs it owns the alpha.
        val alpha = when {
            pauseFade != null -> layout.alpha
            hidden -> 0f
            else -> shownAlpha(context)
        }
        // Nothing changed since the last apply (every page update calls this): no new window layout pass.
        if (!force && appliedLocked == locked && appliedHidden == hidden && layout.flags == flags && layout.alpha == alpha) return
        appliedLocked = locked
        appliedHidden = hidden
        layout.flags = flags
        layout.alpha = alpha
        // Move mode: the render thread draws the lyric area's background (never the lock gutter).
        lockButton?.visibility = if (locked || hidden) View.GONE else View.VISIBLE
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
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
