//! 「暫停時隱藏」 (ADR 0034): whether a lyrics window shows while its setting is on. A pure state machine fed with the
//! playback activity and a clock (ms on any monotonic base), shared by the taskbar and the floating windows; the
//! Android overlay has the same rules in Kotlin (`PauseHide.kt`).
//!
//! Only a real pause or stop hides a shown window at once. The states the players pass through while one track
//! makes way for the next (Windows: `ready` after the new file opened, `ended` while the next one is checked, `loading`
//! when nothing was loaded; Android: buffering, ended) never hide it at once: buffering, loading and seeking hold the
//! current visibility, and the other not-playing states hide only once they lasted [`IDLE_HIDE_DELAY_MS`]
//! (the end of the queue, a track that was stopped at its end, a restored session that does not play).

/// How long a not-playing state other than paused or stopped must last before it hides a shown window.
pub const IDLE_HIDE_DELAY_MS: f64 = 3000.0;
/// The hide / show fade (the project's animation length).
pub const FADE_MS: f64 = 200.0;

/// What the player is doing, as far as hiding is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// Playing: show.
    Playing,
    /// Buffering, loading, seeking, or opening a file while the user wants it to play: keep what is shown.
    Transient,
    /// Paused or stopped by the user: hide.
    Paused,
    /// Any other not-playing state (ready, ended, empty, error, no player): hide after [`IDLE_HIDE_DELAY_MS`].
    Idle,
}

impl Activity {
    /// From the main window's playback sample (`taskbar-lyrics-playback`: the snapshot's `state` and `playing`).
    pub fn from_state(state: &str, playing: bool) -> Self {
        match state {
            "playing" if playing => Activity::Playing,
            // "playing" without the flag (a sample taken mid-change), buffering and opening: on the way to playing.
            "playing" | "buffering" | "loading" | "opening" | "initializing" => Activity::Transient,
            "paused" | "stopped" => Activity::Paused,
            _ => Activity::Idle,
        }
    }
}

/// The visibility one window would have with 「暫停時隱藏」 on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PauseGate {
    shown: bool,
    activity: Option<Activity>,
    /// Since when (ms) the current [`Activity::Idle`] lasts while the window is shown.
    idle_since: Option<f64>,
}

impl PauseGate {
    /// Nothing observed yet: hidden (a window that opens while nothing plays does not flash).
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the activity observed at `now_ms`; returns whether the window shows.
    pub fn observe(&mut self, activity: Activity, now_ms: f64) -> bool {
        match activity {
            Activity::Playing => {
                self.shown = true;
                self.idle_since = None;
            }
            Activity::Transient => self.idle_since = None,
            Activity::Paused => {
                self.shown = false;
                self.idle_since = None;
            }
            Activity::Idle => {
                if self.shown && self.idle_since.is_none() {
                    self.idle_since = Some(now_ms);
                }
            }
        }
        self.activity = Some(activity);
        self.tick(now_ms)
    }

    /// Re-evaluates at `now_ms` with the last activity (call at [`Self::due_at`]); returns whether the window shows.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        if let Some(since) = self.idle_since {
            if now_ms - since >= IDLE_HIDE_DELAY_MS {
                self.shown = false;
                self.idle_since = None;
            }
        }
        self.shown
    }

    /// When a pending idle hide is due (ms), so the caller can wake for it; `None` when nothing is pending.
    pub fn due_at(&self) -> Option<f64> {
        self.idle_since.map(|since| since + IDLE_HIDE_DELAY_MS)
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn activity(&self) -> Option<Activity> {
        self.activity
    }

    /// Whether the window shows given its setting.
    pub fn visible(&self, hide_when_paused: bool) -> bool {
        !hide_when_paused || self.shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_states_map_to_activities() {
        assert_eq!(Activity::from_state("playing", true), Activity::Playing);
        assert_eq!(Activity::from_state("playing", false), Activity::Transient);
        for state in ["buffering", "loading", "opening", "initializing"] {
            assert_eq!(
                Activity::from_state(state, false),
                Activity::Transient,
                "{state}"
            );
        }
        assert_eq!(Activity::from_state("paused", false), Activity::Paused);
        assert_eq!(Activity::from_state("stopped", false), Activity::Paused);
        for state in ["ready", "ended", "empty", "error", "whatever"] {
            assert_eq!(
                Activity::from_state(state, false),
                Activity::Idle,
                "{state}"
            );
        }
    }

    #[test]
    fn a_pause_hides_at_once_and_play_shows_again() {
        let mut gate = PauseGate::new();
        assert!(!gate.shown(), "nothing seen yet: hidden");
        assert!(gate.visible(false), "the setting off always shows");
        assert!(gate.observe(Activity::Playing, 0.0));
        assert!(!gate.observe(Activity::Paused, 10.0));
        assert_eq!(gate.due_at(), None);
        assert!(gate.observe(Activity::Playing, 20.0));
    }

    #[test]
    fn a_track_switch_never_hides() {
        // Windows: playing -> ready (the new file opened) -> playing.
        let mut gate = PauseGate::new();
        gate.observe(Activity::Playing, 0.0);
        assert!(gate.observe(Activity::from_state("ready", false), 100.0));
        assert!(gate.tick(600.0));
        assert!(gate.observe(Activity::Playing, 700.0));
        assert_eq!(gate.due_at(), None);
        // Natural end: ended while the next track is checked, ready, then playing.
        assert!(gate.observe(Activity::from_state("ended", false), 1000.0));
        assert_eq!(gate.due_at(), Some(1000.0 + IDLE_HIDE_DELAY_MS));
        assert!(gate.observe(Activity::from_state("ready", false), 2500.0));
        assert_eq!(
            gate.due_at(),
            Some(1000.0 + IDLE_HIDE_DELAY_MS),
            "the idle time runs on"
        );
        assert!(gate.observe(Activity::Playing, 2900.0));
        // Android: buffering after the load, ended between tracks.
        assert!(gate.observe(Activity::Transient, 3000.0));
        assert!(gate.tick(60_000.0), "buffering holds however long it takes");
        assert!(gate.observe(Activity::Playing, 60_001.0));
    }

    #[test]
    fn a_lasting_idle_state_hides_after_the_delay() {
        let mut gate = PauseGate::new();
        gate.observe(Activity::Playing, 0.0);
        assert!(gate.observe(Activity::Idle, 1000.0));
        assert!(gate.tick(1000.0 + IDLE_HIDE_DELAY_MS - 1.0));
        assert!(!gate.tick(1000.0 + IDLE_HIDE_DELAY_MS));
        assert_eq!(gate.due_at(), None);
        // Still idle later: stays hidden; playing shows.
        assert!(!gate.observe(Activity::Idle, 9000.0));
        assert!(gate.observe(Activity::Playing, 9001.0));
    }

    #[test]
    fn buffering_in_between_restarts_the_idle_time() {
        let mut gate = PauseGate::new();
        gate.observe(Activity::Playing, 0.0);
        gate.observe(Activity::Idle, 0.0);
        gate.observe(Activity::Transient, 2000.0);
        assert!(gate.observe(Activity::Idle, 2500.0));
        assert!(gate.tick(2500.0 + IDLE_HIDE_DELAY_MS - 1.0));
    }

    #[test]
    fn a_window_that_opens_while_nothing_plays_stays_hidden() {
        for first in [Activity::Paused, Activity::Idle, Activity::Transient] {
            let mut gate = PauseGate::new();
            assert!(!gate.observe(first, 0.0), "{first:?}");
            assert_eq!(gate.due_at(), None);
            assert!(gate.observe(Activity::Playing, 5.0));
        }
    }
}
