use player_core::{
    LibraryRoot, MediaLocator, MediaSourceError, MediaTrackRecord, SourceScan, SourceScanState,
    TrackIdentity, TrackMetadata,
};

use crate::models::{ScanResponse, ScanState};

pub(crate) fn parse_recording_date_year(value: &str) -> Option<u16> {
    let bytes = value.as_bytes();
    let year = parse_four_digit_year(bytes.get(..4)?)?;
    match bytes.len() {
        4 => Some(year),
        10 if bytes[4] == b'-' && bytes[7] == b'-' => {
            let month = parse_two_digits(&bytes[5..7])?;
            let day = parse_two_digits(&bytes[8..10])?;
            let days_in_month = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
                2 => 28,
                _ => return None,
            };
            (1..=days_in_month).contains(&day).then_some(year)
        }
        _ => None,
    }
}

fn parse_four_digit_year(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 4 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = std::str::from_utf8(bytes).ok()?.parse::<u16>().ok()?;
    (year != 0).then_some(year)
}

fn parse_two_digits(bytes: &[u8]) -> Option<u8> {
    if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(crate) fn scan_from_response(root: &LibraryRoot, response: ScanResponse) -> SourceScan {
    let mut state = match response.state {
        ScanState::Complete => SourceScanState::Complete,
        ScanState::Incomplete(reason) => SourceScanState::Incomplete { reason },
        ScanState::Unavailable(reason) => SourceScanState::Unavailable { reason },
        ScanState::PermissionRevoked(reason) => SourceScanState::PermissionRevoked { reason },
    };
    let mut errors: Vec<MediaSourceError> = response
        .errors
        .into_iter()
        .map(|error| MediaSourceError {
            source_item_id: error.source_item_id,
            message: error.message,
        })
        .collect();
    let mut tracks = Vec::with_capacity(response.tracks.len());

    for record in response.tracks {
        let returned_source_id = match player_core::SourceId::parse(&record.identity.source_id) {
            Ok(source_id) if source_id == root.id => source_id,
            _ => {
                errors.push(MediaSourceError {
                    source_item_id: Some(record.identity.source_item_id),
                    message: "Android adapter returned an invalid or mismatched source ID".into(),
                });
                state = incomplete_from(state, "one or more records had invalid identity data");
                continue;
            }
        };
        tracks.push(MediaTrackRecord {
            identity: TrackIdentity {
                source_id: returned_source_id,
                source_item_id: record.identity.source_item_id,
                locator_key: record.identity.locator_key,
            },
            locator: MediaLocator::ContentUri(record.locator),
            fingerprint: player_core::FileFingerprint {
                size_bytes: record.fingerprint.size_bytes,
                modified_at_utc_ms: record.fingerprint.modified_at_utc_ms,
            },
            metadata: record.metadata.map(into_core_metadata),
        });
    }

    if !errors.is_empty() {
        state = incomplete_from(state, "one or more records could not be read");
    }

    SourceScan {
        source_id: root.id,
        state,
        tracks,
        errors,
    }
}

fn incomplete_from(state: SourceScanState, fallback_reason: &str) -> SourceScanState {
    match state {
        SourceScanState::Complete => SourceScanState::Incomplete {
            reason: fallback_reason.to_owned(),
        },
        other => other,
    }
}

pub(crate) fn into_core_metadata(metadata: crate::models::TrackMetadata) -> TrackMetadata {
    TrackMetadata {
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
        year: metadata.year,
        bit_depth: metadata.bit_depth,
    }
}

pub(crate) fn media_store_volume(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("content://media/")?;
    let (volume, suffix) = rest.split_once('/')?;
    if volume.is_empty() || !suffix.starts_with("audio/media") {
        return None;
    }
    Some(volume)
}

pub(crate) fn looks_like_tree_uri(uri: &str) -> bool {
    uri.starts_with("content://") && uri.contains("/tree/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use player_core::{MediaSourceKind, SourceId};

    fn root(id: SourceId, kind: MediaSourceKind, locator: &str) -> LibraryRoot {
        LibraryRoot {
            id,
            kind,
            display_name: "test source".into(),
            locator: MediaLocator::ContentUri(locator.into()),
            enabled: true,
        }
    }

    #[test]
    fn recording_date_year_accepts_valid_year_or_calendar_date_only() {
        assert_eq!(parse_recording_date_year("2024"), Some(2024));
        assert_eq!(parse_recording_date_year("2000-02-29"), Some(2000));
        assert_eq!(parse_recording_date_year("1900-02-29"), None);
        assert_eq!(parse_recording_date_year("2024-02-30"), None);
        assert_eq!(parse_recording_date_year("2024-13-01"), None);
        assert_eq!(parse_recording_date_year("0000"), None);
        assert_eq!(parse_recording_date_year("2024-01"), None);
        assert_eq!(parse_recording_date_year("not-a-date"), None);
    }

    #[test]
    fn extracts_only_concrete_media_store_audio_collection_volume() {
        assert_eq!(
            media_store_volume("content://media/external_primary/audio/media"),
            Some("external_primary")
        );
        assert_eq!(
            media_store_volume("content://media/0123-4567/audio/media"),
            Some("0123-4567")
        );
        assert_eq!(
            media_store_volume("content://media/external/audio/albums"),
            None
        );
        assert_eq!(
            media_store_volume("content://other/external_primary/audio/media"),
            None
        );
        assert_eq!(
            media_store_volume("content://media/VOLUME/audio/media/12"),
            Some("VOLUME")
        );
    }

    #[test]
    fn android_metadata_wire_fields_are_optional_and_preserve_year_and_bit_depth() {
        let legacy: crate::models::TrackMetadata = serde_json::from_value(serde_json::json!({
            "title": "legacy",
            "artist": "ARTIST",
            "album": null,
            "albumArtist": null,
            "trackNumber": null,
            "discNumber": null,
            "durationMs": 1234,
            "codec": "FLAC",
            "bitrateBps": null,
            "sampleRateHz": 96000
        }))
        .expect("older Android payload remains readable");
        let converted = into_core_metadata(legacy);
        assert_eq!(converted.artist.as_deref(), Some("ARTIST"));
        assert_eq!(converted.year, None);
        assert_eq!(converted.bit_depth, None);

        let current: crate::models::TrackMetadata = serde_json::from_value(serde_json::json!({
            "title": "hires",
            "artist": "ARTIST",
            "album": null,
            "albumArtist": null,
            "trackNumber": null,
            "discNumber": null,
            "durationMs": 1234,
            "codec": "FLAC",
            "bitrateBps": null,
            "sampleRateHz": 96000,
            "year": 2024,
            "bitDepth": 24
        }))
        .expect("new Android metadata payload");
        let converted = into_core_metadata(current);
        assert_eq!(converted.year, Some(2024));
        assert_eq!(converted.bit_depth, Some(24));
    }

    #[test]
    fn accepts_document_tree_uri_but_not_document_or_arbitrary_content_uri() {
        assert!(looks_like_tree_uri(
            "content://com.android.externalstorage.documents/tree/primary%3AMusic"
        ));
        assert!(!looks_like_tree_uri(
            "content://com.android.externalstorage.documents/document/primary%3AMusic%2Fsong.mp3"
        ));
        assert!(!looks_like_tree_uri("file:///storage/emulated/0/Music"));
    }

    #[test]
    fn complete_result_converts_media_identity_locator_fingerprint_and_metadata() {
        let source = SourceId::new();
        let wire = serde_json::json!({
            "state": { "kind": "complete" },
            "tracks": [{
                "identity": {
                    "sourceId": source.to_string(),
                    "sourceItemId": "external_primary:17",
                    "locatorKey": null
                },
                "locator": "content://media/external_primary/audio/media/17",
                "fingerprint": { "sizeBytes": 12, "modifiedAtUtcMs": 1000 },
                "metadata": { "title": "Song", "artist": null }
            }],
            "errors": []
        });
        let root = root(
            source,
            MediaSourceKind::AndroidMediaStore,
            "content://media/external_primary/audio/media",
        );
        let response: ScanResponse = serde_json::from_value(wire).unwrap();
        let scan = scan_from_response(&root, response);
        assert_eq!(scan.state, SourceScanState::Complete);
        assert_eq!(scan.tracks.len(), 1);
        assert_eq!(scan.tracks[0].identity.source_id, source);
        assert_eq!(
            scan.tracks[0].identity.source_item_id,
            "external_primary:17"
        );
        assert_eq!(scan.tracks[0].identity.locator_key, None);
        assert_eq!(
            scan.tracks[0].locator,
            MediaLocator::ContentUri("content://media/external_primary/audio/media/17".into())
        );
        assert_eq!(scan.tracks[0].fingerprint.size_bytes, 12);
        assert_eq!(scan.tracks[0].fingerprint.modified_at_utc_ms, Some(1000));
        assert_eq!(
            scan.tracks[0].metadata.as_ref().unwrap().title.as_deref(),
            Some("Song")
        );
    }

    #[test]
    fn source_mismatch_and_per_item_error_prevent_deletion_reconciliation() {
        let source = SourceId::new();
        let root = root(
            source,
            MediaSourceKind::AndroidSaf,
            "content://provider/tree/tree-id/document/tree-id",
        );
        let other_source = SourceId::new();
        let wire = serde_json::json!({
            "state": { "kind": "complete" },
            "tracks": [{
                "identity": { "sourceId": other_source.to_string(), "sourceItemId": "document:1" },
                "locator": "content://provider/tree/tree-id/document/1",
                "fingerprint": { "sizeBytes": 9 },
                "metadata": null
            }],
            "errors": []
        });
        let response: ScanResponse = serde_json::from_value(wire).unwrap();
        let scan = scan_from_response(&root, response);
        assert!(matches!(scan.state, SourceScanState::Incomplete { .. }));
        assert!(!scan.state.allows_reconciliation());
        assert_eq!(scan.tracks.len(), 0);
        assert_eq!(scan.errors.len(), 1);
    }

    #[test]
    fn permission_revoked_is_preserved_and_never_reconciles() {
        let source = SourceId::new();
        let root = root(
            source,
            MediaSourceKind::AndroidSaf,
            "content://provider/tree/tree-id/document/tree-id",
        );
        let wire = serde_json::json!({
            "state": { "kind": "permissionRevoked", "reason": "grant missing" },
            "tracks": [],
            "errors": [{ "sourceItemId": null, "message": "grant missing" }]
        });
        let response: ScanResponse = serde_json::from_value(wire).unwrap();
        let scan = scan_from_response(&root, response);
        assert_eq!(scan.source_id, source);
        assert!(matches!(
            scan.state,
            SourceScanState::PermissionRevoked { .. }
        ));
        assert!(!scan.state.allows_reconciliation());
    }
}
