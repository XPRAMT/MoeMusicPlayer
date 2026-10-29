//! The Android floating lyrics overlay's lock state and positions (ADR 0032).
//!
//! `settings.json` (`floatingLyrics.locked`, `position` = portrait, `landscapePosition`) is the only store and Rust the
//! only writer. This module holds a copy for the overlay (`FloatingLyricsStore.kt`), which reads it over JNI when it
//! needs it (null until the settings were read in this process: the overlay then keeps what it has and asks again
//! later) and reports its own changes over JNI: the copy changes at once and a worker thread writes the file and
//! emits `floating-lyrics-settings`, so the Android main thread never waits for the disk. The page's command protocol
//! (`floatingLyricsUpdate { setLocked, resetPosition, savePosition, … }`) is unchanged; only the storage moved here
//! from the overlay's SharedPreferences, whose values are carried over once (`nativeImportLegacy`).

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::settings::{FloatingLyricsPosition, FloatingLyricsSettings};

/// What the overlay owns of the floating settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayState {
    pub(crate) locked: bool,
    /// The portrait position (`floatingLyrics.position`).
    pub(crate) position: Option<FloatingLyricsPosition>,
    pub(crate) landscape_position: Option<FloatingLyricsPosition>,
}

impl OverlayState {
    pub(crate) fn of(settings: &FloatingLyricsSettings) -> Self {
        Self {
            locked: settings.locked,
            position: settings.position,
            landscape_position: settings.landscape_position,
        }
    }

    /// Writes this state into the stored settings (the rest is left as it is).
    pub(crate) fn store_into(self, settings: &mut FloatingLyricsSettings) {
        settings.locked = self.locked;
        settings.position = self.position;
        settings.landscape_position = self.landscape_position;
    }
}

/// A change reported by the overlay: only the keys present change; `null` clears a position (重設位置).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayChange {
    #[serde(default)]
    locked: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    position: Option<Option<FloatingLyricsPosition>>,
    #[serde(default, deserialize_with = "present")]
    landscape_position: Option<Option<FloatingLyricsPosition>>,
}

/// A key that is present (even as `null`) becomes `Some`, so `null` can clear a position.
fn present<'de, D>(deserializer: D) -> Result<Option<Option<FloatingLyricsPosition>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<FloatingLyricsPosition>::deserialize(deserializer).map(Some)
}

/// A position as the settings accept it (an invalid one is dropped, as `FloatingLyricsSettings::normalized` would).
fn valid(position: Option<FloatingLyricsPosition>) -> Option<FloatingLyricsPosition> {
    position.filter(|position| position.is_valid())
}

/// The state after an overlay save: every key the change carries replaces the stored value.
pub(crate) fn merge(state: OverlayState, change: &OverlayChange) -> OverlayState {
    OverlayState {
        locked: change.locked.unwrap_or(state.locked),
        position: change.position.map_or(state.position, valid),
        landscape_position: change
            .landscape_position
            .map_or(state.landscape_position, valid),
    }
}

/// The state after the one-time carry-over of the overlay's old SharedPreferences values: a position already in
/// `settings.json` is kept (only an empty one takes the old value); the lock state comes from the old store when it
/// had one, since `settings.json`'s `locked` was never used on Android before.
pub(crate) fn merge_legacy(state: OverlayState, legacy: &OverlayChange) -> OverlayState {
    let fill = |current: Option<FloatingLyricsPosition>,
                old: Option<Option<FloatingLyricsPosition>>| {
        current.or_else(|| old.and_then(valid))
    };
    OverlayState {
        locked: legacy.locked.unwrap_or(state.locked),
        position: fill(state.position, legacy.position),
        landscape_position: fill(state.landscape_position, legacy.landscape_position),
    }
}

/// The copy the overlay reads; None until the settings were read in this process.
static STATE: RwLock<Option<OverlayState>> = RwLock::new(None);

/// Writes the state into `settings.json` (and tells the page); set once at startup.
type Sink = Arc<dyn Fn(OverlayState) -> Result<(), String> + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();
/// Wakes the writer thread; each wake writes the newest state (several quick saves become one write).
static WORKER: OnceLock<Mutex<Sender<()>>> = OnceLock::new();

