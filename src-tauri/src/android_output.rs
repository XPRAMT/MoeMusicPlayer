//! The Android output device choice as the playback service reads it (ADR 0044).
//!
//! `settings.json` keeps the choice (`androidOutputDevice`); this module holds a copy for the Media3 service, which
//! reads it over JNI when it is created and before every load, so the choice applies with no Activity or WebView. A
//! change made on the settings page is also pushed to a running service through the plugin (`setAudioOutputPreference`).
//! The USB exclusive switch (`androidUsbExclusive`, ADR 0047) travels the same way (`setUsbExclusive`); the DSD native
//! passthrough switch (`dsd.nativePassthrough`, ADR 0049) is only read by the service before each load.

use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::RwLock;

use crate::settings::AndroidOutputDevice;

/// The choice as JSON (`null`: the system default); `None` until the settings have been read in this process.
static PREFERENCE: RwLock<Option<String>> = RwLock::new(None);

/// The JSON the service reads: the device, or `null` for the system default.
pub(crate) fn preference_json(device: Option<&AndroidOutputDevice>) -> String {
    serde_json::to_string(&device).unwrap_or_else(|_| "null".to_owned())
}

/// Remember the choice for the service (at startup and whenever the user changes it).
pub(crate) fn set_preference(device: Option<&AndroidOutputDevice>) {
    *PREFERENCE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(preference_json(device));
}

