# Android: the Bluetooth codec's sample rate follows the track

Status: Accepted and implemented (design approved by the owner; the Kotlin rules and the controller are unit-tested; the first device run failed at the adapter step, which the helper now works around; the fixed helper has not run yet)

## Context

ADR 0044 shows the Bluetooth codec's rate as the last step of the path line and ADR 0048 can show the real mixer rate with root or Shizuku. Both are read-only. On a phone whose Bluetooth route mixes at several rates (a KernelSU module adds one mixer output per rate), the codec itself still runs at the one rate the stack picked, so a 96 kHz track is resampled by the Bluetooth DSP or stack to the codec rate. The owner wants the codec's sample rate to move to the rate closest to the playing track, and to put it back afterwards.

The call that changes the rate is `BluetoothA2dp.setCodecConfigPreference(device, config)`, a `@SystemApi` guarded by `BLUETOOTH_PRIVILEGED`. An app cannot hold that permission, so the call has to be made by a process with shell or root identity. The two privileged channels the app already has (ADR 0048: `su`, and Shizuku's `newProcess` by reflection) can both start `app_process`.

## Decision

At every track change, when the setting is on and the track goes to an A2DP device in the shared mode, ask the system (through a small privileged helper) to change only the **sample rate** of the current codec configuration to the nearest rate the codec offers. Once per track; never changed back until the next track change; restored to the original rate when the follow is turned off, the device disconnects or the service ends (best effort). Any failure leaves things as they are and playback continues; the switch never blocks a load.

### Which rate (pure, `BluetoothRateRules.pickRate`)

Input: the track's sample rate (Rust sends `metadata.sampleRate` with the load command, the library's value; absent when unknown) and the union of `SAMPLE_RATE_*` flags the codec offers (`BluetoothCodecStatus.codecsSelectableCapabilities`, the entries of the current codec type, OR the current rate). Order:

1. The exact rate, when offered.
2. Else the nearest offered rate of the track's family (44.1 kHz family: multiples of 11 025 Hz; 48 kHz family: multiples of 8 000 Hz), so an integer-ratio step is preferred (176.4 kHz on LDAC gives 88.2 kHz, not 96 kHz).
3. Else the nearest offered rate overall.
Ties go to the lower rate. Null when the codec offers nothing or the track rate is unknown (no switch).

### When it may run (pure, `BluetoothRateRules.plan`)

Skipped, without any privileged call: the setting or the code switch (`BluetoothRateConfig.ENABLED`) is off; the breaker tripped; Android before 13 (the codec classes); the track does not go to an A2DP device (the chosen device when attached, else the system media route, `AudioOutputRouter.a2dpTargetAddress`); the USB exclusive route plays the track (`UsbExclusiveRules.route`); a DSD track (native passthrough or converted: the file's rate is the DSD rate); the track rate is unknown; neither root nor an authorized Shizuku (`MixerRateRules.selectSource`, the same sources and the same opt-in as ADR 0048: nothing probes `su` before the user pressed 「檢查 root」). If the announced codec state is known and already at the wanted rate, nothing is called. If it is not known (no broadcast since the app started; the broadcast is not sticky), the helper reads it and decides with the same `pickRate`.

### The privileged helper (`BluetoothRateHelper`)

A class of the app's own APK with a static `main`, started by root (`su -c`) or Shizuku (`sh -c`, shell user) as `CLASSPATH='<base.apk>' app_process /system/bin com.moemusicplayer.mediaindex.BluetoothRateHelper <command>`. Commands: `follow <address> <trackHz>` (read the codec, pick, switch if different), `set <address> <rateFlag>` (a restore, or a switch whose target the app already knows), `state <address> 0`. Every argument is validated before a command line is built (`parseRequest`, `shellCommand`: a MAC address, numbers only, the APK path single-quoted and refused if it contains a quote). It prints one `RESULT …` line (`OK <codec> <from> <to> <changed>`, `STATE …`, `ERR <text>`), a watchdog ends it after 6 s, and the app side times the process out after 8 s.

Inside, it gets a `BluetoothA2dp` proxy through `BluetoothAdapter.getProfileProxy`. A bare `app_process` never ran `ActivityThread.main`, so the Bluetooth module's service manager was never given to `BluetoothFrameworkInitializer` and `BluetoothAdapter.getDefaultAdapter()` returns null (the "no Bluetooth adapter" failure seen on the NX809J). The helper therefore, in this order and each step named in its error text: prepares the main looper (`Looper.prepareMainLooper`, the profile proxy calls back on it); builds the system context from `ActivityThread.systemMain().getSystemContext()` wrapped to speak as the package of its uid (`BluetoothRateRules.packageForUid`: `root` for uid 0, `com.android.shell` for the shell, so the service's package-versus-uid check passes); sets `BluetoothFrameworkInitializer.setBluetoothServiceManager(BluetoothServiceManager())` when none is set (module system APIs, by reflection; `app_process` does not enforce the hidden-API list); builds the adapter with the private `BluetoothAdapter.createAdapter(AttributionSource)`, else `BluetoothManager.getAdapter()`, else `getDefaultAdapter()`; the error lists what each way returned. Not used: calling the `bluetooth_manager` Binder by hand (the interface changes between releases). It then reads `getCodecStatus(device)` and calls `setCodecConfigPreference(device, config)` by reflection (both system APIs), with a config built by `BluetoothCodecConfig.Builder` from the current one: only `sampleRate` differs (codec type, priority, bits per sample, channel mode, the four codec-specific values and, on Android 15+, the extended codec type are copied). The class is kept by `consumer-rules.pro` (a renamed class would not be found by `app_process`).

