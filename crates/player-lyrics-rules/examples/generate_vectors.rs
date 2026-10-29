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
    // and does not shorten a fade to a line shorter than 200 ms (docs/status/lyrics-windows.md, 浮動歌詞), so those cases are Windows only.
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
        // A line without word times but with its own end: one word spanning the line (`whole_line_word`).
        (
            vec![(0.0, 160.0)],
            vec![whole_line_word(1000.0, Some(2200.0)).unwrap()],
        ),
    ];
    // `whole_line_word`: the karaoke word of a line without word times; none without an end after the start.
    let whole_line = [
        (1000.0, Some(2200.0)),
        (1000.0, None),
        (1000.0, Some(1000.0)),
        (1000.0, Some(900.0)),
    ]
    .iter()
    .map(|&(start, end)| {
        J::Obj(vec![
            ("lineStartMs", num(start)),
            ("ownEndMs", opt(end)),
            (
                "expected",
                whole_line_word(start, end).map_or(J::Null, |(s, e)| nums(&[s, e])),
            ),
        ])
    })
    .collect();
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
    // Own line end times (ADR 0041 D10), judged per line by `knot_line_ends`: word times → the last word's end; else
    // the line's own end (SRT cue end, yrc / QRC line length, enhanced LRC closing stamp) → that end; else → the cap.
    // The last line: its own end, else the estimate (`final_arrival_with_end_ms`).
    let judged = |start: f64,
                  cell: f64,
                  anchor: f64,
                  last: f64,
                  text: &str,
                  word_end: Option<f64>,
                  own_end: Option<f64>,
                  is_last: bool| {
        let (end_ms, cap_ms) = knot_line_ends(start, text, word_end, own_end, is_last);
        KnotLine {
            end_ms,
            cap_ms,
            ..line(start, cell, anchor, None, last)
        }
    };
    // SRT-like lines, each with its cue end, the last one's end as the terminal knot.
    strip_glide_cases.push((
        vec![
            judged(
                1000.0,
                0.0,
                5.0,
                95.0,
                "First line",
                None,
                Some(3500.0),
                false,
            ),
            judged(
                9000.0,
                120.0,
                5.0,
                85.0,
                "Second one",
                None,
                Some(11000.0),
                false,
            ),
            judged(
                12000.0,
                230.0,
                10.0,
                110.0,
                "君の声が",
                None,
                Some(15500.0),
                true,
            ),
        ],
        Some(final_arrival_with_end_ms(
            12000.0,
            "君の声が",
            Some(15500.0),
            None,
        )),
    ));
    // A cue ending after the next one starts: the next start replaces the end knot.
    strip_glide_cases.push((
        vec![
            judged(
                1000.0,
                0.0,
                10.0,
                70.0,
                "重叠的句子",
                None,
                Some(7000.0),
                false,
            ),
            judged(
                5000.0,
                120.0,
                10.0,
                50.0,
                "下一句",
                None,
                Some(9000.0),
                true,
            ),
        ],
        Some(final_arrival_with_end_ms(
            5000.0,
            "下一句",
            Some(9000.0),
            None,
        )),
    ));
    // The last line's own end past the track end: capped there.
    strip_glide_cases.push((
        vec![
            judged(1000.0, 0.0, 10.0, 50.0, "最初", None, Some(2500.0), false),
            judged(
                6000.0,
                80.0,
                10.0,
                90.0,
                "最后一句",
                None,
                Some(12000.0),
                true,
            ),
        ],
        Some(final_arrival_with_end_ms(
            6000.0,
            "最后一句",
            Some(12000.0),
            Some(10000.0),
        )),
    ));
    // One song mixing the three kinds: own end, word-timed (its word end wins over its own end), neither (the cap), a
    // line with an end not after its start (the cap), and a last line with its own end.
    let mixed_case = strip_glide_cases.len();
    strip_glide_cases.push((
        vec![
            judged(1000.0, 0.0, 10.0, 30.0, "啊啊", None, Some(4000.0), false),
            judged(
                10000.0,
                60.0,
                10.0,
                70.0,
                "你好吗",
                Some(11500.0),
                Some(13000.0),
                false,
            ),
            judged(20000.0, 140.0, 10.0, 30.0, "再见", None, None, false),
            judged(
                30000.0,
                200.0,
                10.0,
                30.0,
                "嗯嗯",
                None,
                Some(30000.0),
                false,
            ),
            judged(
                40000.0,
                260.0,
                10.0,
                50.0,
                "晚安啦",
                None,
                Some(46000.0),
                true,
            ),
        ],
        Some(final_arrival_with_end_ms(
            40000.0,
            "晚安啦",
            Some(46000.0),
            None,
        )),
    ));
    let mut song_knots = Vec::new();
    let mut mixed_knots = Vec::new();
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
            if i == mixed_case {
                mixed_knots = knots.clone();
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
    assert_eq!(
        mixed_knots,
        vec![
            (1000.0, 10.0),
            (4000.0, 30.0),
            (10000.0, 70.0),
            (11500.0, 130.0),
            (20000.0, 150.0),
            (22000.0, 170.0),
            (30000.0, 210.0),
            (32000.0, 230.0),
            (40000.0, 270.0),
            (46000.0, 310.0),
        ],
        "a line lost its own end, word end or cap"
    );
    // The per-line end and cap (`knot_line_ends`) and the terminal arrival with an own end
    // (`final_arrival_with_end_ms`).
    let line_ends = [
        (1000.0, "啊啊", Some(1800.0), Some(2500.0), false),
        (1000.0, "啊啊", None, Some(2500.0), false),
        (1000.0, "啊啊", None, Some(1000.0), false),
        (1000.0, "啊啊", None, Some(900.0), false),
        (1000.0, "Hello", None, None, false),
        (1000.0, "啊啊", None, Some(2500.0), true),
        (1000.0, "啊啊", Some(1800.0), None, true),
        (1000.0, "啊啊", None, None, true),
    ]
    .iter()
    .map(|&(start, text, word_end, own_end, is_last)| {
        let (end, cap) = knot_line_ends(start, text, word_end, own_end, is_last);
        J::Obj(vec![
            ("startMs", num(start)),
            ("text", s(text)),
            ("lastWordEndMs", opt(word_end)),
            ("ownEndMs", opt(own_end)),
            ("isLast", J::Bool(is_last)),
            ("endMs", opt(end)),
            ("capMs", opt(cap)),
        ])
    })
    .collect();
    let final_arrival_with_end = [
        (1000.0, "啊啊", Some(2500.0), None),
        (1000.0, "啊啊", Some(2500.0), Some(2200.0)),
        (1000.0, "啊啊", None, None),
        (1000.0, "Hello world", None, Some(2000.0)),
        (1000.0, "啊啊", Some(1000.0), None),
        (1000.0, "啊啊", Some(900.0), None),
    ]
    .iter()
    .map(|&(start, text, own_end, track_end)| {
        J::Obj(vec![
            ("lineStartMs", num(start)),
            ("text", s(text)),
            ("ownEndMs", opt(own_end)),
            ("trackEndMs", opt(track_end)),
            (
                "durationMs",
                num(terminal_line_duration_ms(start, own_end, text)),
            ),
            (
                "expected",
                num(final_arrival_with_end_ms(start, text, own_end, track_end)),
            ),
        ])
    })
    .collect();

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
    // Floating panel placement (ADR 0032): ratio <-> pixels per axis, the kept-box clamp and the restore (Android:
    // the display is both the ratio frame and the clamp bounds).
    let bounds =
        |b: PxBounds| nums(&[b.left.into(), b.top.into(), b.right.into(), b.bottom.into()]);
    let pb = |left: i32, top: i32, right: i32, bottom: i32| PxBounds {
        left,
        top,
        right,
        bottom,
    };
    let mut placement_ratio = Vec::new();
    for (px, origin, extent) in [
        (540, 0, 1080),
        (240, 0, 2400),
        (0, 0, 1080),
        (1079, 0, 1080),
        (1080, 0, 1080),
        (-1920, -1920, 4480),
        (0, -1920, 4480),
        (2559, -1920, 4480),
        (333, 0, 1000),
        (1, 0, 3),
        (2, 0, 3),
        (17, 0, 7680),
        (-5, 0, 1080),
        (5, 0, 0),
    ] {
        placement_ratio.push(J::Obj(vec![
            ("px", num(px.into())),
            ("origin", num(origin.into())),
            ("extent", num(extent.into())),
            ("ratio", opt(position_ratio(px, origin, extent))),
        ]));
    }
    let mut placement_px = Vec::new();
    for (ratio, origin, extent) in [
        (0.5, 0, 1080),
        (0.1, 0, 2400),
        (0.0, 0, 1080),
        (1.0, 0, 1080),
        (0.25, -1920, 4480),
        (0.3333333333333333, 0, 3),
        (0.6666666666666666, 0, 3),
        (0.0005, 0, 1000),
        (0.0015, 0, 1000),
        (-0.0005, 0, 1000),
        (-0.25, 100, 400),
        (1.5, 0, 1080),
        (0.5, 7, 0),
    ] {
        placement_px.push(J::Obj(vec![
            ("ratio", num(ratio)),
            ("origin", num(origin.into())),
            ("extent", num(extent.into())),
            ("px", num(position_px(ratio, origin, extent).into())),
        ]));
    }
    let mut placement_clamp = Vec::new();
    for (origin, size, area) in [
        ((100, 200), (300, 60), pb(0, 0, 1080, 2400)),
        ((-10, -5), (300, 60), pb(0, 0, 1080, 2400)),
        ((900, 2390), (300, 60), pb(0, 0, 1080, 2400)),
        ((500, 10), (2000, 60), pb(0, 0, 1080, 2400)),
        ((500, 10), (300, 3000), pb(0, 0, 1080, 2400)),
        ((-3000, 50), (900, 80), pb(-1920, 0, 0, 1040)),
        ((4000, 2100), (900, 80), pb(3840, 712, 6400, 2112)),
        ((0, 0), (0, 0), pb(0, 0, 1920, 1040)),
    ] {
        let (x, y) = clamp_kept_box(origin, size, area);
        placement_clamp.push(J::Obj(vec![
            ("origin", nums(&[origin.0.into(), origin.1.into()])),
            ("size", nums(&[size.0.into(), size.1.into()])),
            ("bounds", bounds(area)),
            ("expected", nums(&[x.into(), y.into()])),
        ]));
    }
    let mut placement_restore = Vec::new();
    for (ratio, size, screen) in [
        ((0.5, 0.1), (300, 60), pb(0, 0, 1080, 2400)),
        ((0.5, 0.1), (300, 60), pb(0, 0, 2400, 1080)),
        ((0.9, 0.95), (300, 60), pb(0, 0, 1080, 2400)),
        ((0.0, 0.0), (300, 60), pb(0, 0, 1080, 2400)),
        ((1.0, 1.0), (300, 60), pb(0, 0, 1080, 2400)),
        ((-0.2, 0.5), (300, 60), pb(0, 0, 1080, 2400)),
        ((0.25, 0.75), (1200, 60), pb(0, 0, 1080, 2400)),
    ] {
        let (x, y) = restore_area_origin(ratio, screen, size, screen);
        placement_restore.push(J::Obj(vec![
            ("ratio", nums(&[ratio.0, ratio.1])),
            ("size", nums(&[size.0.into(), size.1.into()])),
            ("screen", bounds(screen)),
            ("expected", nums(&[x.into(), y.into()])),
        ]));
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
        ("finalArrivalWithEnd", J::Arr(final_arrival_with_end)),
        ("knotLineEnds", J::Arr(line_ends)),
        ("strip", J::Arr(strip)),
        ("windows", J::Arr(windows)),
        ("breaks", J::Arr(breaks)),
        ("weights", J::Arr(weights)),
        ("edgeFade", J::Arr(edge_fade)),
        ("glide", J::Arr(glide)),
        ("karaokeGlide", J::Arr(karaoke)),
        ("wholeLineWord", J::Arr(whole_line)),
        ("stripGlide", J::Arr(strip_glide)),
        ("centreMask", J::Arr(centre_mask)),
        ("sung", J::Arr(sung)),
        ("placementRatio", J::Arr(placement_ratio)),
        ("placementPx", J::Arr(placement_px)),
        ("placementClamp", J::Arr(placement_clamp)),
        ("placementRestore", J::Arr(placement_restore)),
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
