# Android: the real mixer rate of a track (the 「系統」 step of the path line)

Status: Accepted (the lookup order was decided by the owner; the research was verified on one phone with a root shell and an adb shell; the player code is not changed yet, nothing below is built or run on a device)

Decision date: 2026-10-28, Asia/Taipei (UTC+08:00)

## Context

ADR 0044 shows the Android output as one path line, 「AudioTrack 取樣率 → 混音器取樣率（系統） → 編碼取樣率（編碼）．N 聲道」. The middle step, `output.mixerSampleRate`, is read by `AudioOutputRouter.readMixerRate` with the public API `AudioTrack.getNativeOutputSampleRate(AudioManager.STREAM_MUSIC)` and is used for display only; it never drives the AudioTrack rate. ADR 0044 listed "that `getNativeOutputSampleRate` reports the routed output's mixer rate on each device" as still needing a device check.

On a phone (ZTE NX809J, RedMagic OS 11, Android 16, rooted with KernelSU, WF-1000XM6 over LDAC) the page showed 「96 kHz → 44.1 kHz（系統） →（藍牙）」 while the 96 kHz track was really mixed on a 96 kHz thread.

Why this phone differs from a stock one: its Bluetooth route has several mixer outputs with different rates. A KernelSU module (outside this repository) adds one deep-buffer mix port per rate (44.1, 48, 88.2 and 96 kHz) so that each track is mixed at its own rate; a stock ROM has a single 48 kHz deep-buffer output. The module also works around an AOSP behavior of `AudioPolicyManager::selectOutput`: the sample rate is only compared for requests above 48 kHz. That behavior was reported to AOSP (Issue Tracker 578577149, Gerrit change 4396965, work in progress).

## Findings

All of these were checked on the phone (adb shell and root shell, the Android 14 framework sources, and the AOSP `main` sources).

1. **The API is stream-level, not per-track.** `getNativeOutputSampleRate(STREAM_MUSIC)` asks the audio policy for the output of the music stream (`AudioPolicyManager::getOutput(stream)`). That function calls `selectOutput(outputs, flags)` with flags `DEEP_BUFFER` (when deep buffer is used for media) and **no sample rate, format or channel mask**, and its own comment says this "may result in some discrepancy" for `AudioSystem::getOutputSamplingRate()`. With no rate every candidate ties on the rate criterion, so the result is the deep-buffer output with the lowest I/O handle (the one opened first). It does not depend on the playing track, and with several mixer outputs the opening order differs per boot and per Bluetooth reconnect.
2. **Observed.** The same player switching between 44.1, 48 and 96 kHz songs:
   - per-rate ports with the patched policy: every track ran on a thread with its own rate; the API returned 44100 for the whole run (the 44.1 kHz output was opened first);
   - per-rate ports only: the API returned 96000 for the whole run and all three songs ran on the 96 kHz thread (the 96 kHz output was opened first).
   The API never changed with the song in any recording. On a stock device with one mixer rate it is correct.
3. **The real value of one track is available with a privileged shell.** `AudioTrack.getMetrics()` exposes `AudioTrack.MetricsConstants.PORT_ID` (`android.media.audiotrack.portId`), the port id of the track inside AudioFlinger. It can be looked up in two dumps:
   - `dumpsys media.audio_flinger` (live state): the row of the track has a `Port Id` column equal to the portId; the row sits under its `Output thread … name AudioOut_xx`, whose `Sample rate:` line is the real mixer rate (tracks of 44.1, 48 and 96 kHz gave portIds 214, 215, 216 and threads of 44100, 48000, 96000 Hz).
   - `dumpsys media.metrics`: record `audio.track.<portId>` has an `event#=create` event with `threadId=N`, `sampleRate` / `effective.sampleRate` (the client rate), `flags` and `routedDeviceId`; record `audio.thread.N` has an `event#=readParameters` event with `sampleRate` (the real mixer rate). The records are kept in a ring buffer.
   Both are plain `dumpsys` text, not an API.
4. **Who can read the dumps.** The shell user (uid 2000, which is what Shizuku runs as when it was started through adb) can run both dumps (about 73 KB and 335 KB, about 20 ms each); root can too. A normal app cannot.
5. **Without a privileged shell there is no source.** `getMetrics()` has no thread id; `getNativeOutputSampleRate` and `AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE` are stream-level; `getRoutedDevice()` gives only the device; `AudioManager.getSupportedMixerAttributes` and `setPreferredMixerAttributes` return an error for every device that is not a USB output (`audio_is_usb_out_device` in the policy), so they do not help for Bluetooth.
6. **The codec step is a separate value.** The DSP backend rate of the Bluetooth codec (LDAC at 96 kHz on that phone) is visible with root in logcat (`btfm_get_sampling_rate`); in the app it comes from the codec broadcast of ADR 0044. It was not re-tested together with the findings above.

## Decision

The 「系統」 value is taken from the first source that works, in this order:

