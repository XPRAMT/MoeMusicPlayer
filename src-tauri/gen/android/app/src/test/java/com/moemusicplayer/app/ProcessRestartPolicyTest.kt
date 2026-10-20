package com.moemusicplayer.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProcessRestartPolicyTest {
  @Test fun theFirstActivityOfAProcessNeverRestarts() {
    assertFalse(ProcessRestartPolicy.shouldRestart(processHadActivity = false, previousDestroyedForReal = false))
    assertFalse(ProcessRestartPolicy.shouldRestart(processHadActivity = false, previousDestroyedForReal = true))
  }

  @Test fun aConfigurationChangeKeepsTheWindowSoItDoesNotRestart() {
    assertFalse(ProcessRestartPolicy.shouldRestart(processHadActivity = true, previousDestroyedForReal = false))
  }

  @Test fun aNewActivityAfterARealDestroyNeedsAFreshProcess() {
    assertTrue(ProcessRestartPolicy.shouldRestart(processHadActivity = true, previousDestroyedForReal = true))
  }
}
