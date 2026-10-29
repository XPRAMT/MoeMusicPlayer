//! The per-track lyric delay (歌詞同步) lives in the library database (`track_lyrics_timing_offsets`, tenths of a
//! second), the same on Windows and Android. The renderer reads and writes it only through these commands; the old
//! browser-storage values are handed over once by `lyrics_timing_offsets_import`.

use player_core::TrackId;
use serde::Deserialize;
use tauri::State;

use super::AppState;

/// Seconds (any number) to stored tenths: rounded, clamped to ±10 s, non-finite is 0.
fn seconds_to_tenths(delay_sec: f64) -> i64 {
    if !delay_sec.is_finite() {
        return 0;
    }
    ((delay_sec * 10.0).round() as i64).clamp(-100, 100)
}

fn tenths_to_seconds(tenths: i64) -> f64 {
    tenths as f64 / 10.0
}

fn database(state: &AppState) -> Result<&player_db::Database, String> {
    state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫無法使用".to_owned())
    })
}

fn parse_track(track_id: &str) -> Result<TrackId, String> {
    TrackId::parse(track_id).map_err(|_| "曲目識別碼無效。".to_owned())
}

/// The stored delay in seconds (0 when none).
#[tauri::command]
pub async fn lyrics_timing_offset_get(
    state: State<'_, AppState>,
    track_id: String,
) -> Result<f64, String> {
    let track_id = parse_track(&track_id)?;
    let tenths = database(&state)?
        .lyrics_timing_offset_tenths(track_id)
        .map_err(|error| error.to_string())?;
    Ok(tenths_to_seconds(tenths))
}

/// Store the delay (clamped, one decimal; 0 removes it) and return what was stored.
#[tauri::command]
pub async fn lyrics_timing_offset_set(
    state: State<'_, AppState>,
    track_id: String,
    delay_sec: f64,
) -> Result<f64, String> {
    let track_id = parse_track(&track_id)?;
    let _work = super::enter_database_work(&state)?;
    let tenths = database(&state)?
        .set_lyrics_timing_offset_tenths(track_id, seconds_to_tenths(delay_sec))
        .map_err(|error| error.to_string())?;
    Ok(tenths_to_seconds(tenths))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyLyricsTimingOffset {
    track_id: String,
    delay_sec: f64,
}

/// Entries whose key is not a Track ID cannot belong to any track and are dropped.
fn legacy_entries(entries: &[LegacyLyricsTimingOffset]) -> Vec<(TrackId, i64)> {
    entries
        .iter()
        .filter_map(|entry| {
            let track_id = TrackId::parse(&entry.track_id).ok()?;
            Some((track_id, seconds_to_tenths(entry.delay_sec)))
        })
        .collect()
}

/// One-time hand-over of the old browser-storage delays. A track that already has a delay keeps it. Returns the
/// number of rows added; on success the caller removes the old keys.
#[tauri::command]
pub async fn lyrics_timing_offsets_import(
    state: State<'_, AppState>,
    entries: Vec<LegacyLyricsTimingOffset>,
) -> Result<usize, String> {
    let _work = super::enter_database_work(&state)?;
    database(&state)?
        .import_lyrics_timing_offsets(&legacy_entries(&entries))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_round_to_tenths_and_clamp() {
        assert_eq!(seconds_to_tenths(1.25), 13);
        assert_eq!(seconds_to_tenths(-0.04), 0);
        assert_eq!(seconds_to_tenths(-12.0), -100);
        assert_eq!(seconds_to_tenths(10.2), 100);
        assert_eq!(seconds_to_tenths(f64::NAN), 0);
        assert_eq!(tenths_to_seconds(-13), -1.3);
    }

    #[test]
    fn legacy_entries_drop_keys_that_are_not_track_ids() {
        let id = TrackId::new();
        let entries = vec![
            LegacyLyricsTimingOffset {
                track_id: id.to_string(),
                delay_sec: 2.0,
            },
            LegacyLyricsTimingOffset {
                track_id: "C:/music/a.flac".to_owned(),
                delay_sec: 1.0,
            },
        ];
        assert_eq!(legacy_entries(&entries), vec![(id, 20)]);
    }

    #[test]
    fn legacy_import_moves_values_into_a_temporary_database() {
        let db = player_db::Database::open_in_memory().expect("database");
        let id = TrackId::new();
        let entries = vec![LegacyLyricsTimingOffset {
            track_id: id.to_string(),
            delay_sec: -1.5,
        }];
        assert_eq!(
            db.import_lyrics_timing_offsets(&legacy_entries(&entries))
                .unwrap(),
            1
        );
        assert_eq!(db.lyrics_timing_offset_tenths(id).unwrap(), -15);
    }
}
