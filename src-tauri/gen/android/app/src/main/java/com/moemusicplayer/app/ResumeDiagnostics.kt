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
 * Evidence for the "the screen shows only the app's background colour after coming back" symptom. It is rare, so it is
 * recorded by the app itself, in a file that survives the process and can be read with `adb pull` (the logcat buffer is
 * small and is cleared by anyone who looks): `Android/data/<package>/files/diagnostics/resume.log`, also sent to logcat
 * (tag `MoeResume`).
 *
 * Every time the activity resumes, the screen is sampled a few seconds later (what the compositor shows, through
 * [PixelCopy]). A screen of one colour is the symptom: then the state of the page and the view is written down and
 * remedies are tried one after another, each followed by a new sample, so the log says which one (if any) brings the
 * content back.
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
   * The activity came forward or its window got the focus. An activity is resumed while the screen is still off or under
   * the lock screen, when its window is not visible yet; the window becomes visible later, and that is when the page can
   * be left undrawn. So only a moment with a visible, focused window starts a check, and the resumes in between do not
   * cancel a check that is running.
   */
  fun schedule(activity: Activity, webView: () -> WebView?, reason: String) {
    val view = webView()
    val visible = view != null && view.windowVisibility == View.VISIBLE && activity.hasWindowFocus()
    log(activity, "$reason: ${viewState(activity, view)}${if (visible) "" else " (window not visible yet: no check)"}")
    if (!visible) return
    val mine = ++generation
    // Seen on a phone (2026-10-21): the page is alive and producing frames but the compositor shows none of it, and telling
    // the WebView to pause and resume brings the content back. Do that at once instead of waiting to find the screen blank.
    handler.postDelayed({
      if (mine == generation) {
        webView()?.let {
          it.onPause()
          it.onResume()
          log(activity, "WebView onPause + onResume after the window got the focus")
        }
      }
    }, 300L)
    handler.postDelayed({
      if (mine == generation) check(activity, webView(), mine)
    }, 1500L)
  }

  private fun viewState(activity: Activity, webView: WebView?): String {
    if (webView == null) return "no webview yet"
    return "webView shown=${webView.isShown} visibility=${webView.visibility} window=${webView.windowVisibility} " +
      "focus=${activity.hasWindowFocus()} size=${webView.width}x${webView.height} progress=${webView.progress}"
  }

  private fun check(activity: Activity, webView: WebView?, mine: Int) {
    sample(activity) { first ->
      if (mine != generation) return@sample
      if (!first.blank) {
        log(activity, "screen ok: ${first.text}")
        return@sample
      }
      log(activity, "BLANK SCREEN: ${first.text}; ${viewState(activity, webView)}")
      if (webView == null) return@sample
      // What the page says (a renderer that does not answer within 2 s is a finding too).
      var answered = false
      webView.evaluateJavascript(
        "(function(){return document.visibilityState+' focus='+document.hasFocus()+' bodyChildren='+" +
          "(document.body?document.body.children.length:-1)+' raf='+(window.__moeRaf?(Date.now()-window.__moeRaf)+'ms ago':'never');})()",
      ) { result ->
        answered = true
        log(activity, "page says: $result")
      }
      handler.postDelayed({
        if (!answered) log(activity, "page did not answer within 2 s (renderer frozen?)")
      }, 2000L)
      runRemedies(activity, webView, mine)
    }
  }

  /** One remedy at a time, each followed by a sample 1.5 s later; stops at the first that brings the content back. */
  private fun runRemedies(activity: Activity, webView: WebView, mine: Int) {
    val remedies = listOf<Pair<String, () -> Unit>>(
      "WebView onPause + onResume" to {
        webView.onPause()
        webView.onResume()
      },
      "style nudge on the page root" to {
        webView.evaluateJavascript(
          "(function(){var r=document.documentElement;r.style.willChange='transform';r.style.transform='translateZ(0)';" +
            "void r.offsetHeight;requestAnimationFrame(function(){window.__moeRaf=Date.now();" +
            "r.style.transform='';r.style.willChange='';});return 1;})()",
          null,
        )
      },
      "toggle WebView visibility" to {
        webView.visibility = View.INVISIBLE
        webView.visibility = View.VISIBLE
      },
      "switch the layer type to software and back" to {
        val type = webView.layerType
        webView.setLayerType(View.LAYER_TYPE_SOFTWARE, null)
        webView.post { webView.setLayerType(if (type == View.LAYER_TYPE_SOFTWARE) View.LAYER_TYPE_NONE else type, null) }
      },
      "take the WebView out of its parent and put it back" to {
        val parent = webView.parent as? android.view.ViewGroup
        if (parent != null) {
          val index = parent.indexOfChild(webView)
          val params = webView.layoutParams
          parent.removeViewAt(index)
          parent.addView(webView, index, params)
        }
      },
      "resize by one pixel and back" to {
        val params = webView.layoutParams
        val height = params.height
        params.height = if (height > 0) height - 1 else webView.height - 1
        webView.layoutParams = params
        webView.post {
          params.height = height
          webView.layoutParams = params
        }
      },
    )
    fun attempt(index: Int) {
      if (mine != generation || index >= remedies.size) return
      val (name, apply) = remedies[index]
      apply()
      handler.postDelayed({
        sample(activity) { after ->
          if (mine != generation) return@sample
          if (!after.blank) {
            log(activity, "REMEDY WORKED: '$name' ($index): ${after.text}")
          } else {
            log(activity, "remedy did not help: '$name': ${after.text}")
            attempt(index + 1)
          }
        }
      }, 1500L)
    }
    attempt(0)
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
