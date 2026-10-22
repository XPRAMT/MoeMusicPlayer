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
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.TextView
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
    private const val TOUCH_PASS_ALPHA = 0.8f
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

    private var trackId: String? = null
    private var offsetMs = 0L
    private var trackDurationMs: Long? = null
    private var lines: List<Line> = emptyList()
    private var starts = LongArray(0)

    private var root: FrameLayout? = null
    private var params: WindowManager.LayoutParams? = null
    private var mainText: TextView? = null
    private var translationText: TextView? = null
    private var lockButton: ImageButton? = null

    private val tick = object : Runnable {
        override fun run() {
            render()
            if (root != null) main.postDelayed(this, TICK_MS)
        }
    }

    fun canDrawOverlays(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.M || Settings.canDrawOverlays(context)

    fun isLocked(context: Context): Boolean = prefs(context).getBoolean("locked", false)

    fun status(context: Context): JSONObject = JSONObject()
        .put("canDrawOverlays", canDrawOverlays(context))
        .put("locked", isLocked(context))

    /** `{ settings?, data?, setLocked? }` from the page. */
    fun update(context: Context, args: JSONObject) {
        appContext = context.applicationContext
        args.optJSONObject("settings")?.let(::applySettings)
        args.optJSONObject("data")?.let(::applyData)
        if (args.has("setLocked") && !args.isNull("setLocked")) {
            prefs(context).edit().putBoolean("locked", args.optBoolean("setLocked")).apply()
        }
        refresh()
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
        fun line(): TextView = TextView(context).apply {
            setSingleLine(true)
            setHorizontallyScrolling(true)
            ellipsize = null
            includeFontPadding = true
        }
        val first = line()
        val second = line().apply { alpha = 0.8f }
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
        root = frame
        params = layout
        mainText = first
        translationText = second
        lockButton = lock
        main.removeCallbacks(tick)
        main.postDelayed(tick, TICK_MS)
    }

    private fun remove() {
        main.removeCallbacks(tick)
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
        val color = if (useCustomColor) FloatingLyricsRules.parseHexColor(customColor) ?: 0xffffff else 0xffffff
        val effect = FloatingLyricsRules.effectRgb(color)
        val gravity = when (align) {
            "left" -> Gravity.START
            "right" -> Gravity.END
            else -> Gravity.CENTER_HORIZONTAL
        } or Gravity.CENTER_VERTICAL
        for ((view, scale) in listOf(mainText to 100, translationText to translationScalePercent)) {
            view ?: continue
            val size = fontSizePx * scale / 100f
            view.setTextSize(TypedValue.COMPLEX_UNIT_DIP, size)
            view.setTextColor(0xff000000.toInt() or color)
            view.gravity = gravity
            val px = view.textSize
            when (textEffect) {
                "shadow" -> view.setShadowLayer(maxOf(1f, px * 0.25f), 0f, 0f, 0x99000000.toInt() or effect)
                "stroke" -> view.setShadowLayer(maxOf(1f, px * 0.08f), 0f, 0f, 0xE6000000.toInt() or effect)
                else -> view.setShadowLayer(0f, 0f, 0f, 0)
            }
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
        layout.alpha = if (locked && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val input = context.getSystemService(Context.INPUT_SERVICE) as InputManager
            minOf(TOUCH_PASS_ALPHA, input.maximumObscuringOpacityForTouch)
        } else {
            1f
        }
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

    private fun render() {
        val view = root ?: return
        val context = appContext ?: return
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
        mainText?.let { show(it, shown, progress) }
        translationText?.let {
            it.visibility = if (translation.isEmpty()) View.GONE else View.VISIBLE
            show(it, translation, progress)
        }
    }

    /** Set the text; a line wider than the view starts at the left and scrolls by the progress. */
    private fun show(view: TextView, text: String, progress: Float) {
        if (view.text.toString() != text) view.text = text
        val available = view.width - view.paddingLeft - view.paddingRight
        val overflow = if (available > 0) view.paint.measureText(text) - available else 0f
        if (overflow > 0f) {
            view.gravity = Gravity.START or Gravity.CENTER_VERTICAL
            view.scrollTo((overflow * progress).toInt(), 0)
        } else {
            val wanted = when (align) {
                "left" -> Gravity.START
                "right" -> Gravity.END
                else -> Gravity.CENTER_HORIZONTAL
            }
            view.gravity = wanted or Gravity.CENTER_VERTICAL
            if (view.scrollX != 0) view.scrollTo(0, 0)
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