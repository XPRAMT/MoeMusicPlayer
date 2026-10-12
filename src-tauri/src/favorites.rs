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
            .enumerate()
            .filter_map(|(position, entry)| {
                let existing_key = locator_sort_key(&entry.locator);
                (natural_title_cmp(&existing_key, &key) != Ordering::Greater)
                    .then_some((position, existing_key))
            })
            .max_by(|(left_position, left_key), (right_position, right_key)| {
                natural_title_cmp(left_key, right_key)
                    .then_with(|| left_position.cmp(right_position))
            })
            .map_or(0, |(position, _)| position + 1);
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
        MediaLocator::FileSystem(path) => windows_path_sort_key(path),
        MediaLocator::ContentUri(uri) => uri.clone(),
    }
}

fn windows_path_sort_key(path: &std::path::Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    // The Win32 extended-length marker changes file API behavior, not the path's
    // natural order. Keep it on the stored locator but remove it from comparisons.
    if path
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/UNC/"))
    {
        format!("//{}", &path[8..])
    } else if path
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/"))
    {
        path[4..].to_owned()
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use player_core::{MediaLocator, Playlist, PlaylistEntry, TrackId};

    use super::{set_playlist_membership, windows_path_sort_key};

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
    fn favorite_add_scans_the_whole_unsorted_playlist_before_choosing_position() {
        let mut playlist = Playlist::new("Mix");
        let old_paths = [
            r"D:\Music\Loss\[hanser]\older.flac",
            r"D:\Music\CD\[CD][hanser]\older.flac",
            r"D:\Music\Hi-Res\[Hi-Res][hanser]\hanser - Hidden Heart.flac",
            r"D:\Music\Hi-Res\[Hi-Res][hanser]\hanser - 速通滿級攻略.flac",
            r"D:\Music\Hi-Res\@單曲\late.flac",
        ];
        playlist.entries = old_paths
            .iter()
            .map(|path| PlaylistEntry {
                track_id: Some(TrackId::new()),
                locator: MediaLocator::FileSystem(std::path::PathBuf::from(path)),
                title: None,
                duration_ms: None,
            })
            .collect();
        let new_track = TrackId::new();

        assert!(set_playlist_membership(
            &mut playlist,
            new_track,
            MediaLocator::FileSystem(std::path::PathBuf::from(
                r"D:\Music\Hi-Res\[Hi-Res][hanser]\hanser - M87黑洞.flac"
            )),
            true,
        ));

        let actual_paths = playlist
            .entries
            .iter()
            .map(|entry| match &entry.locator {
                MediaLocator::FileSystem(path) => path.to_string_lossy(),
                MediaLocator::ContentUri(_) => panic!("expected filesystem path"),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual_paths[0], old_paths[0]);
        assert_eq!(actual_paths[1], old_paths[1]);
        assert_eq!(actual_paths[2], old_paths[2]);
        assert_eq!(
            actual_paths[3],
            r"D:\Music\Hi-Res\[Hi-Res][hanser]\hanser - M87黑洞.flac"
        );
        assert_eq!(actual_paths[4], old_paths[3]);
        assert_eq!(actual_paths[5], old_paths[4]);
    }

    #[test]
    fn favorite_add_normalizes_windows_extended_path_prefix_for_sorting() {
        let first = TrackId::new();
        let second = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        playlist.entries = vec![entry("song 1.flac", first), entry("song 3.flac", second)];

        assert!(set_playlist_membership(
            &mut playlist,
            inserted,
            MediaLocator::FileSystem(PathBuf::from(r"\\?\D:\Music\song 2.flac")),
            true,
        ));

        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|item| item.track_id)
                .collect::<Vec<_>>(),
            vec![Some(first), Some(inserted), Some(second)]
        );
    }

    #[test]
    fn extended_unc_path_uses_the_same_sort_key_as_regular_unc_path() {
        assert_eq!(
            windows_path_sort_key(std::path::Path::new(
                r"\\?\UNC\server\share\Music\song.flac"
            )),
            windows_path_sort_key(std::path::Path::new(r"\\server\share\Music\song.flac"))
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
