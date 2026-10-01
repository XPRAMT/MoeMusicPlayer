use crate::TrackId;

/// Cumulative listening data used by playback selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlaybackStatistics {
    pub played_ms: u64,
    /// Unknown or zero-duration tracks have no usable denominator.
    pub duration_ms: Option<u64>,
}

/// A runtime's monotonic per-track listening counter checkpoint.
///
/// `played_ms` is cumulative for one runtime and one TrackId. The database
/// adds only the positive difference from that runtime's previously committed
/// checkpoint, so retries are safe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackCheckpoint {
    pub track_id: TrackId,
    pub played_ms: u64,
    pub duration_ms: Option<u64>,
}
