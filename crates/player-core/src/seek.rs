//! Seeking near the end of a track (ADR 0036): the policy both platforms follow.
//!
//! Every seek is clamped to `0..=duration`. A seek to the end (or past it) is the track ending the way it would on
//! its own, so the natural-end walk over the queue runs; it is never an error. Decoders cannot always seek into the
//! last frames of a file (Symphonia's FLAC demuxer, for one, reports the end of the stream when the target is its
//! last sample): such a failure close to the end also ends the track. A failure anywhere else stays an error.

use std::time::Duration;

/// How close to the end of a track a failed seek still counts as reaching the end. Wider than the longest FLAC
/// frame (65,535 samples at 44.1 kHz is about 1.5 s), so a decoder that cannot land inside the final frame is
/// covered.
pub const SEEK_TAIL_WINDOW: Duration = Duration::from_millis(2_000);

/// What a seek request does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeekPlan {
    /// Move to this position (already clamped to the duration).
    To(Duration),
    /// The target is at or after the end: the track ends as if it had played out.
    EndOfTrack,
}

/// A known, non-zero duration (zero means "not known yet" on both platforms).
fn known(duration: Option<Duration>) -> Option<Duration> {
    duration.filter(|duration| !duration.is_zero())
}

/// Clamp a seek target to `0..=duration` (an unknown duration leaves it as it is).
pub fn clamp_seek(position: Duration, duration: Option<Duration>) -> Duration {
    known(duration).map_or(position, |duration| position.min(duration))
}

/// Millisecond form of [`clamp_seek`], for the IPC commands.
pub fn clamp_seek_ms(position_ms: u64, duration_ms: Option<u64>) -> u64 {
    duration_ms
        .filter(|duration| *duration > 0)
        .map_or(position_ms, |duration| position_ms.min(duration))
}

/// The end as the front end sees it: snapshots report the duration in whole milliseconds, rounded down, while a
/// decoder's duration is usually a fraction of a millisecond longer. A seek to the reported duration is the end.
fn reported_end(duration: Duration) -> Duration {
    Duration::from_millis(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

/// Decide what a seek to `position` does for a track of `duration`. A target at or after the duration in whole
/// milliseconds ends the track.
pub fn plan_seek(position: Duration, duration: Option<Duration>) -> SeekPlan {
    match known(duration) {
        Some(duration) if position >= reported_end(duration) => SeekPlan::EndOfTrack,
        _ => SeekPlan::To(clamp_seek(position, duration)),
    }
}

/// Millisecond form of [`plan_seek`].
pub fn plan_seek_ms(position_ms: u64, duration_ms: Option<u64>) -> SeekPlan {
    plan_seek(
        Duration::from_millis(position_ms),
        duration_ms.map(Duration::from_millis),
    )
}

/// Whether a seek to `target` that the decoder could not carry out is the end of the track rather than an error:
/// the target lies within [`SEEK_TAIL_WINDOW`] of a known end.
pub fn seek_failure_ends_track(target: Duration, duration: Option<Duration>) -> bool {
    known(duration).is_some_and(|duration| duration.saturating_sub(target) <= SEEK_TAIL_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn seeks_are_clamped_to_the_track() {
        assert_eq!(clamp_seek(ms(9_000), Some(ms(5_000))), ms(5_000));
        assert_eq!(clamp_seek(ms(1_000), Some(ms(5_000))), ms(1_000));
        assert_eq!(clamp_seek(ms(9_000), None), ms(9_000));
        assert_eq!(clamp_seek(ms(9_000), Some(Duration::ZERO)), ms(9_000));
        assert_eq!(clamp_seek_ms(247_300, Some(247_199)), 247_199);
        assert_eq!(clamp_seek_ms(1_000, Some(247_199)), 1_000);
        assert_eq!(clamp_seek_ms(1_000, Some(0)), 1_000);
        assert_eq!(clamp_seek_ms(u64::MAX, None), u64::MAX);
    }

    #[test]
    fn a_seek_to_or_past_the_end_ends_the_track() {
        let duration = Some(ms(247_199));
        assert_eq!(plan_seek(ms(247_199), duration), SeekPlan::EndOfTrack);
        assert_eq!(plan_seek(ms(300_000), duration), SeekPlan::EndOfTrack);
        assert_eq!(plan_seek_ms(247_199, Some(247_199)), SeekPlan::EndOfTrack);
        assert_eq!(plan_seek_ms(u64::MAX, Some(247_199)), SeekPlan::EndOfTrack);
    }

    #[test]
    fn a_seek_to_the_reported_duration_ends_a_track_a_fraction_of_a_millisecond_longer() {
        // A FLAC file of 276,333.4 ms is reported as 276,333 ms; seeking there is the end, 1 ms before is not.
        let duration = Some(Duration::from_micros(276_333_400));
        assert_eq!(plan_seek(ms(276_333), duration), SeekPlan::EndOfTrack);
        assert_eq!(plan_seek_ms(276_333, Some(276_333)), SeekPlan::EndOfTrack);
        assert_eq!(plan_seek(ms(276_332), duration), SeekPlan::To(ms(276_332)));
    }

    #[test]
    fn a_seek_before_the_end_moves() {
        let duration = Some(ms(247_199));
        assert_eq!(plan_seek(ms(247_198), duration), SeekPlan::To(ms(247_198)));
        assert_eq!(
            plan_seek(Duration::ZERO, duration),
            SeekPlan::To(Duration::ZERO)
        );
        assert_eq!(plan_seek_ms(1_000, Some(247_199)), SeekPlan::To(ms(1_000)));
    }

    #[test]
    fn without_a_known_duration_a_seek_always_moves() {
        assert_eq!(plan_seek(ms(9_000), None), SeekPlan::To(ms(9_000)));
        assert_eq!(plan_seek_ms(9_000, Some(0)), SeekPlan::To(ms(9_000)));
    }

    #[test]
    fn only_failures_in_the_tail_end_the_track() {
        let duration = Some(ms(247_199));
        assert!(seek_failure_ends_track(ms(247_199), duration));
        assert!(seek_failure_ends_track(ms(247_179), duration));
        assert!(seek_failure_ends_track(ms(247_199 - 2_000), duration));
        assert!(!seek_failure_ends_track(ms(247_199 - 2_001), duration));
        assert!(!seek_failure_ends_track(ms(10_000), duration));
        assert!(!seek_failure_ends_track(ms(247_199), None));
        assert!(!seek_failure_ends_track(
            Duration::ZERO,
            Some(Duration::ZERO)
        ));
    }
}
