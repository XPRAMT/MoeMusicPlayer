//! Manual check of the native renderer: a floating window and a taskbar-style window with sample lyrics.
//! `cargo run -p player-lyrics-native --example demo -- [continuous|none] [shadow|stroke|none] [seconds] [windows]`
//! `[x y]` (`windows` = how many lyrics windows: the floating one plus taskbar-style ones; default 2; `x y` = where
//! the floating one goes, default 200 300.)

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use player_lyrics_native::*;
    use std::sync::Arc;
    use windows::Win32::UI::HiDpi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    struct Log;
    impl Events for Log {
        fn lock_clicked(&self, id: &str) {
            println!("lock clicked {id}");
        }
        fn moved(&self, id: &str, x: i32, y: i32) {
            println!("moved {id} {x},{y}");
        }
        fn has_translation(&self, id: &str, has: bool) {
            println!("translation {id} {has}");
        }
        fn close_requested(&self, id: &str) {
            println!("close {id}");
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let animation = Animation::parse(args.first().map_or("continuous", |s| s));
    let effect = match args.get(1).map_or("shadow", |s| s.as_str()) {
        "stroke" => TextEffect::Stroke,
        "none" => TextEffect::None,
        _ => TextEffect::Shadow,
    };
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let count: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2).max(1);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let r = NativeLyrics::start(Arc::new(Log)).unwrap();
    let texts = [
        ("夜空に浮かぶ星を数えて", "數著夜空中漂浮的星星"),
        ("君の声が聞こえた気がした", "彷彿聽見了你的聲音"),
        ("我們在風裡奔跑 🌙", "We ran through the wind"),
        (
            "消えない光を抱きしめて ✨ とても長い歌詞の行がここで続いて画面からはみ出す",
            "擁抱永不熄滅的光",
        ),
        ("明日もきっと", "明天也一定"),
    ];
    // `MOE_DEMO_LINE_MS`: time between lines (default 2500 ms).
    let line_ms: f64 = std::env::var("MOE_DEMO_LINE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2500.0);
    let words = std::env::var("MOE_DEMO_WORDS").is_ok_and(|v| v == "1");
    let lines = texts
        .iter()
        .cycle()
        .take(texts.len() * 40)
        .enumerate()
        .map(|(i, (t, tr))| LyricLine {
            start_ms: 500.0 + i as f64 * line_ms,
            // Unique lines, so every line change brings a line that was never rasterised.
            text: format!("{t} {i}"),
            translation: tr.to_string(),
            // `MOE_DEMO_WORDS=1`: word timing, one word per grapheme over 80 % of the line (uneven: every third
            // word twice as long), as yrc lyrics give.
            words: if words {
                let text = format!("{t} {i}");
                let graphemes: Vec<&str> =
                    unicode_segmentation::UnicodeSegmentation::graphemes(text.as_str(), true)
                        .collect();
                let weights: Vec<f64> = (0..graphemes.len())
                    .map(|k| if k % 3 == 2 { 2.0 } else { 1.0 })
                    .collect();
                let total: f64 = weights.iter().sum();
                let start = 500.0 + i as f64 * line_ms;
                let mut at = start;
                graphemes
                    .iter()
                    .zip(&weights)
                    .map(|(g, w)| {
                        let len = line_ms * 0.8 * w / total;
                        let word = LyricWord {
                            start_ms: at,
                            end_ms: at + len,
                            text: g.to_string(),
                        };
                        at += len;
                        word
                    })
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect();
    // `MOE_DEMO_LYRICS_DELAY_MS`: open the windows first and load the lyrics later (to measure their memory).
    let lyrics_delay: u64 = std::env::var("MOE_DEMO_LYRICS_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let lyrics = LyricsData {
        track_id: "demo".into(),
        lines,
        offset_ms: None,
    };
    let load = {
        let r = r.clone();
        move || r.set_lyrics(Some(lyrics))
    };
    if lyrics_delay == 0 {
        load();
    } else {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(lyrics_delay));
            load();
        });
    }
    // For sync checks against screen captures: the wall time playback position 0 is sent.
    println!(
        "playback-start-unix-us {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros()
    );
    r.set_playback(PlaybackSample {
        track_id: Some("demo".into()),
        state: PlayState::Playing,
        position_ms: 0.0,
        duration_ms: Some(600_000.0),
    });
    let look = |font: f64| Look {
        align: Align::Center,
        font_size_px: font,
        translation_scale_percent: 80.0,
        color: [255, 255, 255],
        effect,
        show_translation: true,
        main_opacity_percent: 100.0,
        secondary_opacity_percent: 50.0,
        animation,
    };
    let place = |hwnd: isize, x: i32, y: i32, w: i32, h: i32| unsafe {
        let _ = SetWindowPos(
            windows::Win32::Foundation::HWND(hwnd as *mut _),
            Some(HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    };
    let floating = r.create("floating-lyrics", Kind::Floating, None).unwrap();
    r.set_look("floating-lyrics", Some(look(28.0)));
    r.set_chrome(
        "floating-lyrics",
        Chrome {
            locked: false,
            unlock_tip: "解除鎖定（可移動）".into(),
            lock_tip: "鎖定位置（滑鼠穿透）".into(),
        },
    );
    let fx: i32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(200);
    let fy: i32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(300);
    place(floating, fx, fy, 1800, 160);
    for i in 0..count - 1 {
        let id = format!("taskbar-lyrics-{i}");
        let bar = r.create(&id, Kind::Taskbar, None).unwrap();
        r.set_look(&id, Some(look(20.0)));
        // `MOE_DEMO_SECOND_AT=x,y`: the second taskbar-style window there (e.g. on a monitor with another DPI).
        let second = std::env::var("MOE_DEMO_SECOND_AT").ok().and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        });
        match second {
            Some((x, y)) if i == 1 => place(bar, x, y, 1200, 96),
            _ => place(bar, 200, 600 + 130 * i as i32, 1200, 96),
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    r.shutdown();
    std::thread::sleep(std::time::Duration::from_millis(300));
}
