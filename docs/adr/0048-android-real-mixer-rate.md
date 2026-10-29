# Android: the real mixer rate of a track (the 「系統」 step of the path line)

Status: Accepted and implemented (the lookup order was decided by the owner; the research was verified on one phone with a root shell and an adb shell; the code below is unit-tested, nothing is built into an APK or run on a device yet)

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

## Implementation (Kotlin `MixerRate*.kt` in the media-index plugin; Rust `android_output.rs`, `lib.rs`; page `mixer-rate-source.js`)

- **Where the value goes.** `output.mixerSampleRate` of the status as before, plus `output.mixerRateSource` (`root`, `shizuku`, `platform`). While a privileged source is in use and the lookup has not finished, the step is left out (never the stream-level value of another thread); when the lookup fails (row missing, command failed, unparsable) the platform value is shown with source `platform` (the "next source" of the decision).
- **Track and route.** `MixerRateLookup.request` is called from `AudioOutputRouter.observeTrack` (Media3 created a track) and from the routing listener (route change). On the lookup's own thread (`moe-mixer-rate`) it reads `AudioTrack.getMetrics()` key `android.media.audiotrack.portId` (API 28+; the key string is used so the call links on older systems) at run time, so a track restored under a new port id is found again. Results (also failures) are cached per port id and routed device id, so one track costs one run.
- **Order inside the privileged path.** One constant, `MixerRateConfig.DUMP_ORDER` (`FLINGER_FIRST`: `dumpsys media.audio_flinger`, then `media.metrics` when the flinger text has no row for the track; `FLINGER_SECOND` flips them). A tripped breaker stops the run between the two. The parsers are `MixerRateDumpParsers.fromFlingerDump` (header-driven: the track table's own header line names the `Port Id` and `Active` columns, two-word names folded; "Local log" lines, "Terminated" tables and input threads are skipped; an active row wins, rows that disagree give null) and `fromMetricsDump` (`audio.track.<id>` create event `threadId` -> `audio.thread.<n>` latest `readParameters` `sampleRate`; `effective.sampleRate` is not the thread rate). The metrics format in the fixtures follows the description in finding 3 and is not captured from a device.
- **Root.** `RootAccess`: `su -c id` (20 s when the user pressed 「檢查 root」, 3 s silently), cached per process. Nothing probes `su` until the user pressed the button once (`MixerRatePrefs.rootOptIn`, kept in `settings.json` as `androidRootOptIn` through Rust `android_root.rs` over JNI; an older SharedPreferences `moe_mixer_rate` value is moved in once when the setting has none and then removed), because the first `su` call shows the root manager's own dialog. After that, a silent probe runs on the lookup thread. A failed run forgets the cached answer.
- **Shizuku: `Shizuku.newProcess` by reflection**, not a user service. Reasons: the supported alternative (`bindUserService`) needs an AIDL interface, a second process and a bind lifecycle for one short `dumpsys`; the reflective call is what RedMagicX already ships, runs as the shell user, and fails safely (an exception is "lookup failed"). Risk: a future Shizuku API may rename the private method; the consumer rules keep `rikka.shizuku.Shizuku`. Dependencies `dev.rikka.shizuku:api` and `:provider` 13.1.5; manifest: `moe.shizuku.manager.permission.API_V23`, a `<queries>` entry for `moe.shizuku.privileged.api`, and `ShizukuProvider` (`${applicationId}.shizuku`). **Deviation from the text above:** the provider's `android:permission` is `android.permission.INTERACT_ACROSS_USERS_FULL`, as in the Shizuku-API documentation (the binder is delivered by the shell/root process, which holds that permission and not the app permission); verify on a device that the binder arrives.
- **Panel.** Kotlin `MixerRateRules.panelState` decides the state (`tripped`, `rootActive`, `shizukuActive`, `shizukuNeedsPermission`, `shizukuDenied`, `shizukuNotRunning`, `none`, `disabled`); `mixer-rate-source.js` gives each its texts and buttons. A line 「系統取樣率：…」 with 「前往授權」 sits under the 「播放裝置」 header. Commands `android_mixer_rate_source_status` / `android_mixer_rate_source_action` (`checkRoot`, `requestShizuku`, `openShizuku`, `recheck`). The status read never asks for anything; Shizuku's own dialog and the root manager's dialog start only from a button.

## Safety (a real incident)

On the tablet (Android 15, Qualcomm, a USB output thread "Blocked in write: yes") a `dumpsys media.audio_flinger` run during testing coincided with `audioserver` aborting (SIGABRT, "AudioFlinger server died!") about 2 seconds later and the audio routing being reset. The cause is unproven (a coincidence, or the dump contending for a lock). So the lookup is built to be rare and stoppable:

- **Rate limit.** `DumpGate`: one run at a time; the first run of a lookup starts at once (creating, pinning and re-routing a track within a moment are coalesced: a request that arrives while a run is pending or running is merged into it, and a running one gets exactly one follow-up lookup of the newest track); two lookups start at least `MIN_GAP_MS` (1 s) apart; no more than `MAX_RUNS_PER_WINDOW` (30) runs start in any `RUN_WINDOW_MS` (60 s), retries included (the gate delays a run until the oldest leaves the window). Nothing polls the dump: the status read only returns the cached value. Each dump has a timeout (6 s) and a size cap (4 MiB) and runs on `moe-mixer-rate`, never on the main or playback thread.
- **Retries instead of a fixed delay.** When a run does not find the track (no row yet, no port id yet, a dump that failed or timed out), the lookup runs again after `RETRY_BACKOFF_MS` (150, 300, 600, 1200 ms) and then gives up (cached as unknown, the platform value is shown). A new request while only a retry is waiting restarts the lookup for the newest track. Each run is two cheap `dumpsys` calls at most (flinger, then metrics).
- **Not when the track is not in AudioFlinger.** `MixerRateRules.mayDump` is false while the USB exclusive output holds the device (`UsbExclusiveEngine.held`) or native DSD is active, and without a port id; only `AudioTrackAudioOutput` tracks are observed at all.
- **Kill switches.** `MixerRateConfig.PRIVILEGED_LOOKUP_ENABLED` (code) and the hidden `settings.json` boolean `androidRealMixerRate` (default true; nothing runs unless root or Shizuku is granted; read over JNI at startup, a hand edit applies after a restart). Either one off: the platform value only, and the panel says so.
- **Circuit breaker.** `DumpBreaker`, process-wide: a symptom within `BREAKER_WINDOW_MS` (8 s) of a dump, or during one, stops the privileged lookup for the rest of the process and logs the reason (tag `MoeMediaIndex`). Symptoms: a Media3 audio sink error, a player error, the playing track's routed device going from a device to none; and a dump whose own short output or error stream says the audio server is gone ("server died", DeadObject, "Failure from system", "Can't find service"). A normal unplug or pause does not count. The panel shows `tripped`.
- **Which dump is lighter.** Not measured. `media.metrics` is another service (mediametrics), so it cannot hold AudioFlinger's lock, but its records are a ring buffer and its format is less certain; the flinger dump shows the live track. The order stays flinger-first (the owner's decision); flip `DUMP_ORDER` to `FLINGER_SECOND` to ask `media.metrics` first, which leaves the flinger dump only as the fallback when the metrics record is missing.

