//! The cover scan of the album detail page (ADR 0037): at most one runs at a time, and it ends the moment the page is left.
//!
//! A scan is registered when its command arrives (before anything else), so a newer scan, or the page leaving, can reach it
//! while it still waits its turn. Registering a scan cancels the one that is registered, and a scan runs only when the
//! previous one has stopped (`wait_turn`). A cancelled scan stops between files and inside a file read (the readers look at
//! the flag before each request to a server), writes nothing and gives its turn back at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

struct Job {
    key: String,
    cancelled: Arc<AtomicBool>,
}

/// The scan that is registered (running or waiting its turn).
static CURRENT: Mutex<Option<Arc<Job>>> = Mutex::new(None);
/// Held by the scan that runs.
static TURN: Mutex<()> = Mutex::new(());

/// A scan of the album detail page. Dropping it unregisters it.
pub(crate) struct Registration {
    job: Arc<Job>,
}

/// The right to run: held while a scan reads files.
pub(crate) struct Turn(#[allow(dead_code)] MutexGuard<'static, ()>);

/// Register the scan of `key`; the scan that was registered is cancelled.
pub(crate) fn register(key: &str) -> Registration {
    let job = Arc::new(Job {
        key: key.to_owned(),
        cancelled: Arc::new(AtomicBool::new(false)),
    });
    let mut current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(previous) = current.replace(Arc::clone(&job)) {
        previous.cancelled.store(true, Ordering::SeqCst);
    }
    Registration { job }
}

/// The page that used these keys was left: the scan registered under one of them is cancelled. Returns how many were.
pub(crate) fn cancel(keys: &[String]) -> usize {
    let current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    match current.as_ref() {
        Some(job) if keys.iter().any(|key| *key == job.key) => {
            job.cancelled.store(true, Ordering::SeqCst);
            1
        }
        _ => 0,
    }
}

impl Registration {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.job.cancelled.load(Ordering::SeqCst)
    }

    /// The flag the readers of the scan look at.
    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.job.cancelled)
    }

    /// Wait until the scan before this one has stopped. `None` when this scan was cancelled meanwhile (nothing is to be
    /// read). Blocks: call it on a blocking worker.
    pub(crate) fn wait_turn(&self) -> Option<Turn> {
        let turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_cancelled() {
            return None;
        }
        Some(Turn(turn))
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
        if current
            .as_ref()
            .is_some_and(|job| Arc::ptr_eq(job, &self.job))
        {
            *current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The registry is process-wide: tests that use it take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn registering_a_scan_cancels_the_previous_one_and_leaving_cancels_by_key() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let first = register("album-a");
        assert!(!first.is_cancelled());
        let second = register("album-b");
        assert!(
            first.is_cancelled(),
            "opening another album cancels the scan before it"
        );
        assert!(!second.is_cancelled());
        // Another page's keys do not touch it; its own do.
        assert_eq!(cancel(&["album-a".to_owned()]), 0);
        assert!(!second.is_cancelled());
        assert_eq!(cancel(&["x".to_owned(), "album-b".to_owned()]), 1);
        assert!(second.is_cancelled());
        drop(second);
        drop(first);
        assert_eq!(
            cancel(&["album-b".to_owned()]),
            0,
            "nothing is registered any more"
        );
    }

    #[test]
    fn only_one_scan_runs_at_a_time_and_a_cancelled_one_never_runs() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let first = register("one");
        let turn = first.wait_turn().expect("the first scan runs");
        let second = register("two");
        let third = register("three");
        assert!(second.is_cancelled());
        let (sender, receiver) = mpsc::channel();
        let waiting_second = std::thread::spawn({
            let sender = sender.clone();
            move || {
                let ran = second.wait_turn().is_some();
                sender.send(("second", ran)).unwrap();
            }
        });
        let waiting_third = std::thread::spawn(move || {
            let ran = third.wait_turn().is_some();
            sender.send(("third", ran)).unwrap();
        });
        assert!(
            receiver.recv_timeout(Duration::from_millis(200)).is_err(),
            "nobody runs while the first scan holds the turn"
        );
        assert!(
            first.is_cancelled(),
            "the first scan was cancelled by the newer ones"
        );
        drop(turn);
        let mut results = vec![
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        ];
        results.sort_unstable();
        assert_eq!(results, vec![("second", false), ("third", true)]);
        waiting_second.join().unwrap();
        waiting_third.join().unwrap();
        drop(first);
    }
}
