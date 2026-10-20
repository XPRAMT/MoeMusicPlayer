use player_core::TrackMetadata;

pub(crate) fn into_core_metadata(metadata: crate::models::TrackMetadata) -> TrackMetadata {
    let release_date = metadata
        .release_date
        .as_deref()
        .and_then(player_core::parse_recording_date)
        .or_else(|| {
            metadata
                .year
                .filter(|year| *year > 0)
                .map(|year| format!("{year:04}"))
        });
    let year = release_date
        .as_deref()
        .and_then(player_core::recording_date_year)
        .or(metadata.year.filter(|year| *year > 0));
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
        release_date,
        year,
        bit_depth: metadata.bit_depth,
        embedded_lyrics: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(converted.release_date, None);
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
            "releaseDate": "2024-03-15",
            "year": 2024,
            "bitDepth": 24
        }))
        .expect("new Android metadata payload");
        let converted = into_core_metadata(current);
        assert_eq!(converted.release_date.as_deref(), Some("2024-03-15"));
        assert_eq!(converted.year, Some(2024));
        assert_eq!(converted.bit_depth, Some(24));

        let year_only: crate::models::TrackMetadata = serde_json::from_value(serde_json::json!({
            "title": "year-only",
            "artist": null,
            "album": null,
            "albumArtist": null,
            "trackNumber": null,
            "discNumber": null,
            "durationMs": 1,
            "codec": null,
            "bitrateBps": null,
            "sampleRateHz": null,
            "year": 1999
        }))
        .expect("year-only Android payload");
        let converted = into_core_metadata(year_only);
        assert_eq!(converted.release_date.as_deref(), Some("1999"));
        assert_eq!(converted.year, Some(1999));
    }

}
