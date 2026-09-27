use serde::{Deserialize, Serialize};

/// A concrete external MediaStore volume. `generation` is present on API 30 and newer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStoreVolume {
    pub volume_name: String,
    pub version: Option<String>,
    pub generation: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStoreVolumes {
    pub api_level: u32,
    pub volumes: Vec<MediaStoreVolume>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanResponse {
    pub state: ScanState,
    pub tracks: Vec<TrackRecord>,
    #[serde(default)]
    pub errors: Vec<SourceError>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "camelCase")]
pub(crate) enum ScanState {
    Complete,
    Incomplete(String),
    Unavailable(String),
    PermissionRevoked(String),
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackRecord {
    pub identity: TrackIdentity,
    pub locator: String,
    pub fingerprint: Fingerprint,
    pub metadata: Option<TrackMetadata>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackIdentity {
    pub source_id: String,
    pub source_item_id: String,
    pub locator_key: Option<String>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Fingerprint {
    pub size_bytes: u64,
    pub modified_at_utc_ms: Option<i64>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub codec: Option<String>,
    pub bitrate_bps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceError {
    pub source_item_id: Option<String>,
    pub message: String,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PermissionResponse {
    pub granted: bool,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafTreeResponse {
    pub uri: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BooleanResponse {
    pub granted: bool,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataResponse {
    pub metadata: Option<TrackMetadata>,
    pub error: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VolumesResponse {
    pub api_level: u32,
    #[serde(default)]
    pub volumes: Vec<VolumeResponse>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VolumeResponse {
    pub volume_name: String,
    pub version: Option<String>,
    pub generation: Option<i64>,
}
