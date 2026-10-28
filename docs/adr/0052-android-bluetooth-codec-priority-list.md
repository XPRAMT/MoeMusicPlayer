# Android: a Bluetooth codec priority list

Status: Accepted and implemented (design approved by the owner; the Kotlin rules and the controller are unit-tested; nothing has run on a device yet)

## Context

ADR 0044 shows the codec an A2DP headset streams with, ADR 0051 moves its sample rate to the track's. The owner also wants to choose the codec: a list of the codecs this phone can encode, in an order he sets, so that a connected headset uses the first listed codec it supports. Changing the codec is `BluetoothA2dp.setCodecConfigPreference`, the same privileged call as ADR 0051 (`BLUETOOTH_PRIVILEGED`), so it goes through the same helper over root or Shizuku.

## Decision

### What the list shows

The codecs **this phone** can encode, not the connected headset's set: `BluetoothCodecStatus.getCodecsLocalCapabilities()` (public since Android 13), deduplicated by codec key, with a readable name (`BluetoothCodecRules.codecName`; Android 15+ vendor codecs by `BluetoothCodecType.getCodecName()`). A codec key is `t<source codec type>` (SBC 0, AAC 1, aptX 2, aptX HD 3, LDAC 4, LC3 5, Opus 6) or `x<extended codec id>` (`BluetoothCodecType.getCodecId()`, Android 15+); the key is what settings.json stores and what the helper receives. The phone's list is only reachable through a connected headset's codec status, so the last one seen is kept (`BluetoothCodecWatch.knownLocal`, persisted in SharedPreferences `moe_bt_codec`); with no copy yet the page shows the plain line 「連接藍牙耳機後可選擇編碼」 instead of the list.

There is **one order for every headset** (`androidBluetoothCodecOrder`, a list of keys in settings.json). The shown list is the phone's codecs in the stored order, then the codecs the order does not know yet in the stack's own priority order (larger `codecPriority` first, ties in the reported order); a stored key the phone does not list is skipped but stays in the stored order (`BluetoothCodecChoiceRules.shown`/`reorder`, mirrored by `buildCodecRows`/`moveCodecInOrder` in `android-output-devices.js`). First use (nothing stored) is the stack's order. With a headset connected, a row says 「使用中」 when it is the codec in use and 「此耳機不支援」 when the headset's selectable set (`codecsSelectableCapabilities`) lacks it (a label, rows are not greyed).

### Which codec a headset gets

The first row of the shown list that the headset's selectable set contains (`BluetoothCodecChoiceRules.target`). The codec switch asks for that codec at `CODEC_PRIORITY_HIGHEST`, with the sample rate nearest to the current one (the same pick as ADR 0051), the current bit depth and channel mode when the codec allows them (else the best bit depth and stereo), and codec-specific values 0 (they mean something else in every codec; LDAC's bitrate mode therefore returns to the stack default). The sample-rate follow of ADR 0051 continues per track as before.

### The master switch (`androidBluetoothCodecCustom`, default off)

