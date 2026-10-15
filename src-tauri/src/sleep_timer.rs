//! Sleep timer (睡眠定時). The Rust core owns the deadline so it fires even when the
//! WebView is throttled or asleep. The deadline is a wall-clock end timestamp; the
//! watcher re-reads `SystemTime` at most every second instead of accumulating ticks,
//! so system suspend or a stalled UI cannot postpone expiry beyond one poll.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

pub const SLEEP_TIMER_CHANGED_EVENT: &str = "sleep-timer-changed";
/// 23:55, the largest value reachable with 5-minute steps below 24 h.
pub const MAX_SLEEP_TIMER_MS: u64 = (23 * 60 + 55) * 60_000;
const POLL_MS: u64 = 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepTimerSnapshot {
    /// Unix epoch milliseconds when playback pauses, or `None` when the timer is off.
    pub ends_at_ms: Option<u64>,
}

#[derive(Default)]
struct SleepTimerInner {
    ends_at_ms: Option<u64>,
    generation: u64,
}

#[derive(Default)]
pub struct SleepTimer {
    inner: Mutex<SleepTimerInner>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// How long the watcher should sleep before re-checking the wall clock, or `None` once expired.
pub fn next_wait(ends_at_ms: u64, now: u64) -> Option<Duration> {
    if now >= ends_at_ms {
        None
    } else {
        Some(Duration::from_millis((ends_at_ms - now).min(POLL_MS)))
    }
}

impl SleepTimer {
    pub fn snapshot(&self) -> SleepTimerSnapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        SleepTimerSnapshot {
            ends_at_ms: inner.ends_at_ms,
        }
    }

    /// Start (duration > 0) or cancel (0) the timer. Returns the new snapshot and its generation.
    pub fn set(&self, duration_ms: u64, now: u64) -> (SleepTimerSnapshot, u64) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        inner.generation = inner.generation.wrapping_add(1);
        let duration = duration_ms.min(MAX_SLEEP_TIMER_MS);
        inner.ends_at_ms = (duration > 0).then(|| now.saturating_add(duration));
        (
            SleepTimerSnapshot {
                ends_at_ms: inner.ends_at_ms,
            },
            inner.generation,
        )
    }

    /// Deadline of a still-current watcher generation.
    fn current_deadline(&self, generation: u64) -> Option<u64> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if inner.generation == generation {
            inner.ends_at_ms
        } else {
            None
        }
    }

    /// Clear the timer when this generation has expired; true means the caller must pause.
    pub fn take_if_expired(&self, generation: u64, now: u64) -> bool {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match inner.ends_at_ms {
            Some(ends_at) if inner.generation == generation && now >= ends_at => {
                inner.ends_at_ms = None;
                true
            }
            _ => false,
        }
    }
}

#[tauri::command]
pub fn sleep_timer_get(timer: State<'_, SleepTimer>) -> SleepTimerSnapshot {
    timer.snapshot()
}

#[tauri::command]
pub fn sleep_timer_set(
    app: AppHandle,
    timer: State<'_, SleepTimer>,
    duration_ms: u64,
) -> SleepTimerSnapshot {
    let (snapshot, generation) = timer.set(duration_ms, now_ms());
    let _ = app.emit(SLEEP_TIMER_CHANGED_EVENT, snapshot);
    if snapshot.ends_at_ms.is_some() {
        tauri::async_runtime::spawn(watch(app, generation));
    }
    snapshot
}

async fn watch(app: AppHandle, generation: u64) {
    loop {
        let Some(ends_at) = app.state::<SleepTimer>().current_deadline(generation) else {
            return;
        };
        match next_wait(ends_at, now_ms()) {
            Some(wait) => tokio::time::sleep(wait).await,
            None => {
                if app
                    .state::<SleepTimer>()
                    .take_if_expired(generation, now_ms())
                {
                    let _ = app.emit(SLEEP_TIMER_CHANGED_EVENT, SleepTimerSnapshot::default());
                    // Pause, not stop: the queue and position stay so playback can resume.
                    if let Err(error) = crate::playback_pause(app.state::<crate::AppState>()).await
                    {
                        eprintln!("sleep timer pause failed: {error}");
                    }
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_positive_duration_starts_and_zero_cancels() {
        let timer = SleepTimer::default();
        let (started, _) = timer.set(30 * 60_000, 1_000);
        assert_eq!(started.ends_at_ms, Some(1_000 + 30 * 60_000));
        let (cancelled, _) = timer.set(0, 2_000);
        assert_eq!(cancelled.ends_at_ms, None);
    }

    #[test]
    fn duration_is_clamped_to_23_55() {
        let timer = SleepTimer::default();
        let (snapshot, _) = timer.set(u64::MAX, 0);
        assert_eq!(snapshot.ends_at_ms, Some(MAX_SLEEP_TIMER_MS));
    }

    #[test]
    fn expiry_uses_wall_clock_and_ignores_stale_generations() {
        let timer = SleepTimer::default();
        let (_, first) = timer.set(60_000, 0);
        let (_, second) = timer.set(120_000, 0);
        assert!(
            !timer.take_if_expired(first, 200_000),
            "replaced timer must not fire"
        );
        assert!(!timer.take_if_expired(second, 119_999));
        assert!(timer.take_if_expired(second, 120_000));
        assert_eq!(
            timer.snapshot().ends_at_ms,
            None,
            "timer resets to off after firing"
        );
        assert!(!timer.take_if_expired(second, 130_000), "fires once");
    }

    #[test]
    fn watcher_polls_at_most_once_per_second() {
        assert_eq!(next_wait(10_000, 0), Some(Duration::from_millis(1_000)));
        assert_eq!(next_wait(10_000, 9_700), Some(Duration::from_millis(300)));
        assert_eq!(next_wait(10_000, 10_000), None);
        assert_eq!(next_wait(10_000, 20_000), None);
    }
}
