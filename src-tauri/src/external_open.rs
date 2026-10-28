//! An audio file another app opens with this one (Android 「開啟方式」, `ACTION_VIEW`, ADR 0054).
//!
//! The file plays at once and the queue is the other audio files of its folder in natural file-name order, with the
//! opened file as the current entry (the same queue Explorer's "open with" builds on Windows). A file the library
//! already has plays as its library track; any other becomes a track that exists only for this queue: it has a fresh
//! `TrackId` and a summary read from the file, kept in the registry below, and nothing is written to the database.
//!
//! This module is the part that does not touch Android: the order of the folder, the plan of the queue and the registry
//! of the queue's own tracks. The playback actor (`android_playback.rs`) reads the registry; the command at the bottom
//! (Android only) takes the request from the plugin and hands the plan to the actor.

#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use player_core::{PlaybackQueueEntry, TrackId, TrackMetadata, TrackSummary};

/// A track of the open-with queue that is not in the library.
#[derive(Clone, Debug)]
pub(crate) struct ExternalTrack {
    pub(crate) path: PathBuf,
    pub(crate) summary: TrackSummary,
}

/// The queue's own tracks by id.
#[derive(Default)]
pub(crate) struct ExternalRegistry(Mutex<HashMap<TrackId, ExternalTrack>>);

impl ExternalRegistry {
    fn tracks(&self) -> std::sync::MutexGuard<'_, HashMap<TrackId, ExternalTrack>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Add the tracks of a new open-with queue (the queue that plays now keeps its own until the new one starts).
    pub(crate) fn register(&self, tracks: impl IntoIterator<Item = ExternalTrack>) {
        let mut registry = self.tracks();
        for track in tracks {
            registry.insert(track.summary.id, track);
        }
    }

    pub(crate) fn get(&self, id: TrackId) -> Option<ExternalTrack> {
        self.tracks().get(&id).cloned()
    }

    pub(crate) fn contains(&self, id: TrackId) -> bool {
        self.tracks().contains_key(&id)
    }

    /// Forget every track that is not in `keep` (the queue that plays now).
    pub(crate) fn retain_only(&self, keep: &HashSet<TrackId>) {
        self.tracks().retain(|id, _| keep.contains(id));
    }

    /// Replace the summary of a registered track (its tags were read after the queue started).
    pub(crate) fn update_summary(&self, id: TrackId, summary: TrackSummary) {
        if let Some(track) = self.tracks().get_mut(&id) {
            track.summary = summary;
        }
    }

    /// The summaries of the registered tracks among `ids`.
    pub(crate) fn summaries(&self, ids: &[TrackId]) -> HashMap<TrackId, TrackSummary> {
        let registry = self.tracks();
        ids.iter()
            .filter_map(|id| registry.get(id).map(|track| (*id, track.summary.clone())))
            .collect()
    }
}

static REGISTRY: std::sync::LazyLock<ExternalRegistry> =
    std::sync::LazyLock::new(ExternalRegistry::default);

/// The registry of the queue that plays (one per process, like the playback actor that reads it).
pub(crate) fn registry() -> &'static ExternalRegistry {
    &REGISTRY
}

/// Whether a folder entry belongs in the queue: an audio file the library would scan, not hidden.
pub(crate) fn is_queue_file_name(name: &str) -> bool {
    !name.starts_with('.') && player_formats::is_android_library_audio(Path::new(name))
}

/// The queue's files in order: the audio files among `siblings` plus `selected` itself (whatever its extension: the
/// user chose it), natural file-name order. Returns the files and the index of `selected`.
pub(crate) fn queue_paths(selected: &Path, siblings: Vec<PathBuf>) -> (Vec<PathBuf>, usize) {
    let mut paths: Vec<PathBuf> = siblings
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_queue_file_name)
        })
        .collect();
    if !paths.iter().any(|path| path == selected) {
        paths.push(selected.to_path_buf());
    }
    paths.sort_by(|left, right| {
        let left_name = left.file_name().unwrap_or_default().to_string_lossy();
        let right_name = right.file_name().unwrap_or_default().to_string_lossy();
        player_db::natural_title_cmp(&left_name, &right_name)
            .then_with(|| left_name.cmp(&right_name))
    });
    paths.dedup();
    let index = paths
        .iter()
        .position(|path| path == selected)
        .unwrap_or_default();
    (paths, index)
}