## Tests

`MixerRateDumpParsersTest`, `MixerRateRulesTest` (source selection with root, Shizuku granted/not running/denied/none, kill switches, breaker, gate spacing and coalescing, panel states), `tests/mixer-rate-source.test.mjs`, `android_output::tests` and `settings::tests` (the hidden switch).

## Not done

Not verified on a device: the dump formats on other ROMs or Android versions (the flinger fixtures follow finding 3, the metrics fixtures only its description), that `getMetrics()` returns the port id as soon as the track is created, the metrics record retention in long sessions, `Shizuku.newProcess` and the binder delivery with the provider permission above, whether the dump is related to the audioserver abort at all, and the Bluetooth codec step together with a per-track lookup.

## Findings from a real capture (NX809J over A2DP)

- **Flinger dump parser bug (fixed).** The real dump has unindented lines inside a thread block ("Bluetooth latency modes are enabled", "Supported latency modes: ..."). The parser took any unindented line for the end of the thread section, so the track table after it was never read and the lookup logged `unknown`. An unindented non-header line now only ends the current track table; a section ends at the next thread header. The same capture also showed that a normal track row has an empty `Type` column (the row starts at `Id`) and that `Client(pid/uid)` prints as two tokens ("19779/  10710"); the row is now aligned by whether the header starts with `Type`, and `pid/ uid` is joined. The tests use trimmed excerpts of that capture (`src/test/resources/flinger-a2dp-excerpt.txt`, `metrics-a2dp-excerpt.txt`).
- **One mixer thread per rate.** This ROM opens one A2DP mixer thread per sample rate (88.2, 48, 44.1, 96 kHz in the capture), so the thread rate of a track legitimately differs from track to track.
- **`dumpsys media.metrics` does give the thread rate of a live track.** Record `audio.track.<portId>` has the `create` event with `threadId=N` (here track 440 -> thread 477), and record `audio.thread.N` has an `event#=readParameters` event with `sampleRate` (477 -> 48000). The `audiothread` records with `sampleRate` are only the summaries of older (destroyed) threads and are not needed; the `audio.thread.N` records of a live thread otherwise carry `createAudioPatch` and `throttleMs` events. The values end in `)}` in the text, which the metrics parser now ignores. The ring buffer keeps the live track's records, so this source works; `DUMP_ORDER` is unchanged (flinger first).

