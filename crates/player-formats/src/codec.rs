//! The codec table: the one place that says what the library's `codec` column may hold and how each codec ranks.
//!
//! The `codec` of a track names the audio codec, never the container (the quality sort looks at the encoding only): an
//! M4A file is `ALAC` or `AAC`, an Ogg file `VORBIS` or `OPUS`, every uncompressed PCM file (WAV, AIFF, AIFC with PCM,
//! IEEE float included) is `WAV`. The shared tag reader writes these names, `player-db` builds its "sort by audio format"
//! and format-group SQL from this table, and `tests/fixtures/codec-table.json` (written by the test below) is what the
//! front end's lists are checked against.

/// One codec: the label stored in the library, the old or alternative spellings read from older rows, how it ranks in
/// the quality sort, whether it is lossless (the list then shows the bit depth instead of a bitrate) and the quick-index
/// group its tracks fall into.
#[derive(Clone, Copy, Debug)]
pub struct CodecInfo {
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    /// Higher sorts first in "highest quality first". Every DSD rate (`DSD64`, ...) ranks 7.
    pub rank: f64,
    pub lossless: bool,
    pub group: &'static str,
}

const fn codec(
    label: &'static str,
    aliases: &'static [&'static str],
    rank: f64,
    lossless: bool,
    group: &'static str,
) -> CodecInfo {
    CodecInfo {
        label,
        aliases,
        rank,
        lossless,
        group,
    }
}

pub const DSD_RANK: f64 = 7.0;

/// Highest quality first. Float32 is WAV like the integer PCM depths: only the bit depth tells them apart.
pub const CODECS: &[CodecInfo] = &[
    codec("WVDSD", &[], 6.5, true, "WVDSD"),
    codec(
        "WAV",
        &["WAVE", "AIFF", "AIF", "AIFC", "PCM"],
        6.0,
        true,
        "WAV",
    ),
    codec("WVLL", &[], 5.5, true, "WVLL"),
    codec("FLAC", &[], 5.0, true, "FLAC"),
    codec("ALAC", &[], 4.8, true, "ALAC"),
    codec("APE", &[], 4.7, true, "APE"),
    codec("WVL", &[], 4.5, false, "WVL"),
    codec("VORBIS", &["OGG VORBIS"], 4.0, false, "VORBIS"),
    codec("OPUS", &[], 3.0, false, "OPUS"),
    codec("AAC", &["MP4", "M4A", "MP4A"], 2.0, false, "MP4"),
    codec("MP3", &[], 1.0, false, "MP3"),
    // Lossless names older rows or other readers may carry, without a place in the ranking.
    codec("WV", &["WAVPACK"], 0.0, true, "WV"),
    codec("TAK", &[], 0.0, true, "TAK"),
    codec("TTA", &[], 0.0, true, "TTA"),
];

fn normalized(name: &str) -> String {
    name.trim().to_ascii_uppercase()
}

fn lookup(name: &str) -> Option<&'static CodecInfo> {
    let name = normalized(name);
    CODECS
        .iter()
        .find(|info| info.label == name || info.aliases.contains(&name.as_str()))
}

/// Whether `name` is a DSD rate label (`DSD64`, `DSD128`, ...).
pub fn is_dsd_rate(name: &str) -> bool {
    normalized(name).starts_with("DSD")
}

/// The canonical label of a codec name as older rows spell it (`Wave`, `Mp4`, `OGG Vorbis`, ...); other names are only
/// upper-cased.
pub fn canonical_codec(name: &str) -> String {
    lookup(name).map_or_else(|| normalized(name), |info| info.label.to_owned())
}

pub fn codec_rank(name: &str) -> f64 {
    if is_dsd_rate(name) {
        DSD_RANK
    } else {
        lookup(name).map_or(0.0, |info| info.rank)
    }
}

pub fn codec_is_lossless(name: &str) -> bool {
    is_dsd_rate(name) || lookup(name).is_some_and(|info| info.lossless)
}