/// Remember the stored values for the overlay (at startup, after the settings were read).
pub(crate) fn seed(settings: &FloatingLyricsSettings) {
    *STATE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(OverlayState::of(settings));
}

pub(crate) fn current() -> Option<OverlayState> {
    *STATE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The state as the overlay reads it: `{"locked":…,"position":{x,y}|null,"landscapePosition":…}`, or None while the
/// settings have not been read.
pub(crate) fn state_json() -> Option<String> {
    current().and_then(|state| serde_json::to_string(&state).ok())
}

/// Install the writer (at startup): `sink` stores a state in `settings.json`; it runs on a thread of its own.
pub(crate) fn install(sink: Sink) {
    if SINK.set(Arc::clone(&sink)).is_err() {
        return;
    }
    let (sender, receiver) = mpsc::channel::<()>();
    if WORKER.set(Mutex::new(sender)).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("floating-lyrics-save".into())
        .spawn(move || {
            while receiver.recv().is_ok() {
                while receiver.try_recv().is_ok() {}
                if let Some(state) = current() {
                    if let Err(error) = sink(state) {
                        eprintln!("floating lyrics: {error}");
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("floating lyrics: no writer thread: {error}");
    }
}

/// An overlay save (`FloatingLyricsStore.nativeSave`): applied to the copy at once, written by the worker. False when
/// the settings are not known yet or the change cannot be read (the overlay keeps it and sends it again later).
pub(crate) fn save(json: &str) -> bool {
    let Ok(change) = serde_json::from_str::<OverlayChange>(json) else {
        return false;
    };
    {
        let mut guard = STATE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = guard.as_mut() else {
            return false;
        };
        *state = merge(*state, &change);
    }
    match WORKER.get() {
        Some(worker) => worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(())
            .is_ok(),
        None => false,
    }
}

/// The one-time carry-over of the old SharedPreferences values (`FloatingLyricsStore.nativeImportLegacy`, called off
/// the main thread): merged with `merge_legacy` and written before it returns, so the overlay clears its old keys
/// only once the values are safely in `settings.json`.
pub(crate) fn import_legacy(json: &str) -> bool {
    let Ok(legacy) = serde_json::from_str::<OverlayChange>(json) else {
        return false;
    };
    let Some(sink) = SINK.get() else {
        return false;
    };
    let next = {
        let mut guard = STATE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = guard.as_mut() else {
            return false;
        };
        *state = merge_legacy(*state, &legacy);
        *state
    };
    match sink(next) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("floating lyrics: carrying the old values over failed: {error}");
            false
        }
    }
}

#[cfg(target_os = "android")]
mod jni_exports {
    use jni::{
        objects::{JObject, JString},
        sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE},
        JNIEnv,
    };

    /// `FloatingLyricsStore.nativeState()`: the state as JSON, or a Java null while the settings have not been read.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_FloatingLyricsStore_nativeState(
        env: JNIEnv,
        _store: JObject,
    ) -> jstring {
        match super::state_json().and_then(|json| env.new_string(json).ok()) {
            Some(value) => value.into_raw(),
            None => std::ptr::null_mut(),
        }
    }

    fn text(env: &mut JNIEnv, raw: &JString) -> Option<String> {
        env.get_string(raw)
            .ok()
            .map(|value| value.to_string_lossy().into_owned())
    }

    /// `FloatingLyricsStore.nativeSave(change)`: see `save`; never blocks on the disk.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_FloatingLyricsStore_nativeSave(
        mut env: JNIEnv,
        _store: JObject,
        change: JString,
    ) -> jboolean {
        match text(&mut env, &change) {
            Some(json) if super::save(&json) => JNI_TRUE,
            _ => JNI_FALSE,
        }
    }

    /// `FloatingLyricsStore.nativeImportLegacy(values)`: see `import_legacy`; writes the file before it returns, so it
    /// must not be called on the main thread.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_FloatingLyricsStore_nativeImportLegacy(
        mut env: JNIEnv,
        _store: JObject,
        values: JString,
    ) -> jboolean {
        match text(&mut env, &values) {
            Some(json) if super::import_legacy(&json) => JNI_TRUE,
            _ => JNI_FALSE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Option<FloatingLyricsPosition> {
        Some(FloatingLyricsPosition { x, y })
    }

    fn change(json: &str) -> OverlayChange {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_save_changes_only_the_keys_it_carries() {
        let state = OverlayState {
            locked: false,
            position: at(0.5, 0.1),
            landscape_position: at(0.2, 0.3),
        };
        assert_eq!(
            merge(state, &change(r#"{"locked":true}"#)),
            OverlayState {
                locked: true,
                ..state
            }
        );
        assert_eq!(
            merge(state, &change(r#"{"landscapePosition":{"x":0.7,"y":0.8}}"#)),
            OverlayState {
                landscape_position: at(0.7, 0.8),
                ..state
            }
        );
        // `null` clears (重設位置 of that orientation); the other orientation keeps its own.
        assert_eq!(
            merge(state, &change(r#"{"position":null}"#)),
            OverlayState {
                position: None,
                ..state
            }
        );
        // An invalid ratio is dropped like the settings would drop it.
        assert_eq!(
            merge(state, &change(r#"{"position":{"x":9,"y":0}}"#)).position,
            None
        );
        assert_eq!(merge(state, &change("{}")), state);
        assert!(serde_json::from_str::<OverlayChange>(r#"{"position":"x"}"#).is_err());
    }

    #[test]
    fn the_old_values_never_replace_a_position_already_in_the_settings() {
        let legacy = change(
            r#"{"locked":true,"position":{"x":0.1,"y":0.2},"landscapePosition":{"x":0.3,"y":0.4}}"#,
        );
        let empty = OverlayState::default();
        assert_eq!(
            merge_legacy(empty, &legacy),
            OverlayState {
                locked: true,
                position: at(0.1, 0.2),
                landscape_position: at(0.3, 0.4),
            }
        );
        let stored = OverlayState {
            locked: false,
            position: at(0.5, 0.5),
            landscape_position: None,
        };
        assert_eq!(
            merge_legacy(stored, &legacy),
            OverlayState {
                locked: true,
                position: at(0.5, 0.5),
                landscape_position: at(0.3, 0.4),
            }
        );
        // No old lock key: the stored one stays.
        assert!(
            merge_legacy(
                OverlayState {
                    locked: true,
                    ..empty
                },
                &change("{}")
            )
            .locked
        );
    }

    #[test]
    fn the_overlay_reads_null_until_the_settings_are_known_then_the_saves_apply() {
        assert_eq!(state_json(), None);
        assert!(
            !save(r#"{"locked":true}"#),
            "not seeded: refused, the overlay keeps it"
        );
        let settings = FloatingLyricsSettings {
            locked: true,
            position: at(0.5, 0.1),
            ..FloatingLyricsSettings::default()
        };
        seed(&settings);
        let json: serde_json::Value = serde_json::from_str(&state_json().unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"locked": true, "position": {"x": 0.5, "y": 0.1}, "landscapePosition": null})
        );
        let written = Arc::new(Mutex::new(Vec::<OverlayState>::new()));
        let sink_written = Arc::clone(&written);
        install(Arc::new(move |state| {
            sink_written.lock().unwrap().push(state);
            Ok(())
        }));
        assert!(save(
            r#"{"locked":false,"landscapePosition":{"x":0.25,"y":0.75}}"#
        ));
        assert_eq!(
            current(),
            Some(OverlayState {
                locked: false,
                position: at(0.5, 0.1),
                landscape_position: at(0.25, 0.75),
            })
        );
        assert!(!save("not json"));
        // The carry-over is written before it returns.
        assert!(import_legacy(
            r#"{"locked":true,"position":{"x":0.9,"y":0.9}}"#
        ));
        let last = current().unwrap();
        assert!(last.locked);
        assert_eq!(last.position, at(0.5, 0.1), "kept: already in the settings");
        assert!(
            written.lock().unwrap().contains(&last),
            "written before it returned"
        );
        let mut stored = FloatingLyricsSettings::default();
        last.store_into(&mut stored);
        assert_eq!(OverlayState::of(&stored), last);
    }
}
