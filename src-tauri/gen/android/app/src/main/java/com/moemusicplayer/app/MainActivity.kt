package com.moemusicplayer.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import org.json.JSONObject

class MainActivity : TauriActivity() {
  private val androidInsetsBridge = AndroidInsetsBridge()
  private var appWebView: WebView? = null

  override fun onStart() {
    super.onStart()
    ensurePlaybackService()
  }

  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) ResumeDiagnostics.onWindowFocus(this) { appWebView }
  }

  override fun onResume() {
    super.onResume()
    ResumeDiagnostics.log(this, "onResume: window=${appWebView?.windowVisibility}")
  }

  /**
   * The playback service is started once when the app boots, but `onTaskRemoved` (swiping the app away while paused)
   * or the system can stop it while this process lives on; Rust keeps queueing commands nobody polls, so tapping a
   * song does nothing. Starting an already running service is a no-op, so do it every time the app comes forward.
   */
  private fun ensurePlaybackService() {
    try {
      startService(android.content.Intent().setClassName(packageName, "com.moemusicplayer.mediaindex.PlaybackService"))
      LifecycleLog.write(this, "ensurePlaybackService")
    } catch (error: Exception) {
      LifecycleLog.write(this, "ensurePlaybackService failed: ${error.message}")
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    // DIAGNOSIS (2026-10-21, temporary): lets `adb forward tcp:9333 localabstract:webview_devtools_remote_<pid>` reach the
    // WebView's DevTools protocol, to trace why the page is sometimes not drawn after coming back. Remove with the diagnosis.
    WebView.setWebContentsDebuggingEnabled(true)
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    LifecycleLog.write(this, "onCreate savedState=${savedInstanceState != null}")
    // Transparent navigation bar for 3-button and gesture navigation alike: content draws
    // behind it and pads by the native bottom inset (--android-safe-bottom).
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      window.isNavigationBarContrastEnforced = false
      window.isStatusBarContrastEnforced = false
    }
    @Suppress("DEPRECATION")
    window.navigationBarColor = android.graphics.Color.TRANSPARENT

    val content = findViewById<View>(android.R.id.content)
    content.setPadding(0, 0, 0, 0)

    ViewCompat.setOnApplyWindowInsetsListener(content) { view, windowInsets ->
      val systemBars = windowInsets.getInsets(WindowInsetsCompat.Type.systemBars())
      val cutout = windowInsets.getInsets(WindowInsetsCompat.Type.displayCutout())
      val mandatoryGestures = windowInsets.getInsets(WindowInsetsCompat.Type.mandatorySystemGestures())
      val tappableType = WindowInsetsCompat.Type.tappableElement()
      val tappable = if (windowInsets.isVisible(tappableType)) {
        windowInsets.getInsets(tappableType)
      } else {
        Insets.NONE
      }
      val imeType = WindowInsetsCompat.Type.ime()
      val ime = if (windowInsets.isVisible(imeType)) windowInsets.getInsets(imeType) else Insets.NONE
      val layoutInsets = AndroidInsetsPolicy.calculate(
        systemBars = systemBars.toPolicyInsets(),
        displayCutout = cutout.toPolicyInsets(),
        mandatoryGestures = mandatoryGestures.toPolicyInsets(),
        tappableElement = tappable.toPolicyInsets(),
        ime = ime.toPolicyInsets(),
      )

      // The WebView spans the window. Only CSS interactive content receives safe insets.
      view.setPadding(
        layoutInsets.contentPadding.left,
        layoutInsets.contentPadding.top,
        layoutInsets.contentPadding.right,
        layoutInsets.contentPadding.bottom,
      )
      val webView = findWebView(content)
      androidInsetsBridge.update(layoutInsets, webView)
      windowInsets
    }
    ViewCompat.requestApplyInsets(content)
    // System Back (predictive-back compatible dispatcher): the web UI closes its top-most
    // overlay / page via window.__moeHandleBack(); when nothing is left, send the task to the
    // background instead of finishing, so the PlaybackService keeps playing.
    onBackPressedDispatcher.addCallback(this, object : androidx.activity.OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val webView = appWebView
        if (webView == null) {
          moveTaskToBack(true)
          return
        }
        webView.evaluateJavascript(
          "(function(){try{return !!(window.__moeHandleBack&&window.__moeHandleBack());}catch(e){return false;}})()",
        ) { result -> if (result != "true") moveTaskToBack(true) }
      }
    })
    requestNotificationPermissionIfNeeded()
  }

  override fun onDestroy() {
    LifecycleLog.write(this, "onDestroy changingConfigurations=$isChangingConfigurations finishing=$isFinishing")
    super.onDestroy()
  }

  /** One system prompt per process start until granted; denial only hides the media notification. */
  private fun requestNotificationPermissionIfNeeded() {
    val granted = Build.VERSION.SDK_INT < NotificationPermissionPolicy.TIRAMISU ||
      ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
    if (!NotificationPermissionPolicy.shouldRequest(Build.VERSION.SDK_INT, granted, notificationPermissionAsked)) return
    notificationPermissionAsked = true
    ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATION_PERMISSION_REQUEST)
  }

  private companion object {
    const val NOTIFICATION_PERMISSION_REQUEST = 4107
    @Volatile var notificationPermissionAsked = false
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.addJavascriptInterface(androidInsetsBridge, "MoeAndroidInsets")
    appWebView = webView
    // App UI, not a document: no pinch / double-tap page zoom (the viewport meta also sets
    // user-scalable=no). Scrolling and JS gestures are unaffected.
    webView.settings.setSupportZoom(false)
    webView.settings.builtInZoomControls = false
    webView.settings.displayZoomControls = false
  }

  private fun findWebView(view: View): WebView? {
    if (view is WebView) return view
    if (view is ViewGroup) {
      for (index in 0 until view.childCount) {
        findWebView(view.getChildAt(index))?.let { return it }
      }
    }
    return null
  }

  private inner class AndroidInsetsBridge {
    @Volatile
    private var currentJson = JSONObject()
      .put("safeLeftPx", 0)
      .put("safeTopPx", 0)
      .put("safeRightPx", 0)
      .put("safeBottomPx", 0)
      .put("imeBottomPx", 0)
      .toString()

    @JavascriptInterface
    fun currentInsetsJson(): String = currentJson

    fun update(
      insets: AndroidInsetsPolicy.LayoutInsets,
      webView: WebView?,
    ) {
      currentJson = JSONObject()
        .put("safeLeftPx", insets.controls.left)
        .put("safeTopPx", insets.controls.top)
        .put("safeRightPx", insets.controls.right)
        .put("safeBottomPx", insets.controls.bottom)
        .put("imeBottomPx", insets.imeBottom)
        .toString()

      val script = "window.dispatchEvent(new CustomEvent('moe:android-insets',{detail:$currentJson}));"
      webView?.let { currentWebView ->
        currentWebView.post { currentWebView.evaluateJavascript(script, null) }
      }
    }
  }

  private fun Insets.toPolicyInsets() = AndroidInsetsPolicy.InsetsPx(
    left = left,
    top = top,
    right = right,
    bottom = bottom,
  )
}
