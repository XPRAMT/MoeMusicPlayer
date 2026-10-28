package com.moemusicplayer.mediaindex

import java.io.ByteArrayInputStream
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BoundedProcessTest {
    /** Like Shizuku's remote process: exitValue() throws until the process has finished; waitFor() blocks. */
    private class FakeProcess(
        private val stdout: String,
        private val runMs: Long,
        private val code: Int = 0,
        private val exitValueThrows: RuntimeException = IllegalArgumentException("process hasn't exited"),
    ) : Process() {
        private val finished = CountDownLatch(1)
        @Volatile var exitValueCalledWhileRunning = false
        @Volatile var destroyed = false
        private val started = System.nanoTime()

        private fun over() = finished.count == 0L || TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started) >= runMs

        init {
            Thread {
                Thread.sleep(runMs)
                finished.countDown()
            }.apply { isDaemon = true }.start()
        }

        override fun getOutputStream(): OutputStream = object : OutputStream() { override fun write(b: Int) {} }
        override fun getInputStream(): InputStream = object : InputStream() {
            private val bytes = ByteArrayInputStream(stdout.toByteArray())
            override fun read(): Int {
                // The stream ends before the process has: the helper prints its result and then halts.
                return bytes.read()
            }
        }
        override fun getErrorStream(): InputStream = ByteArrayInputStream(ByteArray(0))
        override fun waitFor(): Int {
            finished.await()
            return if (destroyed) 143 else code
        }
        override fun exitValue(): Int {
            if (!over() && !destroyed) {
                exitValueCalledWhileRunning = true
                throw exitValueThrows
            }
            return if (destroyed) 143 else code
        }
        override fun destroy() {
            destroyed = true
            finished.countDown()
        }
    }

    @Test fun aProcessThatFinishesAfterItsOutputEndedGivesItsExitCodeWithoutAskingTooEarly() {
        val process = FakeProcess("RESULT OK", runMs = 300)
        val result = BoundedProcess.read(process, 5_000, 1024)
        assertEquals("RESULT OK", result.stdout)
        assertEquals(0, result.exitCode)
        assertFalse(result.timedOut)
        assertFalse("exitValue() must not be called while the process runs", process.exitValueCalledWhileRunning)
    }

    @Test fun anyExceptionTypeOfAnEarlyExitValueIsNeverSeen() {
        for (error in listOf<RuntimeException>(IllegalThreadStateException("x"), IllegalStateException("x"), IllegalArgumentException("x"))) {
            val process = FakeProcess("ok", runMs = 100, code = 3, exitValueThrows = error)
            val result = BoundedProcess.read(process, 5_000, 1024)
            assertEquals(3, result.exitCode)
            assertFalse(process.exitValueCalledWhileRunning)
        }
    }

    @Test fun aProcessThatRunsTooLongIsDestroyedByTheWatchdog() {
        val process = FakeProcess("partial", runMs = 60_000)
        val result = BoundedProcess.read(process, 150, 1024)
        assertTrue(result.timedOut)
        assertTrue(process.destroyed)
        assertEquals("partial", result.stdout)
    }

    @Test fun theKeptOutputIsLimited() {
        val result = BoundedProcess.read(FakeProcess("x".repeat(5_000), runMs = 10), 5_000, 100)
        assertEquals(100, result.stdout.length)
    }

    @Test fun theRunnerBugIsRecognisedAndNotCountedByTheBreaker() {
        assertTrue(PrivilegedRunnerErrors.isRunnerBug("java.lang.IllegalArgumentException: process hasn't exited"))
        assertTrue(PrivilegedRunnerErrors.isRunnerBug("java.lang.IllegalThreadStateException"))
        assertFalse(PrivilegedRunnerErrors.isRunnerBug("timeout at step adapter"))
        assertFalse(PrivilegedRunnerErrors.isRunnerBug(null as String?))
        val breaker = BluetoothRateBreaker(8_000, 3)
        repeat(5) {
            breaker.switchStarted()
            breaker.switchIgnored()
        }
        assertFalse(breaker.tripped)
        // Real failures still trip it.
        repeat(3) {
            breaker.switchStarted()
            breaker.switchEnded(0, false)
        }
        assertTrue(breaker.tripped)
    }

    @Test fun theControllersLeaveTheirBreakersAloneOnARunnerBug() {
        val runnerBug = BluetoothRateRules.HelperResult.Failed("java.lang.IllegalArgumentException: process hasn't exited")
        assertTrue(PrivilegedRunnerErrors.isRunnerBug(runnerBug))
        assertFalse(PrivilegedRunnerErrors.isRunnerBug(BluetoothRateRules.HelperResult.Failed("denied")))
        assertFalse(PrivilegedRunnerErrors.isRunnerBug(BluetoothRateRules.HelperResult.TimedOut))
    }
}
