use std::sync::{Arc, Condvar, Mutex, MutexGuard};
#[cfg(any(target_os = "windows", test))]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "windows", test))]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(any(target_os = "windows", test))]
const SHUTDOWN_OPEN: u8 = 0;
#[cfg(any(target_os = "windows", test))]
const SHUTDOWN_DRAINING: u8 = 1;
#[cfg(any(target_os = "windows", test))]
const SHUTDOWN_READY: u8 = 2;

#[derive(Clone, Default)]
pub(crate) struct DatabaseWorkGate {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
    #[cfg(any(target_os = "windows", test))]
    close_serialization: Mutex<()>,
}

struct State {
    accepting_work: bool,
    active_work: usize,
}

pub(crate) struct DatabaseWorkGuard {
    inner: Arc<Inner>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            accepting_work: true,
            active_work: 0,
        }
    }
}

impl DatabaseWorkGate {
    pub(crate) fn try_enter(&self) -> Option<DatabaseWorkGuard> {
        let mut state = lock_unpoisoned(&self.inner.state);
        if !state.accepting_work {
            return None;
        }
        state.active_work += 1;
        Some(DatabaseWorkGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    #[cfg(test)]
    pub(crate) fn is_accepting_work(&self) -> bool {
        lock_unpoisoned(&self.inner.state).accepting_work
    }

    /// Stop admitting database jobs and wait for every admitted job to finish.
    /// A timeout reopens admission so the application remains usable and can be
    /// closed again after the active work completes.
    #[cfg(any(target_os = "windows", test))]
    pub(crate) fn close_and_wait(&self, timeout: Duration) -> bool {
        let _close = lock_unpoisoned(&self.inner.close_serialization);
        let deadline = Instant::now() + timeout;
        let mut state = lock_unpoisoned(&self.inner.state);
        state.accepting_work = false;

        while state.active_work != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.accepting_work = true;
                self.inner.changed.notify_all();
                return false;
            }
            let result = self.inner.changed.wait_timeout(state, remaining);
            let (next_state, wait) = match result {
                Ok(result) => result,
                Err(poisoned) => poisoned.into_inner(),
            };
            state = next_state;
            if wait.timed_out() && state.active_work != 0 {
                state.accepting_work = true;
                self.inner.changed.notify_all();
                return false;
            }
        }
        true
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShutdownRequest {
    Start,
    InProgress,
    Ready,
}

/// Serializes window-close and process-exit requests onto one database drain.
#[cfg(any(target_os = "windows", test))]
#[derive(Default)]
pub(crate) struct ShutdownCoordinator {
    state: AtomicU8,
}

#[cfg(any(target_os = "windows", test))]
impl ShutdownCoordinator {
    pub(crate) fn request(&self) -> ShutdownRequest {
        loop {
            match self.state.load(Ordering::Acquire) {
                SHUTDOWN_READY => return ShutdownRequest::Ready,
                SHUTDOWN_DRAINING => return ShutdownRequest::InProgress,
                SHUTDOWN_OPEN => {
                    if self
                        .state
                        .compare_exchange(
                            SHUTDOWN_OPEN,
                            SHUTDOWN_DRAINING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return ShutdownRequest::Start;
                    }
                }
                _ => unreachable!("invalid shutdown state"),
            }
        }
    }

    fn reopen(&self) {
        self.state.store(SHUTDOWN_OPEN, Ordering::Release);
    }

    fn mark_ready(&self) {
        self.state.store(SHUTDOWN_READY, Ordering::Release);
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShutdownDrainError {
    TimedOut,
}

/// Drain admitted DB work before running the final checkpoint/session flush.
/// Both Tauri close and exit requests use this exact orchestration path.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn finish_shutdown<T>(
    coordinator: &ShutdownCoordinator,
    gate: &DatabaseWorkGate,
    timeout: Duration,
    finalize: impl FnOnce() -> T,
) -> Result<T, ShutdownDrainError> {
    if !gate.close_and_wait(timeout) {
        coordinator.reopen();
        return Err(ShutdownDrainError::TimedOut);
    }

    let result = finalize();
    coordinator.mark_ready();
    Ok(result)
}

impl Drop for DatabaseWorkGuard {
    fn drop(&mut self) {
        let mut state = lock_unpoisoned(&self.inner.state);
        debug_assert_ne!(state.active_work, 0);
        state.active_work = state.active_work.saturating_sub(1);
        if state.active_work == 0 {
            self.inner.changed.notify_all();
        }
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{
        finish_shutdown, DatabaseWorkGate, ShutdownCoordinator, ShutdownDrainError, ShutdownRequest,
    };
    use std::{
        fs,
        path::PathBuf,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use player_db::{Database, ThemePreferences};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moe-database-shutdown-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn database_path(&self) -> PathBuf {
            self.0.join("shutdown.sqlite3")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn close_waits_for_admitted_database_work_and_rejects_new_work() {
        let gate = DatabaseWorkGate::default();
        let writer = gate.try_enter().expect("writer admitted before close");
        let worker_gate = gate.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let closer = thread::spawn(move || {
            started_tx.send(()).expect("announce close attempt");
            let drained = worker_gate.close_and_wait(Duration::from_secs(2));
            finished_tx.send(drained).expect("report close result");
        });

        started_rx.recv().expect("close attempt started");
        let deadline = Instant::now() + Duration::from_secs(1);
        while gate.is_accepting_work() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            !gate.is_accepting_work(),
            "close should stop new database work"
        );
        assert!(
            gate.try_enter().is_none(),
            "new work must not enter while closing"
        );
        assert!(
            finished_rx.try_recv().is_err(),
            "close must wait for the active writer"
        );

        drop(writer);
        assert!(finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close should drain after writer exits"));
        closer.join().expect("close worker joins");
        assert!(
            gate.try_enter().is_none(),
            "a drained shutdown stays closed"
        );
    }

    #[test]
    fn close_timeout_reopens_work_instead_of_abandoning_a_writer() {
        let gate = DatabaseWorkGate::default();
        let writer = gate.try_enter().expect("writer admitted");

        assert!(!gate.close_and_wait(Duration::from_millis(10)));
        assert!(
            gate.is_accepting_work(),
            "timed-out close must reopen the app gate"
        );
        let next_writer = gate
            .try_enter()
            .expect("work can continue after close timeout");
        drop(next_writer);
        drop(writer);
        assert!(gate.close_and_wait(Duration::from_secs(1)));
    }

    #[test]
    fn shutdown_orchestration_drains_real_database_writer_before_checkpoint() {
        let directory = TestDirectory::new();
        let database = std::sync::Arc::new(
            Database::open(directory.database_path()).expect("open isolated SQLite database"),
        );
        let gate = DatabaseWorkGate::default();
        let coordinator = std::sync::Arc::new(ShutdownCoordinator::default());
        let writer_guard = gate.try_enter().expect("writer admitted before shutdown");
        let expected = ThemePreferences {
            background_hex: "#123456".to_owned(),
            accent_hex: "#654321".to_owned(),
        };
        let expected_in_finalizer = expected.clone();
        let (release_writer_tx, release_writer_rx) = mpsc::channel();
        let writer_database = std::sync::Arc::clone(&database);
        let writer_value = expected.clone();
        let writer = thread::spawn(move || {
            release_writer_rx.recv().expect("release active writer");
            writer_database
                .set_theme_preferences(&writer_value)
                .expect("persist admitted writer result");
            drop(writer_guard);
        });

        assert_eq!(coordinator.request(), ShutdownRequest::Start);
        assert_eq!(coordinator.request(), ShutdownRequest::InProgress);

        let shutdown_gate = gate.clone();
        let shutdown_coordinator = std::sync::Arc::clone(&coordinator);
        let shutdown_database = std::sync::Arc::clone(&database);
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let shutdown = thread::spawn(move || {
            started_tx
                .send(())
                .expect("announce shutdown orchestration");
            let result = finish_shutdown(
                &shutdown_coordinator,
                &shutdown_gate,
                Duration::from_secs(2),
                || {
                    let persisted = shutdown_database
                        .get_theme_preferences()
                        .expect("read state after admitted writer");
                    assert_eq!(persisted, expected_in_finalizer);
                    shutdown_database
                        .checkpoint_wal()
                        .expect("checkpoint after all admitted writes");
                },
            );
            finished_tx.send(result).expect("report shutdown result");
        });

        started_rx.recv().expect("shutdown orchestration started");
        let deadline = Instant::now() + Duration::from_secs(1);
        while gate.is_accepting_work() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(
            !gate.is_accepting_work(),
            "shutdown must stop admitting work before finalization"
        );
        assert!(
            finished_rx.try_recv().is_err(),
            "shutdown finalization must wait for the active SQLite writer"
        );
        assert!(gate.try_enter().is_none());

        release_writer_tx
            .send(())
            .expect("let admitted writer commit");
        writer.join().expect("active writer joins");
        assert_eq!(
            finished_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("shutdown completes after writer drain"),
            Ok(())
        );
        shutdown.join().expect("shutdown worker joins");
        assert_eq!(coordinator.request(), ShutdownRequest::Ready);
        assert_eq!(
            database.get_theme_preferences().expect("read final state"),
            expected
        );
    }

    #[test]
    fn shutdown_timeout_reopens_gate_and_coordinator_for_retry() {
        let gate = DatabaseWorkGate::default();
        let coordinator = ShutdownCoordinator::default();
        let writer = gate.try_enter().expect("writer admitted");
        assert_eq!(coordinator.request(), ShutdownRequest::Start);

        assert_eq!(
            finish_shutdown(&coordinator, &gate, Duration::from_millis(10), || ()),
            Err(ShutdownDrainError::TimedOut)
        );
        assert!(gate.is_accepting_work());
        assert_eq!(coordinator.request(), ShutdownRequest::Start);
        drop(writer);
        assert_eq!(
            finish_shutdown(&coordinator, &gate, Duration::from_secs(1), || ()),
            Ok(())
        );
        assert_eq!(coordinator.request(), ShutdownRequest::Ready);
    }

    use std::time::{SystemTime, UNIX_EPOCH};
}
