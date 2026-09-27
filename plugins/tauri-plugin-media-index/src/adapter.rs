use player_core::{
    LibraryRoot, MediaLocator, MediaSourceError, MediaTrackRecord, SourceScan, SourceScanState,
    TrackIdentity, TrackMetadata,
};

use crate::models::{ScanResponse, ScanState};

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
