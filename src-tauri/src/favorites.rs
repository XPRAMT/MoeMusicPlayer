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
        let insertion = match &locator {
            MediaLocator::FileSystem(path) => {
                filesystem_insertion_position(&playlist.entries, path)
            }
            // Keep the existing Android/content-URI behavior unchanged. Content URIs do not
            // consistently expose a filesystem parent path.
            MediaLocator::ContentUri(_) | MediaLocator::Remote { .. } => {
                locator_insertion_position(&playlist.entries, &locator)
            }
        };
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

/// Write a playlist file that already exists (or create one), keeping exactly one backup of the version before: the
/// original becomes `<name>.bak` (an older `.bak` is deleted first) and the new bytes take its name. The order is safe:
/// the new bytes go to `.moetmp-<name>` beside it and are checked first; if renaming the original or the temporary file
/// fails, everything is put back and the original is untouched. `previous` is what was read before editing (`None`: the
/// file did not exist); a file that no longer holds exactly that is refused (someone else wrote it meanwhile).
#[cfg(any(target_os = "windows", target_os = "android", test))]
pub(crate) fn write_keeping_backup(
    path: &std::path::Path,
    previous: Option<&[u8]>,
    bytes: &[u8],
) -> Result<(), String> {
    const CHANGED: &str = "播放清單檔案在讀取後被其他程式改動，尚未寫入。請重新整理後再試一次。";
    let exists = path.exists();
    match (exists, previous) {
        (true, Some(previous)) => {
            let current = std::fs::read(path).map_err(|error| format!("無法重新讀取播放清單：{error}"))?;
            if current != previous {
                return Err(CHANGED.to_owned());
            }
        }
        (false, None) => {}
        _ => return Err(CHANGED.to_owned()),
    }
    let name = path
        .file_name()
        .ok_or_else(|| "播放清單檔案路徑無效。".to_owned())?
        .to_os_string();
    let mut temp_name = std::ffi::OsString::from(".moetmp-");
    temp_name.push(&name);
    let temp = path.with_file_name(temp_name);
    let mut backup_name = name;
    backup_name.push(".bak");
    let backup = path.with_file_name(backup_name);

    let drop_temp = |error: String| {
        let _ = std::fs::remove_file(&temp);
        error
    };
    std::fs::write(&temp, bytes).map_err(|error| drop_temp(format!("無法寫入播放清單暫存檔：{error}")))?;
    let written = std::fs::metadata(&temp).map(|meta| meta.len()).unwrap_or(0);
    if written != bytes.len() as u64 {
        return Err(drop_temp("播放清單暫存檔的大小不符，尚未寫入。".to_owned()));
    }
    if exists {
        // Windows refuses to rename over a file, so the older backup is deleted first.
        if backup.exists() {
            std::fs::remove_file(&backup).map_err(|error| drop_temp(format!("無法刪除舊的備份檔：{error}")))?;
        }
        std::fs::rename(path, &backup).map_err(|error| drop_temp(format!("無法備份原播放清單：{error}")))?;
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::rename(&backup, path);
            return Err(drop_temp(format!("無法原子更新播放清單：{error}")));
        }
    } else if let Err(error) = std::fs::rename(&temp, path) {
        return Err(drop_temp(format!("無法原子更新播放清單：{error}")));
    }
    Ok(())
}

fn locator_sort_key(locator: &MediaLocator) -> String {
    match locator {
        MediaLocator::FileSystem(path) => windows_path_sort_key(path),
        MediaLocator::ContentUri(uri) => uri.clone(),
        MediaLocator::Remote { source_id, path } => {
            player_core::remote_locator_uri(*source_id, path)
        }
    }
}

