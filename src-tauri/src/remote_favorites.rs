//! The favorite button for playlists that live on a remote server (ADR 0026): the playlist file on the server is
//! rewritten (only the lines of the one song, in the style of the file's own entries; see
//! `player_remote::playlist_edit`), then the playlist source is synchronized again so the database projection, the
//! file's fingerprint and the pages that list it follow.

use std::sync::Arc;

use player_core::MediaLocator;
use player_db::Database;
use player_remote::index::{join_path, RemoteResolver};
use player_remote::playlist_edit::{edit_playlist_file, Edit, EditError};

use crate::settings::{AppSettings, SourceEntry, SourceEntryKind};

/// The track is not on the server the playlist lives on, so the playlist cannot name it.
pub(crate) const NOT_ON_SERVER: &str = "notOnServer";
/// The server account of the playlist is gone from the settings.
pub(crate) const NO_ACCOUNT: &str = "noAccount";

/// The account a remote source belongs to and its base folder under the account's root (a playlist source names its
/// songs from the root, so its base is empty).
fn account_and_base(source: &SourceEntry) -> Option<(&str, String)> {
    match &source.kind {
        SourceEntryKind::Folder {
            path,
            remote_account: Some(account),
            ..
        } => match path.to_media_locator() {
            Ok(MediaLocator::Remote { path, .. }) => {
                Some((account.as_str(), path.trim_matches('/').to_owned()))
            }
            _ => None,
        },
        SourceEntryKind::PlaylistFile {
            remote_account: Some(account),
            ..
        } => Some((account.as_str(), String::new())),
        _ => None,
    }
}

/// The path of the track under the root of `account_id`, when one of its locators is a file on that account.
pub(crate) fn server_path_of_track(
    settings: &AppSettings,
    account_id: &str,
    locators: &[MediaLocator],
) -> Option<String> {
    locators.iter().find_map(|locator| {
        let MediaLocator::Remote { source_id, path } = locator else {
            return None;
        };
        let source = settings
            .sources
            .iter()
            .find(|source| source.id == *source_id)?;
        let (account, base) = account_and_base(source)?;
        (account == account_id).then(|| join_path(&base, path))
    })
}

/// Why the track cannot be put into a playlist of `account_id` (a code the page translates), `None` when it can.
pub(crate) fn unavailable_reason(
    settings: &AppSettings,
    account_id: &str,
    locators: &[MediaLocator],
) -> Option<&'static str> {
    if !settings
        .remote_accounts
        .iter()
        .any(|account| account.id == account_id)
    {
        return Some(NO_ACCOUNT);
    }
    server_path_of_track(settings, account_id, locators)
        .is_none()
        .then_some(NOT_ON_SERVER)
}

pub(crate) struct Job {
    pub database_path: std::path::PathBuf,
    pub settings: AppSettings,
    pub resolver: Arc<dyn RemoteResolver>,
    pub source: SourceEntry,
    pub server_path: String,
    /// `#EXTINF` title and length of the song (written only when the file already uses `#EXTINF`).
    pub title: Option<String>,
    pub duration_ms: Option<u64>,
    pub selected: bool,
    pub synced_at_utc_ms: i64,
}

/// Rewrite the playlist file on the server, then synchronize its source. The file is read right before it is written
/// and replaced only if it is unchanged since (otherwise nothing is written, and the source is synchronized anyway so
/// what the page shows is the file as it now is).
pub(crate) fn run(job: Job) -> Result<(), String> {
    let SourceEntryKind::PlaylistFile { path, .. } = &job.source.kind else {
        return Err("只能收藏到播放清單檔案來源。".to_owned());
    };
    let Ok(MediaLocator::Remote { path: file_rel, .. }) = path.to_media_locator() else {
        return Err("伺服器上的播放清單位置無效。".to_owned());
    };
    let resolved = job
        .resolver
        .resolve(job.source.id)
        .map_err(|error| error.to_string())?;
    let edit = if job.selected {
        Edit::Add {
            server_path: &job.server_path,
            compare: &player_db::natural_title_cmp,
            title: job.title.as_deref(),
            duration_ms: job.duration_ms,
        }
    } else {
        Edit::Remove {
            server_path: &job.server_path,
        }
    };
    let written = edit_playlist_file(resolved.fs.as_ref(), &file_rel, &edit);

    let synced = Database::open(&job.database_path)
        .map_err(|error| error.to_string())
        .and_then(|mut database| {
            crate::remote_playlist_sync::sync_remote_playlist_source(
                &mut database,
                &job.source,
                &job.settings.sources,
                Arc::clone(&job.resolver),
                job.synced_at_utc_ms,
                &mut |_| {},
            )
        });
    match written {
        Err(EditError::Changed) => Err(EditError::Changed.to_string()),
        Err(error) => Err(error.to_string()),
        Ok(_) => synced
            .map(|_| ())
            .map_err(|error| format!("伺服器上的播放清單已更新，但資料庫投影更新失敗：{error}")),
    }
}

#[cfg(test)]
mod tests {
    use player_core::{PlaylistId, SourceId};

    use super::*;
    use crate::settings::StoredPath;