/// The quick-index group of a codec; `None` when the name is neither in the table nor a DSD rate.
pub fn codec_group(name: &str) -> Option<&'static str> {
    if is_dsd_rate(name) {
        Some("DSD")
    } else {
        lookup(name).map(|info| info.group)
    }
}

/// Every spelling of every codec in the table (for the SQL `WHEN` lists).
fn spellings(info: &CodecInfo) -> impl Iterator<Item = &'static str> + '_ {
    std::iter::once(info.label).chain(info.aliases.iter().copied())
}

/// `CASE` expression over `column` that gives a codec its rank (0 for an unknown one, 7 for a DSD rate).
pub fn rank_case_sql(column: &str) -> String {
    let key = format!("TRIM(UPPER(COALESCE({column}, '')))");
    let mut sql = format!("CASE {key}");
    for info in CODECS.iter().filter(|info| info.rank > 0.0) {
        for name in spellings(info) {
            sql.push_str(&format!(" WHEN '{name}' THEN {}", info.rank));
        }
    }
    sql.push_str(&format!(
        " ELSE CASE WHEN {key} LIKE 'DSD%' THEN {DSD_RANK} ELSE 0 END END"
    ));
    sql
}

/// SQL list (`'WAV','WAVE',...`) of the lossless codecs whose list entry shows the bit depth, for `IN (...)`.
pub fn lossless_in_list_sql() -> String {
    CODECS
        .iter()
        .filter(|info| info.lossless)
        .flat_map(spellings)
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quality_order_is_the_agreed_one() {
        let order = [
            "DSD64", "WVDSD", "WAV", "WVLL", "FLAC", "ALAC", "APE", "WVL", "VORBIS", "OPUS", "AAC",
            "MP3",
        ];
        for pair in order.windows(2) {
            assert!(
                codec_rank(pair[0]) > codec_rank(pair[1]),
                "{} must rank above {}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(codec_rank("Aiff"), codec_rank("wav"), "one PCM class");
        assert_eq!(codec_rank("mp4"), codec_rank("AAC"));
        assert_eq!(codec_rank("unknown"), 0.0);
    }

    #[test]
    fn spellings_and_groups() {
        assert_eq!(canonical_codec(" Wave "), "WAV");
        assert_eq!(canonical_codec("AIFF"), "WAV");
        assert_eq!(canonical_codec("Mp4"), "AAC");
        assert_eq!(canonical_codec("alac"), "ALAC");
        assert_eq!(codec_group("ALAC"), Some("ALAC"));
        assert_eq!(codec_group("AIFC"), Some("WAV"));
        assert_eq!(codec_group("m4a"), Some("MP4"));
        assert_eq!(codec_group("dsd256"), Some("DSD"));
        assert!(
            codec_is_lossless("alac") && codec_is_lossless("DSD64") && !codec_is_lossless("Opus")
        );
    }

    /// `tests/fixtures/codec-table.json` is what the front end's lossless/DSD lists are checked against
    /// (`tests/codec-table.test.mjs`). Run with `MOE_WRITE_CODEC_TABLE=1` to rewrite it.
    #[test]
    fn the_shared_vector_file_matches_the_table() {
        let rows: Vec<String> = CODECS
            .iter()
            .flat_map(|info| {
                spellings(info).map(move |name| {
                    format!(
                        "    {{\"name\": \"{name}\", \"label\": \"{}\", \"rank\": {}, \"lossless\": {}, \"group\": \"{}\"}}",
                        info.label, info.rank, info.lossless, info.group
                    )
                })
            })
            .collect();
        let json = format!("{{\n  \"codecs\": [\n{}\n  ]\n}}\n", rows.join(",\n"));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/codec-table.json");
        if std::env::var_os("MOE_WRITE_CODEC_TABLE").is_some() {
            std::fs::write(&path, &json).unwrap();
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(committed, json, "run with MOE_WRITE_CODEC_TABLE=1");
    }
}
