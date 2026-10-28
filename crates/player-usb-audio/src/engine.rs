//! The life of the USB exclusive output (ADR 0047): a pure state machine. The usbfs runtime feeds it what Media3 does
//! and time, and carries out the actions it answers; nothing here touches the device.
//!
//! ```text
//! Unclaimed ──Attach──▶ Claimed ──Play──▶ Streaming ──Pause──▶ Paused ──Play──▶ Streaming
//!                         │  ▲                │ Detach (output released: a seek, the next track)
//!                         │  └─Attach(other)──┤ keeps streaming silence for ORPHAN_GRACE_MS, then ─▶ Paused
//!                         └──── idle IDLE_RELEASE_MS (Claimed or Paused) ─────────────────────────▶ HandedBack
//! Release (stop, setting off, service gone) or a device error, from any state ──────────────────▶ HandedBack
//! ```
//!
//! An Attach of the format that streams keeps the stream running (no alt change, no clock change); another format
//! stops it, changes the alt and the clock, and waits for Play. HandedBack is the end of one claim: the runtime gives
//! the device back and stops; a later output claims again.

/// After the last output was released, how long the stream runs on with silence for the next one (a seek or the next
/// track opens a new output at once; the alt and clock then stay as they are).
pub const ORPHAN_GRACE_MS: u64 = 1_000;
/// Claimed but not streaming this long: the device goes back to Android (Windows exclusive mode's `IDLE_RELEASE`).
pub const IDLE_RELEASE_MS: u64 = 10_000;

/// What the device is set to stream: equal formats share the running stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamFormat {
    pub rate: u32,
    pub channels: u8,
    pub alt: u8,
    pub subslot_bytes: u8,
    pub bits: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Unclaimed,
    /// Interfaces claimed, the alt and the clock set for `format`, no URBs yet.
    Claimed {
        format: StreamFormat,
        since_ms: u64,
    },
    /// URBs in flight. `orphan_since_ms`: no output is attached, silence goes out.
    Streaming {
        format: StreamFormat,
        orphan_since_ms: Option<u64>,
    },
    /// Streaming stopped and the interface at alt 0; still claimed.
    Paused {
        format: StreamFormat,
        since_ms: u64,
    },
    HandedBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// An output for `format` was created (`AudioOutputProvider.getAudioOutput`).
    Attach(StreamFormat),
    /// `AudioOutput.play`.
    Play,
    /// `AudioOutput.pause`.
    Pause,
    /// The output was released (`AudioOutput.release`; Media3 also releases it on every flush).
    Detach,
    /// Time passed.
    Tick,
    /// Stop, the setting turned off, the service or app ending: give the device back now.
    Release,
    /// The device failed or went away.
    DeviceError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// Detach the kernel driver from every audio interface and claim them.
    Claim,
    /// Streaming interface to alt 0 (if not there), the alt of `format`, SET CUR the rate, settle, read it back.
    Configure(StreamFormat),
    /// Submit the feedback and data URBs.
    StartStreaming,
    /// Discard and reap every URB (rewinding what was not played), then alt 0.
    StopStreaming,
    /// Alt 0, release, let the kernel driver bind again.
    HandBack,
}

/// The next state and what to do, for `event` at `now_ms`.
pub fn step(state: State, event: Event, now_ms: u64) -> (State, Vec<Action>) {
    use Action::*;
    use State::*;
    match (state, event) {
        (HandedBack, _) => (HandedBack, vec![]),
        (_, Event::DeviceError) => (HandedBack, vec![HandBack]),
        (Unclaimed, Event::Release) => (HandedBack, vec![]),
        (Streaming { .. }, Event::Release) => (HandedBack, vec![StopStreaming, HandBack]),
        (_, Event::Release) => (HandedBack, vec![HandBack]),

        (Unclaimed, Event::Attach(format)) => (
            Claimed {
                format,
                since_ms: now_ms,
            },
            vec![Claim, Configure(format)],
        ),
        (
            Claimed {
                format: current, ..
            },
            Event::Attach(format),
        ) => (
            Claimed {
                format,
                since_ms: now_ms,
            },
            if current == format {
                vec![]
            } else {
                vec![Configure(format)]
            },
        ),
        (Paused { .. }, Event::Attach(format)) => (
            Paused {
                format,
                since_ms: now_ms,
            },
            vec![],
        ),
        (
            Streaming {
                format: current, ..
            },
            Event::Attach(format),
        ) if current == format => (
            Streaming {
                format,
                orphan_since_ms: None,
            },
            vec![],
        ),
        (Streaming { .. }, Event::Attach(format)) => (
            Claimed {
                format,
                since_ms: now_ms,
            },
            vec![StopStreaming, Configure(format)],
        ),

        (Claimed { format, .. }, Event::Play) => (
            Streaming {
                format,
                orphan_since_ms: None,
            },
            vec![StartStreaming],
        ),
        (Paused { format, .. }, Event::Play) => (
            Streaming {
                format,
                orphan_since_ms: None,
            },
            vec![Configure(format), StartStreaming],
        ),

        (Streaming { format, .. }, Event::Pause) => (
            Paused {
                format,
                since_ms: now_ms,
            },
            vec![StopStreaming],
        ),

        (
            Streaming {
                format,
                orphan_since_ms: None,
            },
            Event::Detach,
        ) => (
            Streaming {
                format,
                orphan_since_ms: Some(now_ms),
            },
            vec![],
        ),

        (
            Streaming {
                format,
                orphan_since_ms: Some(since),
            },
            Event::Tick,
        ) if now_ms.saturating_sub(since) >= ORPHAN_GRACE_MS => (
            Paused {
                format,
                since_ms: now_ms,
            },
            vec![StopStreaming],
        ),
        (Claimed { since_ms, .. } | Paused { since_ms, .. }, Event::Tick)
            if now_ms.saturating_sub(since_ms) >= IDLE_RELEASE_MS =>
        {
            (HandedBack, vec![HandBack])
        }

        (state, _) => (state, vec![]),
    }
}

