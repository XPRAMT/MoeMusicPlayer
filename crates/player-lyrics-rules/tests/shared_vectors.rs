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
    for c in cases(&v, "edgeFade") {
        let (w, h) = (f(&c["areaWidth"]), f(&c["areaHeight"]));
        close(edge_fade_width(w, h), f(&c["width"]), &c.to_string());
        close(
            edge_fade_opacity(f(&c["x"]), w, h),
            f(&c["expected"]),
            &c.to_string(),
        );
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

#[test]
fn word_rules_match_the_shared_vectors() {
    let v = vectors();
    let g = &v["wordGlide"];
    let lines: Vec<KnotLine> = g["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| KnotLine {
            start_ms: f(&l["startMs"]),
            cell_start: f(&l["cellStart"]),
            word_starts: fs(&l["wordStarts"]),
            word_centres: fs(&l["wordCentres"]),
        })
        .collect();
    let knots = word_glide_knots(&lines);
    let expected: Vec<(f64, f64)> = g["knots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (f(&k[0]), f(&k[1])))
        .collect();
    assert_eq!(knots.len(), expected.len());
    for (a, e) in knots.iter().zip(&expected) {
        close(a.0, e.0, "knot time");
        close(a.1, e.1, "knot value");
    }
    for s in g["samples"].as_array().unwrap() {
        close(
            glide_cubic(&knots, f(&s["positionMs"]), f(&g["target"]))[0],
            f(&s["expected"]),
            &s.to_string(),
        );
    }
    for c in cases(&v, "centreMask") {
        close(
            centre_mask_opacity(f(&c["x"]), f(&c["areaWidth"]), f(&c["emPx"])),
            f(&c["expected"]),
            &c.to_string(),
        );
    }
    for c in cases(&v, "sung") {
        let words: Vec<(f64, f64)> = c["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (f(&w[0]), f(&w[1])))
            .collect();
        assert_eq!(
            sung_word(&words, f(&c["t"])).map_or(-1, |i| i as i64),
            c["expected"].as_i64().unwrap(),
            "{c}"
        );
    }
    let pairs = |v: &Value| -> Vec<(f64, f64)> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|w| (f(&w[0]), f(&w[1])))
            .collect()
    };
    for c in cases(&v, "karaoke") {
        close(
            karaoke_clip_x(&pairs(&c["spans"]), &pairs(&c["words"]), f(&c["t"])),
            f(&c["expected"]),
            &c.to_string(),
        );
    }
}

#[test]
fn the_karaoke_edge_is_continuous_per_frame() {
    let spans = [(0.0, 20.0), (20.0, 30.0), (34.0, 60.0)];
    let words = [(1000.0, 1200.0), (1200.0, 1200.0), (1300.0, 1500.0)];
    let step = 1000.0 / 240.0;
    let mut last = f64::NEG_INFINITY;
    let mut t = 900.0;
    while t <= 1600.0 {
        let x = karaoke_clip_x(&spans, &words, t);
        assert!(x >= last - 1e-12, "{t}");
        if t > 1000.0 + step && t < 1200.0 {
            assert!(x - last <= 20.0 / 200.0 * step + 1e-9, "{t}");
        }
        last = x;
        t += step;
    }
}
