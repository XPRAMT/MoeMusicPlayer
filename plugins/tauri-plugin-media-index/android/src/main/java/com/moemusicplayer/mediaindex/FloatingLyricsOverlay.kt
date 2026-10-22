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
    data class Sample(val trackId: String, val positionMs: Long, val durationMs: Long?, val active: Boolean)

    private class Line(val startMs: Long, val text: String, val translation: String)

    private const val PREFS = "moe_floating_lyrics"
    private const val TICK_MS = 100L

    private const val BUTTON_DP = 32
    private const val PADDING_DP = 8

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
    private var opacityPercent = 100

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

    /** `{ settings?, data?, setLocked?, resetPosition? }` from the page. */
    fun update(context: Context, args: JSONObject) {
        appContext = context.applicationContext
        args.optJSONObject("settings")?.let(::applySettings)
        args.optJSONObject("data")?.let(::applyData)
        if (args.has("setLocked") && !args.isNull("setLocked")) {
            prefs(context).edit().putBoolean("locked", args.optBoolean("setLocked")).apply()
        }
        if (args.optBoolean("resetPosition", false)) {
            prefs(context).edit().remove("x").remove("y").apply()
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
        layout.x = FloatingLyricsRules.centredStart(screenWidth, layout.width)
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
        opacityPercent = FloatingLyricsRules.opacityPercent(settings.optInt("opacityPercent", 100))
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

    private fun screenSize(context: Context): Pair<Int, Int> {
        val metrics = context.resources.displayMetrics
        return Pair(metrics.widthPixels, metrics.heightPixels)
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun create(context: Context) {
        val windowManager = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val (screenWidth, screenHeight) = screenSize(context)
        val padding = dp(context, PADDING_DP)
        val button = dp(context, BUTTON_DP)
        val frame = FrameLayout(context).apply { setPadding(padding, padding, padding, padding) }
        val column = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        fun line(): LyricLineView = LyricLineView(context)
        val first = line()
        // Same opacity as the main line: only the whole overlay is dimmed (fixed mode, Android 12+ limit).
        val second = line()
        column.addView(first, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        column.addView(second, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        frame.addView(column, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER_VERTICAL).apply {
            marginStart = button + padding
            marginEnd = button + padding
        })
        val lock = ImageButton(context).apply {
            setImageResource(android.R.drawable.ic_lock_lock)
            setColorFilter(Color.WHITE)
            background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(0x8C000000.toInt()) }
            contentDescription = "Lock"
            setOnClickListener { setLockedFromOverlay() }
        }
        frame.addView(lock, FrameLayout.LayoutParams(button, button, Gravity.END or Gravity.CENTER_VERTICAL))
        val width = (screenWidth * 0.92f).toInt()
        val prefs = prefs(context)
        val layout = WindowManager.LayoutParams(
            width,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
            else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = prefs.getInt("x", (screenWidth - width) / 2)
            y = prefs.getInt("y", (screenHeight * 0.15f).toInt())
        }
        frame.setOnTouchListener(DragListener(context))
        windowManager.addView(frame, layout)
        if (!prefs.contains("y")) frame.post { if (root === frame && !prefs.contains("y")) centre() }
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
        val color = if (useCustomColor) FloatingLyricsRules.parseHexColor(customColor) ?: 0xffffff else 0xffffff
        val effect = FloatingLyricsRules.effectRgb(color)
        for ((view, scale) in listOf(mainText to 100, translationText to translationScalePercent)) {
            view ?: continue
            val px = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, fontSizePx * scale / 100f, context.resources.displayMetrics)
            val (radius, shadow) = when (textEffect) {
                "shadow" -> maxOf(1f, px * 0.25f) to (0x99000000.toInt() or effect)
                "stroke" -> maxOf(1f, px * 0.08f) to (0xE6000000.toInt() or effect)
                else -> 0f to 0
            }
            view.setLook(px, 0xff000000.toInt() or color, align, radius, shadow)
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
        view.background = if (locked) null else GradientDrawable().apply {
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
        val sample = PlaybackService.floatingLyricsSample()
        val index = if (sample != null && sample.active && sample.trackId == trackId) {
            FloatingLyricsRules.activeIndex(starts, sample.positionMs, offsetMs)
        } else {
            -1
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
                    prefs(context).edit().putInt("x", layout.x).putInt("y", layout.y).apply()
                }
            }
            return true
        }
    }
}