use std::time::{SystemTime, UNIX_EPOCH};

use crate::TrackId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueRepeatMode {
    Off,
    One,
    All,
}

/// A stable, session-local traversal over Track IDs. `track_ids` retains source
/// order; `play_order` stores the active shuffled or natural traversal.
#[derive(Clone, Debug)]
pub struct PlaybackQueue {
    track_ids: Vec<TrackId>,
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
        if track_ids.is_empty() || selected_index >= track_ids.len() {
            return None;
        }
        let length = track_ids.len();
        Some(Self {
            track_ids,
            play_order: (0..length).collect(),
            cursor: selected_index,
            shuffle: false,
            repeat: QueueRepeatMode::Off,
            random_state: random_state.max(1),
        })
    }

    pub fn current(&self) -> TrackId {
        self.track_ids[self.play_order[self.cursor]]
    }

    pub fn len(&self) -> usize {
        self.track_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.track_ids.is_empty()
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
        if self.shuffle == enabled || self.track_ids.len() < 2 {
            self.shuffle = enabled;
            return;
        }

        let current_source_index = self.play_order[self.cursor];
        self.play_order = (0..self.track_ids.len()).collect();
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
        if self.repeat == QueueRepeatMode::One && !manual {
            return Some(self.current());
        }

        if self.cursor + 1 < self.play_order.len() {
            self.cursor += 1;
            return Some(self.current());
        }

        if self.repeat == QueueRepeatMode::All {
            self.cursor = 0;
            return Some(self.current());
        }

        None
    }

    pub fn previous(&mut self) -> Option<TrackId> {
        if self.cursor > 0 {
            self.cursor -= 1;
            return Some(self.current());
        }
        if self.repeat == QueueRepeatMode::All {
            self.cursor = self.play_order.len() - 1;
            return Some(self.current());
        }
        None
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
        let storage_bytes = queue.track_ids.capacity() * std::mem::size_of::<TrackId>()
            + queue.play_order.capacity() * std::mem::size_of::<usize>();
        assert_eq!(queue.len(), 100_000);
        assert!(storage_bytes <= 4 * 1024 * 1024, "{storage_bytes} bytes");
        queue.set_shuffle(true);
        let shuffled_bytes = queue.track_ids.capacity() * std::mem::size_of::<TrackId>()
            + queue.play_order.capacity() * std::mem::size_of::<usize>();
        assert_eq!(shuffled_bytes, storage_bytes);
    }
}