/// The state's name for the status (`unclaimed`, `claimed`, `streaming`, `paused`, `handedBack`).
pub fn state_name(state: State) -> &'static str {
    match state {
        State::Unclaimed => "unclaimed",
        State::Claimed { .. } => "claimed",
        State::Streaming { .. } => "streaming",
        State::Paused { .. } => "paused",
        State::HandedBack => "handedBack",
    }
}

/// The format the state holds.
pub fn format_of(state: State) -> Option<StreamFormat> {
    match state {
        State::Claimed { format, .. }
        | State::Streaming { format, .. }
        | State::Paused { format, .. } => Some(format),
        State::Unclaimed | State::HandedBack => None,
    }
}

/// Microseconds of `frames` at `rate` (rounded down), the output's position from the frames played on the bus.
pub fn frames_to_us(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        return 0;
    }
    (u128::from(frames) * 1_000_000 / u128::from(rate)) as u64
}

#[cfg(test)]
mod tests {
    use super::Action::*;
    use super::*;

    const F44: StreamFormat = StreamFormat {
        rate: 44_100,
        channels: 2,
        alt: 6,
        subslot_bytes: 2,
        bits: 16,
    };
    const F96: StreamFormat = StreamFormat {
        rate: 96_000,
        channels: 2,
        alt: 4,
        subslot_bytes: 3,
        bits: 24,
    };

    /// Run `events` from Unclaimed, each at its time; the states and actions in order.
    fn run(events: &[(u64, Event)]) -> (State, Vec<Action>) {
        let mut state = State::Unclaimed;
        let mut all = Vec::new();
        for &(now, event) in events {
            let (next, actions) = step(state, event, now);
            state = next;
            all.extend(actions);
        }
        (state, all)
    }

