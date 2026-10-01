use std::time::{SystemTime, UNIX_EPOCH};

use crate::{TrackFieldFilter, TrackId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueRepeatMode {
    Off,
    One,
    All,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaybackQueueContext {
    Library {
        query: Option<String>,
        #[serde(default)]
        field_filter: Option<TrackFieldFilter>,
    },
    Playlist {
        playlist_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaybackQueueEntry {
    pub track_id: TrackId,
    /// Original playlist entry position, retained to identify duplicate tracks.
    pub source_position: Option<u64>,
}

/// Listening totals used to calculate a track's relative shuffle weight.
/// Missing or zero duration is treated as neutral by `shuffle_weight`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueTrackListeningStats {
    pub played_ms: u64,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaybackQueueSnapshot {
    pub context: PlaybackQueueContext,
    pub entries: Vec<PlaybackQueueEntry>,
    pub play_order: Vec<usize>,
    pub cursor: usize,
    pub shuffle: bool,
    pub repeat: QueueRepeatMode,
    pub random_state: u64,
}

/// A stable, session-local traversal over entries. `entries` retain source
/// order; `play_order` stores the active shuffled or natural traversal.
#[derive(Clone, Debug)]
pub struct PlaybackQueue {
    context: PlaybackQueueContext,
    entries: Vec<PlaybackQueueEntry>,
    play_order: Vec<usize>,
    cursor: usize,
    shuffle: bool,
    repeat: QueueRepeatMode,
    random_state: u64,
}

impl PlaybackQueue {
    pub fn new(track_ids: Vec<TrackId>, selected_index: usize) -> Option<Self> {
        Self::with_seed(track_ids, selected_index, random_seed())
    }

    pub fn with_seed(
        track_ids: Vec<TrackId>,
        selected_index: usize,
        random_state: u64,
    ) -> Option<Self> {
        let entries = track_ids
            .into_iter()
            .map(|track_id| PlaybackQueueEntry {
                track_id,
                source_position: None,
            })
            .collect();
        Self::with_entries_and_seed(
            entries,
            PlaybackQueueContext::Library {
                query: None,
                field_filter: None,
            },
            selected_index,
            random_state,
        )
    }

    pub fn with_entries(
        entries: Vec<PlaybackQueueEntry>,
        context: PlaybackQueueContext,
        selected_index: usize,
    ) -> Option<Self> {
        Self::with_entries_and_seed(entries, context, selected_index, random_seed())
    }

    fn with_entries_and_seed(
        entries: Vec<PlaybackQueueEntry>,
        context: PlaybackQueueContext,
        selected_index: usize,
        random_state: u64,
    ) -> Option<Self> {
        if entries.is_empty() || selected_index >= entries.len() {
            return None;
        }
        let length = entries.len();
        Some(Self {
            context,
            entries,
            play_order: (0..length).collect(),
            cursor: selected_index,
            shuffle: false,
            repeat: QueueRepeatMode::Off,
            random_state: random_state.max(1),
        })
    }

    pub fn current(&self) -> TrackId {
        self.entries[self.play_order[self.cursor]].track_id
    }

    pub fn current_entry_index(&self) -> usize {
        self.play_order[self.cursor]
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat_mode(&self) -> QueueRepeatMode {
        self.repeat
    }

    pub fn can_next(&self) -> bool {
        self.cursor + 1 < self.play_order.len() || self.repeat == QueueRepeatMode::All
    }

    pub fn can_previous(&self) -> bool {
        self.cursor > 0 || self.repeat == QueueRepeatMode::All
    }

    pub fn set_repeat_mode(&mut self, repeat: QueueRepeatMode) {
        self.repeat = repeat;
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled || self.entries.len() < 2 {
            self.shuffle = enabled;
            return;
        }

        let current_source_index = self.play_order[self.cursor];
        self.play_order = (0..self.entries.len()).collect();
        if enabled {
            self.shuffle_from(current_source_index);
        } else {
            self.cursor = current_source_index;
        }
        self.shuffle = enabled;
    }

    /// Enable or disable shuffle, using the latest listening statistics when
    /// enabling it. Calling this with the current state does not reorder an
    /// existing traversal.
    pub fn set_shuffle_with_stats(
        &mut self,
        enabled: bool,
        stats_by_track: &HashMap<TrackId, QueueTrackListeningStats>,
    ) {
        if !enabled || self.shuffle == enabled || self.entries.len() < 2 {
            self.set_shuffle(enabled);
            return;
        }

        let current_source_index = self.play_order[self.cursor];
        self.play_order = weighted_order(
            &self.entries,
            current_source_index,
            stats_by_track,
            &mut self.random_state,
        );
        self.cursor = 0;
        self.shuffle = true;
    }

    /// Move to the next queue entry. Natural completion repeats the current
    /// entry in Repeat One; manual navigation always proceeds through the queue.
    pub fn next(&mut self, manual: bool) -> Option<TrackId> {
        let (_, target_cursor) = self.preview_next(manual)?;
        self.cursor = target_cursor;
        Some(self.current())
    }

    pub fn preview_next(&self, manual: bool) -> Option<(TrackId, usize)> {
        if self.repeat == QueueRepeatMode::One && !manual {
            return Some((self.current(), self.cursor));
        }

        if self.cursor + 1 < self.play_order.len() {
            let cursor = self.cursor + 1;
            return Some((self.entries[self.play_order[cursor]].track_id, cursor));
        }

        if self.repeat == QueueRepeatMode::All {
            return Some((self.entries[self.play_order[0]].track_id, 0));
        }

        None
    }

    pub fn previous(&mut self) -> Option<TrackId> {
        let (_, target_cursor) = self.preview_previous()?;
        self.cursor = target_cursor;
        Some(self.current())
    }

    pub fn preview_previous(&self) -> Option<(TrackId, usize)> {
        if self.cursor > 0 {
            let cursor = self.cursor - 1;
            return Some((self.entries[self.play_order[cursor]].track_id, cursor));
        }
        if self.repeat == QueueRepeatMode::All {
            let cursor = self.play_order.len() - 1;
            return Some((self.entries[self.play_order[cursor]].track_id, cursor));
        }
        None
    }

    pub fn snapshot(&self) -> PlaybackQueueSnapshot {
        PlaybackQueueSnapshot {
            context: self.context.clone(),
            entries: self.entries.clone(),
            play_order: self.play_order.clone(),
            cursor: self.cursor,
            shuffle: self.shuffle,
            repeat: self.repeat,
            random_state: self.random_state,
        }
    }

    pub fn restore(snapshot: PlaybackQueueSnapshot) -> Result<Self, &'static str> {
        let length = snapshot.entries.len();
        if length == 0 || snapshot.play_order.len() != length || snapshot.cursor >= length {
            return Err("queue size or cursor is invalid");
        }
        let mut seen = vec![false; length];
        for index in &snapshot.play_order {
            let Some(slot) = seen.get_mut(*index) else {
                return Err("queue traversal index is out of range");
            };
            if *slot {
                return Err("queue traversal index is duplicated");
            }
            *slot = true;
        }
        if seen.iter().any(|value| !value) || snapshot.random_state == 0 {
            return Err("queue snapshot is incomplete");
        }
        match &snapshot.context {
            PlaybackQueueContext::Library { .. }
                if snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.source_position.is_some()) =>
            {
                return Err("library queue entries cannot have playlist positions");
            }
            PlaybackQueueContext::Playlist { .. } => {
                let mut previous = None;
                for entry in &snapshot.entries {
                    let Some(position) = entry.source_position else {
                        return Err("playlist queue entry has no source position");
                    };
                    if previous.is_some_and(|previous| position <= previous) {
                        return Err("playlist queue positions are not strictly increasing");
                    }
                    previous = Some(position);
                }
            }
            _ => {}
        }
        Ok(Self {
            context: snapshot.context,
            entries: snapshot.entries,
            play_order: snapshot.play_order,
            cursor: snapshot.cursor,
            shuffle: snapshot.shuffle,
            repeat: snapshot.repeat,
            random_state: snapshot.random_state,
        })
    }

    pub fn set_cursor(&mut self, cursor: usize) -> Result<(), &'static str> {
        if cursor >= self.play_order.len() {
            return Err("queue cursor is out of range");
        }
        self.cursor = cursor;
        Ok(())
    }

    fn shuffle_from(&mut self, current_source_index: usize) {
        self.play_order.swap(0, current_source_index);
        for upper in (2..self.play_order.len()).rev() {
            let swap_index = 1 + (self.next_random() as usize % upper);
            self.play_order.swap(upper, swap_index);
        }
        self.cursor = 0;
    }

    fn next_random(&mut self) -> u64 {
        // xorshift64* is sufficient for local song ordering and avoids another
        // dependency on the playback hot path.
        let mut value = self.random_state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.random_state = value;
        value.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn weighted_order(
    entries: &[PlaybackQueueEntry],
    current_source_index: usize,
    stats_by_track: &HashMap<TrackId, QueueTrackListeningStats>,
    random_state: &mut u64,
) -> Vec<usize> {
    let mut order = Vec::with_capacity(entries.len());
    order.push(current_source_index);
    let mut random = QueueRandom(random_state);
    let mut keyed_slots = Vec::with_capacity(entries.len().saturating_sub(1));
    for (source_index, entry) in entries.iter().enumerate() {
        if source_index == current_source_index {
            continue;
        }
        let weight = stats_by_track
            .get(&entry.track_id)
            .copied()
            .unwrap_or_default()
            .shuffle_weight();
        let exponential_key = -random.next_unit().ln() / weight;
        keyed_slots.push((exponential_key, source_index));
    }
    keyed_slots.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
    });
    order.extend(
        keyed_slots
            .into_iter()
            .map(|(_, source_index)| source_index),
    );
    order
}

impl QueueTrackListeningStats {
    fn shuffle_weight(self) -> f64 {
        let Some(duration_ms) = self.duration_ms.filter(|duration_ms| *duration_ms > 0) else {
            return 1.0;
        };
        let played_ms = self.played_ms as f64;
        let duration_ms = duration_ms as f64;
        duration_ms / (duration_ms + played_ms)
    }
}

struct QueueRandom<'a>(&'a mut u64);

impl QueueRandom<'_> {
    fn next_random(&mut self) -> u64 {
        let mut value = *self.0;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        *self.0 = value;
        value.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_unit(&mut self) -> f64 {
        const UNIT_RANGE: f64 = (1_u64 << 53) as f64;
        ((self.next_random() >> 11) as f64 + 1.0) / (UNIT_RANGE + 2.0)
    }
}

fn random_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(count: usize) -> Vec<TrackId> {
        (0..count).map(|_| TrackId::new()).collect()
    }

    #[test]
    fn queue_validates_selected_index_and_keeps_source_order() {
        assert!(PlaybackQueue::new(Vec::new(), 0).is_none());
        let tracks = ids(3);
        assert!(PlaybackQueue::new(tracks.clone(), 3).is_none());
        let mut queue = PlaybackQueue::new(tracks.clone(), 1).unwrap();
        assert_eq!(queue.current(), tracks[1]);
        assert_eq!(queue.next(true), Some(tracks[2]));
        assert_eq!(queue.next(true), None);
        assert_eq!(queue.previous(), Some(tracks[1]));
        assert_eq!(queue.previous(), Some(tracks[0]));
        assert_eq!(queue.previous(), None);
    }

    #[test]
    fn repeat_modes_distinguish_natural_end_from_manual_next() {
        let tracks = ids(2);
        let mut queue = PlaybackQueue::new(tracks.clone(), 1).unwrap();
        queue.set_repeat_mode(QueueRepeatMode::One);
        assert_eq!(queue.next(false), Some(tracks[1]));
        assert_eq!(queue.next(true), None);
        queue.set_repeat_mode(QueueRepeatMode::All);
        assert_eq!(queue.next(true), Some(tracks[0]));
        assert_eq!(queue.previous(), Some(tracks[1]));
        assert_eq!(queue.previous(), Some(tracks[0]));
    }

    #[test]
    fn shuffle_visits_every_entry_and_previous_follows_the_shuffled_history() {
        let tracks = ids(12);
        let mut queue = PlaybackQueue::with_seed(tracks.clone(), 7, 12_345).unwrap();
        queue.set_shuffle(true);
        assert_eq!(queue.current(), tracks[7]);
        let mut visited = vec![queue.current()];
        while let Some(next) = queue.next(true) {
            visited.push(next);
        }
        assert_eq!(visited.len(), tracks.len());
        let unique = visited
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), tracks.len());
        for expected in visited.iter().rev().skip(1) {
            assert_eq!(queue.previous(), Some(*expected));
        }
        assert_eq!(queue.previous(), None);
    }

    #[test]
    fn disabling_shuffle_restores_source_order_at_the_current_track() {
        let tracks = ids(10);
        let mut queue = PlaybackQueue::with_seed(tracks.clone(), 5, 88).unwrap();
        queue.set_shuffle(true);
        assert_eq!(queue.current(), tracks[5]);
        assert_ne!(queue.next(true), Some(tracks[6]));
        queue.previous();
        queue.set_shuffle(false);
        assert_eq!(queue.current(), tracks[5]);
        assert_eq!(queue.next(true), Some(tracks[6]));
        assert_eq!(queue.previous(), Some(tracks[5]));
    }

    #[test]
    fn weighted_shuffle_is_seed_reproducible_and_favors_lower_play_ratio() {
        let tracks = ids(3);
        let stats = HashMap::from([
            (
                tracks[1],
                QueueTrackListeningStats {
                    played_ms: 0,
                    duration_ms: Some(100),
                },
            ),
            (
                tracks[2],
                QueueTrackListeningStats {
                    played_ms: 400,
                    duration_ms: Some(100),
                },
            ),
        ]);
        let mut favorable_first = 0;
        for seed in 1..=2_000 {
            let mut queue_a = PlaybackQueue::with_seed(tracks.clone(), 0, seed).unwrap();
            queue_a.set_shuffle_with_stats(true, &stats);
            let snapshot = queue_a.snapshot();

            let mut queue_b = PlaybackQueue::with_seed(tracks.clone(), 0, seed).unwrap();
            queue_b.set_shuffle_with_stats(true, &stats);
            assert_eq!(queue_b.snapshot(), snapshot);
            assert_eq!(queue_a.current(), tracks[0]);
            if queue_a.next(true) == Some(tracks[1]) {
                favorable_first += 1;
            }
        }

        let observed = favorable_first as f64 / 2_000.0;
        assert!(
            (0.79..0.88).contains(&observed),
            "lower-ratio track was first in {observed:.3} of trials"
        );
    }

    #[test]
    fn weighted_shuffle_keeps_duplicate_entries_as_distinct_slots() {
        let duplicate = TrackId::new();
        let entries = vec![
            PlaybackQueueEntry {
                track_id: TrackId::new(),
                source_position: Some(0),
            },
            PlaybackQueueEntry {
                track_id: duplicate,
                source_position: Some(1),
            },
            PlaybackQueueEntry {
                track_id: duplicate,
                source_position: Some(2),
            },
            PlaybackQueueEntry {
                track_id: TrackId::new(),
                source_position: Some(3),
            },
        ];
        let stats = HashMap::from([(
            duplicate,
            QueueTrackListeningStats {
                played_ms: 100,
                duration_ms: Some(100),
            },
        )]);
        let mut queue = PlaybackQueue::with_entries_and_seed(
            entries,
            PlaybackQueueContext::Playlist {
                playlist_id: "playlist-id".into(),
            },
            0,
            0x1234,
        )
        .unwrap();
        queue.set_shuffle_with_stats(true, &stats);
        assert_eq!(queue.play_order[0], 0);
        let mut visited = queue.play_order.clone();
        visited.sort_unstable();
        assert_eq!(visited, vec![0, 1, 2, 3]);
        assert!(queue.play_order.contains(&1));
        assert!(queue.play_order.contains(&2));
        assert_eq!(queue.entries[1].track_id, queue.entries[2].track_id);
    }

    #[test]
    fn weighted_shuffle_extreme_weights_remain_finite_and_preserve_every_slot() {
        let extreme = TrackId::new();
        let neutral = TrackId::new();
        let entries = (0..128)
            .map(|position| PlaybackQueueEntry {
                track_id: if position % 3 == 0 { extreme } else { neutral },
                source_position: Some(position),
            })
            .collect::<Vec<_>>();
        let stats = HashMap::from([
            (
                extreme,
                QueueTrackListeningStats {
                    played_ms: u64::MAX,
                    duration_ms: Some(1),
                },
            ),
            (
                neutral,
                QueueTrackListeningStats {
                    played_ms: 0,
                    duration_ms: Some(1),
                },
            ),
        ]);
        let extreme_weight = stats[&extreme].shuffle_weight();
        assert!(extreme_weight.is_finite() && extreme_weight > 0.0);

        for seed in 1..=64 {
            let mut queue = PlaybackQueue::with_entries_and_seed(
                entries.clone(),
                PlaybackQueueContext::Playlist {
                    playlist_id: "extreme-ratios".into(),
                },
                37,
                seed,
            )
            .unwrap();
            queue.set_shuffle_with_stats(true, &stats);
            assert_eq!(queue.play_order[0], 37);
            let mut visited = queue.play_order.clone();
            visited.sort_unstable();
            assert_eq!(visited, (0..entries.len()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn queue_random_unit_values_are_strictly_between_zero_and_one() {
        let mut state = 0x5eed;
        let mut random = QueueRandom(&mut state);
        for _ in 0..10_000 {
            let unit = random.next_unit();
            assert!(unit > 0.0 && unit < 1.0, "unit sample was {unit}");
        }
    }

    #[test]
    fn missing_or_zero_duration_has_neutral_weight() {
        assert_eq!(
            QueueTrackListeningStats {
                played_ms: 50_000,
                duration_ms: None,
            }
            .shuffle_weight(),
            1.0
        );
        assert_eq!(
            QueueTrackListeningStats {
                played_ms: 50_000,
                duration_ms: Some(0),
            }
            .shuffle_weight(),
            1.0
        );
        assert_eq!(
            QueueTrackListeningStats {
                played_ms: 400,
                duration_ms: Some(100),
            }
            .shuffle_weight(),
            0.2
        );
    }

    #[test]
    fn one_hundred_thousand_entry_queue_stores_only_compact_ids_and_indices() {
        let tracks = ids(100_000);
        let stats = tracks
            .iter()
            .copied()
            .enumerate()
            .map(|(index, track_id)| {
                (
                    track_id,
                    QueueTrackListeningStats {
                        played_ms: if index % 2 == 0 { u64::MAX } else { 0 },
                        duration_ms: Some(if index % 2 == 0 { 1 } else { 300_000 }),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let mut queue = PlaybackQueue::new(tracks, 50_000).unwrap();
        let storage_bytes = queue.entries.capacity() * std::mem::size_of::<PlaybackQueueEntry>()
            + queue.play_order.capacity() * std::mem::size_of::<usize>();
        assert_eq!(queue.len(), 100_000);
        assert!(storage_bytes <= 4 * 1024 * 1024, "{storage_bytes} bytes");
        let started = std::time::Instant::now();
        queue.set_shuffle_with_stats(true, &stats);
        let elapsed = started.elapsed();
        eprintln!("100,000-entry weighted shuffle: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_secs(5));
        assert_eq!(queue.play_order.first(), Some(&50_000));
        let mut visited = queue.play_order.clone();
        visited.sort_unstable();
        assert_eq!(visited, (0..100_000).collect::<Vec<_>>());
        let shuffled_bytes = queue.entries.capacity() * std::mem::size_of::<PlaybackQueueEntry>()
            + queue.play_order.capacity() * std::mem::size_of::<usize>();
        assert_eq!(shuffled_bytes, storage_bytes);
    }

    #[test]
    fn snapshot_restores_duplicate_playlist_entries_and_exact_shuffle_cursor() {
        let duplicate = TrackId::new();
        let entries = vec![
            PlaybackQueueEntry {
                track_id: TrackId::new(),
                source_position: Some(0),
            },
            PlaybackQueueEntry {
                track_id: duplicate,
                source_position: Some(1),
            },
            PlaybackQueueEntry {
                track_id: duplicate,
                source_position: Some(2),
            },
            PlaybackQueueEntry {
                track_id: TrackId::new(),
                source_position: Some(3),
            },
        ];
        let context = PlaybackQueueContext::Playlist {
            playlist_id: "playlist-id".into(),
        };
        let mut queue = PlaybackQueue::with_entries_and_seed(entries, context, 1, 0x1234).unwrap();
        queue.set_shuffle(true);
        queue.next(true);
        let before = queue.snapshot();
        let restored = PlaybackQueue::restore(before.clone()).unwrap();
        assert_eq!(restored.snapshot(), before);
        assert_eq!(restored.entries[1].track_id, restored.entries[2].track_id);
        assert_ne!(
            restored.entries[1].source_position,
            restored.entries[2].source_position
        );
        assert_eq!(
            restored.current(),
            before.entries[before.play_order[before.cursor]].track_id
        );
    }

    #[test]
    fn restore_rejects_out_of_range_cursor_and_invalid_traversal() {
        let queue = PlaybackQueue::new(ids(3), 0).unwrap();
        let mut snapshot = queue.snapshot();
        snapshot.cursor = 3;
        assert!(PlaybackQueue::restore(snapshot).is_err());

        let mut snapshot = queue.snapshot();
        snapshot.play_order[1] = 0;
        assert!(PlaybackQueue::restore(snapshot).is_err());
    }

    #[test]
    fn one_hundred_thousand_entry_snapshot_restores_exact_order() {
        let mut queue = PlaybackQueue::new(ids(100_000), 51_234).unwrap();
        queue.set_shuffle(true);
        for _ in 0..321 {
            queue.next(true);
        }
        let snapshot = queue.snapshot();
        let restored = PlaybackQueue::restore(snapshot.clone()).unwrap();
        assert_eq!(restored.snapshot(), snapshot);
    }
}
