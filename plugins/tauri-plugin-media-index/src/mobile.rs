use std::{io::Seek, path::PathBuf, sync::Arc};

use lofty::{
    config::ParseOptions,
    file::{FileType, TaggedFile},
    mpeg::{Layer, MpegFile},
    prelude::{Accessor, AudioFile, TaggedFileExt},
    probe::Probe,
    tag::ItemKey,
};

use player_core::{
    LibraryRoot, MediaLocator, MediaSourceError, MediaSourceKind, MediaTrackRecord, SourceScan,
    SourceScanState, TrackMetadata, TrackMetadataError,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::models::{
    ContentCacheLeaseResponse, ContentUriProbeResponse, ReadMetadataResponse, UriResponse,
};
use crate::{ContentCacheLease, Result};

#[derive(Clone)]
pub(crate) struct PlatformMediaIndex<R: Runtime>(Arc<PluginHandle<R>>);

impl<R: Runtime> PlatformMediaIndex<R> {
    pub(crate) fn open_official_url(&self, url: &str) -> Result<()> {
        self.0
            .run_mobile_plugin::<serde_json::Value>("openOfficialUrl", json!({ "url": url }))?;
        Ok(())
    }
    pub(crate) fn init<C: DeserializeOwned>(
        _app: &AppHandle<R>,
        api: PluginApi<R, C>,
    ) -> Result<Self> {
        let handle =
            api.register_android_plugin("com.moemusicplayer.mediaindex", "MediaIndexPlugin")?;
        Ok(Self(Arc::new(handle)))
    }

    pub(crate) fn scan(&self, root: &LibraryRoot, excluded: &[PathBuf]) -> SourceScan {
        match (&root.kind, &root.locator) {
            (MediaSourceKind::AndroidFilesystem, MediaLocator::FileSystem(_)) => {
                match self.all_files_access_status() {
                    Ok(status) if status.granted => crate::fs_scan::scan_directory(
                        root,
                        excluded,
                        &mut |_| {},
                        &player_core::SyncCancellation::default(),
                    ),
                    Ok(_) => unavailable_scan(root, "All files access is not granted"),
                    Err(error) => unavailable_scan(
                        root,
                        &format!("could not check All files access: {error}"),
                    ),
                }
            }
            (MediaSourceKind::AndroidMediaStore | MediaSourceKind::AndroidSaf, _) => unavailable_scan(
                root,
                "legacy MediaStore/SAF source must be migrated to a folder path first (ADR 0014)",
            ),
            _ => unavailable_scan(root, "source kind is not supported by Android"),
        }
    }

    pub(crate) fn read_metadata(
        &self,
        record: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        let uri = match &record.locator {
            MediaLocator::ContentUri(uri) => uri.clone(),
            MediaLocator::FileSystem(path) => return self.read_path_metadata(path),
        };
        let uri = &uri;
        let lease = self
            .cache_content_uri(uri, 512 * 1024 * 1024)
            .map_err(|error| TrackMetadataError {
                message: format!("could not read authorized Android audio content: {error}"),
            })?;
        match parse_lofty_metadata(lease.path(), lease.mime_type()) {
            Ok(metadata) => Ok(metadata),
            Err(lofty_error) => {
                match crate::android_metadata::read(lease.path()) {
                    Ok(metadata) if has_useful_metadata(&metadata) => return Ok(metadata),
                    _ => {}
                }
                match self.read_metadata_with_android(uri) {
                    Ok(metadata) if has_useful_metadata(&metadata) => Ok(metadata),
                    Ok(_) => Err(TrackMetadataError {
                        message: format!(
                            "{}; Symphonia and Android returned no usable audio properties",
                            lofty_error.message
                        ),
                    }),
                    Err(android_error) => Err(TrackMetadataError {
                        message: format!(
                            "{}; Android metadata fallback failed: {}",
                            lofty_error.message, android_error.message
                        ),
                    }),
                }
            }
        }
    }

    /// Tags of a real file (All files access): lofty, then Symphonia, then Android's retriever.
    fn read_path_metadata(
        &self,
        path: &std::path::Path,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        match parse_lofty_metadata(path, None) {
            Ok(metadata) => Ok(metadata),
            Err(lofty_error) => {
                if let Ok(metadata) = crate::android_metadata::read(path) {
                    if has_useful_metadata(&metadata) {
                        return Ok(metadata);
                    }
                }
                let uri = player_core::file_uri_for_path(path).ok_or_else(|| TrackMetadataError {
                    message: format!("{}; path is not absolute", lofty_error.message),
                })?;
                self.read_metadata_with_android(&uri)
                    .ok()
                    .filter(has_useful_metadata)
                    .ok_or(lofty_error)
            }
        }
    }

    fn read_metadata_with_android(
        &self,
        uri: &str,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        let response = self
            .0
            .run_mobile_plugin::<ReadMetadataResponse>("readMetadata", json!({ "contentUri": uri }))
            .map_err(|error| TrackMetadataError {
                message: error.to_string(),
            })?;
        let mut metadata = response.metadata.ok_or_else(|| TrackMetadataError {
            message: response
                .error
                .unwrap_or_else(|| "Android returned no audio metadata".to_owned()),
        })?;
        metadata.codec = metadata.codec.as_deref().and_then(normalize_android_codec);
        Ok(crate::adapter::into_core_metadata(metadata))
    }

    pub(crate) fn all_files_access_status(&self) -> Result<crate::AllFilesAccessStatus> {
        Ok(self.0.run_mobile_plugin::<crate::AllFilesAccessStatus>("allFilesAccessStatus", json!({}))?)
    }

    pub(crate) fn request_all_files_access(&self) -> Result<()> {
        self.0.run_mobile_plugin::<serde_json::Value>("requestAllFilesAccess", json!({}))?;
        Ok(())
    }

    pub(crate) fn resolve_direct_path(&self, uri: &str) -> Result<Option<String>> {
        #[derive(serde::Deserialize)]
        struct DirectPathResponse {
            path: Option<String>,
        }
        Ok(self
            .0
            .run_mobile_plugin::<DirectPathResponse>("resolveDirectPath", json!({ "contentUri": uri }))?
            .path)
    }

    /// Real paths for many MediaStore / ExternalStorageProvider URIs (source migration).
    pub(crate) fn resolve_direct_paths(&self, uris: &[String]) -> Result<Vec<Option<String>>> {
        #[derive(serde::Deserialize)]
        struct DirectPathsResponse {
            paths: Vec<Option<String>>,
        }
        let response = self
            .0
            .run_mobile_plugin::<DirectPathsResponse>("resolveDirectPaths", json!({ "uris": uris }))?;
        if response.paths.len() != uris.len() {
            return Err(crate::Error::Response(
                "Android returned a different number of resolved paths".to_owned(),
            ));
        }
        Ok(response.paths)
    }

    /// Folder picker that only yields a real path; no persisted SAF grant is kept (ADR 0014).
    pub(crate) fn clipboard_has_image(&self) -> Result<bool> {
        let value: serde_json::Value = self.0.run_mobile_plugin("clipboardImageStatus", json!({}))?;
        Ok(value.get("hasImage").and_then(serde_json::Value::as_bool).unwrap_or(false))
    }

    pub(crate) fn read_clipboard_image(&self) -> Result<Option<String>> {
        let value: serde_json::Value = self.0.run_mobile_plugin("readClipboardImage", json!({}))?;
        Ok(value.get("path").and_then(|path| path.as_str()).map(str::to_owned))
    }

    pub(crate) fn open_folder_in_file_manager(&self, path: &str) -> Result<bool> {
        let value: serde_json::Value = self
            .0
            .run_mobile_plugin("openFolderInFileManager", json!({ "path": path }))?;
        Ok(value.get("opened").and_then(serde_json::Value::as_bool).unwrap_or(false))
    }

    pub(crate) fn update_download_directory(&self) -> Result<PathBuf> {
        let value: serde_json::Value = self.0.run_mobile_plugin("updateDownloadDirectory", json!({}))?;
        value
            .get("path")
            .and_then(|path| path.as_str())
            .map(PathBuf::from)
            .ok_or_else(|| crate::Error::Response("no update directory".into()))
    }

    pub(crate) fn install_apk(&self, path: &str) -> Result<String> {
        let value: serde_json::Value = self.0.run_mobile_plugin("installApk", json!({ "path": path }))?;
        Ok(value.get("status").and_then(|status| status.as_str()).unwrap_or("started").to_owned())
    }

    pub(crate) fn pick_directory_path(&self) -> Result<Option<String>> {
        let value: serde_json::Value = self.0.run_mobile_plugin("pickDirectoryPath", json!({}))?;
        Ok(value.get("path").and_then(|path| path.as_str()).map(str::to_owned))
    }

    /// Document picker that only yields a real file path (no persisted grant).
    pub(crate) fn pick_document_path(&self, mime_types: &[&str]) -> Result<Option<String>> {
        let value: serde_json::Value = self
            .0
            .run_mobile_plugin("pickDocumentPath", json!({ "mimeTypes": mime_types }))?;
        Ok(value.get("path").and_then(|path| path.as_str()).map(str::to_owned))
    }

    /// "Save as" picker that only yields the real path of the created file (no persisted grant).
    pub(crate) fn pick_create_document_path(
        &self,
        suggested_name: &str,
        mime_type: &str,
    ) -> Result<Option<String>> {
        let value: serde_json::Value = self.0.run_mobile_plugin(
            "pickCreateDocumentPath",
            json!({ "suggestedName": suggested_name, "mimeType": mime_type }),
        )?;
        Ok(value.get("path").and_then(|path| path.as_str()).map(str::to_owned))
    }

    pub(crate) fn scan_file(&self, path: &str) -> Result<()> {
        self.0.run_mobile_plugin::<serde_json::Value>("scanFile", json!({ "path": path }))?;
        Ok(())
    }

    pub(crate) fn read_system_wallpaper(&self) -> Result<crate::SystemWallpaperRead> {
        Ok(self.0.run_mobile_plugin::<crate::SystemWallpaperRead>("readSystemWallpaper", json!({}))?)
    }

    pub(crate) fn set_wallpaper_backdrop(&self, enabled: bool, blur_px: u32) -> Result<crate::WallpaperBackdropStatus> {
        Ok(self.0.run_mobile_plugin::<crate::WallpaperBackdropStatus>(
            "setWallpaperBackdrop",
            json!({ "enabled": enabled, "blurPx": blur_px }),
        )?)
    }

    pub(crate) fn probe_content_uri(&self, uri: &str) -> Result<Option<crate::ContentUriMetadata>> {
        let response = self.0.run_mobile_plugin::<ContentUriProbeResponse>(
            "probeContentUri",
            json!({"contentUri": uri}),
        )?;
        match (
            response.content_uri,
            response.size_bytes,
            response.modified_at_utc_ms,
        ) {
            (None, None, None) => Ok(None),
            (Some(content_uri), size_bytes, modified_at_utc_ms) => {
                Ok(Some(crate::ContentUriMetadata {
                    content_uri,
                    size_bytes,
                    modified_at_utc_ms,
                }))
            }
            (None, _, _) => Err(crate::Error::Response(
                "Android content URI probe returned metadata without an authorized URI".to_owned(),
            )),
        }
    }

    pub(crate) fn ensure_playback_service_started(&self) -> Result<()> {
        let _: serde_json::Value = self
            .0
            .run_mobile_plugin("ensurePlaybackServiceStarted", ())?;
        Ok(())
    }

    pub(crate) fn cache_content_uri(&self, uri: &str, max_bytes: u64) -> Result<ContentCacheLease> {
        let response = self.0.run_mobile_plugin::<ContentCacheLeaseResponse>(
            "cacheContentUri",
            json!({ "contentUri": uri, "maxBytes": max_bytes }),
        )?;
        self.content_cache_lease(response, max_bytes)
    }

    pub(crate) fn cache_artwork_bytes(
        &self,
        uri: &str,
        tree_uri: Option<&str>,
        max_bytes: u64,
    ) -> Result<ContentCacheLease> {
        let response = self.0.run_mobile_plugin::<ContentCacheLeaseResponse>(
            "cacheArtworkBytes",
            json!({ "contentUri": uri, "treeUri": tree_uri, "maxBytes": max_bytes }),
        )?;
        self.content_cache_lease(response, max_bytes)
    }

    pub(crate) fn create_cache_lease(&self) -> Result<ContentCacheLease> {
        let response = self
            .0
            .run_mobile_plugin::<ContentCacheLeaseResponse>("createCacheLease", ())?;
        self.content_cache_lease(response, 0)
    }

    pub(crate) fn write_content_cache_lease(
        &self,
        lease: &ContentCacheLease,
        destination_uri: &str,
    ) -> Result<()> {
        let metadata = std::fs::metadata(lease.path())?;
        if !metadata.is_file() || metadata.len() > MAX_PLAYLIST_BYTES {
            return Err(crate::Error::Response(
                "playlist cache lease is missing or exceeds 32 MiB".to_owned(),
            ));
        }
        let _: serde_json::Value = self.0.run_mobile_plugin(
            "writeContentCacheLease",
            json!({ "token": lease.token(), "destinationUri": destination_uri }),
        )?;
        Ok(())
    }

    /// Cache a library audio document (up to 512 MiB) together with its display name, for
    /// tag editing / spectrogram rendering on a private work copy.
    pub(crate) fn cache_audio_for_edit(
        &self,
        uri: &str,
    ) -> Result<(ContentCacheLease, Option<String>)> {
        let value: serde_json::Value = self.0.run_mobile_plugin(
            "cacheContentUriNamed",
            json!({ "contentUri": uri, "maxBytes": MAX_EDIT_BYTES }),
        )?;
        let display_name = value
            .get("displayName")
            .and_then(|name| name.as_str())
            .map(str::to_owned);
        let response: ContentCacheLeaseResponse = serde_json::from_value(value)
            .map_err(|error| crate::Error::Response(error.to_string()))?;
        Ok((self.content_cache_lease(response, MAX_EDIT_BYTES)?, display_name))
    }

    /// System document picker (ACTION_OPEN_DOCUMENT). Returns (uri, display name).
    /// Photo Picker (image only), falling back to the document picker with `mime_types`.
    pub(crate) fn pick_visual_image(&self, mime_types: &[&str]) -> Result<Option<String>> {
        let value: serde_json::Value = self
            .0
            .run_mobile_plugin("pickVisualImage", json!({ "mimeTypes": mime_types }))?;
        Ok(value.get("uri").and_then(|uri| uri.as_str()).map(str::to_owned))
    }

    pub(crate) fn pick_open_document(
        &self,
        mime_types: &[&str],
    ) -> Result<Option<(String, Option<String>)>> {
        let value: serde_json::Value = self
            .0
            .run_mobile_plugin("pickOpenDocument", json!({ "mimeTypes": mime_types }))?;
        let uri = value.get("uri").and_then(|uri| uri.as_str()).map(str::to_owned);
        let name = value
            .get("displayName")
            .and_then(|name| name.as_str())
            .map(str::to_owned);
        Ok(uri.map(|uri| (uri, name)))
    }

    /// System "save as" picker (ACTION_CREATE_DOCUMENT). Returns the created document URI.
    pub(crate) fn pick_create_document(
        &self,
        suggested_name: &str,
        mime_type: &str,
    ) -> Result<Option<String>> {
        let response = self.0.run_mobile_plugin::<UriResponse>(
            "pickCreateDocument",
            json!({ "suggestedName": suggested_name, "mimeType": mime_type }),
        )?;
        Ok(response.uri)
    }

    fn content_cache_lease(
        &self,
        response: ContentCacheLeaseResponse,
        max_bytes: u64,
    ) -> Result<ContentCacheLease> {
        if !is_uuid_token(&response.token) {
            return Err(crate::Error::Response(
                "Android returned an invalid cache lease token".to_owned(),
            ));
        }
        let path = PathBuf::from(&response.path);
        let file_name = path.file_name().and_then(|value| value.to_str());
        let parent = path.parent();
        let expected_name = format!("{}.bin", response.token);
        if !path.is_absolute()
            || file_name != Some(expected_name.as_str())
            || parent
                .and_then(|value| value.file_name())
                .and_then(|value| value.to_str())
                != Some("leases")
            || parent
                .and_then(|value| value.parent())
                .and_then(|value| value.file_name())
                .and_then(|value| value.to_str())
                != Some("media-index")
        {
            return Err(crate::Error::Response(
                "Android cache lease path is outside its private lease directory".to_owned(),
            ));
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != response.size_bytes
            || metadata.len() > max_bytes
        {
            return Err(crate::Error::Response(
                "Android cache lease size or file type is invalid".to_owned(),
            ));
        }
        let handle = Arc::clone(&self.0);
        let release = Box::new(move |token: String| {
            let _: std::result::Result<serde_json::Value, _> =
                handle.run_mobile_plugin("releaseCacheLease", json!({ "token": token }));
        });
        Ok(ContentCacheLease::new(
            response.token,
            path,
            response.mime_type,
            release,
        ))
    }
}

fn parse_lofty_metadata(
    path: &std::path::Path,
    mime_type: Option<&str>,
) -> std::result::Result<TrackMetadata, TrackMetadataError> {
    let probe = Probe::open(path).map_err(|error| TrackMetadataError {
        message: format!("could not open Android audio metadata: {error}"),
    })?;
    let probe = probe
        .guess_file_type()
        .map_err(|error| TrackMetadataError {
            message: format!("could not identify Android audio format: {error}"),
        })?;
    let file_type = probe
        .file_type()
        .or_else(|| mime_file_type(mime_type))
        .ok_or_else(|| TrackMetadataError {
            message: "could not identify Android audio format".to_owned(),
        })?;
    let probe = if probe.file_type().is_some() {
        probe
    } else {
        probe.set_file_type(file_type)
    };
    let (tagged_file, codec) = if file_type == FileType::Mpeg {
        let mut reader = probe.into_inner();
        reader
            .seek(std::io::SeekFrom::Start(0))
            .map_err(|error| TrackMetadataError {
                message: format!("could not rewind Android MPEG audio: {error}"),
            })?;
        let mpeg_file = MpegFile::read_from(&mut reader, ParseOptions::new()).map_err(|error| {
            TrackMetadataError {
                message: format!("could not parse Android MPEG audio properties: {error}"),
            }
        })?;
        let codec = match *mpeg_file.properties().layer() {
            Layer::Layer1 => "MP1",
            Layer::Layer2 => "MP2",
            Layer::Layer3 => "MP3",
        };
        (TaggedFile::from(mpeg_file), codec.to_owned())
    } else {
        let tagged_file = probe.read().map_err(|error| TrackMetadataError {
            message: format!("could not parse Android audio metadata: {error}"),
        })?;
        (tagged_file, format!("{file_type:?}"))
    };
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let properties = tagged_file.properties();
    let text = |value: Option<std::borrow::Cow<'_, str>>| value.map(|value| value.into_owned());
    let release_date = tag
        .and_then(|tag| tag.get_string(ItemKey::RecordingDate))
        .and_then(player_core::parse_recording_date);
    let year = release_date
        .as_deref()
        .and_then(player_core::recording_date_year);
    Ok(TrackMetadata {
        title: tag.and_then(|tag| text(tag.title())),
        artist: tag.and_then(|tag| text(tag.artist())),
        album: tag.and_then(|tag| text(tag.album())),
        album_artist: tag.and_then(|tag| tag.get_string(ItemKey::AlbumArtist).map(str::to_owned)),
        track_number: tag.and_then(|tag| tag.track()),
        disc_number: tag.and_then(|tag| tag.disk()),
        duration_ms: Some(properties.duration().as_millis().min(u64::MAX as u128) as u64),
        codec: Some(codec),
        bitrate_bps: properties
            .audio_bitrate()
            .map(|kbps| kbps.saturating_mul(1000)),
        sample_rate_hz: properties.sample_rate(),
        release_date,
        year,
        bit_depth: if matches!(file_type, FileType::Flac | FileType::Ape) {
            properties
                .bit_depth()
                .filter(|depth| (8..=32).contains(depth))
        } else {
            None
        },
    })
}

fn mime_file_type(mime_type: Option<&str>) -> Option<FileType> {
    match mime_type?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "audio/mpeg" | "audio/mp3" => Some(FileType::Mpeg),
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "video/mp4" => Some(FileType::Mp4),
        "audio/aac" | "audio/aacp" => Some(FileType::Aac),
        "audio/flac" | "audio/x-flac" => Some(FileType::Flac),
        "audio/ogg" | "application/ogg" => Some(FileType::Vorbis),
        "audio/opus" => Some(FileType::Opus),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some(FileType::Wav),
        "audio/aiff" | "audio/x-aiff" => Some(FileType::Aiff),
        _ => None,
    }
}

fn normalize_android_codec(value: &str) -> Option<String> {
    let mime = value.split(';').next()?.trim().to_ascii_lowercase();
    let codec = match mime.as_str() {
        "audio/mp4a-latm" | "audio/aac" | "audio/aacp" => "AAC",
        "audio/mpeg" | "audio/mp3" => "MP3",
        "audio/alac" => "ALAC",
        "audio/flac" => "FLAC",
        "audio/3gpp" | "audio/amr" => "AMR",
        "audio/opus" => "OPUS",
        "audio/vorbis" => "VORBIS",
        "audio/raw" => "PCM",
        _ => return None,
    };
    Some(codec.to_owned())
}

fn has_useful_metadata(metadata: &TrackMetadata) -> bool {
    metadata.title.is_some()
        || metadata.artist.is_some()
        || metadata.album.is_some()
        || metadata.duration_ms.is_some()
        || metadata.sample_rate_hz.is_some()
        || metadata.bitrate_bps.is_some()
}

const MAX_PLAYLIST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EDIT_BYTES: u64 = 512 * 1024 * 1024;

fn is_uuid_token(token: &str) -> bool {
    token.len() == 36
        && token.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn unavailable_scan(root: &LibraryRoot, reason: &str) -> SourceScan {
    SourceScan {
        source_id: root.id,
        state: SourceScanState::Unavailable {
            reason: reason.to_owned(),
        },
        tracks: Vec::new(),
        errors: vec![MediaSourceError {
            source_item_id: None,
            message: reason.to_owned(),
        }],
    }
}
