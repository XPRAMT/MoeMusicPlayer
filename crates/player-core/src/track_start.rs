//! Starting another track (ADR 0036): the policy both platforms follow.
//!
//! A different track starts only after it has been opened and checked ("prepare, then commit"); until then the music
//! that is playing keeps playing. A *manual* start (a tap, next, previous, any choice of the user) makes exactly one
//! attempt: when it fails nothing changes and the user is told. A *natural* start (the playing track ended) walks the
//! queue from the next entry, trying each entry once and going on after a failure; a server that could not be reached
//! is skipped for the rest of the walk. The platform code only does the opening; every decision is made here.

use std::collections::{HashSet, VecDeque};

use crate::{is_remote_unreachable, QueueRepeatMode, LOAD_SUPERSEDED, REMOTE_LINK_LOST};

/// The message of a track that could not be started for any reason but an unreachable server (the file is missing,
/// the format is not supported, the decoder or the open failed). The front end shows its notice for exactly this
/// text, and for `REMOTE_LINK_LOST` when the server could not be reached.
pub const TRACK_UNPLAYABLE: &str = "無法播放這首歌";

/// Why a track could not be started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartFailure {
    /// The server of a remote track could not be reached.
    Unreachable,
    /// Anything else.
    Unplayable,
}

impl StartFailure {
    /// Classify the message of a failed start. `None` for a start that was replaced by a newer choice (nothing is
    /// reported for it).
    pub fn from_message(message: &str) -> Option<Self> {
        if message.contains(LOAD_SUPERSEDED) || message == "superseded" {
            None
        } else if is_remote_unreachable(message) {
            Some(Self::Unreachable)
        } else {
            Some(Self::Unplayable)
        }
    }

    /// The stable text the front end recognises.
    pub fn message(self) -> &'static str {
        match self {
            Self::Unreachable => REMOTE_LINK_LOST,
            Self::Unplayable => TRACK_UNPLAYABLE,
        }
    }

    /// The name sent to the front end.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::Unplayable => "unplayable",
        }
    }

    /// The more relevant of two failures: an unreachable server wins.
    pub fn worst(self, other: Self) -> Self {
        if self == Self::Unreachable || other == Self::Unreachable {
            Self::Unreachable
        } else {
            Self::Unplayable
        }
    }
}

/// Whether an error message says a track could not be played (see [`TRACK_UNPLAYABLE`]).
pub fn is_track_unplayable(message: &str) -> bool {
    message.contains(TRACK_UNPLAYABLE)
}

/// The queue positions (cursors, in traversal order) a natural start tries, in order: repeat-one retries only the
/// track that ended; repeat-all goes round the whole queue once and ends with the track that ended; repeat-off goes to
/// the end of the queue.
pub fn natural_walk_cursors(len: usize, cursor: usize, repeat: QueueRepeatMode) -> Vec<usize> {
    if len == 0 || cursor >= len {
        return Vec::new();
    }
    match repeat {
        QueueRepeatMode::One => vec![cursor],
        QueueRepeatMode::Off => (cursor + 1..len).collect(),
        QueueRepeatMode::All => (cursor + 1..len).chain(0..=cursor).collect(),
    }
}

/// A natural start in progress: which entries are left to try, which servers are known to be down.
#[derive(Debug)]
pub struct NaturalWalk {
    remaining: VecDeque<usize>,
    down: HashSet<String>,
    worst: Option<StartFailure>,
}

impl NaturalWalk {
    pub fn new(len: usize, cursor: usize, repeat: QueueRepeatMode) -> Self {
        Self {
            remaining: natural_walk_cursors(len, cursor, repeat).into(),
            down: HashSet::new(),
            worst: None,
        }
    }

    /// The next queue position to try. `server_of` names the server of an entry (`None` for a local file). An entry of
    /// a server that was found down is passed over without being tried.
    pub fn next_candidate(
        &mut self,
        mut server_of: impl FnMut(usize) -> Option<String>,
    ) -> Option<usize> {
        while let Some(cursor) = self.remaining.pop_front() {
            if server_of(cursor).is_some_and(|server| self.down.contains(&server)) {
                self.note(StartFailure::Unreachable);
                continue;
            }
            return Some(cursor);
        }
        None
    }

