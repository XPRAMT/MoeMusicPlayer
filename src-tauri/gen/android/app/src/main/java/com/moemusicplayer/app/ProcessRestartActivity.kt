package com.moemusicplayer.app

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle

/**
 * Starts the app in a fresh process (declared with `android:process=":restart"`, so it survives the death of the main
 * process). [MainActivity] uses it when a new activity is created in a process whose window was already destroyed: the
 * Rust runtime still runs but has no window to attach the new WebView to (the screen stays black and the page cannot talk
 * to the backend), so the only reliable recovery is a clean process. The queue, the position and the "was playing" flag
 * are saved by the backend, so playback resumes in the new process.
 */
class ProcessRestartActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val target = if (Build.VERSION.SDK_INT >= 33) {
      intent.getParcelableExtra(EXTRA_TARGET, Intent::class.java)
    } else {
      @Suppress("DEPRECATION")
      intent.getParcelableExtra(EXTRA_TARGET)
    }
    if (target != null) startActivity(target)
    finish()
    Runtime.getRuntime().exit(0)
  }

  companion object {
    const val EXTRA_TARGET = "moe_restart_target"
  }
}
