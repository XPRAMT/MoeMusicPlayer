package com.moemusicplayer.mediaindex

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.hardware.input.InputManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
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
    /** What the playback service reports for the overlay. */
    data class Sample(
        val trackId: String,
        val positionMs: Long,
        val durationMs: Long?,
        val active: Boolean,
        val playing: Boolean = active,
    )

    private class Line(val startMs: Long, val text: String, val translation: String)

    private const val PREFS = "moe_floating_lyrics"
    private const val TICK_MS = 100L


    private val main = Handler(Looper.getMainLooper())
    private var appContext: Context? = null

    private var enabled = false
    private var align = "center"
    private var fontSizePx = 28
    private var translationScalePercent = 80
    private var useCustomColor = false
    private var customColor = "#00e1ff"
    private var showTranslation = true
    private var textEffect = "shadow"
    /** "none": one line at a time; "continuous": all lines in one strip that scrolls with playback (ADR 0032). */
    private var animation = "none"
    // The strip of the continuous animation, rebuilt when the lyrics or the look change (see stripFor).
    private class Strip(
        val key: List<Any?>,
        val starts: LongArray,
        val texts: List<String>,
        val translations: List<String>,
        val cells: FloatArray,
        val widths: FloatArray,
        val mainWidths: FloatArray,
        val lastGlyphCenters: FloatArray,
    )
    private var strip: Strip? = null
    private var opacityPercent = 100
    /** lyricsPreferences.inactiveOpacityPercent, pushed by the page on its own; the continuous strip dims the other lines by it. */
    private var inactiveOpacityPercent = FloatingLyricsRules.DEFAULT_INACTIVE_OPACITY_PERCENT
    private var widthDp = 800
    private var landscapeWidthDp = 800
    private var rotationWatch: android.content.ComponentCallbacks? = null
    // The position carried on between ExoPlayer's coarse reports (FloatingLyricsRules.smoothPositionMs).
    private var smoothedMs: Double? = null
    private var smoothedAt = 0L
    private var lastReportedMs = Long.MIN_VALUE
    private var lastReportedAt = 0L
    private var lastReportedTrack: String? = null
    private var shownLandscape: Boolean? = null

    private var trackId: String? = null
    private var offsetMs = 0L
    private var trackDurationMs: Long? = null
    private var lines: List<Line> = emptyList()
    private var starts = LongArray(0)

    private var root: FrameLayout? = null
    private var params: WindowManager.LayoutParams? = null
    private var mainText: LyricLineView? = null
    private var translationText: LyricLineView? = null
    private var lockButton: ImageButton? = null

    /** Every display frame while a line slides during playback, otherwise every 100 ms (line changes, pauses). */
    private var frameScheduled = false
    private val frame = Choreographer.FrameCallback {
        frameScheduled = false
        tickOnce()
    }
    private val tick = Runnable { tickOnce() }

    private fun tickOnce() {
        main.removeCallbacks(tick)
        if (root == null) return
        val sliding = render()
        if (root == null) return
        if (sliding) {
            if (!frameScheduled) {
                frameScheduled = true
                Choreographer.getInstance().postFrameCallback(frame)
            }
        } else {
            main.postDelayed(tick, TICK_MS)
        }
    }

    private fun stopTicking() {
        main.removeCallbacks(tick)
        if (frameScheduled) {
            Choreographer.getInstance().removeFrameCallback(frame)
            frameScheduled = false
        }
    }

    fun canDrawOverlays(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.M || Settings.canDrawOverlays(context)

    fun isLocked(context: Context): Boolean = prefs(context).getBoolean("locked", false)

    fun status(context: Context): JSONObject = JSONObject()
        .put("canDrawOverlays", canDrawOverlays(context))
        .put("locked", isLocked(context))
        .put("screenWidthDp", Math.round(screenSize(context).first / context.resources.displayMetrics.density))

    /** `{ settings?, data?, setLocked?, resetPosition? }` from the page. */
    fun update(context: Context, args: JSONObject) {
        appContext = context.applicationContext
        args.optJSONObject("settings")?.let(::applySettings)
        args.optJSONObject("data")?.let(::applyData)
        // lyricsPreferences.inactiveOpacityPercent (10-100), pushed by the page whenever that setting changes.
        if (args.has("inactiveOpacityPercent") && !args.isNull("inactiveOpacityPercent")) {
            applyInactiveOpacity(args.optInt("inactiveOpacityPercent", FloatingLyricsRules.DEFAULT_INACTIVE_OPACITY_PERCENT))
        }
        if (args.has("setLocked") && !args.isNull("setLocked")) {
            prefs(context).edit().putBoolean("locked", args.optBoolean("setLocked")).apply()
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
        val height = if (view.height > 0) view.height else view.measuredHeight
        // The lyric area (not the lock gutter) at the centre.
        layout.x = FloatingLyricsRules.lyricCentredStart(screenWidth, layout.width, lockPx(context) + 2 * marginPx(context), marginPx(context))
        layout.y = FloatingLyricsRules.centredStart(screenHeight, height)
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
    }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun applySettings(settings: JSONObject) {
        enabled = settings.optBoolean("enabled", false)
        align = settings.optString("align", "center")
        fontSizePx = settings.optInt("fontSizePx", 28).coerceIn(1, 50)
        translationScalePercent = settings.optInt("translationScalePercent", 80).coerceIn(1, 100)
        useCustomColor = settings.optBoolean("useCustomColor", false)
        customColor = settings.optString("customColor", "#00e1ff")
        showTranslation = settings.optBoolean("showTranslation", true)
        textEffect = settings.optString("textEffect", "shadow")
        animation = if (settings.optString("animation", "none") == "continuous") "continuous" else "none"
        opacityPercent = FloatingLyricsRules.opacityPercent(settings.optInt("opacityPercent", 100))
        // The look settings do not carry it (it is a general lyrics setting, pushed on its own, see update): keep the last one.
        if (settings.has("inactiveOpacityPercent")) applyInactiveOpacity(settings.optInt("inactiveOpacityPercent"))
        widthDp = FloatingLyricsRules.widthDp(settings.optInt("widthPx", 800))
        landscapeWidthDp = FloatingLyricsRules.widthDp(settings.optInt("landscapeWidthPx", 800))
    }

    private fun applyInactiveOpacity(percent: Int) {
        inactiveOpacityPercent = FloatingLyricsRules.inactiveOpacityPercent(percent)
    }

    private fun applyData(data: JSONObject) {
        trackId = if (data.isNull("trackId")) null else data.optString("trackId")
        offsetMs = data.optLong("offsetMs", 0L)
        trackDurationMs = if (data.isNull("durationMs")) null else data.optLong("durationMs").takeIf { it > 0 }
        val array = data.optJSONArray("lines")
        val parsed = ArrayList<Line>()
        if (array != null) {
            for (index in 0 until array.length()) {
                val item = array.optJSONObject(index) ?: continue
                parsed.add(Line(item.optLong("startMs"), item.optString("text", ""), item.optString("translation", "")))
            }
        }
        parsed.sortBy { it.startMs }
        lines = parsed
        starts = LongArray(parsed.size) { parsed[it].startMs }
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
            layout.y = FloatingLyricsRules.centredOnBar(statusBarHeight(context), view.height)
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
        val (x, y) = FloatingLyricsRules.clampPosition(
            layout.x + dx.coerceIn(-1, 1), layout.y + dy.coerceIn(-1, 1), layout.width, view.height, screenWidth, screenHeight,
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
        else layout.y = FloatingLyricsRules.centredStart(screenHeight, view.height)
        savePosition(context, layout)
        try {
            (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
        } catch (_: Exception) {
        }
    }

    private fun savePosition(context: Context, layout: WindowManager.LayoutParams) {
        val (keyX, keyY) = positionKeys(context)
        prefs(context).edit().putInt(keyX, layout.x).putInt(keyY, layout.y).apply()
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
    private fun applyOrientation() {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val landscape = isLandscape(context)
        if (shownLandscape == landscape) return
        shownLandscape = landscape
        applyLook()
        layout.width = overlayWidth(context)
        val prefs = prefs(context)
        val (keyX, keyY) = positionKeys(context)
        if (prefs.contains(keyX) && prefs.contains(keyY)) {
            val (screenWidth, screenHeight) = screenSize(context)
            val height = if (view.height > 0) view.height else view.measuredHeight
            val (x, y) = FloatingLyricsRules.clampPosition(
                prefs.getInt(keyX, 0), prefs.getInt(keyY, 0), layout.width, height, screenWidth, screenHeight,
            )
            layout.x = x
            layout.y = y
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).updateViewLayout(view, layout)
            } catch (_: Exception) {
            }
        } else {
            centre()
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun create(context: Context) {
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val (screenWidth, screenHeight) = screenSize(context)
        val button = lockPx(context)
        // The shadow / glow may draw past the lyric area into the transparent room around it.
        val frame = FrameLayout(context).apply { clipChildren = false; clipToPadding = false }
        val column = EdgeFadeLayout(context).apply { orientation = LinearLayout.VERTICAL; clipChildren = false; clipToPadding = false }
        // The panel is text height x 1.5: the room above and below the area (text height x 1.1) tracks its height
        // (the translation line comes and goes).
        column.addOnLayoutChangeListener { _, _, top, _, bottom, _, _, _, _ ->
            val room = FloatingLyricsRules.panelRoomPx(bottom - top)
            if (frame.paddingTop != room || frame.paddingBottom != room) {
                frame.post { frame.setPadding(frame.paddingLeft, room, frame.paddingRight, room) }
            }
        }
        fun line(): LyricLineView = LyricLineView(context)
        val first = line()
        // Same opacity as the main line: only the whole overlay is dimmed (fixed mode, Android 12+ limit).
        val second = line()
        column.addView(first, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        column.addView(second, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        frame.addView(column, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER_VERTICAL).apply {
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
        frame.addView(lock, FrameLayout.LayoutParams(button, button, Gravity.END or Gravity.CENTER_VERTICAL))
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
            x = prefs.getInt(
                positionKeys(context).first,
                FloatingLyricsRules.lyricCentredStart(screenWidth, width, button + 2 * marginPx(context), marginPx(context)),
            )
            y = prefs.getInt(positionKeys(context).second, (screenHeight * 0.15f).toInt())
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
        stopTicking()
        main.postDelayed(tick, TICK_MS)
    }

    private fun remove() {
        stopTicking()
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
        if (context != null) {
            try {
                (context.getSystemService(Context.WINDOW_SERVICE) as WindowManager).removeView(view)
            } catch (_: Exception) {
            }
        }
    }

    private fun setLockedFromOverlay() {
        val context = appContext ?: return
        prefs(context).edit().putBoolean("locked", true).apply()
        applyMode()
    }

    private fun applyLook() {
        val context = appContext ?: return
        val view = root
        val layout = params
        if (view != null && layout != null) {
            val width = overlayWidth(context)
            if (layout.width != width) {
                layout.width = width
                val (screenWidth, _) = screenSize(context)
                layout.x = layout.x.coerceIn(0, maxOf(0, screenWidth - width))
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
        }
        (mainText?.parent as? View)?.let { column ->
            (column.layoutParams as? FrameLayout.LayoutParams)?.let {
                it.marginStart = 0
                it.marginEnd = button
                column.layoutParams = it
            }
        }
        // Side margins; the room above and below follows the lyric area's height (the column's layout listener).
        root?.let { frame -> frame.setPadding(marginPx(context), frame.paddingTop, marginPx(context), frame.paddingBottom) }
        for ((view, scale) in listOf(mainText to 100, translationText to translationScalePercent)) {
            view ?: continue
            val px = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, fontSizePx * scale / 100f, context.resources.displayMetrics)
            val effectiveEffect = FloatingLyricsRules.effectiveTextEffect(animation, textEffect)
            val radius = if (effectiveEffect == "shadow") FloatingLyricsRules.shadowLayerRadius(px) else 0f
            val shadow = if (effectiveEffect == "shadow") (FloatingLyricsRules.SHADOW_ALPHA shl 24) or effect else 0
            val strokeWidth = if (effectiveEffect == "stroke") maxOf(1f, px * 0.08f) else 0f
            val strokeColor = if (effectiveEffect == "stroke") 0x80000000.toInt() or effect else 0
            view.setLook(px, 0xff000000.toInt() or color, align, radius, shadow, strokeWidth, strokeColor)
            view.setInactiveFactor(FloatingLyricsRules.inactiveFactor(animation, inactiveOpacityPercent, 0f))
        }
    }

    private fun applyMode() {
        val view = root ?: return
        val layout = params ?: return
        val context = appContext ?: return
        val locked = isLocked(context)
        layout.flags = if (locked) {
            layout.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        } else {
            layout.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
        }
        // The user's opacity for both lines. Android 12+ only lets touches pass through an overlay up to the system's
        // maximum obscuring opacity (normally 0.8), so fixed mode cannot go above that limit.
        val systemLimit = if (locked && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (context.getSystemService(Context.INPUT_SERVICE) as InputManager).maximumObscuringOpacityForTouch
        } else {
            null
        }
        layout.alpha = FloatingLyricsRules.windowAlpha(opacityPercent, systemLimit)
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

    /** Draw the current line; true while a line slides during playback (then the next frame is wanted). */
    private fun render(): Boolean {
        val view = root ?: return false
        val context = appContext ?: return false
        val sample = PlaybackService.floatingLyricsSample()?.let { reported ->
            val now = android.os.SystemClock.uptimeMillis()
            if (reported.trackId != lastReportedTrack || reported.positionMs != lastReportedMs) {
                if (reported.trackId != lastReportedTrack) smoothedMs = null
                lastReportedTrack = reported.trackId
                lastReportedMs = reported.positionMs
                lastReportedAt = now
            }
            val smoothed = FloatingLyricsRules.smoothPositionMs(
                smoothedMs, smoothedAt, reported.positionMs, lastReportedAt, now, reported.playing,
            )
            smoothedMs = smoothed
            smoothedAt = now
            reported.copy(positionMs = kotlin.math.round(smoothed).toLong())
        }
        val index = if (sample != null && sample.active && sample.trackId == trackId) {
            FloatingLyricsRules.activeIndex(starts, sample.positionMs, offsetMs)
        } else {
            -1
        }
        if (animation == "continuous" && renderStrip(view, sample)) {
            return sample?.playing == true && view.visibility == View.VISIBLE
        }
        val line = lines.getOrNull(index)
        val text = line?.text?.trim().orEmpty()
        val translation = if (showTranslation) line?.translation?.trim().orEmpty() else ""
        val locked = isLocked(context)
        val shown = if (text.isEmpty() && !locked) "♪" else text
        view.visibility = if (shown.isEmpty()) View.INVISIBLE else View.VISIBLE
        val progress = if (line != null && sample != null) {
            val duration = FloatingLyricsRules.lineDurationMs(starts, index, sample.durationMs ?: trackDurationMs, offsetMs)
            FloatingLyricsRules.scrollProgress(sample.positionMs + offsetMs - line.startMs, duration)
        } else {
            0f
        }

        var sliding = false
        mainText?.let {
            it.setLine(shown, progress)
            sliding = it.overflows()
        }
        translationText?.let {
            it.visibility = if (translation.isEmpty()) View.GONE else View.VISIBLE
            it.setLine(translation, progress)
            if (translation.isNotEmpty() && it.overflows()) sliding = true
        }
        return sliding && sample?.active == true && view.visibility == View.VISIBLE
    }

    /**
     * The strip for the current lyrics and look: lines with some text (main or translation), the translation strip
     * using the same cells (each cell as wide as the wider text, one U+3000 of the main line apart).
     */
    private fun stripFor(main: LyricLineView, second: LyricLineView?): Strip? {
        val key = listOf(lines, showTranslation, main.textWidth("\u3000"), second?.textWidth("\u3000"))
        strip?.let { if (it.key == key) return it }
        val kept = lines.mapNotNull { line ->
            val text = line.text.trim()
            val translation = if (showTranslation) line.translation.trim() else ""
            if (text.isEmpty() && translation.isEmpty()) null else Triple(line.startMs, text, translation)
        }
        if (kept.isEmpty()) return null
        val texts = kept.map { it.second }
        val translations = kept.map { it.third }
        val mainWidths = FloatArray(kept.size) { main.textWidth(texts[it]) }
        val lastGlyphCenters = FloatArray(kept.size) { index -> main.lastGlyphCenter(texts[index]) }
        val (cells, widths) = FloatingLyricsRules.stripLayout(
            mainWidths,
            FloatArray(kept.size) { second?.textWidth(translations[it]) ?: 0f },
            main.textWidth("\u3000"),
        )
        return Strip(key, LongArray(kept.size) { kept[it].first }, texts, translations, cells, widths, mainWidths, lastGlyphCenters).also { strip = it }
    }

    /** Continuous animation: draw both strips at the current place; false when there is nothing to draw as a strip. */
    private fun renderStrip(view: View, sample: Sample?): Boolean {
        val main = mainText ?: return false
        if (sample == null || !sample.active || sample.trackId != trackId) return false
        val current = stripFor(main, translationText) ?: return false
        val lyricsMs = sample.positionMs + offsetMs
        val trackEndAtLyricsMs = (sample.durationMs ?: trackDurationMs)?.plus(offsetMs)
        val finalArrivalAtLyricsMs = current.starts.lastOrNull()?.let { lastStart ->
            FloatingLyricsRules.finalLineArrivalMs(
                lastStart,
                current.texts.lastOrNull().orEmpty(),
                trackEndAtLyricsMs,
            )
        }
        val offset = FloatingLyricsRules.stripOffset(
            current.starts, current.cells, current.widths, FloatingLyricsRules.STRIP_ALIGN,
            lyricsMs, main.availableWidth(), FloatingLyricsRules.STRIP_TARGET,
            finalArrivalAtLyricsMs, current.mainWidths, current.lastGlyphCenters,
        )
        view.visibility = View.VISIBLE
        main.setStrip(current.texts, current.cells, current.widths, offset, current.starts, lyricsMs)
        translationText?.let {
            val any = current.translations.any { text -> text.isNotEmpty() }
            it.visibility = if (any) View.VISIBLE else View.GONE
            if (any) it.setStrip(current.translations, current.cells, current.widths, offset, current.starts, lyricsMs)
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
                    val (x, y) = FloatingLyricsRules.clampPosition(
                        startX + (event.rawX - touchX).toInt(),
                        startY + (event.rawY - touchY).toInt(),
                        view.width,
                        view.height,
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
