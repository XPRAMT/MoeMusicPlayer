use std::{env, path::PathBuf, time::Instant};

use player_core::{
    LibraryRepository, LibraryRoot, ListTracksQuery, MediaIndex, MediaLocator, MediaSourceKind,
    MediaTrackRecord, SourceId, SourceScan, SourceScanState, SyncEngine, TrackIdentity,
    TrackMetadata, TrackMetadataError,
};
use player_db::Database;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sizes = env::args()
        .skip(1)
        .map(|value| value.parse::<usize>())
        .collect::<Result<Vec<_>, _>>()?;
    let sizes = if sizes.is_empty() {
        vec![10_000, 50_000, 100_000]
    } else {
        sizes
    };

    for size in sizes {
        run_size(size)?;
    }
    Ok(())
}

struct ReplayIndex {
    source_id: SourceId,
    records: Option<Vec<MediaTrackRecord>>,
    metadata_reads: usize,
}

impl MediaIndex for ReplayIndex {
    fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
        SourceScan {
            source_id: self.source_id,
            state: SourceScanState::Complete,
            tracks: self.records.take().unwrap_or_default(),
            errors: Vec::new(),
        }
    }

    fn read_metadata(
        &mut self,
        _track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        self.metadata_reads += 1;
        Ok(TrackMetadata::default())
    }
}

fn run_size(size: usize) -> Result<(), Box<dyn std::error::Error>> {
    let token = player_core::TrackId::new();
    let database_path = std::env::temp_dir().join(format!("moemusic-bench-{token}.sqlite"));
    let mut database = Database::open(&database_path)?;
    let root = database.add_library_root(
        MediaSourceKind::WindowsFilesystem,
        "benchmark",
        MediaLocator::FileSystem(PathBuf::from(r"C:\Benchmark")),
    )?;
    let records = (0..size)
        .map(|index| MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: format!("item-{index}"),
                locator_key: Some(format!("win-path:c:\\benchmark\\track-{index:06}.flac")),
            },
            locator: MediaLocator::FileSystem(PathBuf::from(format!(
                r"C:\Benchmark\track-{index:06}.flac"
            ))),
            fingerprint: player_core::FileFingerprint {
                size_bytes: 3_000_000 + index as u64,
                modified_at_utc_ms: Some(1_800_000_000_000),
            },
            metadata: Some(TrackMetadata {
                title: Some(format!("Track {index}")),
                artist: Some(format!("Artist {}", index % 1_000)),
                album: Some(format!("Album {}", index / 10)),
                duration_ms: Some(180_000),
                codec: Some("FLAC".to_owned()),
                ..TrackMetadata::default()
            }),
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    database.apply_source_scan(
        &root,
        &SourceScanState::Complete,
        &records,
        &records,
        &[],
        1_800_000_000_000,
    )?;
    let initial_sync = started.elapsed();

    let mut index = ReplayIndex {
        source_id: root.id,
        records: Some(records),
        metadata_reads: 0,
    };
    let started = Instant::now();
    let unchanged_report = SyncEngine::sync(&root, &mut index, &mut database, 1_800_000_000_100)?;
    let unchanged_scan = started.elapsed();

    let started = Instant::now();
    let total_tracks = database.count_tracks(None)?;
    let count_query = started.elapsed();

    let started = Instant::now();
    let page_items = database.list_tracks_window(None, 0, 100)?;
    let page_fetch = started.elapsed();

    let started = Instant::now();
    let first_page = database.list_tracks_page(ListTracksQuery {
        offset: 0,
        limit: 100,
        query: None,
    })?;
    let combined_page = started.elapsed();

    println!(
        "tracks={size} unchanged={}; initial_sync_ms={} unchanged_sync_ms={} metadata_reads={} count_ms={} fetch_100_ms={} combined_page_ms={} totals={}/{} fetched={}",
        unchanged_report.unchanged,
        initial_sync.as_millis(),
        unchanged_scan.as_millis(),
        index.metadata_reads,
        count_query.as_millis(),
        page_fetch.as_millis(),
        combined_page.as_millis(),
        total_tracks,
        first_page.total_count,
        page_items.len(),
    );

    drop(database);
    for suffix in ["", "-wal", "-shm"] {
        let mut file = database_path.as_os_str().to_os_string();
        file.push(suffix);
        let file = PathBuf::from(file);
        if file.exists() {
            std::fs::remove_file(file)?;
        }
    }
    Ok(())
}