    #[test]
    fn attach_claims_and_configures_play_starts_and_pause_stops_but_keeps_the_claim() {
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (5, Event::Play),
            (900, Event::Pause),
        ]);
        assert_eq!(
            actions,
            vec![Claim, Configure(F44), StartStreaming, StopStreaming]
        );
        assert_eq!(
            state,
            State::Paused {
                format: F44,
                since_ms: 900
            }
        );
    }

    #[test]
    fn a_resume_within_ten_seconds_restarts_without_claiming_again() {
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (0, Event::Play),
            (1_000, Event::Pause),
            (10_999, Event::Tick),
            (10_999, Event::Play),
        ]);
        assert_eq!(
            actions,
            vec![
                Claim,
                Configure(F44),
                StartStreaming,
                StopStreaming,
                Configure(F44),
                StartStreaming
            ]
        );
        assert!(matches!(
            state,
            State::Streaming {
                orphan_since_ms: None,
                ..
            }
        ));
    }

    #[test]
    fn ten_seconds_paused_or_claimed_hand_the_device_back() {
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (0, Event::Play),
            (1_000, Event::Pause),
            (11_000, Event::Tick),
        ]);
        assert_eq!(actions.last(), Some(&HandBack));
        assert_eq!(state, State::HandedBack);
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (9_999, Event::Tick),
            (10_000, Event::Tick),
        ]);
        assert_eq!(actions, vec![Claim, Configure(F44), HandBack]);
        assert_eq!(state, State::HandedBack);
        // Handed back is the end of this claim: nothing happens any more.
        assert_eq!(
            step(State::HandedBack, Event::Attach(F44), 0),
            (State::HandedBack, vec![])
        );
    }

    #[test]
    fn a_released_output_keeps_the_stream_running_for_the_next_one_of_the_same_format() {
        // A seek: Media3 releases the output and opens a new one with the same format.
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (0, Event::Play),
            (5_000, Event::Detach),
            (5_010, Event::Attach(F44)),
            (5_011, Event::Play),
        ]);
        assert_eq!(actions, vec![Claim, Configure(F44), StartStreaming]);
        assert_eq!(
            state,
            State::Streaming {
                format: F44,
                orphan_since_ms: None
            }
        );
    }

    #[test]
    fn an_orphaned_stream_stops_after_the_grace_and_then_hands_back_after_ten_seconds() {
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (0, Event::Play),
            (100, Event::Detach),
            (1_099, Event::Tick),
            (1_100, Event::Tick),
        ]);
        assert_eq!(
            actions,
            vec![Claim, Configure(F44), StartStreaming, StopStreaming]
        );
        assert_eq!(
            state,
            State::Paused {
                format: F44,
                since_ms: 1_100
            }
        );
        let (state, actions) = step(state, Event::Tick, 11_100);
        assert_eq!((state, actions), (State::HandedBack, vec![HandBack]));
    }

    #[test]
    fn a_format_change_stops_reconfigures_and_waits_for_play() {
        let (state, actions) = run(&[
            (0, Event::Attach(F44)),
            (0, Event::Play),
            (100, Event::Detach),
            (120, Event::Attach(F96)),
        ]);
        assert_eq!(
            actions,
            vec![
                Claim,
                Configure(F44),
                StartStreaming,
                StopStreaming,
                Configure(F96)
            ]
        );
        assert_eq!(
            state,
            State::Claimed {
                format: F96,
                since_ms: 120
            }
        );
        let (state, actions) = step(state, Event::Play, 130);
        assert_eq!(actions, vec![StartStreaming]);
        assert_eq!(format_of(state), Some(F96));
        // Claimed with another format before any play: only the alt and the clock change.
        let (_, actions) = run(&[
            (0, Event::Attach(F44)),
            (1, Event::Attach(F96)),
            (2, Event::Attach(F96)),
        ]);
        assert_eq!(actions, vec![Claim, Configure(F44), Configure(F96)]);
        // Paused: the new format is configured when it plays.
        let (_, actions) = run(&[
            (0, Event::Attach(F44)),
            (0, Event::Play),
            (1, Event::Pause),
            (2, Event::Attach(F96)),
            (3, Event::Play),
        ]);
        assert_eq!(
            actions[3..],
            [StopStreaming, Configure(F96), StartStreaming]
        );
    }

    #[test]
    fn release_and_device_errors_always_hand_back() {
        for (prefix, expect) in [
            (
                vec![(0, Event::Attach(F44)), (0, Event::Play)],
                vec![StopStreaming, HandBack],
            ),
            (vec![(0, Event::Attach(F44))], vec![HandBack]),
            (
                vec![(0, Event::Attach(F44)), (0, Event::Play), (1, Event::Pause)],
                vec![HandBack],
            ),
        ] {
            let (state, _) = run(&prefix);
            assert_eq!(step(state, Event::Release, 5), (State::HandedBack, expect));
            assert_eq!(
                step(state, Event::DeviceError, 5),
                (State::HandedBack, vec![HandBack])
            );
        }
        assert_eq!(
            step(State::Unclaimed, Event::Release, 0),
            (State::HandedBack, vec![])
        );
    }

    #[test]
    fn events_that_do_not_apply_change_nothing() {
        assert_eq!(
            step(State::Unclaimed, Event::Play, 0),
            (State::Unclaimed, vec![])
        );
        let paused = State::Paused {
            format: F44,
            since_ms: 0,
        };
        assert_eq!(step(paused, Event::Pause, 1), (paused, vec![]));
        assert_eq!(step(paused, Event::Detach, 1), (paused, vec![]));
        let streaming = State::Streaming {
            format: F44,
            orphan_since_ms: None,
        };
        assert_eq!(step(streaming, Event::Play, 1), (streaming, vec![]));
        assert_eq!(step(streaming, Event::Tick, 99_999), (streaming, vec![]));
        assert_eq!(state_name(streaming), "streaming");
    }

    #[test]
    fn the_position_is_the_frames_played_in_microseconds() {
        assert_eq!(frames_to_us(44_100, 44_100), 1_000_000);
        assert_eq!(frames_to_us(1, 44_100), 22);
        assert_eq!(frames_to_us(441, 44_100), 10_000);
        assert_eq!(
            frames_to_us(u64::from(u32::MAX) * 10, 384_000),
            111_848_106_640
        );
        assert_eq!(frames_to_us(5, 0), 0);
    }
}