#### Diagnostic by hand (adb, root)

`state` prints a readable `INFO codec=LDAC(4) sampleRate=48000 selectable=44100,48000,96000` line and then the `RESULT STATE <codec> <rateFlag> <selectableMask>` line; failures print `RESULT ERR <step>: <text>` (steps: looper, system-context, adapter, profile-proxy, remote-device, codec-status, set-codec-preference; a hang ends as `timeout at step <step>`). With the debug APK installed and the headset connected (address from `adb shell dumpsys bluetooth_manager`, or the log line of the app):

```
adb shell "pm path com.moemusicplayer.app"      # package:<base.apk>
adb shell "su -c \"CLASSPATH=<base.apk> app_process /system/bin com.moemusicplayer.mediaindex.BluetoothRateHelper state AA:BB:CC:DD:EE:FF 0\""
adb shell "su -c \"CLASSPATH=<base.apk> app_process /system/bin com.moemusicplayer.mediaindex.BluetoothRateHelper set AA:BB:CC:DD:EE:FF 8\""   # 8 = 96 kHz flag
```

`set` takes the `SAMPLE_RATE_*` flag (1 = 44.1, 2 = 48, 4 = 88.2, 8 = 96, 16 = 176.4, 32 = 192 kHz); `follow <address> <Hz>` picks the rate itself.

### The controller (`BluetoothRateController`, thread `moe-bt-rate`)

