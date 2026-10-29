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
2. Else, when the track is above every offered rate, the highest offered rate (the system mixer picks the same one; one resample only).
3. Else the nearest offered rate of the track's family (44.1 kHz family: multiples of 11 025 Hz; 48 kHz family: multiples of 8 000 Hz), so an integer-ratio step is preferred (88.2 kHz on a codec with 44.1, 48 and 96 kHz gives 44.1 kHz).
4. Else the nearest offered rate overall.
Ties go to the lower rate. Null when the codec offers nothing or the track rate is unknown (no switch).

### When it may run (pure, `BluetoothRateRules.plan`)

Skipped, without any privileged call: the setting or the code switch (`BluetoothRateConfig.ENABLED`) is off; the breaker tripped; Android before 13 (the codec classes); the track does not go to an A2DP device (the chosen device when attached, else the system media route, `AudioOutputRouter.a2dpTargetAddress`); the USB exclusive route plays the track (`UsbExclusiveRules.route`); a native DSD passthrough track (raw DSD to a USB device, where Bluetooth is not involved; a DSD file converted to PCM is a normal PCM track, see below); the track rate is unknown; neither root nor an authorized Shizuku (`MixerRateRules.selectSource`, the same sources and the same opt-in as ADR 0048: nothing probes `su` before the user pressed 「檢查 root」). If the announced codec state is known and already at the wanted rate, nothing is called. If it is not known (no broadcast since the app started; the broadcast is not sticky), the helper reads it and decides with the same `pickRate`.

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

`settings.json` `androidBluetoothRateFollow`, **default off** (Rust `AppSettings::default` and `unwrap_or(false)`, front-end `DEFAULT_ANDROID_BLUETOOTH_RATE_FOLLOW`, locked by `tests/android-output-devices.test.mjs` and `settings::tests`). Commands `settings_get/set_android_bluetooth_rate_follow`; the setter stores the value for the service (`android_output::set_bluetooth_rate_follow`, read over JNI as `nativeBluetoothRateFollowEnabled`, -1 counts as off) and pushes it to a running service through the plugin command `setBluetoothRateFollow`. The page has a plain switch under the output-mode buttons in 「播放裝置」 (Android only, every device). Without root or an authorized Shizuku (`bluetoothRateSourceAvailable`, read from the mixer-rate panel status of ADR 0048) one short notice shared by the whole 「藍牙」 section (it also covers the codec list of ADR 0052) and a single 「前往授權」 button that opens that same panel appear; the switch can still be saved. No new path-line text: the encoding step of ADR 0044 already shows the codec rate from the same broadcast.

### LDAC bitrate mode (read through the same helper)

The Bluetooth row's codec step names LDAC's bitrate mode: 「LDAC 990 kbps」 (`codecSpecific1` 1000; 909 in the 44.1 kHz family), 660 (1001), 330 (1002), 「LDAC 自適應」 (1003, adaptive bitrate), 「LDAC 預設」 (0), plain 「LDAC」 when the mode was not read. `getCodecSpecific1()` is a system API an app cannot call, so the value is read by the privileged helper: `state` now prints `codecSpecific=a,b,c,d` in the `INFO` line and `RESULT STATE <codec> <rateFlag> <selectableMask> <cs1> <cs2> <cs3> <cs4>` (`parseResult` still accepts the three-number form; a partial tail is an error). `BluetoothLdacModeReader` (own thread, separate from the follow and independent of its setting) runs it only when root or an authorized Shizuku exists, only while the stack announced LDAC for the address, on the codec-config-changed broadcast and on a track change when the mode is not known yet (the announcement drops the known mode); never polled. Requests are debounced (500 ms), coalesced and at least 2 s apart; 3 failed reads in a row stop the reading for the process. Without a source nothing is read and nothing is shown, no guess. The mode is kept in `BluetoothCodecWatch` (`ldacMode`) and merged into the codec JSON (`ldacKbps`, `ldacAdaptive`, `ldacDefault`); the app no longer reads `codecSpecific1` from the broadcast itself.

What 0 means: the stack's codec-specific value is unset, i.e. the stack's own default, not one of the four user-selectable modes (those are 1000 to 1003, the numbers the developer-options "playback quality" sets). The Bluetooth stack log of the NX809J shows `mCodecSpecific1:0` for the current LDAC session. AOSP's LDAC encoder is believed to start in adaptive bitrate when nothing is set, but that is not confirmed here, so 0 is shown as 「LDAC 預設」 and never as adaptive. The instantaneous bitrate the adaptive mode picks is chosen inside the Qualcomm DSP / encoder and is not readable by any API; only the mode is shown, never a current bitrate.

## Consequences

- Adds a helper class and some code that only does something with root or Shizuku and a Bluetooth codec that offers more than one rate; everything else behaves as before.
- The load command carries `sampleRate`; the Kotlin side reads `metadata.sampleRate`.
- A switch makes the Bluetooth stack reconfigure the codec: a short mute, and possibly a new AudioTrack route; the breaker treats an error right after a switch as the cause.

## Tests

LDAC mode: Kotlin `BluetoothLdacModeReaderTest` (rules, coalescing, interval, no source, failure stop), `BluetoothRateRulesTest` (state answer with and without the values), `BluetoothCodecRulesTest` (mode naming); JS `tests/output-format.test.mjs`.