/// The remembered choice, or `None` when it is not known yet (the service then keeps what it has).
pub(crate) fn current_preference_json() -> Option<String> {
    PREFERENCE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The USB exclusive switch for the service: -1 until the settings were read, else 0 or 1.
static USB_EXCLUSIVE: AtomicI8 = AtomicI8::new(-1);

/// Remember the USB exclusive switch for the service (at startup and whenever the user changes it).
pub(crate) fn set_usb_exclusive(enabled: bool) {
    USB_EXCLUSIVE.store(i8::from(enabled), Ordering::Relaxed);
}

/// The switch as the service reads it: -1 not known yet (keep what it has), 0 off, 1 on.
pub(crate) fn usb_exclusive_state() -> i8 {
    USB_EXCLUSIVE.load(Ordering::Relaxed)
}

/// The DSD native passthrough switch for the service (`dsd.nativePassthrough`, ADR 0049): -1 until the settings were read.
static DSD_PASSTHROUGH: AtomicI8 = AtomicI8::new(-1);

/// Remember the DSD native passthrough switch for the service (at startup and whenever the user changes it). The
/// service reads it before every load, so a change applies from the next track on.
pub(crate) fn set_dsd_passthrough(enabled: bool) {
    DSD_PASSTHROUGH.store(i8::from(enabled), Ordering::Relaxed);
}

/// The switch as the service reads it: -1 not known yet (keep what it has), 0 off, 1 on.
pub(crate) fn dsd_passthrough_state() -> i8 {
    DSD_PASSTHROUGH.load(Ordering::Relaxed)
}

/// The hidden real-mixer-rate switch for the service (`androidRealMixerRate`, ADR 0048): -1 until the settings were read.
static REAL_MIXER_RATE: AtomicI8 = AtomicI8::new(-1);

/// Remember the switch for the service (at startup; the file is edited by hand, so a change applies after a restart).
pub(crate) fn set_real_mixer_rate(enabled: bool) {
    REAL_MIXER_RATE.store(i8::from(enabled), Ordering::Relaxed);
}

/// The switch as the service reads it: -1 not known yet, 0 off, 1 on.
pub(crate) fn real_mixer_rate_state() -> i8 {
    REAL_MIXER_RATE.load(Ordering::Relaxed)
}

/// The Bluetooth sample-rate follow switch for the service (`androidBluetoothRateFollow`, ADR 0051): -1 until the
/// settings were read.
static BLUETOOTH_RATE_FOLLOW: AtomicI8 = AtomicI8::new(-1);

/// Remember the switch for the service (at startup and whenever the user changes it). The service reads it before
/// every load.
pub(crate) fn set_bluetooth_rate_follow(enabled: bool) {
    BLUETOOTH_RATE_FOLLOW.store(i8::from(enabled), Ordering::Relaxed);
}

/// The switch as the service reads it: -1 not known yet (treated as off), 0 off, 1 on.
pub(crate) fn bluetooth_rate_follow_state() -> i8 {
    BLUETOOTH_RATE_FOLLOW.load(Ordering::Relaxed)
}

/// The Bluetooth codec priority list for the service (ADR 0052): `{"enabled":bool,"order":[key]}` as JSON;
/// `None` until the settings were read.
static BLUETOOTH_CODEC_PREFS: RwLock<Option<String>> = RwLock::new(None);

/// The JSON the service reads.
pub(crate) fn bluetooth_codec_prefs_json(enabled: bool, order: &[String]) -> String {
    serde_json::json!({ "enabled": enabled, "order": order }).to_string()
}

/// Remember the switch and the order for the service (at startup and whenever the user changes either).
pub(crate) fn set_bluetooth_codec_prefs(enabled: bool, order: &[String]) {
    *BLUETOOTH_CODEC_PREFS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(bluetooth_codec_prefs_json(enabled, order));
}

/// The remembered prefs, or `None` when they are not known yet (the service then keeps the list inert).
pub(crate) fn bluetooth_codec_prefs() -> Option<String> {
    BLUETOOTH_CODEC_PREFS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[cfg(test)]
mod bluetooth_codec_prefs_tests {
    use super::*;

    #[test]
    fn the_codec_prefs_json_carries_the_switch_and_the_order() {
        let order = vec!["t4".to_owned(), "t1".to_owned()];
        let value: serde_json::Value =
            serde_json::from_str(&bluetooth_codec_prefs_json(true, &order)).unwrap();
        assert_eq!(value["enabled"], true);
        assert_eq!(value["order"][1], "t1");
    }
}

#[cfg(target_os = "android")]
mod jni_exports {
    use jni::{objects::JObject, sys::jstring, JNIEnv};

    /// `PlaybackService.nativeOutputDevicePreference()`: the chosen device as JSON (`"null"` for the system default),
    /// or a Java null while the settings have not been read yet.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeOutputDevicePreference(
        env: JNIEnv,
        _service: JObject,
    ) -> jstring {
        match super::current_preference_json().and_then(|json| env.new_string(json).ok()) {
            Some(value) => value.into_raw(),
            None => std::ptr::null_mut(),
        }
    }

    /// `PlaybackService.nativeUsbExclusiveEnabled()`: -1 while the settings have not been read, else 0 or 1.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeUsbExclusiveEnabled(
        _env: JNIEnv,
        _service: JObject,
    ) -> jni::sys::jint {
        jni::sys::jint::from(super::usb_exclusive_state())
    }

    /// `PlaybackService.nativeRealMixerRateEnabled()`: -1 while the settings have not been read, else 0 or 1.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeRealMixerRateEnabled(
        _env: JNIEnv,
        _service: JObject,
    ) -> jni::sys::jint {
        jni::sys::jint::from(super::real_mixer_rate_state())
    }

    /// `PlaybackService.nativeBluetoothRateFollowEnabled()`: -1 while the settings have not been read, else 0 or 1.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeBluetoothRateFollowEnabled(
        _env: JNIEnv,
        _service: JObject,
    ) -> jni::sys::jint {
        jni::sys::jint::from(super::bluetooth_rate_follow_state())
    }

    /// `PlaybackService.nativeBluetoothCodecPrefs()`: the codec priority list prefs as JSON, or a Java null while the
    /// settings have not been read yet.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeBluetoothCodecPrefs(
        env: JNIEnv,
        _service: JObject,
    ) -> jstring {
        match super::bluetooth_codec_prefs().and_then(|json| env.new_string(json).ok()) {
            Some(value) => value.into_raw(),
            None => std::ptr::null_mut(),
        }
    }

    /// `PlaybackService.nativeDsdPassthroughEnabled()`: -1 while the settings have not been read, else 0 or 1.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeDsdPassthroughEnabled(
        _env: JNIEnv,
        _service: JObject,
    ) -> jni::sys::jint {
        jni::sys::jint::from(super::dsd_passthrough_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_reads_null_for_the_system_default_and_the_identity_otherwise() {
        assert_eq!(preference_json(None), "null");
        let device = AndroidOutputDevice {
            kind: "bluetooth".into(),
            product_name: "WH-1000XM5".into(),
            address: Some("AA:BB:CC:DD:EE:FF".into()),
        };
        let value: serde_json::Value =
            serde_json::from_str(&preference_json(Some(&device))).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "bluetooth",
                "productName": "WH-1000XM5",
                "address": "AA:BB:CC:DD:EE:FF"
            })
        );
        set_preference(Some(&device));
        assert_eq!(
            current_preference_json(),
            Some(preference_json(Some(&device)))
        );
        set_preference(None);
        assert_eq!(current_preference_json().as_deref(), Some("null"));
    }

    #[test]
    fn the_usb_exclusive_switch_is_unknown_until_read() {
        assert_eq!(usb_exclusive_state(), -1);
        set_usb_exclusive(true);
        assert_eq!(usb_exclusive_state(), 1);
        set_usb_exclusive(false);
        assert_eq!(usb_exclusive_state(), 0);
    }

    #[test]
    fn the_real_mixer_rate_switch_is_unknown_until_read() {
        assert_eq!(real_mixer_rate_state(), -1);
        set_real_mixer_rate(false);
        assert_eq!(real_mixer_rate_state(), 0);
        set_real_mixer_rate(true);
        assert_eq!(real_mixer_rate_state(), 1);
    }

    #[test]
    fn the_dsd_passthrough_switch_is_unknown_until_read() {
        assert_eq!(dsd_passthrough_state(), -1);
        set_dsd_passthrough(true);
        assert_eq!(dsd_passthrough_state(), 1);
        set_dsd_passthrough(false);
        assert_eq!(dsd_passthrough_state(), 0);
    }
}