- `request` is called on the main thread at the load (only memory is touched); everything else runs on the controller's own scheduler thread: never on the main or playback thread, and the load does not wait for it. Playback may therefore start before the switch finishes; the short mute that the Bluetooth stack causes when it reconfigures the codec is accepted (ADR decision of the owner). Blocking the load until the switch is done was not chosen because it would hold the load acknowledgement hostage to a privileged process; if a gap before the first note proves annoying, the start can be held with `playWhenReady=false` until the result, as a later change.
- **Rate limit and coalescing.** Two privileged switches start at least `MIN_INTERVAL_MS` (1 s) apart; requests while one is waiting replace it (the newest track wins); a task that fires early waits for the rest of the interval. A skipped track (no call) does not use up the interval. Switching the follow on re-requests the loaded track at once (the service keeps the last track rate); a restore waits for the codec broadcast of the original rate before the next switch may plan from the codec state.
- **Wait.** After a switch that changed the rate, wait up to `WAIT_CHANGE_MS` (3 s) for the stack's codec broadcast (`BluetoothCodecWatch.awaitRate`, the existing `CODEC_CONFIG_CHANGED` receiver, now also keeping plain-value states with the offered rates). No broadcast (no permission, timeout) is logged and nothing else happens.
- **Once per track.** Only a load triggers a request. If the rate later drifts (developer options, the system), nothing is scheduled; the next track plans from the then-current state.
- **Saved rates.** The first switch of an address saves `original,applied` in SharedPreferences `moe_bt_rate` (address, upper case; the original is kept from the first switch, the applied value is updated). Restore (`restoreAll`) puts the original back only while the codec still runs at the applied rate, so a rate the user changed meanwhile is not overwritten; a failed restore stays saved for the next chance.
- **Restore triggers.** The setting turned off (also with no playback service: `BluetoothRateFollow.restoreWithoutService`); the service ends (`onDestroy`: the restore task is queued, then the worker thread finishes it and stops); a saved device that disappears (one attempt per disappearance, usually too late because the stack forgets the config); a saved device that is attached again while the setting is off; the service starting with the setting off and saved rates left over (process death).
- **Safety (same style as ADR 0048).** A process-wide `BluetoothRateBreaker`: `FAILURES_TO_TRIP` (3) failed or timed-out calls in a row, or a Media3 sink error or player error within `BREAKER_WINDOW_MS` (8 s) of a switch, stop the feature for the rest of the process (it also stops restores; the saved rates remain). Kill switches: the user setting and the code constant `BluetoothRateConfig.ENABLED`. It never reboots or restarts anything and never touches anything but one device's sample rate.

### Setting and UI

`settings.json` `androidBluetoothRateFollow`, **default off** (Rust `AppSettings::default` and `unwrap_or(false)`, front-end `DEFAULT_ANDROID_BLUETOOTH_RATE_FOLLOW`, locked by `tests/android-output-devices.test.mjs` and `settings::tests`). Commands `settings_get/set_android_bluetooth_rate_follow`; the setter stores the value for the service (`android_output::set_bluetooth_rate_follow`, read over JNI as `nativeBluetoothRateFollowEnabled`, -1 counts as off) and pushes it to a running service through the plugin command `setBluetoothRateFollow`. The page has a plain switch under the output-mode buttons in 「播放裝置」 (Android only, every device). Without root or an authorized Shizuku (`bluetoothRateSourceAvailable`, read from the mixer-rate panel status of ADR 0048) a one-line notice and a 「設定」 button that opens that same panel appear; the switch can still be saved. No new path-line text: the encoding step of ADR 0044 already shows the codec rate from the same broadcast.

## Consequences

- Adds a helper class and some code that only does something with root or Shizuku and a Bluetooth codec that offers more than one rate; everything else behaves as before.
- The load command carries `sampleRate`; the Kotlin side reads `metadata.sampleRate`.
- A switch makes the Bluetooth stack reconfigure the codec: a short mute, and possibly a new AudioTrack route; the breaker treats an error right after a switch as the cause.

## Tests

`BluetoothRateRulesTest` (rate pick, plan and skip reasons, restore rule, helper protocol and command quoting, breaker), `BluetoothRateControllerTest` (switch with a known and an unknown state, the original kept from the first switch, spacing and coalescing, no correction of a drift, every restore trigger, the breaker), `tests/android-output-devices.test.mjs` (default and notice), `settings::tests` (Rust), `android_playback` (the load command's `sampleRate`).

## Not done

Not verified on any device: that `app_process` can load the app's APK from the root and Shizuku shells; that the service-manager hand-over and the adapter construction above work on Android 13 to 16 (the earlier system-context-only attempt returned no adapter) and that uid 0 and uid 2000 passes the Bluetooth service's `BLUETOOTH_PRIVILEGED` and package checks (any exception text comes back in the `ERR` line and the log); that `setCodecConfigPreference` with a changed sample rate takes effect on the WF-1000XM6 over LDAC on the NX809J and what the mute sounds like; the Shizuku path (`Shizuku.newProcess` with `sh -c`); the stack forgetting or keeping the rate across reconnects (decides how useful the disconnect restore is); LE Audio and codecs whose selectable list is empty.
