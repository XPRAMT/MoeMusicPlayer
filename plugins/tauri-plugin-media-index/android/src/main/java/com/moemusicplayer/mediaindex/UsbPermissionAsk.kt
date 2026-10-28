package com.moemusicplayer.mediaindex

import android.app.Activity
import android.app.ActivityManager
import android.app.Application
import android.content.Context
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper

/**
 * Asks Android's USB permission dialog by itself (ADR 0047): in the exclusive mode, when the USB audio device the routing
 * table plays on is attached and not permitted, the dialog is requested at once, with no panel of ours in between (the mode
 * choice is the request, the system dialog the consent). The decision is [UsbExclusiveRules.decideAsk]; the foreground is
 * asked of the system at each check ([foreground]). This object keeps whether a prompt is open and the devices the user
 * refused since they were attached (until they are unplugged or the process restarts). Main thread only.
 */
internal object UsbPermissionAsk {
    private var installed = false
    private var inFlight = false
    private var refused: Set<String> = emptySet()

    /**
     * Whether the app is in the foreground now, asked of the system at the time of the check (no hand-kept count):
     * the process importance is foreground ([UsbExclusiveRules.isForegroundImportance]).
     */
    fun foreground(): Boolean {
        val info = ActivityManager.RunningAppProcessInfo()
        return try {
            ActivityManager.getMyMemoryState(info)
            UsbExclusiveRules.isForegroundImportance(info.importance)
        } catch (_: RuntimeException) {
            false
        }
    }

    /**
     * Asks again the moment an activity of the app is resumed (the app comes to the front, or returns from a system
     * dialog), so a device attached or a mode chosen in the background is asked for then. No state is kept here; an
     * activity that is already resumed when the plugin loads gets one check at once (no callback comes for it).
     */
    fun install(activity: Activity) {
        if (installed) return
        installed = true
        activity.application.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityResumed(resumed: Activity) {
                PlaybackService.requestUsbPermissionIfNeeded("foreground")
            }

            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityStarted(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
        val state = (activity as? androidx.lifecycle.LifecycleOwner)?.lifecycle?.currentState
        if (state?.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED) == true) {
            PlaybackService.requestUsbPermissionIfNeeded("foreground")
        }
    }

    /**
     * The decision point: ask for the device [wish] plays on when [UsbExclusiveRules.decideAsk] says so. Logs one line
     * with the facts and the outcome; [trigger] names what caused the check.
     */
    fun check(context: Context, wish: UsbExclusiveRules.Wish, trigger: String) {
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager ?: return
        val devices = try {
            UsbAudioReader.audioDevices(usb)
        } catch (_: RuntimeException) {
            return
        }
        val facts = UsbExclusiveEngine.usbDevices(usb)
        refused = UsbExclusiveRules.pruneRefused(refused, facts)
        val askFacts = UsbExclusiveRules.AskFacts(
            wish = wish,
            devices = facts,
            permitted = devices.filter { usb.hasPermission(it) }.mapTo(HashSet()) { it.deviceName },
            refused = refused,
            foreground = foreground(),
            inFlight = inFlight,
        )
        val decision = UsbExclusiveRules.decideAsk(askFacts)
        android.util.Log.i("MoeMediaIndex", UsbExclusiveRules.decisionLine(trigger, askFacts, decision))
        val key = (decision as? UsbExclusiveRules.AskDecision.Ask)?.key ?: return
        val device = devices.firstOrNull { it.deviceName == key } ?: return
        android.util.Log.i("MoeMediaIndex", "USB exclusive: asking the USB permission of $key")
        request(context, usb, device) {}
    }

    /**
     * Shows the system dialog for [device] (the user's click on the card's 「前往授權」 asks again for a refused device
     * too). A grant applies to the playing track at once; [then] gets the answer.
     */
    fun request(context: Context, usb: UsbManager, device: UsbDevice, then: (Boolean) -> Unit) {
        if (usb.hasPermission(device)) {
            refused = UsbExclusiveRules.refusedAfterAnswer(refused, device.deviceName, true)
            PlaybackService.usbPermissionGranted()
            then(true)
            return
        }
        inFlight = true
        UsbAudioReader.askInTurn(context.applicationContext, usb, Handler(Looper.getMainLooper()), listOf(device)) {
            inFlight = false
            val granted = usb.hasPermission(device)
            refused = UsbExclusiveRules.refusedAfterAnswer(refused, device.deviceName, granted)
            if (granted) PlaybackService.usbPermissionGranted()
            then(granted)
        }
    }
}
