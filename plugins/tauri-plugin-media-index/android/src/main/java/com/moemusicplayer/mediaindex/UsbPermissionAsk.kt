package com.moemusicplayer.mediaindex

import android.app.Activity
import android.app.Application
import android.content.Context
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View

/**
 * Asks Android's USB permission dialog by itself (ADR 0047): in the exclusive mode, when the USB audio device the routing
 * table plays on is attached and not permitted, the dialog is requested at once, with no panel of ours in between (the mode
 * choice is the request, the system dialog the consent). The decision is [UsbExclusiveRules.autoRequestTarget]; this object
 * keeps what it needs: whether the app is in the foreground (a started activity), whether a prompt is open, and the devices
 * the user refused since they were attached (until they are unplugged or the process restarts). Main thread only.
 */
internal object UsbPermissionAsk {
    private var started = 0
    private var installed = false
    private var inFlight = false
    private var refused: Set<String> = emptySet()

    val foreground: Boolean get() = started > 0

    /**
     * Follows the app's activities (the app is in the foreground while one is started), and asks again the moment the
     * app comes to the front, so a device attached or a mode chosen in the background is asked for then.
     */
    fun install(activity: Activity) {
        if (installed) return
        installed = true
        started = if (activity.window?.decorView?.windowVisibility == View.VISIBLE) 1 else 0
        activity.application.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(started: Activity) {
                this@UsbPermissionAsk.started++
                if (this@UsbPermissionAsk.started == 1) PlaybackService.requestUsbPermissionIfNeeded()
            }

            override fun onActivityStopped(stopped: Activity) {
                this@UsbPermissionAsk.started = maxOf(0, this@UsbPermissionAsk.started - 1)
            }

            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
            override fun onActivityDestroyed(activity: Activity) = Unit
        })
    }

    /** The decision point: ask for the device [wish] plays on when [UsbExclusiveRules.autoRequestTarget] says so. */
    fun check(context: Context, wish: UsbExclusiveRules.Wish) {
        val usb = context.getSystemService(Context.USB_SERVICE) as? UsbManager ?: return
        val devices = try {
            UsbAudioReader.audioDevices(usb)
        } catch (_: RuntimeException) {
            return
        }
        val facts = UsbExclusiveEngine.usbDevices(usb)
        refused = UsbExclusiveRules.pruneRefused(refused, facts)
        val key = UsbExclusiveRules.autoRequestTarget(
            UsbExclusiveRules.AskFacts(
                wish = wish,
                devices = facts,
                permitted = devices.filter { usb.hasPermission(it) }.mapTo(HashSet()) { it.deviceName },
                refused = refused,
                foreground = foreground,
                inFlight = inFlight,
            ),
        ) ?: return
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