/// What the actor needs to start the queue.
#[derive(Debug)]
pub(crate) struct OpenPlan {
    pub(crate) entries: Vec<PlaybackQueueEntry>,
    pub(crate) selected_index: usize,
    /// The tracks that are not in the library.
    pub(crate) externals: Vec<ExternalTrack>,
}

/// The summary of a file the library does not have, from what could be read of it (`metadata` is empty before the tags
/// are read: the file name stands for the title then).
pub(crate) fn external_summary(
    path: &Path,
    id: TrackId,
    metadata: TrackMetadata,
    created_at_utc_ms: Option<i64>,
    modified_at_utc_ms: Option<i64>,
) -> TrackSummary {
    TrackSummary {
        id,
        title: metadata.title,
        artist: metadata.artist,
        album: metadata.album,
        album_artist: metadata.album_artist,
        track_number: metadata.track_number,
        disc_number: metadata.disc_number,
        duration_ms: metadata.duration_ms,
        codec: metadata.codec,
        bitrate_bps: metadata.bitrate_bps,
        sample_rate_hz: metadata.sample_rate_hz,
        release_date: metadata.release_date,
        year: metadata.year,
        created_at_utc_ms,
        bit_depth: metadata.bit_depth,
        played_ms: 0,
        file_name: path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned()),
        // A file opened from outside the library is a local file (the name the library gives local tracks).
        source_name: Some("本地".to_owned()),
        modified_at_utc_ms,
    }
}

