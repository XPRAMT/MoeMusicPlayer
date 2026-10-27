//! The Android output device choice as the playback service reads it (ADR 0044).
//!
//! `settings.json` keeps the choice (`androidOutputDevice`); this module holds a copy for the Media3 service, which
//! reads it over JNI when it is created and before every load, so the choice applies with no Activity or WebView. A
//! change made on the settings page is also pushed to a running service through the plugin (`setAudioOutputPreference`).

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
}
