use player_core::{
    LibraryRoot, MediaSourceError, MediaTrackRecord, SourceScan, SourceScanState, TrackMetadata,
    TrackMetadataError,
};
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{Error, MediaStoreVolumes, Result};

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

    pub(crate) fn list_media_store_volumes(&self) -> Result<MediaStoreVolumes> {
        Err(Error::Unsupported("MediaStore volume enumeration"))
    }

    pub(crate) fn request_media_store_permission(&self) -> Result<bool> {
        Err(Error::Unsupported("Android media permission request"))
    }

    pub(crate) fn pick_saf_tree(&self) -> Result<Option<String>> {
        Err(Error::Unsupported("Android document-tree picker"))
    }

    pub(crate) fn has_saf_permission(&self, _uri: &str) -> Result<bool> {
        Err(Error::Unsupported("Android document-tree permissions"))
    }

    pub(crate) fn release_saf_tree(&self, _uri: &str) -> Result<()> {
        Err(Error::Unsupported("Android document-tree permissions"))
    }
}
