use player_core::{
    LibraryRoot, MediaSourceError, MediaTrackRecord, SourceScan, SourceScanState, TrackMetadata,
    TrackMetadataError,
};
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{ContentCacheLease, Error, Result};

pub(crate) struct PlatformMediaIndex<R: Runtime>(std::marker::PhantomData<fn() -> R>);

impl<R: Runtime> PlatformMediaIndex<R> {
    pub(crate) fn init<C: DeserializeOwned>(
        _app: &AppHandle<R>,
        _api: PluginApi<R, C>,
    ) -> Result<Self> {
        Ok(Self(std::marker::PhantomData))
    }

    pub(crate) fn scan(&self, root: &LibraryRoot) -> SourceScan {
        SourceScan {
            source_id: root.id,
            state: SourceScanState::Unavailable {
                reason: format!(
                    "Android source {:?} cannot be scanned on this platform",
                    root.kind
                ),
            },
            tracks: Vec::new(),
            errors: vec![MediaSourceError {
                source_item_id: None,
                message: "Android MediaStore and SAF adapters are available only on Android".into(),
            }],
        }
    }

    pub(crate) fn read_metadata(
        &self,
        _record: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        Err(TrackMetadataError {
            message: "Android content URI metadata is available only on Android".into(),
        })
    }

    pub(crate) fn probe_content_uri(
        &self,
        _uri: &str,
    ) -> Result<Option<crate::ContentUriMetadata>> {
        Err(Error::Unsupported("Android content URI metadata probe"))
    }

    pub(crate) fn ensure_playback_service_started(&self) -> Result<()> {
        Err(Error::Unsupported("Android MediaSession playback service"))
    }

    pub(crate) fn cache_content_uri(
        &self,
        _uri: &str,
        _max_bytes: u64,
    ) -> Result<ContentCacheLease> {
        Err(Error::Unsupported("Android content URI cache lease"))
    }

    pub(crate) fn cache_artwork_bytes(
        &self,
        _uri: &str,
        _tree_uri: Option<&str>,
        _max_bytes: u64,
    ) -> Result<ContentCacheLease> {
        Err(Error::Unsupported("Android content URI artwork"))
    }

    pub(crate) fn create_cache_lease(&self) -> Result<ContentCacheLease> {
        Err(Error::Unsupported("Android SAF playlist export cache"))
    }

    pub(crate) fn write_content_cache_lease(
        &self,
        _lease: &ContentCacheLease,
        _destination_uri: &str,
    ) -> Result<()> {
        Err(Error::Unsupported("Android SAF playlist export"))
    }

}
