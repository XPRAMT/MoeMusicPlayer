package com.moemusicplayer.app

import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import org.json.JSONObject

class MainActivity : TauriActivity() {
  private val androidInsetsBridge = AndroidInsetsBridge()

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

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
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.addJavascriptInterface(androidInsetsBridge, "MoeAndroidInsets")
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
