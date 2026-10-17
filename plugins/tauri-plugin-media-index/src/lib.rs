use player_core::{
    LibraryRoot, MediaIndex as CoreMediaIndex, MediaTrackRecord, SourceScan, TrackMetadata,
    TrackMetadataError,
};
use std::path::{Path, PathBuf};
use tauri::{Manager, Runtime, plugin::Builder, plugin::TauriPlugin};

#[cfg(any(target_os = "android", test))]
mod adapter;
#[cfg(any(target_os = "android", test))]
mod android_metadata;
#[cfg(not(target_os = "android"))]
mod desktop;
mod error;
#[cfg(target_os = "android")]
mod mobile;
mod models;

pub use error::{Error, Result};
pub use models::{MediaStoreVolume, MediaStoreVolumes};

/// A playlist document selected from a persisted SAF tree, allowing relative M3U entries to be resolved safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafPlaylistImport {
    pub playlist_uri: String,
    pub tree_uri: String,
    pub display_name: String,
}

/// A newly created M3U8 document inside a persisted read/write SAF tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafPlaylistCreate {
    pub playlist_uri: String,
    pub tree_uri: String,
    pub display_name: String,
}

/// Fingerprint metadata from an authorized Android content URI without copying its media bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentUriMetadata {
    pub content_uri: String,
    pub size_bytes: Option<u64>,
    pub modified_at_utc_ms: Option<i64>,
}

/// A playlist-relative document resolved within its persisted SAF tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSafPlaylistEntry {
    pub content_uri: String,
    pub size_bytes: Option<u64>,
    pub modified_at_utc_ms: Option<u64>,
}

/// An app-private temporary copy of an authorized Android content URI.
/// The path is available only to Rust callers; dropping the lease releases the cache token.
pub struct ContentCacheLease {
    token: String,
    path: PathBuf,
    release: Option<Box<dyn FnOnce(String)>>,
    mime_type: Option<String>,
}

/// Android All files access state.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllFilesAccessStatus {
    pub supported: bool,
    /// Android 10 or older: legacy READ/WRITE_EXTERNAL_STORAGE runtime permission.
    #[serde(default)]
    pub legacy: bool,
    pub granted: bool,
}

/// Static wallpaper copy (path) or why there is none ("permission" | "live" | "unavailable").
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemWallpaperRead {
    pub path: Option<String>,
    pub reason: Option<String>,
}

/// Result of toggling the Android wallpaper backdrop.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperBackdropStatus {
    pub active: bool,
    pub blur_applied: bool,
    pub blur_supported: bool,
}

impl ContentCacheLease {
    #[cfg(target_os = "android")]
    pub(crate) fn new(
        token: String,
        path: PathBuf,
        mime_type: Option<String>,
        release: Box<dyn FnOnce(String)>,
    ) -> Self {
        Self {
            token,
            path,
            mime_type,
            release: Some(release),
        }
    }

