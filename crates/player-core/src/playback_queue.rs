use std::time::{SystemTime, UNIX_EPOCH};

use crate::TrackId;
use serde::{Deserialize, Serialize};

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
    Library { query: Option<String> },
    Playlist { playlist_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaybackQueueEntry {
    pub track_id: TrackId,
    /// Original playlist entry position, retained to identify duplicate tracks.
    pub source_position: Option<u64>,
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
            PlaybackQueueContext::Library { query: None },
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
    fn one_hundred_thousand_entry_queue_stores_only_compact_ids_and_indices() {
        let tracks = ids(100_000);
        let mut queue = PlaybackQueue::new(tracks, 50_000).unwrap();
        let storage_bytes = queue.entries.capacity() * std::mem::size_of::<PlaybackQueueEntry>()
            + queue.play_order.capacity() * std::mem::size_of::<usize>();
        assert_eq!(queue.len(), 100_000);
        assert!(storage_bytes <= 4 * 1024 * 1024, "{storage_bytes} bytes");
        queue.set_shuffle(true);
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
