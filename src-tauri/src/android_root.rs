//! Android: whether the user asked to use root (ADR 0048), kept in `settings.json` (`androidRootOptIn`).
//!
//! Rust is the only writer. The Kotlin side (`MixerRatePrefs`) reads this copy over JNI and reports the user's answer
//! over JNI: the copy changes at once and a worker thread writes the file, so the Android main thread never waits for
//! the disk. The value an older build kept in SharedPreferences (`moe_mixer_rate`, key `rootOptIn`) is carried over
//! once (`import_legacy`, written before it returns, off the main thread) and only when `settings.json` has none.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

/// The settings were not read yet in this process.
pub(crate) const NOT_READ: i32 = -1;
/// `settings.json` holds no answer (never asked).
pub(crate) const UNSET: i32 = 0;
pub(crate) const OFF: i32 = 1;
pub(crate) const ON: i32 = 2;

pub(crate) fn encode(value: Option<bool>) -> i32 {
    match value {
        None => UNSET,
        Some(false) => OFF,
        Some(true) => ON,
    }
}

pub(crate) fn decode(code: i32) -> Option<Option<bool>> {
    match code {
        UNSET => Some(None),
        OFF => Some(Some(false)),
        ON => Some(Some(true)),
        _ => None,
    }
}

static STATE: AtomicI32 = AtomicI32::new(NOT_READ);

type Sink = Arc<dyn Fn(Option<bool>) -> Result<(), String> + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();
static WORKER: OnceLock<Mutex<Sender<()>>> = OnceLock::new();

/// Remember the stored value (at startup, after the settings were read).
pub(crate) fn seed(value: Option<bool>) {
    STATE.store(encode(value), Ordering::SeqCst);
}

/// The copy as Kotlin reads it (`NOT_READ`, `UNSET`, `OFF`, `ON`).
pub(crate) fn current() -> i32 {
    STATE.load(Ordering::SeqCst)
}

/// Install the writer (at startup); `sink` stores a value in `settings.json` on a thread of its own.
pub(crate) fn install(sink: Sink) {
    if SINK.set(Arc::clone(&sink)).is_err() {
        return;
    }
    let (sender, receiver) = mpsc::channel::<()>();
    if WORKER.set(Mutex::new(sender)).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("root-opt-in-save".into())
        .spawn(move || {
            while receiver.recv().is_ok() {
                while receiver.try_recv().is_ok() {}
                if let Some(value) = decode(current()) {
                    if let Err(error) = sink(value) {
                        eprintln!("root opt-in: {error}");
                    }
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("root opt-in: no writer thread: {error}");
    }
}

/// The user's answer (`MixerRatePrefs.nativeSetRootOptIn`): applied to the copy at once, written by the worker. False
/// while the settings are not known (Kotlin keeps the value and sends it again later).
pub(crate) fn save(value: bool) -> bool {
    if current() == NOT_READ {
        return false;
    }
    STATE.store(encode(Some(value)), Ordering::SeqCst);
    match WORKER.get() {
        Some(worker) => worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(())
            .is_ok(),
        None => false,
    }
}

/// The one-time carry-over of the old SharedPreferences value (`MixerRatePrefs.nativeImportLegacyRootOptIn`, not on
/// the main thread). An answer already in `settings.json` is kept (true: the old key can go). Otherwise the old value
/// is written before this returns; false when it could not be (Kotlin keeps the old key and tries again later).
pub(crate) fn import_legacy(value: bool) -> bool {
    match current() {
        NOT_READ => return false,
        UNSET => {}
        _ => return true,
    }
    let Some(sink) = SINK.get() else {
        return false;
    };
    if STATE
        .compare_exchange(
            UNSET,
            encode(Some(value)),
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        // The user answered meanwhile: that answer wins.
        return true;
    }
    match sink(Some(value)) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("root opt-in: carrying the old value over failed: {error}");
            let _ = STATE.compare_exchange(
                encode(Some(value)),
                UNSET,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            false
        }
    }
}

#[cfg(target_os = "android")]
mod jni_exports {
    use jni::{
        objects::JObject,
        sys::{jboolean, jint, JNI_FALSE, JNI_TRUE},
        JNIEnv,
    };

    /// `MixerRatePrefs.nativeRootOptIn()`: -1 not read yet, 0 never asked, 1 off, 2 on.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_MixerRatePrefs_nativeRootOptIn(
        _env: JNIEnv,
        _prefs: JObject,
    ) -> jint {
        super::current()
    }

    /// `MixerRatePrefs.nativeSetRootOptIn(value)`: see `save`; never blocks on the disk.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_MixerRatePrefs_nativeSetRootOptIn(
        _env: JNIEnv,
        _prefs: JObject,
        value: jboolean,
    ) -> jboolean {
        if super::save(value != JNI_FALSE) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }

    /// `MixerRatePrefs.nativeImportLegacyRootOptIn(value)`: see `import_legacy`; writes the file before it returns.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_MixerRatePrefs_nativeImportLegacyRootOptIn(
        _env: JNIEnv,
        _prefs: JObject,
        value: jboolean,
    ) -> jboolean {
        if super::import_legacy(value != JNI_FALSE) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_is_kept_by_rust_and_the_old_value_only_fills_an_empty_one() {
        for value in [None, Some(false), Some(true)] {
            assert_eq!(decode(encode(value)), Some(value));
        }
        assert_eq!(decode(NOT_READ), None);

        assert_eq!(current(), NOT_READ);
        assert!(!save(true), "not read yet: refused, Kotlin keeps it");
        assert!(!import_legacy(true));

        let written = Arc::new(Mutex::new(Vec::<Option<bool>>::new()));
        let sink_written = Arc::clone(&written);
        install(Arc::new(move |value| {
            sink_written.lock().unwrap().push(value);
            Ok(())
        }));
        seed(None);
        assert_eq!(current(), UNSET);
        // The old value fills the empty setting and is written before the call returns.
        assert!(import_legacy(true));
        assert_eq!(current(), ON);
        assert_eq!(written.lock().unwrap().last(), Some(&Some(true)));
        // Once there is an answer, an old value never replaces it (the old key may still go).
        assert!(import_legacy(false));
        assert_eq!(current(), ON);
        // The user's answer replaces it.
        assert!(save(false));
        assert_eq!(current(), OFF);
    }
}
