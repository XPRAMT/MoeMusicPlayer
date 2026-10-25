//! Regenerates `tests/fixtures/lyrics-rules-vectors.json` from the Rust rules (the reference implementation, ADR 0034).
//! The same file is checked by `tests/shared_vectors.rs` (Rust, which also checks that this generator reproduces it
//! byte for byte), `LyricsRulesVectorsTest.kt` / `LyricsFramePlannerTest.kt` (Android) and
//! `tests/lyrics-rules-vectors.test.mjs` (the JS rules the app still runs: active line, offsets, scroll, karaoke glide).
//! Usage: `cargo run -p player-lyrics-rules --example generate_vectors`
//!
//! The output is what `JSON.stringify(vectors, null, 1)` printed when the JS rules generated it: one-space indent,
//! JS number formatting (integers without `.0`, shortest round-trip digits, `e-7` exponents), raw non-ASCII.

use player_lyrics_rules::*;

/// `(time, value)` pairs.
type Points = Vec<(f64, f64)>;

/// A JSON value with ordered keys.
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(&'static str, J)>),
}

fn num(x: f64) -> J {
    J::Num(x)
}
fn opt(x: Option<f64>) -> J {
    x.map_or(J::Null, J::Num)
}
fn nums(xs: &[f64]) -> J {
    J::Arr(xs.iter().map(|&x| J::Num(x)).collect())
}
fn pairs(xs: &[(f64, f64)]) -> J {
    J::Arr(xs.iter().map(|&(a, b)| nums(&[a, b])).collect())
}
fn s(x: &str) -> J {
    J::Str(x.to_owned())
}

/// `Number.prototype.toString` (what `JSON.stringify` prints for a finite number).
fn js_number(x: f64) -> String {
    if !x.is_finite() {
        return "null".to_owned();
    }
    if x == 0.0 {
        return "0".to_owned();
    }
    // Rust's `{:e}` prints the shortest round-trip digits, as JS does.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap() + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        let m = if k == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        format!("{m}e{sign}{}", e.abs())
    };
    if x < 0.0 {
        format!("-{body}")
    } else {
        body
    }
}

