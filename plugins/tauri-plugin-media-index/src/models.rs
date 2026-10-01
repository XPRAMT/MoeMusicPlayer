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
    #[serde(default)]
    pub year: Option<u16>,
    #[serde(default)]
    pub bit_depth: Option<u8>,
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

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContentUriProbeResponse {
    pub content_uri: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at_utc_ms: Option<i64>,
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

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContentCacheLeaseResponse {
    pub token: String,
    pub path: String,
    pub size_bytes: u64,
    pub mime_type: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
pub(crate) struct UriResponse {
    pub uri: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafPlaylistImportResponse {
    pub playlist_uri: Option<String>,
    pub tree_uri: Option<String>,
    pub display_name: Option<String>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedSafPlaylistEntryResponse {
    pub content_uri: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at_utc_ms: Option<u64>,
}

#[cfg(target_os = "android")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LyricsTextResponse {
    pub text: Option<String>,
}

#[cfg(test)]
mod content_uri_probe_tests {
    use super::ContentUriProbeResponse;

    #[test]
    fn parses_readable_content_uri_fingerprint_without_copy_fields() {
        let response: ContentUriProbeResponse = serde_json::from_str(
            r#"{"contentUri":"content://provider/document/audio","sizeBytes":12345,"modifiedAtUtcMs":1800000000000}"#,
        )
        .expect("probe response should deserialize");

        assert_eq!(
            response.content_uri.as_deref(),
            Some("content://provider/document/audio")
        );
        assert_eq!(response.size_bytes, Some(12345));
        assert_eq!(response.modified_at_utc_ms, Some(1_800_000_000_000));
    }

    #[test]
    fn parses_unreadable_content_uri_as_none() {
        let response: ContentUriProbeResponse =
            serde_json::from_str(r#"{"contentUri":null,"sizeBytes":null,"modifiedAtUtcMs":null}"#)
                .expect("unreadable probe response should deserialize");

        assert!(response.content_uri.is_none());
        assert!(response.size_bytes.is_none());
        assert!(response.modified_at_utc_ms.is_none());
    }
}
