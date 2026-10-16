package com.moemusicplayer.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NotificationPermissionPolicyTest {
  @Test fun requestsOnlyOnAndroid13PlusWhenNotGranted() {
    assertTrue(NotificationPermissionPolicy.shouldRequest(33, granted = false, alreadyAskedThisProcess = false))
    assertTrue(NotificationPermissionPolicy.shouldRequest(36, granted = false, alreadyAskedThisProcess = false))
    assertFalse(NotificationPermissionPolicy.shouldRequest(32, granted = false, alreadyAskedThisProcess = false))
    assertFalse(NotificationPermissionPolicy.shouldRequest(34, granted = true, alreadyAskedThisProcess = false))
    assertFalse(NotificationPermissionPolicy.shouldRequest(34, granted = false, alreadyAskedThisProcess = true))
  }
}
