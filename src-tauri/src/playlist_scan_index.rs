//! The index both playlist-file syncs (a playlist on this device, a playlist on a server) hand to the sync engine.
//!
//! A playlist lists the songs it holds, so its "scan" is looking those songs up (a file check each, a server round trip
//! each for a remote one). That lookup runs inside `scan_with_progress`, like the walk of a folder source does: it
//! reports `MediaScanProgress` and the engine's one reporter turns that into the banner events, so a playlist and a
//! folder show their progress the same way, whatever the platform or where the songs are. The `.lrc` files beside the
//! songs are looked up right after, and the tags are read through the same reader the source's own folders use.

use player_core::{
    LibraryRoot, LyricSidecar, MediaIndex, MediaScanProgress, MediaTrackRecord, MetadataReader,
    SourceId, SourceScan, SourceScanState, SyncCancellation, TrackMetadata, TrackMetadataError,
};

/// Looks the songs up, telling the callback how far it is.
pub(crate) type LookupFn<'a> = Box<dyn FnOnce(&mut dyn FnMut(MediaScanProgress)) -> SourceScan + 'a>;
/// Finds the `.lrc` files beside the songs that were found.
pub(crate) type SidecarFn<'a> = Box<dyn FnOnce(&[MediaTrackRecord]) -> Vec<LyricSidecar> + 'a>;
/// Reads the tags of one song.
pub(crate) type ReadFn<'a> =
    Box<dyn FnMut(&MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError> + 'a>;

pub(crate) struct PlaylistScanIndex<'a> {
    lookup: Option<LookupFn<'a>>,
    find_sidecars: Option<SidecarFn<'a>>,
    sidecars: Vec<LyricSidecar>,
    read: ReadFn<'a>,
    /// A reader that can parse many songs at once (a remote server), and how many.
    parallel: Option<(&'a dyn MetadataReader, usize)>,
}

impl<'a> PlaylistScanIndex<'a> {
    pub(crate) fn new(
        lookup: LookupFn<'a>,
        find_sidecars: SidecarFn<'a>,
        read: ReadFn<'a>,
        parallel: Option<(&'a dyn MetadataReader, usize)>,
    ) -> Self {
        Self {
            lookup: Some(lookup),
            find_sidecars: Some(find_sidecars),
            sidecars: Vec::new(),
            read,
            parallel,
        }
    }
}

fn incomplete(source_id: SourceId, reason: &str) -> SourceScan {
    SourceScan {
        source_id,
        state: SourceScanState::Incomplete {
            reason: reason.to_owned(),
        },
        tracks: Vec::new(),
        errors: Vec::new(),
    }
}

impl MediaIndex for PlaylistScanIndex<'_> {
    fn parallel_metadata_reader(&self) -> Option<&dyn MetadataReader> {
        self.parallel.map(|(reader, _)| reader)
    }

    fn metadata_read_threads(&self) -> usize {
        self.parallel
            .map_or(player_core::sync::METADATA_READ_THREADS, |(_, threads)| threads)
    }

    fn take_lyric_sidecars(&mut self) -> Vec<LyricSidecar> {
        std::mem::take(&mut self.sidecars)
    }

    /// A playlist looks for the `.lrc` files beside its songs itself.
    fn scans_lyric_sidecars(&self) -> bool {
        true
    }

    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        self.scan_with_progress(root, &mut |_| {}, &SyncCancellation::default())
    }

    fn scan_with_progress(
        &mut self,
        root: &LibraryRoot,
        progress: &mut dyn FnMut(MediaScanProgress),
        cancellation: &SyncCancellation,
    ) -> SourceScan {
        if cancellation.is_cancelled() {
            return incomplete(root.id, "scan cancelled before enumeration");
        }
        let Some(lookup) = self.lookup.take() else {
            return incomplete(root.id, "playlist scan was already consumed");
        };
        let scan = lookup(progress);
        if let Some(find_sidecars) = self.find_sidecars.take() {
            self.sidecars = find_sidecars(&scan.tracks);
        }
        scan
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        (self.read)(track)
    }
}
