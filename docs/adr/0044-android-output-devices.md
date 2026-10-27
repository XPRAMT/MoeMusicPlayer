# Android output device choice

Status: Accepted (built and unit-tested; not yet verified on a device)

Decision date: 2026-10-27, Asia/Taipei (UTC+08:00)

## Context

Windows has a playback device list in Settings → 播放 (ADR 0020). On Android the player always followed the system's routing: plugging in a USB DAC or connecting a headset moved the sound, and there was no way to keep playing on the phone speaker while a headset is connected, to prefer a USB DAC over Bluetooth, or to see what the output really is. The user asked for the Android equivalent of the Windows list, with the same look, limited to what Android can do.

Android facts that shape the design:

- `AudioManager.getDevices(GET_DEVICES_OUTPUTS)` lists the attached outputs without any permission; `AudioDeviceCallback` reports every change (and is called at once with every attached device on registration).
- `AudioDeviceInfo.getId()` changes every time a device is plugged in, so it cannot identify a device across plugs or restarts. Type, product name and (for Bluetooth) address are stable.
- Media3 `ExoPlayer.setPreferredAudioDevice(AudioDeviceInfo?)` pins the AudioTrack to a device (`AudioTrack.setPreferredDevice`); null returns to the system's routing. A preferred device that goes away makes the platform route to its default.
- Capabilities per device: `getSampleRates()`, `getChannelCounts()`, `getEncodings()`; an empty array means "any" (the platform converts).
- What really plays: Media3's `AnalyticsListener.onAudioTrackInitialized` gives the `AudioSink.AudioTrackConfig` (encoding, sample rate, channel mask, offload); the routed device is `AudioTrack.getRoutedDevice()`, which Media3 does not expose, so the default `AudioTrackAudioOutputProvider` is wrapped (`RouteObservingOutputProvider`) to see each AudioTrack it creates. On Android 13+ `AudioManager.getAudioDevicesForAttributes(media)` says where the system sends media when no track is open.
- Everything still goes through the Android mixer (AudioFlinger), which may resample: nothing here is bit-perfect, and the page says so.

## Decision

- **One choice, single selection**: 「系統預設」 or one device, stored in `settings.json` as `androidOutputDevice: { kind, productName, address? } | null` (Rust `settings::AndroidOutputDevice`, serde default null; an unreadable value loads as null). Both defaults (Rust and the page) are the system default.
- **Identity** (Kotlin `AudioOutputRules`, the only place the rule lives): `kind` groups `AudioDeviceInfo` types — `speaker`, `wired` (headset and headphones: one jack reports either), `usb` (device, accessory, headset: one DAC can report either), `bluetooth` (A2DP), `bleAudio` (LE Audio headset/speaker), `hearingAid`, `hdmi` (HDMI, ARC, eARC), `line` (analog, digital, aux), `dock`. Earpiece, telephony, SCO, the alarm "safe" speaker, buses, remote submix, FM, IP and the BLE broadcast are not listed. Identity = kind + trimmed product name, plus the address for the Bluetooth kinds; a hidden (empty) address on either side does not count against a match. A device the platform reports twice under one identity is one row whose capability lists are the union. The page stores the `identity` object Kotlin sends with each device, so the page has no copy of the rule; the status command passes the saved choice to Kotlin, which answers with the matching `chosenId`.
- **Applied by the service, not the page**: the Rust process keeps the choice (`android_output::set_preference`, set at startup from the settings and on every change) and `PlaybackService` reads it over JNI (`nativeOutputDevicePreference`) when it is created and before every load, so the choice applies with no Activity or WebView. A change on the settings page is also pushed to a running service (`setAudioOutputPreference`). `AudioOutputRouter` registers an `AudioDeviceCallback` and on every device change pins the player to the chosen device if it is attached, else to the system default; when the device is plugged back in it is pinned again automatically. It calls `setPreferredAudioDevice` only when the target changes.
- **What the page shows** (`AndroidOutputDevicesSettings.svelte`, `android-output-devices.js`; the row classes are shared with the Windows list in `app.css`): 「系統預設」 first, then the listed devices, each with its capabilities in the Windows row style (`44.1／48／96 kHz．2 聲道．16／24 位元／浮點`, 不限 for "any"), and on the chosen row (or 「系統預設」) the actual output from the open AudioTrack and its routed device (`實際輸出：48 kHz．24 位元．2 聲道 → KA3（USB）`). A chosen device that is not attached stays as a row with 「已中斷連線，目前改由 {0} 播放」 naming the device actually used (the routed device, else the system's media route on Android 13+; without either it says 「已中斷連線」). The list is re-read every 2 s while it is shown (the service itself reacts to the callback at once). The settings page and the quick-settings drawer share one module state (src/lib/AGENTS.md rule).
- **Not on Android**: shared/exclusive, presets, volume link, per-device volume and delay, channel map, several devices at once, DSEE HX.
- **Permissions**: none added. `getDevices` needs none; product names come from the audio framework, not from the Bluetooth API. If a Bluetooth name is empty, the row shows the kind (藍牙) and the address still tells two headsets apart; BLUETOOTH_CONNECT is not requested.
- No code was taken from other players; EchoTide-Music (AGPL) was not used as a source.

## Consequences

- A chosen USB DAC keeps the player on it while Bluetooth connects and disconnects; unplugging it falls back to the system's choice and plugging it back re-pins, with no user action.
- The AudioTrack provider is the default one built by `AudioTrackAudioOutputProvider.Builder(context)` (what `DefaultAudioSink.Builder` builds when none is given), wrapped only to observe tracks. `DefaultAudioSink.getAudioCapabilities()` returns null with a provider that is not the default class; Media3 uses it only for IAMF output, which the player does not play.
- Unit tests: Kotlin `AudioOutputRulesTest` (filtering, dedup, identity, re-plug matching, Bluetooth address rule, preference parsing), JS `tests/android-output-devices.test.mjs` (labels, capabilities, actual output, rows incl. the disconnected row), Rust settings and `android_output` tests.
- Future work, not done: direct/bit-perfect output (`AudioManager.setPreferredMixerAttributes` on Android 14+ for USB, `AudioTrack` direct playback), USB exclusive access, per-device volume.
- Needs device verification: pinning and re-pinning with a real USB DAC and Bluetooth headset, the routed-device and AudioTrack format display, Bluetooth names and addresses without BLUETOOTH_CONNECT (Android 12+), the system route on Android 13+ vs older devices, and that wrapping the AudioTrack provider changes nothing else in playback.
