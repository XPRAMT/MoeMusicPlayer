package com.moemusicplayer.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalNetworkPermissionPolicyTest {
  @Test fun requestsOnlyOnAndroid17PlusWhenNotGranted() {
    assertFalse(LocalNetworkPermissionPolicy.shouldRequest(36, granted = false))
    assertFalse(LocalNetworkPermissionPolicy.shouldRequest(36, granted = true))
    assertTrue(LocalNetworkPermissionPolicy.shouldRequest(37, granted = false))
    assertTrue(LocalNetworkPermissionPolicy.shouldRequest(38, granted = false))
    assertFalse(LocalNetworkPermissionPolicy.shouldRequest(37, granted = true))
  }
}