## Scheduling (why the value used to arrive seconds late)

The first version waited a fixed `SETTLE_MS` (1 s) before the first run and kept runs `MIN_INTERVAL_MS` (5 s) apart. On the NX809J (root) both dumps take about 20 ms each, yet `AudioTrack initialized` was logged about 3.4 s before `mixer rate of port ... via root`, because of the settle wait plus the follow-up interval. The delays protected nothing the dump itself needs, so they were replaced by the immediate first run, the short retry chain and the run cap above (the breaker, the kill switches, the USB/DSD exclusion and the single in-flight run are unchanged). Expected: the value is read about 20 to 50 ms after the AudioTrack exists when the track is already in the dump, otherwise within the retry delays (at most about 2.3 s in total before giving up); the settings page reads the status every 500 ms, so it shows the value at most 500 ms later. Not verified on a device: the real time to first value, whether the track is in the flinger dump right after creation (retries cover it otherwise), and whether the denser dumps change the audioserver incident risk (the breaker still stops everything on a symptom).

## Measured comparison of the two dumps (real device)

On the NX809J over root: 4 minutes, one poll every 0.3 s, 4 track changes at 96000, 44100, 96000 and 48000 Hz. Both dumps were measured on the device with the grep filter.

| | `media.audio_flinger` | `media.metrics` |
| --- | --- | --- |
| Time per dump | about 86 ms on average, 708 ms at most | about 264 ms on average, 453 ms at most |
| Output size | about 190 KB | about 770 KB |
| Steps to the thread rate | one (the flinger row gives the rate of the thread) | two (`audio.track.<port>` gives the thread id, then `audio.thread.<id>` `readParameters` gives `sampleRate`) |

- Both showed the new track in the same poll after the track started; the 0.3 s poll interval was too coarse to tell them apart.
- App lookups through the flinger dump finished about 60 ms after `AudioTrack initialized`.
- No audioserver crash was seen in hundreds of dumps.

Decision: keep `FLINGER_FIRST` (faster, smaller, one step); `media.metrics` stays the fallback. Revisit only if an audioserver crash recurs; the circuit breaker stays either way.

## One panel for the three features that need root or Shizuku

The explanation panel is shared by the three features that need a privileged source: the real mixer rate (this ADR), the Bluetooth sample-rate follow (ADR 0051) and the Bluetooth codec priority list (ADR 0052). Its title is 「需要 root 或 Shizuku」 and its text names all three and what root or Shizuku is used for. The entry buttons (the 「系統取樣率」 line under 「裝置」 and the one notice of the 「藍牙」 section) are labelled 「前往授權」 and open this same panel; inside it the button that asks for the permission is labelled 「授權」 (root check, the Shizuku permission request, or opening the Shizuku app when it is denied or not running, as before). No button is labelled 「設定」.

## Shizuku runner

A process started through Shizuku (`Shizuku.newProcess`) is a remote process whose `exitValue()` throws "process hasn't exited" while it runs (as `IllegalArgumentException`, not only `IllegalThreadStateException`). The helper of ADR 0051 needs about a second to start, so reading the exit value after the output streams ended failed every Shizuku helper call (the fast `dumpsys` calls mostly escaped). `BoundedProcess.read` therefore never calls `exitValue()`: the exit code comes from `waitFor()` on its own thread, the two output streams are drained on their own threads, and the watchdog is the caller's time limit, after which the process is destroyed and given a short grace period. A runner failure of this kind is also not a device failure: the breakers of the rate follow, the codec list and the LDAC mode read ignore it (`PrivilegedRunnerErrors`, `BluetoothRateBreaker.switchIgnored`); the dump breaker here only reacts to the audio server's death text. Tests: `BoundedProcessTest` (a fake process whose `exitValue()` throws until it ends).