/// Plan the queue of `paths` with `selected_index` current. `library_track` finds the library's track for a file;
/// `read_summary` makes the summary of a file the library does not have (for the file that starts, with its tags).
pub(crate) fn plan_open(
    paths: &[PathBuf],
    selected_index: usize,
    library_track: &dyn Fn(&Path) -> Result<Option<TrackId>, String>,
    read_summary: &dyn Fn(&Path, TrackId, bool) -> TrackSummary,
) -> Result<OpenPlan, String> {
    if selected_index >= paths.len() {
        return Err("選取的音樂檔案不在資料夾清單中。".to_owned());
    }
    let mut entries = Vec::with_capacity(paths.len());
    let mut externals = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let track_id = match library_track(path)? {
            Some(id) => id,
            None => {
                let id = TrackId::new();
                externals.push(ExternalTrack {
                    path: path.clone(),
                    summary: read_summary(path, id, index == selected_index),
                });
                id
            }
        };
        entries.push(PlaybackQueueEntry {
            track_id,
            source_position: None,
        });
    }
    Ok(OpenPlan {
        entries,
        selected_index,
        externals,
    })
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use crate::{AppState, android_playback, enter_database_work};
    use player_core::{
        FileFingerprint, MediaLocator, MediaTrackRecord, SourceId, TrackIdentity, unix_path_key,
    };
    use tauri::{AppHandle, Manager as _, State};
    use tauri_plugin_media_index::MediaIndexExt as _;

    fn utc_ms(time: std::time::SystemTime) -> Option<i64> {
        time.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
    }

    fn times_of(path: &Path) -> (Option<i64>, Option<i64>, u64) {
        match std::fs::metadata(path) {
            Ok(metadata) => (
                metadata.created().ok().and_then(utc_ms),
                metadata.modified().ok().and_then(utc_ms),
                metadata.len(),
            ),
            Err(_) => (None, None, 0),
        }
    }

    /// Tags of a file through the plugin's reader (the library's own, with Android's fallbacks).
    fn read_metadata(app: &AppHandle, path: &Path) -> TrackMetadata {
        let (created, modified, size) = times_of(path);
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: SourceId::new(),
                source_item_id: path.to_string_lossy().into_owned(),
                locator_key: Some(unix_path_key(path)),
            },
            locator: MediaLocator::FileSystem(path.to_path_buf()),
            fingerprint: FileFingerprint {
                size_bytes: size,
                modified_at_utc_ms: modified,
                created_at_utc_ms: created,
            },
            metadata: None,
        };
        app.media_index().read_metadata(&record).unwrap_or_default()
    }

    fn folder_files(folder: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect()
    }

    /// Reads the tags of the queue's other files in the background, so the notification and the queue page show titles
    /// for them too without making the first track wait for the whole folder.
    fn read_tags_later(app: AppHandle, tracks: Vec<ExternalTrack>) {
        std::thread::Builder::new()
            .name("moe-external-open-tags".to_owned())
            .spawn(move || {
                for track in tracks {
                    let (created, modified, _) = times_of(&track.path);
                    let metadata = read_metadata(&app, &track.path);
                    let summary = external_summary(
                        &track.path,
                        track.summary.id,
                        metadata,
                        created,
                        modified,
                    );
                    registry().update_summary(track.summary.id, summary);
                }
            })
            .ok();
    }

    /// Takes the file another app opened with this one and starts it (ADR 0054). `false`: nothing was waiting.
    pub(super) async fn open(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
        let _database_work = enter_database_work(&state)?;
        // The service first: a request is taken only when it can be started (else it stays for the next ask).
        let service = crate::android_playback_service(&state)?.clone();
        let taking = app.clone();
        let request = tauri::async_runtime::spawn_blocking(move || {
            taking
                .media_index()
                .external_open_take()
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        if !request.pending {
            return Ok(false);
        }
        let Some(raw_path) = request.path else {
            return Err(format!(
                "無法開啟這個檔案：{}",
                request.error.unwrap_or_else(|| "讀不到檔案內容".to_owned())
            ));
        };
        let planning = app.clone();
        let copied = request.copied;
        let plan = tauri::async_runtime::spawn_blocking(move || -> Result<OpenPlan, String> {
            let selected = std::fs::canonicalize(&raw_path)
                .map_err(|error| format!("無法開啟音樂檔案：{error}"))?;
            if !selected.is_file() {
                return Err("選取的項目不是檔案。".to_owned());
            }
            // A private copy is alone in its folder; a folder that cannot be listed leaves the one file.
            let siblings = match selected.parent() {
                Some(folder) if !copied => folder_files(folder),
                _ => Vec::new(),
            };
            let (paths, selected_index) = queue_paths(&selected, siblings);
            let app_state = planning.state::<AppState>();
            let database = app_state.database.as_ref();
            let library_track = |path: &Path| -> Result<Option<TrackId>, String> {
                let Some(database) = database else {
                    return Ok(None);
                };
                database
                    .resolve_track_id_for_locator(&MediaLocator::FileSystem(path.to_path_buf()))
                    .map_err(|error| format!("檢查曲庫路徑失敗：{error}"))
            };
            let read_summary = |path: &Path, id: TrackId, with_tags: bool| -> TrackSummary {
                let (created, modified, _) = times_of(path);
                let metadata = if with_tags {
                    read_metadata(&planning, path)
                } else {
                    TrackMetadata::default()
                };
                external_summary(path, id, metadata, created, modified)
            };
            plan_open(&paths, selected_index, &library_track, &read_summary)
        })
        .await
        .map_err(|error| error.to_string())??;

        let started = plan.entries[plan.selected_index].track_id;
        let later: Vec<ExternalTrack> = plan
            .externals
            .iter()
            .filter(|track| track.summary.id != started)
            .cloned()
            .collect();
        registry().register(plan.externals);
        service
            .execute(android_playback::PlaybackAction::OpenExternal {
                entries: plan.entries,
                selected_index: plan.selected_index,
            })
            .await?;
        read_tags_later(app, later);
        Ok(true)
    }

    /// Whether an opened file waits to be taken (read when the start page is decided).
    pub(super) fn is_pending(app: &AppHandle) -> bool {
        app.media_index().external_open_pending().unwrap_or(false)
    }
}

/// Whether an opened file waits to be taken (read when the start page is decided, so the remembered page does not
/// cover it).
#[cfg(target_os = "android")]
pub(crate) fn pending(app: &tauri::AppHandle) -> bool {
    android::is_pending(app)
}

/// Takes the file another app opened with this one and starts it; `false` when none waits (ADR 0054).
#[cfg(target_os = "android")]
#[tauri::command]
pub(crate) async fn android_external_open(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<bool, String> {
    android::open(app, state).await
}

/// The command exists on every platform so the page can call it unconditionally; only Android has a request.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub(crate) async fn android_external_open() -> Result<bool, String> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| PathBuf::from(format!("/storage/emulated/0/Music/{name}")))
            .collect()
    }

    #[test]
    fn the_queue_is_the_folders_audio_files_in_natural_name_order() {
        let selected = PathBuf::from("/storage/emulated/0/Music/10 b.flac");
        let (queue, index) = queue_paths(
            &selected,
            paths(&[
                "2 a.flac",
                "10 b.flac",
                "1 c.MP3",
                "cover.jpg",
                "notes.txt",
                ".hidden.flac",
                "song.lrc",
                "11 d.wv",
            ]),
        );
        let names: Vec<_> = queue
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["1 c.MP3", "2 a.flac", "10 b.flac", "11 d.wv"]);
        assert_eq!(index, 2);
        assert_eq!(queue[index], selected);
    }

    #[test]
    fn the_opened_file_is_in_the_queue_even_when_the_folder_listing_lacks_it() {
        let selected = PathBuf::from("/data/user/0/app/cache/external-open/a.flac");
        assert_eq!(
            queue_paths(&selected, Vec::new()),
            (vec![selected.clone()], 0)
        );
        // Not an extension the library scans (the user chose it anyway), and listed twice.
        let odd = PathBuf::from("/m/Song");
        let (queue, index) = queue_paths(
            &odd,
            vec![odd.clone(), PathBuf::from("/m/a.flac"), odd.clone()],
        );
        assert_eq!(queue, vec![PathBuf::from("/m/a.flac"), odd]);
        assert_eq!(index, 1);
    }

    #[test]
    fn equal_names_in_different_case_keep_a_stable_order() {
        let selected = PathBuf::from("/m/B.flac");
        let (first, _) = queue_paths(
            &selected,
            vec![PathBuf::from("/m/b.flac"), PathBuf::from("/m/B.flac")],
        );
        let (second, _) = queue_paths(
            &selected,
            vec![PathBuf::from("/m/B.flac"), PathBuf::from("/m/b.flac")],
        );
        assert_eq!(first, second);
    }

    fn summary(path: &Path, id: TrackId, with_tags: bool) -> TrackSummary {
        let metadata = TrackMetadata {
            title: with_tags.then(|| "Tagged".to_owned()),
            ..TrackMetadata::default()
        };
        external_summary(path, id, metadata, Some(1), Some(2))
    }

    #[test]
    fn a_library_file_keeps_its_track_and_the_others_get_their_own() {
        let library = TrackId::new();
        let files = paths(&["1.flac", "2.flac", "3.flac"]);
        let known = files[1].clone();
        let plan = plan_open(
            &files,
            1,
            &|path| Ok((path == known).then_some(library)),
            &summary,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 3);
        assert_eq!(plan.selected_index, 1);
        assert_eq!(plan.entries[1].track_id, library);
        assert_eq!(plan.externals.len(), 2);
        assert!(
            plan.externals
                .iter()
                .all(|track| track.summary.id != library)
        );
        assert_eq!(plan.entries[0].track_id, plan.externals[0].summary.id);
        assert_eq!(plan.entries[2].track_id, plan.externals[1].summary.id);
        // The tags are read for the file that starts only.
        assert_eq!(plan.externals[0].summary.title, None);
        assert_eq!(plan.externals[0].summary.file_name.as_deref(), Some("1"));
        assert_eq!(
            plan.externals[0].summary.source_name.as_deref(),
            Some("本地")
        );
    }

    #[test]
    fn the_started_file_reads_its_tags_and_a_bad_index_or_lookup_is_an_error() {
        let files = paths(&["1.flac", "2.flac"]);
        let plan = plan_open(&files, 0, &|_| Ok(None), &summary).unwrap();
        assert_eq!(plan.externals[0].summary.title.as_deref(), Some("Tagged"));
        assert_eq!(plan.externals[1].summary.title, None);
        assert!(plan_open(&files, 2, &|_| Ok(None), &summary).is_err());
        assert!(plan_open(&files, 0, &|_| Err("down".to_owned()), &summary).is_err());
    }

    #[test]
    fn the_registry_keeps_the_queues_tracks_and_forgets_the_rest() {
        let files = paths(&["a.flac", "b.flac"]);
        let plan = plan_open(&files, 0, &|_| Ok(None), &summary).unwrap();
        let ids: Vec<TrackId> = plan
            .externals
            .iter()
            .map(|track| track.summary.id)
            .collect();
        let registry = ExternalRegistry::default();
        registry.register(plan.externals);
        assert!(registry.contains(ids[0]) && registry.contains(ids[1]));
        assert_eq!(registry.get(ids[1]).unwrap().path, files[1]);
        assert_eq!(registry.summaries(&ids).len(), 2);
        let tagged = TrackMetadata {
            title: Some("New".to_owned()),
            ..TrackMetadata::default()
        };
        registry.update_summary(
            ids[1],
            external_summary(&files[1], ids[1], tagged, None, None),
        );
        assert_eq!(
            registry.get(ids[1]).unwrap().summary.title.as_deref(),
            Some("New")
        );
        registry.retain_only(&HashSet::from([ids[0]]));
        assert!(registry.contains(ids[0]));
        assert!(!registry.contains(ids[1]));
        assert!(registry.get(ids[1]).is_none());
    }
}