fn locator_insertion_position(entries: &[PlaylistEntry], locator: &MediaLocator) -> usize {
    let key = locator_sort_key(locator);
    entries
        .iter()
        .enumerate()
        .filter_map(|(position, entry)| {
            let existing_key = locator_sort_key(&entry.locator);
            (natural_title_cmp(&existing_key, &key) != Ordering::Greater)
                .then_some((position, existing_key))
        })
        .max_by(|(left_position, left_key), (right_position, right_key)| {
            natural_title_cmp(left_key, right_key).then_with(|| left_position.cmp(right_position))
        })
        .map_or(0, |(position, _)| position + 1)
}

fn filesystem_insertion_position(entries: &[PlaylistEntry], path: &std::path::Path) -> usize {
    let keys: Vec<Option<String>> = entries
        .iter()
        .map(|entry| match &entry.locator {
            MediaLocator::FileSystem(existing) => Some(windows_path_sort_key(existing)),
            _ => None,
        })
        .collect();
    player_core::playlist::favorite_insertion_index(
        &keys,
        &windows_path_sort_key(path),
        &natural_title_cmp,
    )
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
    fn favorite_add_uses_parent_folder_and_keeps_existing_relative_order() {
        let first = TrackId::new();
        let second = TrackId::new();
        let third = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        // Filename ordering is ignored within one folder, so the new entry follows that folder's
        // last existing entry even when filenames would sort in the middle.
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
            vec![Some(first), Some(second), Some(third), Some(inserted)]
        );
    }

    #[test]
    fn favorite_add_inserts_after_the_last_entry_from_the_same_folder() {
        let first = TrackId::new();
        let other_folder = TrackId::new();
        let last_same_folder = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: Some(first),
                locator: MediaLocator::FileSystem(PathBuf::from("D:/Music/A/one.flac")),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: Some(other_folder),
                locator: MediaLocator::FileSystem(PathBuf::from("D:/Music/B/one.flac")),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: Some(last_same_folder),
                locator: MediaLocator::FileSystem(PathBuf::from("D:/Music/A/two.flac")),
                title: None,
                duration_ms: None,
            },
        ];

        set_playlist_membership(
            &mut playlist,
            inserted,
            MediaLocator::FileSystem(PathBuf::from("D:/Music/A/0.flac")),
            true,
        );

        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|entry| entry.track_id)
                .collect::<Vec<_>>(),
            vec![
                Some(first),
                Some(other_folder),
                Some(last_same_folder),
                Some(inserted)
            ]
        );
    }

    #[test]
    fn favorite_add_falls_back_to_the_nearest_ancestor_folder() {
        let ancestor_entry = TrackId::new();
        let other_entry = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: Some(ancestor_entry),
                locator: MediaLocator::FileSystem(PathBuf::from("D:/Music/root.flac")),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: Some(other_entry),
                locator: MediaLocator::FileSystem(PathBuf::from("D:/Other/track.flac")),
                title: None,
                duration_ms: None,
            },
        ];

        set_playlist_membership(
            &mut playlist,
            inserted,
            MediaLocator::FileSystem(PathBuf::from("D:/Music/Artist/Album/new.flac")),
            true,
        );

        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|entry| entry.track_id)
                .collect::<Vec<_>>(),
            vec![Some(ancestor_entry), Some(inserted), Some(other_entry)]
        );
    }

    #[test]
    fn favorite_add_appends_when_no_folder_or_ancestor_is_present() {
        let existing = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Mix");
        playlist.entries.push(PlaylistEntry {
            track_id: Some(existing),
            locator: MediaLocator::FileSystem(PathBuf::from("E:/Archive/track.flac")),
            title: None,
            duration_ms: None,
        });

        set_playlist_membership(
            &mut playlist,
            inserted,
            MediaLocator::FileSystem(PathBuf::from("D:/Music/new.flac")),
            true,
        );

        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|entry| entry.track_id)
                .collect::<Vec<_>>(),
            vec![Some(existing), Some(inserted)]
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
                MediaLocator::ContentUri(_) | MediaLocator::Remote { .. } => {
                    panic!("expected filesystem path")
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual_paths,
            vec![
                old_paths[0].to_owned(),
                old_paths[1].to_owned(),
                old_paths[2].to_owned(),
                old_paths[3].to_owned(),
                r"D:\Music\Hi-Res\[Hi-Res][hanser]\hanser - M87黑洞.flac".to_owned(),
                old_paths[4].to_owned(),
            ]
        );
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
            vec![Some(first), Some(second), Some(inserted)]
        );
    }

    #[test]
    fn content_uri_favorites_keep_the_existing_uri_sorting() {
        let first = TrackId::new();
        let second = TrackId::new();
        let inserted = TrackId::new();
        let mut playlist = Playlist::new("Android");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: Some(first),
                locator: MediaLocator::ContentUri("content://media/audio/1".to_owned()),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: Some(second),
                locator: MediaLocator::ContentUri("content://media/audio/10".to_owned()),
                title: None,
                duration_ms: None,
            },
        ];

        set_playlist_membership(
            &mut playlist,
            inserted,
            MediaLocator::ContentUri("content://media/audio/2".to_owned()),
            true,
        );

        assert_eq!(
            playlist
                .entries
                .iter()
                .map(|entry| entry.track_id)
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

    // ---- the single `.bak` backup of a local playlist file ----

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moe-favorites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files_in(dir: &PathBuf) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writing_keeps_one_backup_of_the_version_before_the_latest_change() {
        for name in ["MyLove.m3u8", "mix.m3u"] {
            let dir = scratch(&format!("bak-{name}"));
            let file = dir.join(name);
            std::fs::write(&file, b"v1").unwrap();
            super::write_keeping_backup(&file, Some(b"v1"), b"v2").unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), b"v2");
            assert_eq!(std::fs::read(dir.join(format!("{name}.bak"))).unwrap(), b"v1");
            super::write_keeping_backup(&file, Some(b"v2"), b"v3").unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), b"v3");
            assert_eq!(std::fs::read(dir.join(format!("{name}.bak"))).unwrap(), b"v2");
            assert_eq!(files_in(&dir), vec![name.to_owned(), format!("{name}.bak")]);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_new_file_has_no_backup_and_a_changed_file_is_refused_untouched() {
        let dir = scratch("bak-new");
        let file = dir.join("MyLove.m3u8");
        super::write_keeping_backup(&file, None, b"first").unwrap();
        assert_eq!(files_in(&dir), vec!["MyLove.m3u8".to_owned()]);
        std::fs::write(&file, b"edited elsewhere").unwrap();
        assert!(super::write_keeping_backup(&file, Some(b"first"), b"second").is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"edited elsewhere");
        assert_eq!(files_in(&dir), vec!["MyLove.m3u8".to_owned()]);
        // A file created by someone else meanwhile is not overwritten either.
        assert!(super::write_keeping_backup(&file, None, b"x").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_backup_is_replaced() {
        let dir = scratch("bak-stale");
        let file = dir.join("mix.m3u8");
        std::fs::write(&file, b"now").unwrap();
        std::fs::write(dir.join("mix.m3u8.bak"), b"very old").unwrap();
        super::write_keeping_backup(&file, Some(b"now"), b"next").unwrap();
        assert_eq!(std::fs::read(dir.join("mix.m3u8.bak")).unwrap(), b"now");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backup_that_cannot_be_replaced_leaves_the_original_and_no_temporary_file() {
        let dir = scratch("bak-blocked");
        let file = dir.join("mix.m3u8");
        std::fs::write(&file, b"now").unwrap();
        // A folder named like the backup cannot be deleted as a file: the sequence stops before touching the original.
        std::fs::create_dir_all(dir.join("mix.m3u8.bak").join("inside")).unwrap();
        assert!(super::write_keeping_backup(&file, Some(b"now"), b"next").is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"now");
        assert_eq!(files_in(&dir), vec!["mix.m3u8".to_owned(), "mix.m3u8.bak".to_owned()]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