    /// The entry just tried could not be started. `server` is the server of that entry, if it is a remote one.
    pub fn failed(&mut self, failure: StartFailure, server: Option<String>) {
        if failure == StartFailure::Unreachable {
            if let Some(server) = server {
                self.down.insert(server);
            }
        }
        self.note(failure);
    }

    /// What to tell the user when no entry could be started: the unreachable server if there was one, else the last
    /// kind of failure. `None` when nothing failed (the walk had nothing to try).
    pub fn final_failure(&self) -> Option<StartFailure> {
        self.worst
    }

    fn note(&mut self, failure: StartFailure) {
        self.worst = Some(self.worst.map_or(failure, |worst| worst.worst(failure)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_are_classified_by_a_stable_text() {
        assert_eq!(
            StartFailure::from_message(REMOTE_LINK_LOST),
            Some(StartFailure::Unreachable)
        );
        assert_eq!(
            StartFailure::from_message("無法開啟遠端檔案：無法連線到伺服器：timed out"),
            Some(StartFailure::Unreachable)
        );
        assert_eq!(
            StartFailure::from_message(TRACK_UNPLAYABLE),
            Some(StartFailure::Unplayable)
        );
        assert_eq!(
            StartFailure::from_message("no such file"),
            Some(StartFailure::Unplayable)
        );
        assert_eq!(StartFailure::from_message(LOAD_SUPERSEDED), None);
        assert_eq!(StartFailure::from_message("superseded"), None);
        assert!(is_track_unplayable(StartFailure::Unplayable.message()));
        assert!(!is_track_unplayable(StartFailure::Unreachable.message()));
        assert_eq!(
            StartFailure::Unplayable.worst(StartFailure::Unreachable),
            StartFailure::Unreachable
        );
        assert_eq!(
            StartFailure::Unplayable.worst(StartFailure::Unplayable),
            StartFailure::Unplayable
        );
    }

    #[test]
    fn a_natural_walk_follows_the_queue_and_the_repeat_mode() {
        // Off: the rest of the queue, nothing after the last entry.
        assert_eq!(
            natural_walk_cursors(5, 1, QueueRepeatMode::Off),
            vec![2, 3, 4]
        );
        assert!(natural_walk_cursors(5, 4, QueueRepeatMode::Off).is_empty());
        // One: only the track that ended.
        assert_eq!(natural_walk_cursors(5, 2, QueueRepeatMode::One), vec![2]);
        // All: once round the queue, every entry at most once, the track that ended last.
        assert_eq!(
            natural_walk_cursors(5, 3, QueueRepeatMode::All),
            vec![4, 0, 1, 2, 3]
        );
        assert_eq!(natural_walk_cursors(1, 0, QueueRepeatMode::All), vec![0]);
        assert!(natural_walk_cursors(0, 0, QueueRepeatMode::All).is_empty());
        assert!(natural_walk_cursors(3, 7, QueueRepeatMode::All).is_empty());
    }

    #[test]
    fn a_walk_goes_on_after_a_failure_and_stops_at_the_first_entry_that_loads() {
        let mut walk = NaturalWalk::new(5, 0, QueueRepeatMode::Off);
        let local = |_: usize| None;
        assert_eq!(walk.next_candidate(local), Some(1));
        walk.failed(StartFailure::Unplayable, None);
        assert_eq!(walk.next_candidate(local), Some(2));
        // The caller starts entry 2 successfully and drops the walk; nothing else is tried.
        assert_eq!(walk.final_failure(), Some(StartFailure::Unplayable));
    }

    #[test]
    fn an_unreachable_server_is_skipped_for_the_rest_of_the_walk_without_waiting() {
        // Entries 1, 2 and 4 are on server "a", entry 3 is local, entry 5 is on server "b".
        let server = |cursor: usize| match cursor {
            1 | 2 | 4 => Some("a".to_owned()),
            5 => Some("b".to_owned()),
            _ => None,
        };
        let mut walk = NaturalWalk::new(6, 0, QueueRepeatMode::Off);
        assert_eq!(walk.next_candidate(server), Some(1));
        walk.failed(StartFailure::Unreachable, server(1));
        // 2 is on the dead server: passed over; 3 is local and is tried.
        assert_eq!(walk.next_candidate(server), Some(3));
        walk.failed(StartFailure::Unplayable, None);
        // 4 is passed over too; 5 is on another server and is tried.
        assert_eq!(walk.next_candidate(server), Some(5));
        walk.failed(StartFailure::Unplayable, server(5));
        assert_eq!(walk.next_candidate(server), None);
        // All failed: the unreachable server is what the user is told.
        assert_eq!(walk.final_failure(), Some(StartFailure::Unreachable));
    }

    /// A provider where `cached` paths are whole in the disk cache; every path is on server "a".
    struct Cached(&'static [&'static str]);

    impl crate::RemoteContent for Cached {
        fn open(&self, _: crate::SourceId, _: &str) -> std::io::Result<crate::RemoteFileHandle> {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        }

        fn server_key(&self, _: crate::SourceId) -> String {
            "a".to_owned()
        }

        fn available_offline(&self, _: crate::SourceId, path: &str) -> bool {
            self.0.contains(&path)
        }
    }

    #[test]
    fn a_song_in_the_disk_cache_is_not_passed_over_when_its_server_is_down() {
        let source = crate::SourceId::new();
        let content = Cached(&["cached1", "cached2"]);
        // Entries 1..=5 of one dead server: 1 and 3 are not cached, 2 and 4 are, 5 is not.
        let paths = [
            "",
            "uncached1",
            "cached1",
            "uncached2",
            "cached2",
            "uncached3",
        ];
        let server = |cursor: usize| crate::walk_server_key(&content, source, paths[cursor]);
        let mut walk = NaturalWalk::new(6, 0, QueueRepeatMode::Off);
        assert_eq!(walk.next_candidate(server), Some(1));
        walk.failed(StartFailure::Unreachable, server(1));
        // The cached entry is tried (and would play); the uncached one after it is passed over.
        assert_eq!(walk.next_candidate(server), Some(2));
        walk.failed(StartFailure::Unplayable, server(2));
        assert_eq!(walk.next_candidate(server), Some(4));
        walk.failed(StartFailure::Unplayable, server(4));
        assert_eq!(walk.next_candidate(server), None);
        assert_eq!(walk.final_failure(), Some(StartFailure::Unreachable));
        assert_eq!(
            crate::walk_server_key(&content, source, "x"),
            Some("a".to_owned())
        );
        assert_eq!(crate::walk_server_key(&content, source, "cached1"), None);
    }

    #[test]
    fn a_failure_that_is_not_the_network_does_not_mark_the_server_down() {
        let server = |_: usize| Some("a".to_owned());
        let mut walk = NaturalWalk::new(4, 0, QueueRepeatMode::Off);
        assert_eq!(walk.next_candidate(server), Some(1));
        walk.failed(StartFailure::Unplayable, server(1));
        assert_eq!(walk.next_candidate(server), Some(2));
    }

    #[test]
    fn repeat_one_retries_only_the_same_track_and_repeat_all_tries_each_entry_once() {
        let local = |_: usize| None;
        let mut one = NaturalWalk::new(4, 2, QueueRepeatMode::One);
        assert_eq!(one.next_candidate(local), Some(2));
        one.failed(StartFailure::Unplayable, None);
        assert_eq!(one.next_candidate(local), None, "no walking in repeat-one");
        assert_eq!(one.final_failure(), Some(StartFailure::Unplayable));

        let mut all = NaturalWalk::new(3, 1, QueueRepeatMode::All);
        let mut tried = Vec::new();
        while let Some(cursor) = all.next_candidate(local) {
            tried.push(cursor);
            all.failed(StartFailure::Unplayable, None);
        }
        assert_eq!(tried, vec![2, 0, 1]);
        assert_eq!(all.final_failure(), Some(StartFailure::Unplayable));
    }

    #[test]
    fn a_walk_with_nothing_to_try_has_no_failure_to_report() {
        let mut walk = NaturalWalk::new(3, 2, QueueRepeatMode::Off);
        assert_eq!(walk.next_candidate(|_| None), None);
        assert_eq!(walk.final_failure(), None);
    }
}
