//! The Rust rules match `tests/fixtures/lyrics-rules-vectors.json` (generated from the JS rules; Kotlin checks it too).
use player_lyrics_rules::*;
use serde_json::Value;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/lyrics-rules-vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn fs(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(f).collect()
}
fn opt(v: &Value) -> Option<f64> {
    v.as_f64()
}
fn close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{what}: {actual} != {expected}"
    );
}
fn cases<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    v[key].as_array().unwrap()
}

#[test]
fn rules_match_the_shared_vectors() {
    let v = vectors();
    for c in cases(&v, "active") {
        assert_eq!(
            active_index(&fs(&c["starts"]), f(&c["positionMs"]), f(&c["offsetMs"])) as i64,
            c["expected"].as_i64().unwrap(),
            "{c}"
        );
    }
    for c in cases(&v, "offsets") {
        close(
            display_offset_ms(opt(&c["lyricsOffsetMs"]), f(&c["delaySec"])),
            f(&c["expected"]),
            &c.to_string(),
        );
    }
    for c in cases(&v, "scroll") {
        close(
            scroll_progress(f(&c["elapsedMs"]), f(&c["durationMs"])),
            f(&c["expected"]),
            &c.to_string(),
        );
    }
    for c in cases(&v, "finalArrival") {
        close(
            final_arrival_ms(
                f(&c["lineStartMs"]),
                c["text"].as_str().unwrap(),
                opt(&c["trackEndMs"]),
            ),
            f(&c["expected"]),
            &c.to_string(),
        );
    }
    for c in cases(&v, "strip") {
        let layout = layout_strip(&fs(&c["main"]), &fs(&c["translation"]), f(&c["gap"]));
        let fin = c["finalLine"].as_object().map(|o| FinalLine {
            end_ms: o["endMs"].as_f64(),
            last_glyph_center: o["lastGlyphCenter"].as_f64(),
        });
        let actual = strip_offset(
            &fs(&c["starts"]),
            &layout,
            Align::parse(c["align"].as_str().unwrap()),
            f(&c["positionMs"]),
            f(&c["viewWidth"]),
            Align::parse(c["target"].as_str().unwrap()),
            fin,
        );
        close(actual, f(&c["expected"]), &c.to_string());
    }
    for c in cases(&v, "windows") {
        let layout = layout_strip(&fs(&c["main"]), &fs(&c["translation"]), f(&c["gap"]));
        let range = strip_window(
            &layout,
            f(&c["fromOffset"]),
            f(&c["toOffset"]),
            f(&c["viewWidth"]),
            f(&c["margin"]),
        );
        let (first, last) = (c["first"].as_i64().unwrap(), c["last"].as_i64().unwrap());
        match range {
            Some((a, b)) => assert_eq!((a as i64, b as i64), (first, last), "{c}"),
            None => assert!(last < first, "{c}"),
        }
    }
    for c in cases(&v, "breaks") {
        assert_eq!(
            next_strip_break(
                &fs(&c["starts"]),
                f(&c["positionMs"]),
                opt(&c["finalArrivalMs"])
            ),
            opt(&c["expected"]),
            "{c}"
        );
    }
    for c in cases(&v, "weights") {
        let starts = fs(&c["starts"]);
        let focus = strip_focus(&starts, f(&c["positionMs"]), FADE_MS);
        for (i, expected) in fs(&c["expected"]).into_iter().enumerate() {
            close(line_weight(&focus, i), expected, &c.to_string());
        }
    }
    for c in cases(&v, "opacity") {
        let inactive = normalize_inactive_opacity_percent(f(&c["inactive"]));
        let actual = line_opacity_factor(
            f(&c["weight"]),
            inactive,
            Animation::parse(c["animation"].as_str().unwrap()),
        );
        close(actual, f(&c["expected"]), &c.to_string());
    }
}
