use std::cmp::Ordering;

use player_core::{MediaLocator, Playlist, PlaylistEntry, TrackId};
use player_db::natural_title_cmp;

/// Apply one playlist checkbox change without moving any pre-existing item relative to another.
/// Existing duplicate references to the selected track are removed together so a checkbox always
/// represents membership, rather than a count of duplicate slots.
pub(crate) fn set_playlist_membership(
    playlist: &mut Playlist,
    track_id: TrackId,
    locator: MediaLocator,
    selected: bool,
) -> bool {
    let already_present = playlist
        .entries
        .iter()
        .any(|entry| entry.track_id == Some(track_id));

    if selected {
        if already_present {
            return false;
        }
        let key = locator_sort_key(&locator);
        let insertion = playlist
            .entries
            .iter()
            .position(|entry| {
                natural_title_cmp(&locator_sort_key(&entry.locator), &key) == Ordering::Greater
            })
            .unwrap_or(playlist.entries.len());
        playlist.entries.insert(
            insertion,
            PlaylistEntry {
                track_id: Some(track_id),
                locator,
                title: None,
                duration_ms: None,
            },
        );
        true
    } else {
        let original_len = playlist.entries.len();
        playlist
            .entries
            .retain(|entry| entry.track_id != Some(track_id));
        playlist.entries.len() != original_len
    }
}

fn locator_sort_key(locator: &MediaLocator) -> String {
    match locator {
        MediaLocator::FileSystem(path) => path.to_string_lossy().replace('\\', "/"),
        MediaLocator::ContentUri(uri) => uri.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use player_core::{MediaLocator, Playlist, PlaylistEntry, TrackId};

    use super::set_playlist_membership;

    fn entry(name: &str, id: TrackId) -> PlaylistEntry {
        PlaylistEntry {
            track_id: Some(id),
            locator: MediaLocator::FileSystem(PathBuf::from(format!("D:/Music/{name}"))),
            title: Some(name.to_owned()),
            duration_ms: None,
        }
    }

    #[test]
    fn favorite_add_uses_natural_path_order_and_keeps_existing_relative_order() {
        let first = TrackId::new();
        let second = TrackId::new();
        let third = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        // The original playlist is deliberately not globally sorted; its existing sequence must
        // remain [first, second, third] after placing the new path before the first later path.
        playlist.entries = vec![
            entry("song 1.flac", first),
            entry("song 10.flac", second),
            entry("song 3.flac", third),
        ];

        assert!(set_playlist_membership(
            &mut playlist,
            inserted,
            MediaLocator::FileSystem(PathBuf::from("D:/Music/song 2.flac")),
            true,
        ));

        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|item| item.track_id)
                .collect::<Vec<_>>(),
            vec![Some(first), Some(inserted), Some(second), Some(third)]
        );
    }

    #[test]
    fn each_playlist_membership_is_independent_and_uncheck_removes_duplicate_slots() {
        let track = TrackId::new();
        let mut first = Playlist::new("First");
        let mut second = Playlist::new("Second");
        first.entries = vec![
            entry("song 1.flac", track),
            entry("duplicate/song 1.flac", track),
        ];
        second.entries = vec![entry("song 9.flac", TrackId::new())];

        assert!(set_playlist_membership(
            &mut first,
            track,
            MediaLocator::FileSystem(PathBuf::from("D:/Music/song 1.flac")),
            false,
        ));

        assert!(first.entries.is_empty());
        assert_eq!(second.entries.len(), 1);
    }

    #[test]
    fn checking_an_existing_membership_is_idempotent() {
        let track = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        playlist.entries.push(entry("song 10.flac", track));
        assert!(!set_playlist_membership(
            &mut playlist,
            track,
            MediaLocator::FileSystem(PathBuf::from("D:/Music/song 10.flac")),
            true,
        ));
        assert_eq!(playlist.entries.len(), 1);
    }
}