    /// The lease file is private to the Android app cache and is deleted when this value drops.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// MIME type reported by ContentResolver for the leased original bytes, if available.
    pub fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }

    #[cfg(target_os = "android")]
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for ContentCacheLease {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release(self.token.clone());
        }
    }
}

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
    /// Open a Rust-validated official HTTPS link in Android's browser.
    #[cfg(target_os = "android")]
    pub fn open_official_url(&self, url: &str) -> Result<()> {
        self.platform.open_official_url(url)
    }
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

    /// MANAGE_EXTERNAL_STORAGE ("All files access") state.
    #[cfg(target_os = "android")]
    pub fn all_files_access_status(&self) -> Result<AllFilesAccessStatus> {
        self.platform.all_files_access_status()
    }

    /// Open the system All files access page for this app.
    #[cfg(target_os = "android")]
    pub fn request_all_files_access(&self) -> Result<()> {
        self.platform.request_all_files_access()
    }

    /// Real file path for a content URI when All files access is granted and the file exists.
    #[cfg(target_os = "android")]
    pub fn resolve_direct_path(&self, uri: &str) -> Result<Option<String>> {
        self.platform.resolve_direct_path(uri)
    }

    /// Ask MediaStore to rescan a file changed through its direct path.
    #[cfg(target_os = "android")]
    pub fn scan_file(&self, path: &str) -> Result<()> {
        self.platform.scan_file(path)
    }

    /// Copy the static system wallpaper into the app cache (needs All files access on Android 13+).
    #[cfg(target_os = "android")]
    pub fn read_system_wallpaper(&self) -> Result<SystemWallpaperRead> {
        self.platform.read_system_wallpaper()
    }

    /// Recursive, path-sorted image document URIs inside a persisted SAF tree (slideshow source).
    #[cfg(target_os = "android")]
    pub fn list_saf_images(&self, tree_uri: &str) -> Result<Vec<String>> {
        self.platform.list_saf_images(tree_uri)
    }

    /// Composite the system wallpaper behind the transparent WebView (FLAG_SHOW_WALLPAPER).
    #[cfg(target_os = "android")]
    pub fn set_wallpaper_backdrop(&self, enabled: bool, blur_px: u32) -> Result<WallpaperBackdropStatus> {
        self.platform.set_wallpaper_backdrop(enabled, blur_px)
    }

    /// Show the Android document-tree picker and persist its read grant.
    pub fn pick_saf_tree(&self) -> Result<Option<String>> {
        self.platform.pick_saf_tree()
    }

    /// Check that Android still holds a persisted grant for this tree URI.
    pub fn has_saf_permission(&self, uri: &str) -> Result<bool> {
        self.platform.has_saf_permission(uri)
    }

    /// Check that Android still holds a persisted SAF write grant for this tree URI.
    pub fn has_saf_write_permission(&self, uri: &str) -> Result<bool> {
        self.platform.has_saf_write_permission(uri)
    }

    /// Open the SAF folder picker, initially pointing at the shared Music folder when available.
    pub fn pick_my_love_folder(&self) -> Result<Option<String>> {
        self.platform.pick_my_love_folder()
    }

    /// Create or find the app-owned MyLove.m3u8 document within a granted tree.
    pub fn ensure_my_love_playlist(&self, tree_uri: &str) -> Result<(String, String)> {
        self.platform.ensure_my_love_playlist(tree_uri)
    }

    /// Probe an Android content URI and return available size/mtime without copying its contents.
    pub fn probe_content_uri(&self, uri: &str) -> Result<Option<ContentUriMetadata>> {
        self.platform.probe_content_uri(uri)
    }

    /// Release a persisted grant when the user removes a SAF source.
    pub fn release_saf_tree(&self, uri: &str) -> Result<()> {
        self.platform.release_saf_tree(uri)
    }

    /// Start the single Android MediaSessionService without coupling its lifecycle to the activity.
    pub fn ensure_playback_service_started(&self) -> Result<()> {
        self.platform.ensure_playback_service_started()
    }

    /// Copy a content URI to a bounded private cache lease for a Rust parser.
    pub fn cache_content_uri(&self, uri: &str, max_bytes: u64) -> Result<ContentCacheLease> {
        self.platform.cache_content_uri(uri, max_bytes)
    }

    /// Extract embedded compressed artwork bytes into a bounded cache lease without decoding or re-encoding.
    pub fn cache_artwork_bytes(
        &self,
        uri: &str,
        tree_uri: Option<&str>,
        max_bytes: u64,
    ) -> Result<ContentCacheLease> {
        self.platform.cache_artwork_bytes(uri, tree_uri, max_bytes)
    }

    /// Create an empty cache lease for Rust to populate before a SAF export.
    pub fn create_cache_lease(&self) -> Result<ContentCacheLease> {
        self.platform.create_cache_lease()
    }

    /// Stream a Rust-generated cache lease into a user-selected SAF document.
    pub fn write_content_cache_lease(
        &self,
        lease: &ContentCacheLease,
        destination_uri: &str,
    ) -> Result<()> {
        self.platform
            .write_content_cache_lease(lease, destination_uri)
    }

    /// Let the user choose an M3U/M3U8 document inside a persisted SAF tree.
    pub fn pick_playlist_import(&self) -> Result<Option<SafPlaylistImport>> {
        self.platform.pick_playlist_import()
    }

    /// Resolve one relative playlist entry only within the selected, persisted SAF tree.
    pub fn resolve_saf_playlist_entry(
        &self,
        tree_uri: &str,
        playlist_uri: &str,
        locator: &str,
    ) -> Result<Option<ResolvedSafPlaylistEntry>> {
        self.platform
            .resolve_saf_playlist_entry(tree_uri, playlist_uri, locator)
    }

    /// Let the user choose an M3U/M3U8 document destination for export.
    pub fn pick_playlist_export(&self, suggested_name: &str) -> Result<Option<String>> {
        self.platform.pick_playlist_export(suggested_name)
    }

    /// Ask the user to choose a destination folder and create a new M3U8 there.
    #[cfg(target_os = "android")]
    pub fn pick_playlist_create(&self, suggested_name: &str) -> Result<Option<SafPlaylistCreate>> {
        self.platform.pick_playlist_create(suggested_name)
    }

    /// Cache a library audio document with its display name (track details / spectrogram).
    #[cfg(target_os = "android")]
    pub fn cache_audio_for_edit(&self, uri: &str) -> Result<(ContentCacheLease, Option<String>)> {
        self.platform.cache_audio_for_edit(uri)
    }

    /// Write an edited audio work copy back over its document (may prompt for write consent).
    #[cfg(target_os = "android")]
    pub fn write_audio_cache_lease(&self, lease: &ContentCacheLease, destination_uri: &str) -> Result<()> {
        self.platform.write_audio_cache_lease(lease, destination_uri)
    }

    /// Android Photo Picker (image only); falls back to the document picker.
    #[cfg(target_os = "android")]
    pub fn pick_visual_image(&self, mime_types: &[&str]) -> Result<Option<String>> {
        self.platform.pick_visual_image(mime_types)
    }

    /// System document picker; returns (uri, display name).
    #[cfg(target_os = "android")]
    pub fn pick_open_document(&self, mime_types: &[&str]) -> Result<Option<(String, Option<String>)>> {
        self.platform.pick_open_document(mime_types)
    }

    /// System "save as" document picker; returns the created document URI.
    #[cfg(target_os = "android")]
    pub fn pick_create_document(&self, suggested_name: &str, mime_type: &str) -> Result<Option<String>> {
        self.platform.pick_create_document(suggested_name, mime_type)
    }

    /// Read an adjacent LRC only when both URIs are inside the same persisted SAF tree.
    pub fn read_saf_lyric_sibling(
        &self,
        tree_uri: &str,
        audio_uri: &str,
        max_bytes: u64,
    ) -> Result<Option<String>> {
        self.platform
            .read_saf_lyric_sibling(tree_uri, audio_uri, max_bytes)
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
