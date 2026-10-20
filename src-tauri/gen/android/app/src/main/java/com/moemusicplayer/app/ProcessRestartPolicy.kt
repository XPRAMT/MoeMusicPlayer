package com.moemusicplayer.app

import android.content.Context
import java.io.File

/**
 * When a new [MainActivity] needs a fresh process: the process already ran an activity whose window the Rust runtime has
 * destroyed (the activity was destroyed for a reason other than a configuration change: the system reclaimed it in the
 * background, or the user removed the app from the recent list while the playback service kept the process alive).
 */
object ProcessRestartPolicy {
  fun shouldRestart(processHadActivity: Boolean, previousDestroyedForReal: Boolean): Boolean =
    processHadActivity && previousDestroyedForReal
}

/** A tiny lifecycle journal (`files/diagnostics/lifecycle.log`, bounded) to explain a black screen after the fact. */
object LifecycleLog {
  private const val MAX_BYTES = 64 * 1024

  fun write(context: Context, event: String) {
    try {
      val file = File(context.filesDir, "diagnostics/lifecycle.log")
      file.parentFile?.mkdirs()
      if (file.length() > MAX_BYTES) {
        val tail = file.readBytes().takeLast(MAX_BYTES / 2).toByteArray()
        file.writeBytes(tail)
      }
      val stamp = java.text.SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", java.util.Locale.US).format(java.util.Date())
      file.appendText("$stamp pid=${android.os.Process.myPid()} $event\n")
    } catch (_: Exception) {
      // The journal is best effort.
    }
  }
}