fn js_string(x: &str, out: &mut String) {
    out.push('"');
    for c in x.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.stringify(value, null, 1)`.
fn write(value: &J, depth: usize, out: &mut String) {
    let pad = |d: usize| " ".repeat(d);
    match value {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Num(x) => out.push_str(&js_number(*x)),
        J::Str(x) => js_string(x, out),
        J::Arr(items) if items.is_empty() => out.push_str("[]"),
        J::Arr(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                write(item, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        J::Obj(fields) if fields.is_empty() => out.push_str("{}"),
        J::Obj(fields) => {
            out.push_str("{\n");
            for (i, (key, item)) in fields.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                js_string(key, out);
                out.push_str(": ");
                write(item, depth + 1, out);
                out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
    }
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// 61 sample times from `from` to `to`, rounded to 0.25 ms so every language reads the same numbers.
fn sample_times(from: f64, to: f64) -> Vec<f64> {
    (0..=60)
        .map(|i| js_round((from + ((to - from) * f64::from(i)) / 60.0) * 4.0) / 4.0)
        .collect()
}

fn glide_samples(pieces: &[GlidePiece], times: &[f64]) -> J {
    J::Arr(
        times
            .iter()
            .map(|&t| {
                J::Obj(vec![
                    ("t", num(t)),
                    ("expected", nums(&glide_at(pieces, t))),
                ])
            })
            .collect(),
    )
}

fn piece_list(pieces: &[GlidePiece]) -> J {
    J::Arr(pieces.iter().map(|p| nums(&[p.t, p.v, p.s, p.c])).collect())
}

fn line(
    start_ms: f64,
    cell_start: f64,
    anchor: f64,
    end_ms: Option<f64>,
    last_centre: f64,
) -> KnotLine {
    KnotLine {
        start_ms,
        cell_start,
        anchor,
        end_ms,
        last_centre,
        cap_ms: None,
    }
}

/// A line without word times with its duration cap (`line_cap_ms` of its text).
fn capped(start_ms: f64, cell_start: f64, anchor: f64, last_centre: f64, text: &str) -> KnotLine {
    KnotLine {
        cap_ms: Some(line_cap_ms(text)),
        ..line(start_ms, cell_start, anchor, None, last_centre)
    }
}

fn line_json(l: &KnotLine) -> J {
    J::Obj(vec![
        ("startMs", num(l.start_ms)),
        ("cellStart", num(l.cell_start)),
        ("anchor", num(l.anchor)),
        ("endMs", opt(l.end_ms)),
        ("lastCentre", num(l.last_centre)),
        ("capMs", opt(l.cap_ms)),
    ])
}

/// The whole vectors file.
pub fn render() -> String {
    let starts = [1000.0, 3000.0, 5000.0, 9000.0];
    let mut active = Vec::new();
    for (position_ms, offset_ms) in [
        (0.0, 0.0),
        (999.0, 0.0),
        (1000.0, 0.0),
        (2700.0, 300.0),
        (4999.0, 0.0),
        (5000.0, 0.0),
        (20000.0, 0.0),
        (1500.0, -600.0),
    ] {
        active.push(J::Obj(vec![
            ("starts", nums(&starts)),
            ("positionMs", num(position_ms)),
            ("offsetMs", num(offset_ms)),
            (
                "expected",
                num(active_index(&starts, position_ms, offset_ms) as f64),
            ),
        ]));
    }
    active.push(J::Obj(vec![
        ("starts", nums(&[])),
        ("positionMs", num(500.0)),
        ("offsetMs", num(0.0)),
        ("expected", num(active_index(&[], 500.0, 0.0) as f64)),
    ]));

    let offsets = [
        (None, 0.0),
        (Some(250.0), 0.0),
        (Some(0.0), 0.15),
        (Some(100.0), -1.25),
        (Some(0.0), 25.0),
        (Some(-40.0), -0.0),
    ]
    .iter()
    .map(|&(own, delay)| {
        J::Obj(vec![
            ("lyricsOffsetMs", opt(own)),
            ("delaySec", num(delay)),
            ("expected", num(display_offset_ms(own, delay))),
        ])
    })
    .collect();

    let scroll = [
        (0.0, 4000.0),
        (1800.0, 4000.0),
        (3600.0, 4000.0),
        (5000.0, 4000.0),
        (-10.0, 4000.0),
        (100.0, 0.0),
    ]
    .iter()
    .map(|&(elapsed, duration)| {
        J::Obj(vec![
            ("elapsedMs", num(elapsed)),
            ("durationMs", num(duration)),
            ("expected", num(scroll_progress(elapsed, duration))),
        ])
    })
    .collect();

    // The terminal line estimate (ADR 0041 D10): 500 ms per grapheme whose base code point is East Asian W/F, 250 ms
    // per other one (spaces included), at least 250 ms. Cases: Latin, empty, a track-end cap, a ZWJ emoji family (one
    // wide grapheme), CJK, Latin with spaces, mixed kana + Latin, Hangul, full-width ASCII, half-width katakana,
    // e + U+0301 (one narrow grapheme), か + U+3099 (one wide), a wide emoji, U+3000 (F), ambiguous punctuation, ❤️
    // (U+2764 is N even with VS16) and a flag (regional indicators are N).
    let arrival_cases = [
        (1000.0, "abc", None),
        (1000.0, "", None),
        (1000.0, "夜に駆ける", Some(2500.0)),
        (0.0, "あ👨\u{200d}👩\u{200d}👧", None),
        (0.0, "啦啦啦啦啦啦啦啦", None),
        (0.0, "Hello world", None),
        (0.0, "愛してる love you", None),
        (0.0, "사랑해", None),
        (0.0, "ＡＢＣ！", None),
        (0.0, "ｱｲｳｴｵ", None),
        (0.0, "cafe\u{301}", None),
        (0.0, "か\u{3099}", None),
        (0.0, "🙂", None),
        (0.0, "a b", None),
        (0.0, "\u{3000}", None),
        (0.0, "“…”", None),
        (0.0, "❤\u{fe0f}", None),
        (0.0, "🇯🇵", None),
    ];
    let final_arrival = arrival_cases
        .iter()
        .map(|&(start, text, end)| {
            J::Obj(vec![
                ("lineStartMs", num(start)),
                ("text", s(text)),
                ("trackEndMs", opt(end)),
                ("expected", num(final_arrival_ms(start, text, end))),
            ])
        })
        .collect();
    // The duration cap of a line without word times (ADR 0041 D10): the same graphemes and table, 1000 ms per wide
    // one, 500 ms per other one, at least 500 ms.
    let line_cap = arrival_cases
        .iter()
        .map(|&(_, text, _)| text)
        .chain(["重新学会做梦和期望"])
        .map(|text| {
            J::Obj(vec![
                ("text", s(text)),
                ("expected", num(line_cap_ms(text))),
            ])
        })
        .collect();

    let main = [100.0, 50.0, 80.0, 120.0];
    let translation = [60.0, 90.0, 0.0, 40.0];
    let gap = 20.0;
    let layout = layout_strip(&main, &translation, gap);
    let mut strip = Vec::new();
    // The second list has a very short line (150 ms) and a long gap, so the curve's tangent choice is pinned too.
    let lists: [(Vec<f64>, Vec<f64>); 2] = [
        (
            starts.to_vec(),
            vec![
                0.0, 1000.0, 1500.0, 2000.0, 2500.0, 3000.0, 4500.0, 6000.0, 7500.0, 9000.0,
                9500.0, 10000.0, 12000.0,
            ],
        ),
        (
            vec![1000.0, 1150.0, 5000.0, 5200.0],
            vec![
                1000.0, 1075.0, 1150.0, 2000.0, 4000.0, 5000.0, 5100.0, 5200.0, 6000.0, 7000.0,
            ],
        ),
    ];
    for (list, positions) in &lists {
        for align in ["left", "center", "right"] {
            for target in ["left", "center"] {
                for &position_ms in positions {
                    for final_line in [
                        None,
                        Some(FinalLine {
                            end_ms: Some(11000.0),
                            last_glyph_center: Some(110.0),
                        }),
                    ] {
                        let expected = strip_offset(
                            list,
                            &layout,
                            Align::parse(align),
                            position_ms,
                            400.0,
                            Align::parse(target),
                            final_line,
                        );
                        strip.push(J::Obj(vec![
                            ("starts", nums(list)),
                            ("main", nums(&main)),
                            ("translation", nums(&translation)),
                            ("gap", num(gap)),
                            ("align", s(align)),
                            ("target", s(target)),
                            ("positionMs", num(position_ms)),
                            ("viewWidth", num(400.0)),
                            (
                                "finalLine",
                                final_line.map_or(J::Null, |f| {
                                    J::Obj(vec![
                                        ("endMs", opt(f.end_ms)),
                                        ("lastGlyphCenter", opt(f.last_glyph_center)),
                                    ])
                                }),
                            ),
                            ("expected", num(expected)),
                        ]));
                    }
                    // The first glyph's centre as the left anchor (ADR 0041 D10); 200 is clamped into its cell.
                    if align == "left" {
                        let anchors = [8.0, 12.5, 200.0, 0.0];
                        let mut anchored = layout_strip(&main, &translation, gap);
                        anchored.anchors = anchors.to_vec();
                        let expected = strip_offset(
                            list,
                            &anchored,
                            Align::parse(align),
                            position_ms,
                            400.0,
                            Align::parse(target),
                            None,
                        );
                        strip.push(J::Obj(vec![
                            ("starts", nums(list)),
                            ("main", nums(&main)),
                            ("translation", nums(&translation)),
                            ("gap", num(gap)),
                            ("anchors", nums(&anchors)),
                            ("align", s(align)),
                            ("target", s(target)),
                            ("positionMs", num(position_ms)),
                            ("viewWidth", num(400.0)),
                            ("finalLine", J::Null),
                            ("expected", num(expected)),
                        ]));
                    }
                }
            }
        }
    }

    let windows = [
        (0.0, 0.0, 100.0, 0.0),
        (-200.0, -330.0, 300.0, 300.0),
        (-500.0, -500.0, 50.0, 0.0),
        (5000.0, 5000.0, 10.0, 0.0),
    ]
    .iter()
    .map(|&(from, to, view, margin)| {
        let (first, end) = strip_window_span(&layout, from, to, view, margin);
        J::Obj(vec![
            ("main", nums(&main)),
            ("translation", nums(&translation)),
            ("gap", num(gap)),
            ("fromOffset", num(from)),
            ("toOffset", num(to)),
            ("viewWidth", num(view)),
            ("margin", num(margin)),
            ("first", num(first as f64)),
            ("last", num(end as f64 - 1.0)),
        ])
    })
    .collect();

    let breaks = [
        (0.0, None),
        (1000.0, None),
        (9000.0, None),
        (9000.0, Some(11000.0)),
        (12000.0, Some(11000.0)),
    ]
    .iter()
    .map(|&(position_ms, arrival)| {
        J::Obj(vec![
            ("starts", nums(&starts)),
            ("positionMs", num(position_ms)),
            ("finalArrivalMs", opt(arrival)),
            (
                "expected",
                opt(next_strip_break(&starts, position_ms, arrival)),
            ),
        ])
    })
    .collect();

    // `android` marks cases the Kotlin `cellWeight` must match too: Android's lead-in is a function of the position
    // and does not shorten a fade to a line shorter than 200 ms (AGENTS.md, 浮動歌詞), so those cases are Windows only.
    let mut weights = Vec::new();
    for (list, android) in [
        (starts.to_vec(), true),
        (vec![100.0, 250.0, 400.0], false),
        (vec![1000.0, 1100.0, 3000.0], false),
    ] {
        for position_ms in [
            0.0, 500.0, 799.0, 850.0, 900.0, 1000.0, 1050.0, 1100.0, 2800.0, 2900.0, 3000.0,
            4900.0, 5000.0, 8850.0, 9000.0, 15000.0,
        ] {
            let focus = strip_focus(&list, position_ms, FADE_MS);
            let expected: Vec<f64> = (0..list.len()).map(|i| line_weight(&focus, i)).collect();
            weights.push(J::Obj(vec![
                ("starts", nums(&list)),
                ("positionMs", num(position_ms)),
                ("android", J::Bool(android)),
                ("expected", nums(&expected)),
            ]));
        }
    }

    let mut edge_fade = Vec::new();
    for (area_width, area_height) in [
        (400.0, 40.0),
        (60.0, 40.0),
        (80.0, 40.0),
        (0.0, 40.0),
        (400.0, 0.0),
        (301.0, 33.5),
    ] {
        for x in [
            -5.0,
            0.0,
            10.0,
            20.0,
            39.9,
            40.0,
            200.0,
            360.0,
            380.0,
            area_width,
            area_width + 3.0,
        ] {
            edge_fade.push(J::Obj(vec![
                ("areaWidth", num(area_width)),
                ("areaHeight", num(area_height)),
                ("x", num(x)),
                ("width", num(edge_fade_width(area_width, area_height))),
                (
                    "expected",
                    num(edge_fade_opacity(x, area_width, area_height)),
                ),
            ]));
        }
    }

    // The smoothed glide (ADR 0041 D10): pieces and samples of the core curve, the karaoke edge and the strip glide.
    let glide_cases: Vec<(Points, Vec<Points>, bool)> = vec![
        // Dense words: many guides close together, one sharing a time, one off the band.
        (
            vec![(1000.0, 0.0), (3000.0, 200.0)],
            vec![vec![
                (1100.0, 5.0),
                (1150.0, 40.0),
                (1150.0, 41.0),
                (1200.0, 42.0),
                (1300.0, 45.0),
                (2000.0, 190.0),
                (2900.0, 199.0),
            ]],
            false,
        ),
        // Long gap between lines (more than 3x apart: the knot's tangent is the slower stretch's upper bound).
        (
            vec![
                (1000.0, 0.0),
                (2000.0, 100.0),
                (30000.0, 200.0),
                (31000.0, 300.0),
            ],
            vec![vec![(1500.0, 70.0)], vec![(10000.0, 110.0)], vec![]],
            true,
        ),
        // Knots sharing a time are dropped, backward values raised.
        (
            vec![(0.0, 0.0), (0.0, 50.0), (100.0, 40.0), (200.0, 10.0)],
            vec![vec![], vec![(50.0, 90.0)], vec![]],
            true,
        ),
        (vec![(500.0, 20.0)], vec![], true),
    ];
    let glide = glide_cases
        .iter()
        .map(|(hard, guides, ease_ends)| {
            let pieces = smooth_glide(hard, guides, *ease_ends);
            let end = hard[hard.len() - 1].0;
            J::Obj(vec![
                ("hard", pairs(hard)),
                ("guides", J::Arr(guides.iter().map(|g| pairs(g)).collect())),
                ("easeEnds", J::Bool(*ease_ends)),
                ("pieces", piece_list(&pieces)),
                (
                    "samples",
                    glide_samples(&pieces, &sample_times(hard[0].0 - 100.0, end + 100.0)),
                ),
            ])
        })
        .collect();

    let karaoke_cases: Vec<(Points, Points)> = vec![
        (
            vec![(0.0, 20.0), (20.0, 30.0), (34.0, 60.0)],
            vec![(1000.0, 1200.0), (1200.0, 1200.0), (1300.0, 1500.0)],
        ),
        (
            vec![
                (0.0, 20.0),
                (20.0, 40.0),
                (40.0, 60.0),
                (64.0, 80.0),
                (80.0, 120.0),
            ],
            vec![
                (1000.0, 1100.0),
                (1100.0, 1150.0),
                (1150.0, 1600.0),
                (1700.0, 1720.0),
                (1720.0, 2400.0),
            ],
        ),
        (vec![(5.0, 9.0)], vec![(700.0, 700.0)]),
        (vec![], vec![]),
    ];
    let karaoke = karaoke_cases
        .iter()
        .map(|(spans, words)| {
            let pieces = karaoke_glide(spans, words);
            J::Obj(vec![
                ("spans", pairs(spans)),
                ("words", pairs(words)),
                ("pieces", piece_list(&pieces)),
                (
                    "samples",
                    glide_samples(&pieces, &sample_times(600.0, 2500.0)),
                ),
            ])
        })
        .collect();

    // The strip glide ignores word times and judges each line on its own: every line start, a word-timed line's
    // last-word end and, after a last line without word times, the terminal knot (`finalArrivalMs`, null = none).
    // Cases: ordinary lines, a line without word times, a one-glyph line, an end not after its start, an overlap, and a
    // long gap.
    let song_tail = vec![
        line(207810.0, 0.0, 10.0, None, 0.0),
        line(212700.0, 280.0, 10.0, Some(215180.0), 130.0),
        line(215190.0, 460.0, 10.0, None, 0.0),
    ];
    let mut strip_glide_cases: Vec<(Vec<KnotLine>, Option<f64>)> = [
        vec![
            line(1000.0, 0.0, 10.0, Some(2400.0), 110.0),
            line(3000.0, 160.0, 9.0, None, 0.0),
            line(5000.0, 300.0, 8.0, Some(6000.0), 120.0),
        ],
        vec![
            line(1000.0, 0.0, 10.0, Some(1800.0), 60.0),
            line(2000.0, 90.0, 5.0, Some(2300.0), 5.0),
            line(2500.0, 150.0, 6.0, Some(2500.0), 70.0),
            line(9000.0, 260.0, 6.0, Some(12000.0), 200.0),
            line(11000.0, 520.0, 7.0, Some(13000.0), 90.0),
            line(40000.0, 640.0, 7.0, Some(41000.0), 100.0),
        ],
        vec![line(500.0, 0.0, 4.0, Some(900.0), 4.0)],
        // A line ending 10 ms before the next starts (20 px glyphs, 40 px gap): the gap would be faster than the
        // line, so its end knot is skipped and the strip glides from its start straight to the next start.
        vec![
            line(207810.0, 0.0, 10.0, None, 0.0),
            line(212700.0, 280.0, 10.0, Some(215180.0), 130.0),
            line(215190.0, 460.0, 10.0, None, 0.0),
            line(231300.0, 680.0, 10.0, None, 0.0),
        ],
        // A long pause keeps the end knot; a gap exactly as fast as the line keeps it too.
        vec![
            line(1000.0, 0.0, 10.0, Some(2000.0), 110.0),
            line(30000.0, 140.0, 10.0, Some(31000.0), 110.0),
            line(32000.0, 340.0, 10.0, None, 0.0),
        ],
    ]
    .into_iter()
    .map(|lines| (lines, None))
    .collect();
    // 十年有多长: a word-timed line, then the last line 啦啦啦啦啦啦啦啦 without word times glides from its first glyph to
    // its last at 500 ms per grapheme (231300 → 235300), or up to the track end.
    let song_case = strip_glide_cases.len();
    let last_line = line(231300.0, 680.0, 10.0, None, 150.0);
    let with_last = |last: KnotLine| {
        let mut lines = song_tail.clone();
        lines.push(last);
        lines
    };
    strip_glide_cases.push((
        with_last(last_line.clone()),
        Some(final_arrival_ms(231300.0, "啦啦啦啦啦啦啦啦", None)),
    ));
    strip_glide_cases.push((
        with_last(last_line.clone()),
        Some(final_arrival_ms(
            231300.0,
            "啦啦啦啦啦啦啦啦",
            Some(233000.0),
        )),
    ));
    // A word-timed last line uses its own end; the estimate is ignored.
    strip_glide_cases.push((
        with_last(KnotLine {
            end_ms: Some(240000.0),
            ..last_line.clone()
        }),
        Some(235300.0),
    ));
    // Line-timed only, with the terminal knot (the same motion as the line glide).
    strip_glide_cases.push((
        vec![
            line(1000.0, 0.0, 8.0, None, 70.0),
            line(3000.0, 104.0, 12.0, None, 40.0),
            line(3500.0, 180.0, 9.0, None, 90.0),
        ],
        Some(5000.0),
    ));
    // The cap of a line without word times (ADR 0041 D10): 1000 ms per wide grapheme, 500 ms per other one, when the
    // next line starts later. 重新学会做梦和期望 (03:35.19, 9 wide graphemes, 20 px each) ends at 03:44.19 and the
    // strip glides on to 啦啦啦啦啦啦啦啦 (03:51.30) over the remaining 7.11 s.
    let cap_case = strip_glide_cases.len();
    let interlude = vec![
        capped(215190.0, 0.0, 10.0, 170.0, "重新学会做梦和期望"),
        capped(231300.0, 220.0, 10.0, 150.0, "啦啦啦啦啦啦啦啦"),
    ];
    strip_glide_cases.push((
        interlude,
        Some(final_arrival_ms(231300.0, "啦啦啦啦啦啦啦啦", None)),
    ));
    // The next line within the cap: no end knot (unchanged).
    strip_glide_cases.push((
        vec![
            capped(1000.0, 0.0, 10.0, 170.0, "重新学会做梦和期望"),
            capped(8000.0, 220.0, 10.0, 150.0, "啦啦啦啦啦啦啦啦"),
        ],
        None,
    ));
    // A half-width English line: 11 graphemes (the space included) x 500 ms.
    strip_glide_cases.push((
        vec![
            capped(1000.0, 0.0, 5.0, 105.0, "Hello world"),
            capped(20000.0, 130.0, 5.0, 60.0, "Next"),
        ],
        None,
    ));
    // A line-timed song: capped lines before long gaps, lines whose next start is within the cap, a one-glyph line,
    // and the terminal knot.
    strip_glide_cases.push((
        vec![
            capped(1000.0, 0.0, 10.0, 90.0, "星を数えて"),
            capped(3000.0, 120.0, 10.0, 130.0, "君と二人きり"),
            capped(20000.0, 270.0, 10.0, 10.0, "嗯"),
            capped(30000.0, 310.0, 5.0, 85.0, "la la la"),
            capped(36000.0, 420.0, 10.0, 70.0, "夜明けまで"),
        ],
        Some(final_arrival_ms(36000.0, "夜明けまで", None)),
    ));
    let mut song_knots = Vec::new();
    let mut cap_knots = Vec::new();
    let strip_glide = strip_glide_cases
        .iter()
        .enumerate()
        .map(|(i, (lines, arrival))| {
            let knots = word_strip_knots(lines, *arrival);
            if i == song_case {
                song_knots = knots.clone();
            }
            if i == cap_case {
                cap_knots = knots.clone();
            }
            let (from, to) = match (knots.first(), knots.last()) {
                (Some(a), Some(b)) => (a.0 - 100.0, b.0 + 100.0),
                _ => (0.0, 100.0),
            };
            let samples = sample_times(from, to)
                .into_iter()
                .map(|t| {
                    J::Obj(vec![
                        ("t", num(t)),
                        ("expected", nums(&knot_cubic(&knots, t))),
                    ])
                })
                .collect();
            J::Obj(vec![
                ("lines", J::Arr(lines.iter().map(line_json).collect())),
                ("finalArrivalMs", opt(*arrival)),
                ("knots", pairs(&knots)),
                ("samples", J::Arr(samples)),
            ])
        })
        .collect();
    assert_eq!(
        song_knots.last(),
        Some(&(235300.0, 830.0)),
        "the last line lost its terminal knot"
    );
    assert_eq!(
        cap_knots.get(1),
        Some(&(224190.0, 170.0)),
        "重新学会做梦和期望 lost its capped end"
    );

    let mut centre_mask = Vec::new();
    for (area_width, em_px, main, secondary) in [
        (1000.0, 40.0, 1.0, 0.5),
        (80.0, 40.0, 1.0, 0.5),
        (300.0, 24.0, 1.0, 0.05),
        (1000.0, 40.0, 1.0, 1.0),
        (400.0, 30.0, 1.0, 0.0),
        (0.0, 40.0, 1.0, 0.5),
        // secondary above main, a small main, absolute values, a narrow area
        (1000.0, 40.0, 1.0, 0.01),
        (1000.0, 40.0, 0.01, 0.01),
        (80.0, 40.0, 0.5, 0.01),
        (1000.0, 40.0, 0.3, 0.8),
        (1000.0, 40.0, 0.6, 0.3),
        (1000.0, 40.0, 0.01, 0.1),
        (80.0, 40.0, 0.4, 0.9),
        (60.0, 40.0, 0.7, 0.2),
        (1000.0, 40.0, 0.0, 0.0),
    ] {
        for x in [
            0.0, 10.0, 20.0, 39.0, 40.0, 41.0, 120.0, 130.0, 150.0, 160.0, 170.0, 178.0, 180.0,
            182.0, 200.0, 470.0, 480.0, 500.0, 520.0, 600.0, 960.0, area_width,
        ] {
            centre_mask.push(J::Obj(vec![
                ("areaWidth", num(area_width)),
                ("emPx", num(em_px)),
                ("main", num(main)),
                ("secondary", num(secondary)),
                ("x", num(x)),
                (
                    "expected",
                    num(centre_mask_opacity(x, area_width, em_px, main, secondary)),
                ),
            ]));
        }
    }
    let sung_words = [(1000.0, 1200.0), (1200.0, 1200.0), (1300.0, 1500.0)];
    let sung = [999.0, 1000.0, 1199.0, 1200.0, 1250.0, 1400.0, 1500.0]
        .iter()
        .map(|&t| {
            J::Obj(vec![
                ("words", pairs(&sung_words)),
                ("t", num(t)),
                (
                    "expected",
                    num(sung_word(&sung_words, t).map_or(-1.0, |i| i as f64)),
                ),
            ])
        })
        .collect();

    let vectors = J::Obj(vec![
        (
            "note",
            s("Generated by `cargo run -p player-lyrics-rules --example generate_vectors`; do not edit by hand."),
        ),
        ("active", J::Arr(active)),
        ("offsets", J::Arr(offsets)),
        ("scroll", J::Arr(scroll)),
        ("finalArrival", J::Arr(final_arrival)),
        ("lineCap", J::Arr(line_cap)),
        ("strip", J::Arr(strip)),
        ("windows", J::Arr(windows)),
        ("breaks", J::Arr(breaks)),
        ("weights", J::Arr(weights)),
        ("edgeFade", J::Arr(edge_fade)),
        ("glide", J::Arr(glide)),
        ("karaokeGlide", J::Arr(karaoke)),
        ("stripGlide", J::Arr(strip_glide)),
        ("centreMask", J::Arr(centre_mask)),
        ("sung", J::Arr(sung)),
    ]);
    let mut out = String::new();
    write(&vectors, 0, &mut out);
    out.push('\n');
    out
}

/// Where the vectors live.
pub fn vectors_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/lyrics-rules-vectors.json")
}

#[allow(dead_code)]
fn main() {
    let path = vectors_path();
    std::fs::write(&path, render()).unwrap();
    println!("wrote {}", path.display());
}
