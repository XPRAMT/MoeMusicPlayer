use player_core::{
    LibraryRoot, MediaIndex, MediaTrackRecord, SourceScan, SourceScanState, TrackMetadata,
    TrackMetadataError,
};

/// Windows media-index provider.
///
/// This public type is kept small so the Tauri shell can construct one per sync operation.
#[derive(Default)]
pub struct WindowsMediaIndex;

impl WindowsMediaIndex {
    pub fn new() -> Self {
        Self
    }
}

impl MediaIndex for WindowsMediaIndex {
    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        SourceScan {
            source_id: root.id,
            state: SourceScanState::Unavailable {
                reason: "Windows filesystem index is not initialized yet".to_owned(),
            },
            tracks: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn read_metadata(
        &mut self,
        _track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        Err(TrackMetadataError {
            message: "Windows metadata reader is not initialized yet".to_owned(),
        })
    }
}