`BluetoothRateRulesTest` (rate pick, plan and skip reasons, restore rule, helper protocol and command quoting, breaker), `BluetoothRateControllerTest` (switch with a known and an unknown state, the original kept from the first switch, spacing and coalescing, no correction of a drift, every restore trigger, the breaker), `tests/android-output-devices.test.mjs` (default and notice), `settings::tests` (Rust), `android_playback` (the load command's `sampleRate`).

## Not done

LDAC mode: not verified on a device that the helper's `state` returns `codecSpecific1` as the stack uses it (1000 to 1003 after the developer option is changed, 0 otherwise), nor which of these the NX809J reports for its default session.


Not verified on any device: that `app_process` can load the app's APK from the root and Shizuku shells; that the service-manager hand-over and the adapter construction above work on Android 13 to 16 (the earlier system-context-only attempt returned no adapter) and that uid 0 and uid 2000 passes the Bluetooth service's `BLUETOOTH_PRIVILEGED` and package checks (any exception text comes back in the `ERR` line and the log); that `setCodecConfigPreference` with a changed sample rate takes effect on the WF-1000XM6 over LDAC on the NX809J and what the mute sounds like; the Shizuku path (`Shizuku.newProcess` with `sh -c`); the stack forgetting or keeping the rate across reconnects (decides how useful the disconnect restore is); LE Audio and codecs whose selectable list is empty.

## Correction: a rate request pins the current codec (found on the NX809J)

The first version copied the current config, including `codecPriority`, which is 0 in the stack's own config ("no user preference"). On the NX809J with a headset that supports LDAC, aptX Adaptive, LHDC, aptX HD, aptX, AAC and SBC, the request `setCodecConfigPreference({LDAC, priority 0, 48 kHz})` was followed about 1.5 s later by the stack choosing aptX Adaptive at 96 kHz: with no user preference the stack re-selects its own favourite (its log says it found a saved codec for the device), so the codec jumped and the requested rate was lost ("not confirmed within 3000 ms"). A request with priority `CODEC_PRIORITY_HIGHEST` (1000000) is a user preference and is kept, as the explicit codec switch of ADR 0052 showed.

Fix: every rate-only request (follow, set and the restore) sends the **current codec** with `codecPriority = CODEC_PRIORITY_HIGHEST` and only the sample rate changed (`BluetoothRateRules.rateRequest`, unit-tested: codec type kept, priority highest). Consequence: the rate request also pins the current codec as the preferred one, so the stack keeps it afterwards.

With the codec list of ADR 0052 off and the rate follow on, nothing but the rate follow sends a request, and it keeps the codec in use. The codec controller's "already in use" check compares the announced codec key with the target and ignores rate and priority, so the two do not fight; a switch by the codec list changes the preferred codec, and later rate requests pin whatever codec is then in use.

Another app, the Qualcomm aptX service (`com.qualcomm.qtil.aptxals`), also calls `setCodecConfigPreference` with aptX Adaptive quality values at times. This app does not try to control it; a codec or rate change it makes shows up as an unconfirmed switch or as a different codec in use.

## Shizuku runner

Over Shizuku every helper call failed with "process hasn't exited" because the runner read `exitValue()` while the helper (about a second to start) still ran, and three such failures tripped the breaker for the process. The runner now takes the exit code from `waitFor()` on its own thread (see ADR 0048, "Shizuku runner"), and a failure of that class is not counted by the breaker, the codec list's breaker or the LDAC mode reader. The breakers stay process-wide: a tripped breaker lasts until the app process restarts, which is intended for real device failures only.

## Knowing the codec reliably

The stack's codec broadcast is not sticky: nothing arrives when the app starts, the service restarts or the headset was already connected, so the path line could show only 「（藍牙）」 until the codec next changed. With root or an authorized Shizuku the codec is therefore read through the helper (`state`, which gives codec type, rate flag and `codecSpecific1`) by `BluetoothCodecStatusReader` whenever it is unknown: when the playback service starts with an A2DP route, at every track load, when the output page asks (the status poll), at a connection or route change (a device that appears among the attached A2DP devices starts the backoff over), and as a forced refresh when a codec or rate change was not announced within the wait (`BluetoothCodecWatch.onNeedRefresh`). A failed read is retried after 0.5, 1, 2 and 4 s and then stops until the next connection, route change or track load re-arms it; a known codec is never read; a read is one privileged call on its own thread with its own breaker (3 real failures in a row, or a sink or player error within 8 s, stop it for the process), and a failure of the runner itself (ADR 0048, "Shizuku runner") is not counted. The result is stored at once in `BluetoothCodecWatch`, so the next status poll of the page (every 500 ms) shows it; a later broadcast replaces it with the full data. A codec that cannot be named from `state` (an Android 15+ vendor codec has no source type and its name only comes with the broadcast) stays unknown. Without a source nothing is read, and the codec rate is never inferred from the mixer rate.

## A DSD file converted to PCM follows

A DSD file (DSF or WavPack-DSD) that is converted to PCM is a normal PCM track over Bluetooth. Its rate is the converter's output, 176400 Hz on Android (`BluetoothRateRules.DSD_PCM_OUTPUT_HZ`), not the DSD bit rate the library holds (2.8224 MHz and so on), so the service passes that rate and `dsd = false` (`BluetoothRateRules.trackRate`); only a native passthrough load is skipped (`SkipReason.DSD`). Earlier every DSD track was skipped, so the codec stayed at the previous track's rate while the mixer ran at the converter's. `pickRate` then chooses from the codec's selectable rates: 176.4 kHz is above everything LDAC offers (44.1, 48, 88.2, 96 kHz), so it gets the codec's highest rate, 96 kHz. That is the rate the system mixer already picks for such a track (the path line showed 「176.4 kHz → 96 kHz（系統）」); pinning the codec lower (the 44.1 kHz family's 88.2 kHz) would resample twice, 176.4 → 96 kHz in the mixer and 96 → 88.2 kHz in the Bluetooth path, and keep less bandwidth.

修復經過、邊界情況與驗證方式：[`docs/lessons/bluetooth-rate-follow.md`](../lessons/bluetooth-rate-follow.md)。