1. **Root.** If the device is rooted and the app is allowed to use it, run `su -c "dumpsys media.audio_flinger"` and read the thread of the track.
2. **Shizuku.** Otherwise, if Shizuku is running and this app has its permission, run the same command through Shizuku.
3. **`getNativeOutputSampleRate`.** Only when neither root nor a permitted Shizuku exists, keep today's behavior (the stream-level value, `output.mixerSampleRate`).

Rules that stay as they are: an unreadable value leaves the 「系統」 step out (never a guessed number), and the path line never claims bit-perfect. The value of the first two sources is the rate of the AudioFlinger thread the track runs on; the third is the stream-level value of finding 1.

### How the sources are checked and requested

These details follow the existing convention for permissions (ADR 0039): nothing is requested at startup, an explanation panel comes first, and only the panel's 「前往授權」 starts a request.

- **Root check.** Run `su -c id` off the main thread with a timeout (a few seconds); root is available when it exits with 0 and prints `uid=0`. The first `su` call of an app makes the root manager (KernelSU, Magisk) show its own grant dialog; the app cannot force it, so a refusal, a timeout or a missing `su` all count as "no root". Cache the result and check again when the user asks (the explanation panel) or after a failed lookup.
- **Shizuku check and request** (what RedMagicX, an app of the same owner, already does with `dev.rikka.shizuku:api` and `:provider` 13.1.5; the dumps were not run through it):
  1. Dependencies and manifest: the two Shizuku artifacts; `<uses-permission android:name="moe.shizuku.manager.permission.API_V23"/>`; a `rikka.shizuku.ShizukuProvider` with `android:authorities="${applicationId}.shizuku"`, `enabled="true"`, `exported="true"`, `multiprocess="false"` and `android:permission="moe.shizuku.manager.permission.API_V23"`. To open the Shizuku app from the panel a `<queries>` entry for its package (`moe.shizuku.privileged.api`) is needed on Android 11+.
  2. `Shizuku.pingBinder()` false (wrapped in try/catch): Shizuku is not running or not installed. The panel says so and, when installed, offers to open the Shizuku app, where the user starts the service (wireless debugging, adb or root). Nothing can be requested yet.
  3. `Shizuku.isPreV11()` true: the running Shizuku is too old; treat it as unavailable.
  4. `Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED`: use it.
  5. Otherwise, if `Shizuku.shouldShowRequestPermissionRationale()` is true the user chose "deny and don't ask again", and a request would be ignored: tell the user to allow the app in the Shizuku app's list of authorized apps.
  6. Otherwise register a `Shizuku.OnRequestPermissionResultListener`, call `Shizuku.requestPermission(requestCode)` (Shizuku shows its own dialog) and continue from the listener's result; remove the listener when the service or page goes away. A denial is a normal "no Shizuku" result, not an error.
  7. Run the command with `Shizuku.newProcess(arrayOf("dumpsys", "media.audio_flinger"), null, null)`. In Shizuku API 13.1.5 this method is private and RedMagicX reaches it by reflection; the supported alternative is a Shizuku user service (`Shizuku.bindUserService`), heavier but not tied to a private method. Which one to use is an implementation choice; either way run it off the main thread with a timeout and read stdout.
- **Which track and when.** The app already wraps the AudioTrack provider (`RouteObservingOutputProvider`, ADR 0044) and sees every AudioTrack it creates; read `getMetrics().getInt(AudioTrack.MetricsConstants.PORT_ID)` there. Look the value up when a track is created and when the route changes, never on the playback thread, and cache it per track.
- **Parsing.** Find the row of the active track table whose `Port Id` column (the sixth number after the row's `Id`, `Active`, `Client(pid/uid)`, `Session`) equals the portId; take the nearest preceding `Output thread … name AudioOut_xx` header and its `Sample rate:` line. Ignore the "Local log" rows that repeat removed tracks. If the row or the rate is missing, or the text does not parse, fall back to the next source. The dump is not an API: the parser must tolerate unknown fields and other ROMs. `dumpsys media.metrics` is the alternative (the chain in finding 3); it was not chosen as the first implementation because its records live in a ring buffer while the flinger dump shows the live track.

## Consequences

- Rooted or Shizuku-enabled phones show the real mixer rate of the track; other phones behave exactly as before, which is correct on a device with a single mixer rate.
- Adds a Shizuku dependency, a manifest provider, a root probe and an explanation panel with the states above (no root and no Shizuku; Shizuku installed but not running; permission denied for good; granted). The panel texts follow ADR 0039.
- Unit tests belong with the parser (captured dumps of both formats, missing row, other ROM text) and with the source selection (root, Shizuku granted, Shizuku not running, none); the on-device check needs a phone with several mixer outputs and one with a single output, once with root and once with Shizuku only.
- The path line still must not claim bit-perfect.

## Not done

Nothing in the player was changed. The values above were produced by test programs (`app_process`) and by reading the dumps from a PC over adb as the shell user and as root; the app itself has not read a dump, and Shizuku has not been used to run them. Not verified: the dump formats on other ROMs or Android versions, the metrics record retention in long sessions, `Shizuku.newProcess` and the user-service path for `dumpsys` on a device, and the Bluetooth codec step together with a per-track lookup.
