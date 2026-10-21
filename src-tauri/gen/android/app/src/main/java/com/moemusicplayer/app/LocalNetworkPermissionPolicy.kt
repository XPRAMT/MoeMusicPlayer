package com.moemusicplayer.app

/** Android 17 enforces local-network access only for apps targeting API 37 or newer. */
object LocalNetworkPermissionPolicy {
  const val ANDROID_17 = 37

  fun requiresRuntimePermission(sdkVersion: Int): Boolean = sdkVersion >= ANDROID_17

  fun shouldRequest(sdkVersion: Int, granted: Boolean): Boolean =
    requiresRuntimePermission(sdkVersion) && !granted
}