- **Off**: nothing is applied, on connect or on reorder. The list stays visible and editable (not greyed) and the order is still stored; 「使用中」 still shows the codec the headset really uses. Turning the switch off puts back the codec that was in use before our first switch, only while the headset still runs the codec we set (best effort, same pattern as the rate follow; the original and the applied key are saved per address in SharedPreferences `moe_bt_codec`). A restore with no playback service runs in a short-lived worker.
- **On**: the top supported codec is applied at once to every attached headset; afterwards it is applied when a headset connects (an address appearing among the attached A2DP devices, including the service starting with one attached) and when the user reorders. A connection with no stored order changes nothing (first use keeps the headset's own codec); turning the switch on or reordering applies the order. A codec the user changes elsewhere is not forced back: the order is applied on those three events only.
- Without root or an authorized Shizuku the page shows only the short permission notice with 「設定」 in place of the switch and the list (ADR 0044 permission rule); with a source but no headset connected the switch stays and the list or the plain line is shown as above.

### Helper and controller

Two commands are added to `BluetoothRateHelper` (ADR 0051): `codecs <address> 0` prints `RESULT CODECS <active key|-> <local offers|-> <selectable offers|->` (offers are `key:priority:rateMask:bitsMask:channelMask:name`, comma-joined, the name ASCII with `_` for spaces), and `codec <address> <key>` switches (`RESULT CODEC <from> <to> <changed>`); the key is validated as `^[tx][0-9]{1,18}$` before a command line is built. `BluetoothCodecChoiceController` (thread `moe-bt-rate` style own scheduler, never the main or playback thread) coalesces requests per address, starts two privileged calls at least 1 s apart, waits up to 3 s for the stack's codec broadcast after a switch, and has its own circuit breaker (3 failed calls in a row, or a sink or player error within 8 s of a switch, stop it for the process). A failure shows a short message (「無法切換藍牙編碼」) in the list for about 12 s and leaves the codec as it was. When the settings page is open and a connected headset has no known codecs (the broadcast is not sticky), the controller reads them through the helper (`Trigger.READ`, at most two tries per connection), which also yields the phone's list.

### Settings and wiring

`androidBluetoothCodecCustom` (bool, default off) and `androidBluetoothCodecOrder` (list of keys, default empty, lenient load: invalid or duplicate keys dropped) in settings.json (Rust `AppSettings`, front-end `DEFAULT_ANDROID_BLUETOOTH_CODEC_CUSTOM`, locked by `settings::tests` and `tests/android-output-devices.test.mjs`); commands `settings_get/set_android_bluetooth_codec_custom` and `settings_get/set_android_bluetooth_codec_order`. Both setters store `{"enabled","order"}` for the service (`android_output::set_bluetooth_codec_prefs`, read over JNI as `nativeBluetoothCodecPrefs`) and push the plugin command `setBluetoothCodecPrefs`. The prefs hold no secrets and live in the same file as the other settings, so a whole-file export carries them; the phone's cached codec list and the saved originals are SharedPreferences and are not exported. The status the page polls gains `bluetoothCodecChoices` (`local` and, per headset address, `active`, `supported`, `error`). The list reuses the 曲目欄位 rows and long-press drag (`longPressDragGuard`, `.track-column-setting-row`). There is no keyboard reordering (the owner does not want it).

## Consequences

- A codec switch makes the stack reconfigure the link (a short mute and possibly a new AudioTrack route); the breaker treats an error right after it as the cause.
- The codec we set stays after the app ends; only turning the switch off puts the original back.
- A switch to a codec at the same instant as the sample-rate follow's switch is serialized only by each controller's own 1 s spacing; the two features do not coordinate, and the stack may take the later request.

## Tests

Kotlin `BluetoothCodecChoiceTest` (keys, stack order, shown/reorder, target, switch values, helper wire format and command quoting, the plan and skip reasons, saved-original restore, prefs parsing, the controller: save once, read-then-switch, failure message, breaker, switch off inert, switch on applies, connect needs a stored order, reorder, spacing, restore, read tries), Rust `settings::tests` and `android_output` tests, `tests/android-output-devices.test.mjs` (defaults, visibility conditions, rows, hints, order merge, headset pick).

## Not done / not verified

Not verified on any device: that `codecsLocalCapabilities` carries the codecs the phone can encode on the NX809J (including LHDC and other vendor codecs on Android 15+, and what `getCodecId()` returns for them); that `setCodecConfigPreference` with the built config (extended type copied from the headset's capability, priority highest, codec-specific 0) switches the WF-1000XM6 between LDAC and AAC/SBC and what the mute sounds like; that capability priorities give a sensible stack order; the restore after the switch is turned off; the broadcast timing at connect (the order is applied once the stack announced or the helper read the codecs); Shizuku; Android 13 and 14 (no extended codecs).
