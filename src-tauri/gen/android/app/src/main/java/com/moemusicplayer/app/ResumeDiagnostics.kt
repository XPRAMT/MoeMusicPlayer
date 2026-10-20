package com.moemusicplayer.app

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.PixelCopy
import android.view.View
import android.webkit.WebView
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * The WebView can come back to the front with its page alive but none of it drawn: only the window's background colour
 * shows (seen on a phone 2026-10-21; the page was visible and producing frames, the compositor showed none of it). Telling
 * the WebView to pause and resume brings the content back, so that is done the moment the window gets the focus.
 *
 * What happens is also written down, in a file that survives the process and can be read with `adb shell cat`
 * (`Android/data/<package>/files/diagnostics/resume.log`, also logcat tag `MoeResume`): 1.5 s after the focus the screen
 * is sampled (what the compositor shows, through [PixelCopy]); a screen of one colour is logged as `BLANK SCREEN`. The
 * sample only records, it repairs nothing.
 */
object ResumeDiagnostics {
  private const val TAG = "MoeResume"
  private const val MAX_BYTES = 128 * 1024
  private val stamp = SimpleDateFormat("MM-dd HH:mm:ss.SSS", Locale.US)
  private val handler = Handler(Looper.getMainLooper())

  /** Bumped on every resume so the checks of an earlier resume do nothing. */
  private var generation = 0

  fun log(context: Context, message: String) {
    android.util.Log.i(TAG, message)
    try {
      val directory = File(context.getExternalFilesDir(null), "diagnostics")
      directory.mkdirs()
      val file = File(directory, "resume.log")
      if (file.length() > MAX_BYTES) {
        file.writeText(file.readText().takeLast(MAX_BYTES / 2))
      }
      file.appendText("${stamp.format(Date())} $message\n")
    } catch (_: Exception) {
      // The log is a convenience; it never breaks the app.
    }
  }

  /**
   * The window got the focus. DIAGNOSIS PHASE (2026-10-21): the repair, pausing and resuming the WebView, waits
   * [REPAIR_DELAY_MS] so that a base-colour screen stays long enough to be examined from outside (`adb shell dumpsys`
   * on SurfaceFlinger and the window manager). The screen is sampled 1.5 s and 4.5 s after the focus, the repair is made at
   * 5 s, and the screen is sampled again 1.5 s after it. Set the delay to 0 to repair at once.
   */
  fun onWindowFocus(activity: Activity, webView: () -> WebView?) {
    val mine = ++generation
    log(activity, "window focus: ${viewState(activity, webView())}")
    fun look(label: String) {
      sample(activity) { result ->
        if (mine != generation) return@sample
        if (result.blank) {
          log(activity, "BLANK SCREEN ($label): ${result.text}; ${webViewDetail(webView())}")
        } else {
          log(activity, "screen ok ($label): ${result.text}")
        }
      }
    }
    handler.postDelayed({ if (mine == generation) look("focus+1.5s") }, 1500L)
    handler.postDelayed({ if (mine == generation) look("focus+4.5s, before the repair") }, 4500L)
    handler.postDelayed({
      if (mine != generation) return@postDelayed
      val view = webView()
      if (view != null && activity.hasWindowFocus()) {
        view.onPause()
        view.onResume()
        log(activity, "REPAIR: WebView onPause + onResume at focus+${REPAIR_DELAY_MS}ms")
      }
    }, REPAIR_DELAY_MS)
    handler.postDelayed({ if (mine == generation) look("after the repair") }, REPAIR_DELAY_MS + 1500L)
  }

  /** How long after the focus the repair is made; 0 repairs at once. */
  private const val REPAIR_DELAY_MS = 5000L

  /** What the view and its window look like: for the log of a base-colour screen. */
  private fun webViewDetail(webView: WebView?): String {
    if (webView == null) return "no webview"
    val visible = android.graphics.Rect()
    val hasVisible = webView.getGlobalVisibleRect(visible)
    return "attached=${webView.isAttachedToWindow} alpha=${webView.alpha} layerType=${webView.layerType} " +
      "hardwareAccelerated=${webView.isHardwareAccelerated} globalVisibleRect=${if (hasVisible) visible.toShortString() else "none"} " +
      "parent=${webView.parent?.javaClass?.simpleName} windowToken=${webView.windowToken != null}"
  }

  private fun viewState(activity: Activity, webView: WebView?): String {
    if (webView == null) return "no webview yet"
    return "webView shown=${webView.isShown} visibility=${webView.visibility} window=${webView.windowVisibility} " +
      "focus=${activity.hasWindowFocus()} size=${webView.width}x${webView.height}"
  }

  private class Sample(val blank: Boolean, val text: String)

  /** Copy the window as the compositor has it into a small bitmap and count its colours. */
  private fun sample(activity: Activity, done: (Sample) -> Unit) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
      done(Sample(false, "PixelCopy needs Android 8"))
      return
    }
    val view = activity.window.decorView
    if (view.width <= 0 || view.height <= 0) {
      done(Sample(false, "window has no size"))
      return
    }
    val width = 90
    val height = (90L * view.height / view.width).toInt().coerceIn(1, 400)
    val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
    try {
      PixelCopy.request(activity.window, bitmap, { result ->
        if (result != PixelCopy.SUCCESS) {
          done(Sample(false, "PixelCopy failed ($result)"))
          return@request
        }
        val pixels = IntArray(width * height)
        bitmap.getPixels(pixels, 0, width, 0, 0, width, height)
        val counts = HashMap<Int, Int>()
        for (pixel in pixels) counts[pixel] = (counts[pixel] ?: 0) + 1
        val dominant = counts.maxByOrNull { it.value }
        val share = (dominant?.value ?: 0).toDouble() / pixels.size
        val text = "colours=${counts.size} dominant=#%08x share=%.3f".format(dominant?.key ?: 0, share)
        // Content on screen has text, covers and lines: far more than a handful of colours and no colour near the whole.
        done(Sample(share > 0.97 && counts.size < 40, text))
      }, handler)
    } catch (error: Exception) {
      done(Sample(false, "PixelCopy threw ${error.message}"))
    }
  }
}
