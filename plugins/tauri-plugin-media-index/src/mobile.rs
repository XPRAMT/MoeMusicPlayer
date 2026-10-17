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

use crate::adapter::{looks_like_tree_uri, media_store_volume, scan_from_response};
use crate::models::{
    BooleanResponse, ContentCacheLeaseResponse, ContentUriProbeResponse, LyricsTextResponse,
    MyLovePlaylistResponse, PermissionResponse, ReadMetadataResponse,
    ResolvedSafPlaylistEntryResponse, SafPlaylistImportResponse, SafTreeResponse, ScanResponse,
    UriResponse, VolumesResponse,
};
use crate::{
    ContentCacheLease, MediaStoreVolume, MediaStoreVolumes, ResolvedSafPlaylistEntry, Result,
    SafPlaylistCreate, SafPlaylistImport,
};

#[derive(Clone)]
pub(crate) struct PlatformMediaIndex<R: Runtime>(Arc<PluginHandle<R>>);

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistCreateResponse {
    uri: Option<String>,
    display_name: Option<String>,
}

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

    pub(crate) fn scan(&self, root: &LibraryRoot) -> SourceScan {
        let request = match (&root.kind, &root.locator) {
            (MediaSourceKind::AndroidMediaStore, MediaLocator::ContentUri(uri)) => {
                match media_store_volume(uri) {
                    Some(volume_name) => (
                        "scanMediaStore",
                        json!({
                            "sourceId": root.id.to_string(),
                            "volumeName": volume_name,
                        }),
                    ),
                    None => {
                        return unavailable_scan(
                            root,
                            "MediaStore source URI must identify a concrete media volume",
                        );
                    }
                }
            }
            (MediaSourceKind::AndroidSaf, MediaLocator::ContentUri(uri)) => {
                if !looks_like_tree_uri(uri) {
                    return unavailable_scan(root, "SAF source is not a document-tree URI");
                }
                (
                    "scanSafTree",
                    json!({"sourceId": root.id.to_string(), "treeUri": uri}),
                )
            }
            (MediaSourceKind::AndroidMediaStore | MediaSourceKind::AndroidSaf, _) => {
                return unavailable_scan(root, "Android source requires a content URI");
            }
            _ => return unavailable_scan(root, "source kind is not supported by Android"),
        };

        let response = self
            .0
            .run_mobile_plugin::<ScanResponse>(request.0, request.1);
        match response {
            Ok(response) => scan_from_response(root, response),
            Err(error) => unavailable_scan(root, &format!("Android source query failed: {error}")),
        }
    }

    pub(crate) fn read_metadata(
        &self,
        record: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        let MediaLocator::ContentUri(uri) = &record.locator else {
            return Err(TrackMetadataError {
                message: "Android media metadata requires a content URI".into(),
            });
        };
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

    pub(crate) fn list_media_store_volumes(&self) -> Result<MediaStoreVolumes> {
        let response = self
            .0
            .run_mobile_plugin::<VolumesResponse>("mediaStoreVolumes", ())?;
        Ok(MediaStoreVolumes {
            api_level: response.api_level,
            volumes: response
                .volumes
                .into_iter()
                .map(|volume| MediaStoreVolume {
                    volume_name: volume.volume_name,
                    version: volume.version,
                    generation: volume.generation,
                })
                .collect(),
        })
    }

    pub(crate) fn request_media_store_permission(&self) -> Result<bool> {
        let response = self
            .0
            .run_mobile_plugin::<PermissionResponse>("requestMediaReadPermission", ())?;
        Ok(response.granted)
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

    pub(crate) fn scan_file(&self, path: &str) -> Result<()> {
        self.0.run_mobile_plugin::<serde_json::Value>("scanFile", json!({ "path": path }))?;
        Ok(())
    }

    pub(crate) fn read_system_wallpaper(&self) -> Result<crate::SystemWallpaperRead> {
        Ok(self.0.run_mobile_plugin::<crate::SystemWallpaperRead>("readSystemWallpaper", json!({}))?)
    }

    pub(crate) fn list_saf_images(&self, tree_uri: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct SafImagesResponse {
            uris: Vec<String>,
        }
        let response = self
            .0
            .run_mobile_plugin::<SafImagesResponse>("listSafImages", json!({ "treeUri": tree_uri }))?;
        Ok(response.uris)
    }

    pub(crate) fn set_wallpaper_backdrop(&self, enabled: bool, blur_px: u32) -> Result<crate::WallpaperBackdropStatus> {
        Ok(self.0.run_mobile_plugin::<crate::WallpaperBackdropStatus>(
            "setWallpaperBackdrop",
            json!({ "enabled": enabled, "blurPx": blur_px }),
        )?)
    }

    pub(crate) fn pick_saf_tree(&self) -> Result<Option<String>> {
        let response = self
            .0
            .run_mobile_plugin::<SafTreeResponse>("pickSafTree", ())?;
        Ok(response.uri)
    }

    pub(crate) fn has_saf_permission(&self, uri: &str) -> Result<bool> {
        let response = self
            .0
            .run_mobile_plugin::<BooleanResponse>("hasSafPermission", json!({"treeUri": uri}))?;
        Ok(response.granted)
    }

    pub(crate) fn has_saf_write_permission(&self, uri: &str) -> Result<bool> {
        let response = self.0.run_mobile_plugin::<BooleanResponse>(
            "hasSafWritePermission",
            json!({"treeUri": uri}),
        )?;
        Ok(response.granted)
    }

    pub(crate) fn pick_my_love_folder(&self) -> Result<Option<String>> {
        let response = self
            .0
            .run_mobile_plugin::<SafTreeResponse>("pickMyLoveFolder", ())?;
        Ok(response.uri)
    }

    pub(crate) fn pick_playlist_create(
        &self,
        suggested_name: &str,
    ) -> Result<Option<SafPlaylistCreate>> {
        let Some(tree_uri) = self.pick_my_love_folder()? else {
            return Ok(None);
        };
        let response = self.0.run_mobile_plugin::<PlaylistCreateResponse>(
            "pickPlaylistCreate",
            json!({ "treeUri": tree_uri, "suggestedName": suggested_name }),
        )?;
        Ok(response.uri.map(|playlist_uri| SafPlaylistCreate {
            playlist_uri,
            tree_uri,
            display_name: response
                .display_name
                .unwrap_or_else(|| suggested_name.to_owned()),
        }))
    }

    pub(crate) fn ensure_my_love_playlist(&self, tree_uri: &str) -> Result<(String, String)> {
        let response = self.0.run_mobile_plugin::<MyLovePlaylistResponse>(
            "ensureMyLovePlaylist",
            json!({"treeUri": tree_uri}),
        )?;
        Ok((response.playlist_uri, response.display_name))
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

    pub(crate) fn release_saf_tree(&self, uri: &str) -> Result<()> {
        let _: serde_json::Value = self
            .0
            .run_mobile_plugin("releaseSafTree", json!({"treeUri": uri}))?;
        Ok(())
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

    /// Write an edited audio work copy back over its document. Android asks the user for
    /// MediaStore write consent (createWriteRequest / RecoverableSecurityException) when needed;
    /// a refusal surfaces as an error containing `WRITE_ACCESS_DENIED`.
    pub(crate) fn write_audio_cache_lease(
        &self,
        lease: &ContentCacheLease,
        destination_uri: &str,
    ) -> Result<()> {
        let metadata = std::fs::metadata(lease.path())?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EDIT_BYTES {
            return Err(crate::Error::Response(
                "audio cache lease is missing, empty or exceeds 512 MiB".to_owned(),
            ));
        }
        let _: serde_json::Value = self.0.run_mobile_plugin(
            "writeAudioCacheLease",
            json!({ "token": lease.token(), "destinationUri": destination_uri }),
        )?;
        Ok(())
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

    pub(crate) fn pick_playlist_import(&self) -> Result<Option<SafPlaylistImport>> {
        let response = self
            .0
            .run_mobile_plugin::<SafPlaylistImportResponse>("pickPlaylistImport", ())?;
        match (
            response.playlist_uri,
            response.tree_uri,
            response.display_name,
        ) {
            (Some(playlist_uri), Some(tree_uri), Some(display_name)) => {
                Ok(Some(SafPlaylistImport {
                    playlist_uri,
                    tree_uri,
                    display_name,
                }))
            }
            (None, None, None) => Ok(None),
            _ => Err(crate::Error::Response(
                "Android playlist picker returned an incomplete SAF grant".to_owned(),
            )),
        }
    }

    pub(crate) fn resolve_saf_playlist_entry(
        &self,
        tree_uri: &str,
        playlist_uri: &str,
        locator: &str,
    ) -> Result<Option<ResolvedSafPlaylistEntry>> {
        let response = self
            .0
            .run_mobile_plugin::<ResolvedSafPlaylistEntryResponse>(
                "resolveSafPlaylistEntry",
                json!({ "treeUri": tree_uri, "playlistUri": playlist_uri, "locator": locator }),
            )?;
        match response.content_uri {
            Some(content_uri) => Ok(Some(ResolvedSafPlaylistEntry {
                content_uri,
                size_bytes: response.size_bytes,
                modified_at_utc_ms: response.modified_at_utc_ms,
            })),
            None if response.size_bytes.is_none() && response.modified_at_utc_ms.is_none() => {
                Ok(None)
            }
            None => Err(crate::Error::Response(
                "Android playlist resolver returned fingerprint data without a URI".to_owned(),
            )),
        }
    }

    pub(crate) fn pick_playlist_export(&self, suggested_name: &str) -> Result<Option<String>> {
        let name = std::path::Path::new(suggested_name)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| {
                let extension = value.rsplit_once('.').map(|(_, extension)| extension);
                matches!(extension, Some("m3u" | "m3u8"))
            })
            .unwrap_or("playlist.m3u8");
        let response = self.0.run_mobile_plugin::<UriResponse>(
            "pickPlaylistExport",
            json!({ "suggestedName": name }),
        )?;
        Ok(response.uri)
    }

    pub(crate) fn read_saf_lyric_sibling(
        &self,
        tree_uri: &str,
        audio_uri: &str,
        max_bytes: u64,
    ) -> Result<Option<String>> {
        if max_bytes == 0 || max_bytes > MAX_LYRICS_BYTES {
            return Err(crate::Error::Response(
                "lyric sidecar cap must be between 1 byte and 2 MiB".to_owned(),
            ));
        }
        let response = self.0.run_mobile_plugin::<LyricsTextResponse>(
            "readSafLyricSibling",
            json!({ "treeUri": tree_uri, "audioUri": audio_uri, "maxBytes": max_bytes }),
        )?;
        Ok(response.text)
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
const MAX_LYRICS_BYTES: u64 = 2 * 1024 * 1024;

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
