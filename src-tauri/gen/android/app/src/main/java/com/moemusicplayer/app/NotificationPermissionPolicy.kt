package com.moemusicplayer.app

/** Decides whether to ask for POST_NOTIFICATIONS (needed for the media notification on Android 13+). */
object NotificationPermissionPolicy {
  const val TIRAMISU = 33

  fun shouldRequest(sdkInt: Int, granted: Boolean, alreadyAskedThisProcess: Boolean): Boolean =
    sdkInt >= TIRAMISU && !granted && !alreadyAskedThisProcess
}
