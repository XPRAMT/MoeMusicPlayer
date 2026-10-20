package com.moemusicplayer.app

import android.content.Context
import android.content.pm.ApplicationInfo
import java.io.File

/**
 * A tiny lifecycle journal (`files/diagnostics/lifecycle.log`, bounded) for black-screen reports. It only writes in
 * debuggable builds, and it is a temporary aid: remove it (and its calls in [MainActivity]) if the problem does not come
 * back.
 */
object LifecycleLog {
  private const val MAX_BYTES = 64 * 1024

  fun write(context: Context, event: String) {
    if (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return
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
