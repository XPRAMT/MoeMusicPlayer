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
        let mut layout = layout_strip(&fs(&c["main"]), &fs(&c["translation"]), f(&c["gap"]));
        if c["anchors"].is_array() {
            layout.anchors = fs(&c["anchors"]);
        }
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
}

fn pairs(v: &Value) -> Vec<(f64, f64)> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|w| (f(&w[0]), f(&w[1])))
        .collect()
}

fn same_pieces(actual: &[GlidePiece], expected: &Value, what: &str) {
    let expected = expected.as_array().unwrap();
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (a, e) in actual.iter().zip(expected) {
        for (x, k) in [a.t, a.v, a.s, a.c].into_iter().zip(e.as_array().unwrap()) {
            close(x, f(k), what);
        }
    }
}

#[test]
fn word_rules_match_the_shared_vectors() {
    let v = vectors();
    for c in cases(&v, "glide") {
        let guides: Vec<Vec<(f64, f64)>> =
            c["guides"].as_array().unwrap().iter().map(pairs).collect();
        let pieces = smooth_glide(
            &pairs(&c["hard"]),
            &guides,
            c["easeEnds"].as_bool().unwrap(),
        );
        same_pieces(&pieces, &c["pieces"], &c["hard"].to_string());
        for s in c["samples"].as_array().unwrap() {
            for (x, e) in glide_at(&pieces, f(&s["t"]))
                .into_iter()
                .zip(fs(&s["expected"]))
            {
                close(x, e, &s.to_string());
            }
        }
    }
    for c in cases(&v, "karaokeGlide") {
        let pieces = karaoke_glide(&pairs(&c["spans"]), &pairs(&c["words"]));
        same_pieces(&pieces, &c["pieces"], "karaoke");
        for s in c["samples"].as_array().unwrap() {
            close(
                glide_value(&pieces, f(&s["t"])),
                f(&s["expected"][0]),
                &s.to_string(),
            );
        }
    }
    for c in cases(&v, "stripGlide") {
        let lines: Vec<KnotLine> = c["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| KnotLine {
                start_ms: f(&l["startMs"]),
                cell_start: f(&l["cellStart"]),
                anchor: f(&l["anchor"]),
                end_ms: l["endMs"].as_f64(),
                last_centre: f(&l["lastCentre"]),
            })
            .collect();
        let knots = word_strip_knots(&lines, c["finalArrivalMs"].as_f64());
        let expected = pairs(&c["knots"]);
        assert_eq!(knots.len(), expected.len(), "strip knots");
        for (a, e) in knots.iter().zip(&expected) {
            close(a.0, e.0, "strip knot t");
            close(a.1, e.1, "strip knot v");
        }
        for s in c["samples"].as_array().unwrap() {
            for (x, e) in knot_cubic(&knots, f(&s["t"]))
                .into_iter()
                .zip(fs(&s["expected"]))
            {
                close(x, e, &s.to_string());
            }
        }
    }
    for c in cases(&v, "centreMask") {
        close(
            centre_mask_opacity(
                f(&c["x"]),
                f(&c["areaWidth"]),
                f(&c["emPx"]),
                f(&c["main"]),
                f(&c["secondary"]),
            ),
            f(&c["expected"]),
            &c.to_string(),
        );
    }
    for c in cases(&v, "sung") {
        assert_eq!(
            sung_word(&pairs(&c["words"]), f(&c["t"])).map_or(-1, |i| i as i64),
            c["expected"].as_i64().unwrap(),
            "{c}"
        );
    }
}
