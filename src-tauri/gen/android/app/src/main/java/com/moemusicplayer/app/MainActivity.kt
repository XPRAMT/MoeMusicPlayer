package com.moemusicplayer.app

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    val content = findViewById<android.view.View>(android.R.id.content)
    val initialLeft = content.paddingLeft
    val initialTop = content.paddingTop
    val initialRight = content.paddingRight
    val initialBottom = content.paddingBottom
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, windowInsets ->
      val safeInsets = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.mandatorySystemGestures(),
      )
      view.setPadding(
        initialLeft + safeInsets.left,
        initialTop + safeInsets.top,
        initialRight + safeInsets.right,
        initialBottom + safeInsets.bottom,
      )
      windowInsets
    }
    ViewCompat.requestApplyInsets(content)
  }
}
