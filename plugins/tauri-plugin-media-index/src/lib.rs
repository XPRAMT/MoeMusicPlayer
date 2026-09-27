use player_core::{
    LibraryRoot, MediaIndex as CoreMediaIndex, MediaTrackRecord, SourceScan, TrackMetadata,
    TrackMetadataError,
};
use tauri::{Manager, Runtime, plugin::Builder, plugin::TauriPlugin};

#[cfg(any(target_os = "android", test))]
mod adapter;
#[cfg(not(target_os = "android"))]
mod desktop;
mod error;
#[cfg(target_os = "android")]
mod mobile;
mod models;

pub use error::{Error, Result};
pub use models::{MediaStoreVolume, MediaStoreVolumes};

#[cfg(not(target_os = "android"))]
use desktop::PlatformMediaIndex;
#[cfg(target_os = "android")]
use mobile::PlatformMediaIndex;

/// Tauri-managed Android media source adapter.
pub struct MediaIndex<R: Runtime> {
    platform: PlatformMediaIndex<R>,
}

/// Access the media-index adapter from an `App`, `AppHandle`, or `Window`.
pub trait MediaIndexExt<R: Runtime> {
    fn media_index(&self) -> &MediaIndex<R>;
}

impl<R: Runtime, T: Manager<R>> MediaIndexExt<R> for T {
    fn media_index(&self) -> &MediaIndex<R> {
        self.state::<MediaIndex<R>>().inner()
    }
}

impl<R: Runtime> MediaIndex<R> {
    /// Enumerate a configured source. Only `SourceScanState::Complete` authorizes reconciliation.
    pub fn scan(&self, root: &LibraryRoot) -> SourceScan {
        self.platform.scan(root)
    }

    /// Read metadata from a content URI when source-indexed metadata is insufficient.
    pub fn read_metadata(
        &self,
        record: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        self.platform.read_metadata(record)
    }

    /// Let existing Core orchestration consume this adapter through its platform trait.
    pub fn with_adapter<T>(&self, operation: impl FnOnce(&mut dyn CoreMediaIndex) -> T) -> T {
        operation(&mut Adapter { index: self })
    }

    /// List concrete external MediaStore volumes. Generation is available on API 30+.
    pub fn list_media_store_volumes(&self) -> Result<MediaStoreVolumes> {
        self.platform.list_media_store_volumes()
    }

    /// Request the narrow audio-reading permission required by MediaStore.
    pub fn request_media_store_permission(&self) -> Result<bool> {
        self.platform.request_media_store_permission()
    }

    /// Show the Android document-tree picker and persist its read grant.
    pub fn pick_saf_tree(&self) -> Result<Option<String>> {
        self.platform.pick_saf_tree()
    }

    /// Check that Android still holds a persisted grant for this tree URI.
    pub fn has_saf_permission(&self, uri: &str) -> Result<bool> {
        self.platform.has_saf_permission(uri)
    }

    /// Release a persisted grant when the user removes a SAF source.
    pub fn release_saf_tree(&self, uri: &str) -> Result<()> {
        self.platform.release_saf_tree(uri)
    }
}

struct Adapter<'a, R: Runtime> {
    index: &'a MediaIndex<R>,
}

impl<R: Runtime> CoreMediaIndex for Adapter<'_, R> {
    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        self.index.scan(root)
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        self.index.read_metadata(track)
    }
}

/// Initializes the media-index plugin. Android behavior is provided by MediaStore and SAF.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("media-index")
        .setup(|app, api| {
            let platform = PlatformMediaIndex::init(app, api)?;
            app.manage(MediaIndex { platform });
            Ok(())
        })
        .build()
}
