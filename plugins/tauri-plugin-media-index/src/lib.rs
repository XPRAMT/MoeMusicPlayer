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
#[cfg(any(target_os = "android", all(test, unix)))]
pub mod fs_scan;
#[cfg(target_os = "android")]
mod mobile;
mod models;

pub use error::{Error, Result};

/// Fingerprint metadata from an authorized Android content URI without copying its media bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentUriMetadata {
    pub content_uri: String,
    pub size_bytes: Option<u64>,
    pub modified_at_utc_ms: Option<i64>,
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

/// The "read images" permission some systems want before they hand over the wallpaper.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaImagesPermission {
    pub granted: bool,
    /// The system's prompt could not be shown, so the app's settings page was opened instead.
    #[serde(default)]
    pub opened_settings: bool,
}

/// Static wallpaper copy (path) or why there is none ("permission" | "mediaImages" | "live" | "unavailable").
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
        self.platform.scan(root, &[])
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
        self.with_adapter_excluding(&[], operation)
    }

    /// Like [`Self::with_adapter`], but folder scans skip `excluded` real-path folders
    /// (「排除資料夾」) and everything under them.
    pub fn with_adapter_excluding<T>(
        &self,
        excluded: &[std::path::PathBuf],
        operation: impl FnOnce(&mut dyn CoreMediaIndex) -> T,
    ) -> T {
        operation(&mut Adapter { index: self, excluded })
    }

    /// MANAGE_EXTERNAL_STORAGE ("All files access") state.
    #[cfg(target_os = "android")]
    pub fn all_files_access_status(&self) -> Result<AllFilesAccessStatus> {
        self.platform.all_files_access_status()
    }

    /// Whether the "read images" permission is held.
    #[cfg(target_os = "android")]
    pub fn media_images_permission_status(&self) -> Result<MediaImagesPermission> {
        self.platform.media_images_permission_status()
    }

    /// Ask for the "read images" permission (or open the app's settings when the prompt cannot be shown).
    #[cfg(target_os = "android")]
    pub fn request_media_images_permission(&self) -> Result<MediaImagesPermission> {
        self.platform.request_media_images_permission()
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

    /// Whether the clipboard holds an image (clip description only; no data is read).
    #[cfg(target_os = "android")]
    pub fn clipboard_has_image(&self) -> Result<bool> {
        self.platform.clipboard_has_image()
    }

    /// Copy the clipboard image into the app cache; returns `Ok(path)`, or `Err(reason)` without
    /// an image (`noClip`: nothing readable, e.g. not focused; `noImage`).
    #[cfg(target_os = "android")]
    pub fn read_clipboard_image(&self) -> Result<std::result::Result<String, String>> {
        self.platform.read_clipboard_image()
    }

    /// Private cache folder whose image files `write_clipboard_image` may put on the clipboard.
    #[cfg(target_os = "android")]
    pub fn clipboard_share_directory(&self) -> Result<std::path::PathBuf> {
        self.platform.clipboard_share_directory()
    }

    /// Put an image file (inside `clipboard_share_directory`) on the system clipboard as a
    /// FileProvider content URI with its image MIME type.
    #[cfg(target_os = "android")]
    pub fn write_clipboard_image(&self, path: &str) -> Result<()> {
        self.platform.write_clipboard_image(path)
    }

    /// Best-effort: show a real-path folder in a file manager. False when no app accepted it.
    #[cfg(target_os = "android")]
    pub fn open_folder_in_file_manager(&self, path: &str) -> Result<bool> {
        self.platform.open_folder_in_file_manager(path)
    }

    /// App-private folder for update APK downloads (exposed to the installer via FileProvider).
    #[cfg(target_os = "android")]
    pub fn update_download_directory(&self) -> Result<std::path::PathBuf> {
        self.platform.update_download_directory()
    }

    /// Hand an APK under [`Self::update_download_directory`] to the system installer.
    /// Returns `started`, or `permission` after opening the "install unknown apps" page.
    #[cfg(target_os = "android")]
    pub fn install_apk(&self, path: &str) -> Result<String> {
        self.platform.install_apk(path)
    }

    /// Folder picker yielding a real path; the picker grant is not persisted (ADR 0014).
    #[cfg(target_os = "android")]
    pub fn pick_directory_path(&self) -> Result<Option<String>> {
        self.platform.pick_directory_path()
    }

    /// Document picker yielding a real file path; the picker grant is not persisted.
    #[cfg(target_os = "android")]
    pub fn pick_document_path(&self, mime_types: &[&str]) -> Result<Option<String>> {
        self.platform.pick_document_path(mime_types)
    }

    /// "Save as" picker yielding the created file's real path; the grant is not persisted.
    #[cfg(target_os = "android")]
    pub fn pick_create_document_path(&self, suggested_name: &str, mime_type: &str) -> Result<Option<String>> {
        self.platform.pick_create_document_path(suggested_name, mime_type)
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

    /// Composite the system wallpaper behind the transparent WebView (FLAG_SHOW_WALLPAPER).
    #[cfg(target_os = "android")]
    pub fn set_wallpaper_backdrop(&self, enabled: bool, blur_px: u32) -> Result<WallpaperBackdropStatus> {
        self.platform.set_wallpaper_backdrop(enabled, blur_px)
    }

    /// Probe an Android content URI and return available size/mtime without copying its contents.
    pub fn probe_content_uri(&self, uri: &str) -> Result<Option<ContentUriMetadata>> {
        self.platform.probe_content_uri(uri)
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

    /// Stream a file of the app cache folder into a user-selected document (backup export).
    pub fn copy_file_to_uri(&self, source: &std::path::Path, destination_uri: &str) -> Result<()> {
        self.platform.copy_file_to_uri(source, destination_uri)
    }

    /// Copy a user-selected document into a file of the app cache folder (backup import).
    pub fn copy_uri_to_file(
        &self,
        source_uri: &str,
        destination: &std::path::Path,
        max_bytes: u64,
    ) -> Result<u64> {
        self.platform.copy_uri_to_file(source_uri, destination, max_bytes)
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

    /// Cache a library audio document with its display name (track details / spectrogram).
    #[cfg(target_os = "android")]
    pub fn cache_audio_for_edit(&self, uri: &str) -> Result<(ContentCacheLease, Option<String>)> {
        self.platform.cache_audio_for_edit(uri)
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

}

struct Adapter<'a, R: Runtime> {
    index: &'a MediaIndex<R>,
    excluded: &'a [std::path::PathBuf],
}

impl<R: Runtime> CoreMediaIndex for Adapter<'_, R> {
    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        self.index.platform.scan(root, self.excluded)
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        self.index.read_metadata(track)
    }

    fn scans_lyric_sidecars(&self) -> bool {
        cfg!(target_os = "android")
    }

    fn take_lyric_sidecars(&mut self) -> Vec<player_core::LyricSidecar> {
        #[cfg(target_os = "android")]
        {
            fs_scan::take_sidecars()
        }
        #[cfg(not(target_os = "android"))]
        {
            Vec::new()
        }
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