    fn remote_source(id: SourceId, account: &str, base: &str, playlist: bool) -> SourceEntry {
        let locator = MediaLocator::Remote {
            source_id: id,
            path: base.to_owned(),
        };
        SourceEntry {
            id,
            display_name: "x".to_owned(),
            enabled: true,
            kind: if playlist {
                SourceEntryKind::PlaylistFile {
                    playlist_id: PlaylistId::new(),
                    path: StoredPath::from_locator(&locator).unwrap(),
                    tree_uri: None,
                    remote_account: Some(account.to_owned()),
                }
            } else {
                SourceEntryKind::Folder {
                    media_kind: player_core::MediaSourceKind::Remote,
                    path: StoredPath::from_locator(&locator).unwrap(),
                    remote_account: Some(account.to_owned()),
                }
            },
        }
    }

    #[test]
    fn only_a_track_of_the_same_account_can_join_and_its_path_is_from_the_root() {
        let folder = SourceId::new();
        let other = SourceId::new();
        let mut settings = AppSettings::default();
        settings.sources = vec![
            remote_source(folder, "acc", "Music/Loss", false),
            remote_source(other, "acc2", "Songs", false),
        ];
        let on_acc = [MediaLocator::Remote {
            source_id: folder,
            path: "Artist/a.flac".to_owned(),
        }];
        assert_eq!(
            server_path_of_track(&settings, "acc", &on_acc).as_deref(),
            Some("Music/Loss/Artist/a.flac")
        );
        let on_other = [MediaLocator::Remote {
            source_id: other,
            path: "b.flac".to_owned(),
        }];
        assert_eq!(server_path_of_track(&settings, "acc", &on_other), None);
        let local = [MediaLocator::FileSystem("D:\\a.flac".into())];
        assert_eq!(server_path_of_track(&settings, "acc", &local), None);
        // The account itself must still exist.
        assert_eq!(
            unavailable_reason(&settings, "acc", &on_acc),
            Some(NO_ACCOUNT)
        );
    }

    #[test]
    fn a_track_listed_by_a_playlist_source_is_named_from_the_account_root() {
        let list = SourceId::new();
        let mut settings = AppSettings::default();
        settings.sources = vec![remote_source(list, "acc", "Lists/p.m3u8", true)];
        let locators = [MediaLocator::Remote {
            source_id: list,
            path: "Music/x.flac".to_owned(),
        }];
        assert_eq!(
            server_path_of_track(&settings, "acc", &locators).as_deref(),
            Some("Music/x.flac")
        );
    }

    #[test]
    fn adding_and_removing_rewrites_the_server_file_in_its_style_and_follows_it_in_the_database() {
        use std::collections::HashMap;
        use std::path::{Path, PathBuf};

        use player_remote::fs::{RemoteError, RemoteFs};
        use player_remote::index::ResolvedSource;
        use player_remote::local::DirFs;

        struct Resolver {
            fs: Arc<dyn RemoteFs>,
            bases: HashMap<SourceId, String>,
        }
        impl RemoteResolver for Resolver {
            fn resolve(&self, source_id: SourceId) -> Result<ResolvedSource, RemoteError> {
                let base = self.bases.get(&source_id).ok_or(RemoteError::NotFound)?;
                Ok(ResolvedSource {
                    fs: Arc::clone(&self.fs),
                    base_path: base.clone(),
                    excluded: Vec::new(),
                })
            }
        }

        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/player-wavpack/tests/fixtures");
        let root: PathBuf =
            std::env::temp_dir().join(format!("moe-remote-favorites-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (relative, fixture) in [
            ("Music/Loss/01 a.wv", "s16.wv"),
            ("Music/Loss/02 b.wv", "s24.wv"),
        ] {
            let target = root.join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(fixtures.join(fixture), target).unwrap();
        }
        std::fs::create_dir_all(root.join("Music/Playlists")).unwrap();
        let file = root.join("Music/Playlists/mix.m3u8");
        std::fs::write(&file, b"#EXTM3U\r\nD:\\Music\\Loss\\01 a.wv\r\n").unwrap();

        let list = SourceId::new();
        let source = remote_source(list, "acc", "Music/Playlists/mix.m3u8", true);
        let mut settings = AppSettings::default();
        settings.sources = vec![source.clone()];
        let resolver: Arc<dyn RemoteResolver> = Arc::new(Resolver {
            fs: Arc::new(DirFs::new(&root)),
            bases: HashMap::from([(list, String::new())]),
        });
        let database_path = root.join("test.sqlite3");
        let job = |selected: bool, now: i64| Job {
            database_path: database_path.clone(),
            settings: settings.clone(),
            resolver: Arc::clone(&resolver),
            source: source.clone(),
            server_path: "Music/Loss/02 b.wv".to_owned(),
            title: None,
            duration_ms: None,
            selected,
            synced_at_utc_ms: now,
        };

        run(job(true, 1_000)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "#EXTM3U\r\nD:\\Music\\Loss\\01 a.wv\r\nD:\\Music\\Loss\\02 b.wv\r\n"
        );
        let SourceEntryKind::PlaylistFile { playlist_id, .. } = &source.kind else {
            unreachable!()
        };
        let database = Database::open(&database_path).unwrap();
        assert_eq!(
            database
                .get_playlist(*playlist_id)
                .unwrap()
                .unwrap()
                .entries
                .len(),
            2
        );
        drop(database);

        run(job(false, 2_000)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "#EXTM3U\r\nD:\\Music\\Loss\\01 a.wv\r\n"
        );
        let database = Database::open(&database_path).unwrap();
        assert_eq!(
            database
                .get_playlist(*playlist_id)
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
        drop(database);
        let _ = std::fs::remove_dir_all(root);
    }
}
