//! Real NetEase lyric responses (not in the repository): `MOE_LYRICS_SAMPLES=<folder of *.json>`.
//! Only structural properties are checked, never the lyric text.

use player_core::{merge_lrc_auxiliary, parse_lrc, parse_yrc, yrc_matches_lrc, LyricAuxiliaryKind};

fn lyric(root: &serde_json::Value, key: &str) -> Option<String> {
    root.get(key)?
        .get("lyric")?
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
}

#[test]
#[ignore = "needs MOE_LYRICS_SAMPLES"]
fn real_netease_yrc_samples_parse_into_consistent_words() {
    let folder = std::env::var("MOE_LYRICS_SAMPLES").expect("MOE_LYRICS_SAMPLES");
    let mut checked = 0;
    for entry in std::fs::read_dir(folder).expect("sample folder") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(root) =
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
        else {
            continue;
        };
        let Some(yrc_text) = lyric(&root, "yrc") else {
            continue;
        };
        let mut yrc = parse_yrc(&yrc_text).expect("bounded");
        assert!(yrc.synced, "{path:?}");
        for line in &yrc.lines {
            assert!(!line.words.is_empty(), "{path:?}");
            let joined: String = line.words.iter().map(|w| w.text.as_str()).collect();
            assert_eq!(joined, line.text, "{path:?}");
            for word in &line.words {
                assert!(word.end_ms >= word.start_ms, "{path:?}");
            }
            let mut backwards = 0;
            for pair in line.words.windows(2) {
                if pair[1].start_ms < pair[0].start_ms {
                    backwards += 1;
                }
            }
            assert!(backwards <= 1, "{path:?}: words out of order");
        }
        if let Some(lrc) = lyric(&root, "lrc") {
            let lrc = parse_lrc(&lrc).unwrap();
            assert!(
                yrc_matches_lrc(&yrc, &lrc),
                "{path:?}: yrc should replace the LRC"
            );
        }
        if let Some(ytlrc) = lyric(&root, "ytlrc") {
            merge_lrc_auxiliary(&mut yrc, &ytlrc, LyricAuxiliaryKind::Translation).unwrap();
            let with = yrc.lines.iter().filter(|l| l.translation.is_some()).count();
            assert!(
                with * 2 >= yrc.lines.len(),
                "{path:?}: ytlrc matched {with} lines"
            );
        }
        checked += 1;
    }
    println!("{checked} yrc samples checked");
    assert!(checked > 0);
}
