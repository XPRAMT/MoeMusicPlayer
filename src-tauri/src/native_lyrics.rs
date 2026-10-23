//! Bridge between the app and the native lyrics renderer (ADR 0034, Windows): one renderer thread for every taskbar
//! window and the floating window. The main window stays the single source of the lyrics, the playback samples, the
//! shared inactive opacity, the per-track delay and the translated padlock tooltips: it broadcasts them as events
//! (`taskbar-lyrics-data`, `taskbar-lyrics-playback`, `lyrics-inactive-opacity`, `lyrics-native-delay`,
//! `floating-lyrics-tips`) and this module listens on the Rust side. When the renderer starts it asks the main
//! window to send everything once (`taskbar-lyrics-request`, `taskbar-lyrics-playback-request`).

use std::sync::{Arc, OnceLock};

use player_lyrics_native::{
    Align, Animation, Chrome, Events, Look, LyricLine, LyricsData, NativeLyrics, PlayState,
    PlaybackSample, TextEffect,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Listener, Manager, PhysicalPosition};

use crate::settings::{
    FloatingLyricsSettings, LyricsTextEffect, TaskbarLyricsAlign, TaskbarLyricsSettings,
};

static RENDERER: OnceLock<Option<NativeLyrics>> = OnceLock::new();

struct AppEvents(AppHandle);

impl Events for AppEvents {
    fn lock_clicked(&self, id: &str) {
        if !crate::floating_lyrics::is_window_label(id) {
            return;
        }
        let app = self.0.clone();
        tauri::async_runtime::spawn(async move {
            let locked = crate::floating_lyrics::current().is_some_and(|s| s.locked);
            if let Err(error) = crate::set_floating_lyrics_locked(&app, !locked).await {
                eprintln!("floating lyrics lock: {error}");
            }
        });
    }
    fn moved(&self, id: &str, x: i32, y: i32) {
        if crate::floating_lyrics::is_window_label(id) {
            crate::floating_lyrics::remember_position(&self.0, PhysicalPosition::new(x, y));
        }
    }
    fn has_translation(&self, id: &str, has: bool) {
        if crate::floating_lyrics::is_window_label(id) {
            let app = self.0.clone();
            std::thread::spawn(move || crate::floating_lyrics::fit_lines(&app, has));
        }
    }
    fn close_requested(&self, _id: &str) {
        // WM_CLOSE to a lyrics window (logoff, tools closing "the main window") quits the app the normal way.
        if let Some(main) = self.0.get_webview_window("main") {
            let _ = main.close();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DataPayload {
    track_id: Option<String>,
    #[serde(default)]
    offset_ms: Option<f64>,
    #[serde(default)]
    lines: Vec<LinePayload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinePayload {
    start_ms: f64,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    translation: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackPayload {
    track_id: Option<String>,
    position_ms: f64,
    duration_ms: Option<f64>,
    state: String,
    playing: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DelayPayload {
    track_id: String,
    delay_sec: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TipsPayload {
    lock: String,
    unlock: String,
}

static TIPS: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// The renderer, started on first use (None when it could not start).
pub fn renderer(app: &AppHandle) -> Option<&'static NativeLyrics> {
    RENDERER
        .get_or_init(|| {
            let renderer = NativeLyrics::start(Arc::new(AppEvents(app.clone())))
                .map_err(|error| eprintln!("lyrics renderer: {error}"))
                .ok()?;
            listen(app, renderer.clone());
            // Ask the main window for the current state now that we listen.
            let _ = app.emit("taskbar-lyrics-request", ());
            let _ = app.emit("taskbar-lyrics-playback-request", ());
            Some(renderer)
        })
        .as_ref()
}

fn listen(app: &AppHandle, renderer: NativeLyrics) {
    let r = renderer.clone();
    app.listen_any("taskbar-lyrics-data", move |event| {
        if let Ok(data) = serde_json::from_str::<DataPayload>(event.payload()) {
            r.set_lyrics(data.track_id.map(|track_id| {
                LyricsData {
                    track_id,
                    offset_ms: data.offset_ms,
                    lines: data
                        .lines
                        .into_iter()
                        .map(|line| LyricLine {
                            start_ms: line.start_ms,
                            text: line.text.unwrap_or_default(),
                            translation: line.translation.unwrap_or_default(),
                        })
                        .collect(),
                }
            }));
        }
    });
    let r = renderer.clone();
    app.listen_any("taskbar-lyrics-playback", move |event| {
        if let Ok(p) = serde_json::from_str::<PlaybackPayload>(event.payload()) {
            let state = match PlayState::parse(&p.state) {
                PlayState::Stopped => PlayState::Stopped,
                _ if p.playing => PlayState::Playing,
                PlayState::Buffering => PlayState::Buffering,
                _ => PlayState::Paused,
            };
            r.set_playback(PlaybackSample {
                track_id: p.track_id,
                state,
                position_ms: p.position_ms,
                duration_ms: p.duration_ms,
            });
        }
    });
    let r = renderer.clone();
    app.listen_any("lyrics-inactive-opacity", move |event| {
        if let Ok(percent) = serde_json::from_str::<f64>(event.payload()) {
            r.set_inactive_opacity(percent.round().clamp(0.0, 100.0) as u32);
        }
    });
    let r = renderer.clone();
    app.listen_any("lyrics-native-delay", move |event| {
        if let Ok(d) = serde_json::from_str::<DelayPayload>(event.payload()) {
            r.set_timing_delay(&d.track_id, d.delay_sec);
        }
    });
    let app_handle = app.clone();
    app.listen_any("floating-lyrics-tips", move |event| {
        if let Ok(tips) = serde_json::from_str::<TipsPayload>(event.payload()) {
            if let Ok(mut guard) = TIPS.lock() {
                *guard = Some((tips.lock, tips.unlock));
            }
            if let Some(settings) = crate::floating_lyrics::current() {
                apply_floating_chrome(&app_handle, &settings);
            }
        }
    });
}

fn align(value: TaskbarLyricsAlign) -> Align {
    match value {
        TaskbarLyricsAlign::Left => Align::Left,
        TaskbarLyricsAlign::Center => Align::Center,
        TaskbarLyricsAlign::Right => Align::Right,
    }
}

fn effect(value: LyricsTextEffect) -> TextEffect {
    match value {
        LyricsTextEffect::Shadow => TextEffect::Shadow,
        LyricsTextEffect::Stroke => TextEffect::Stroke,
        _ => TextEffect::None,
    }
}

fn hex(value: &str) -> Option<[u8; 3]> {
    let v = value.strip_prefix('#')?;
    if v.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&v[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `getTaskbarLyricsColor`: the custom colour, or black/white against the taskbar theme.
fn colour(use_custom: bool, custom: &str, light: bool) -> [u8; 3] {
    match (use_custom, hex(custom)) {
        (true, Some(c)) => c,
        _ if light => [0, 0, 0],
        _ => [255, 255, 255],
    }
}

fn animation(value: crate::settings::LyricsAnimation) -> Animation {
    if matches!(value, crate::settings::LyricsAnimation::Continuous) {
        Animation::Continuous
    } else {
        Animation::None
    }
}

pub fn taskbar_look(settings: &TaskbarLyricsSettings, light: bool) -> Look {
    Look {
        align: align(settings.align),
        font_size_px: f64::from(settings.font_size_px),
        translation_scale_percent: f64::from(settings.translation_scale_percent),
        color: colour(settings.use_custom_color, &settings.custom_color, light),
        effect: effect(settings.text_effect),
        show_translation: settings.show_translation,
        opacity_percent: f64::from(settings.opacity_percent),
        animation: animation(settings.animation),
    }
}

/// The floating window does not follow the taskbar theme (white unless a custom colour is chosen).
pub fn floating_look(settings: &FloatingLyricsSettings) -> Look {
    Look {
        align: align(settings.align),
        font_size_px: f64::from(settings.font_size_px),
        translation_scale_percent: f64::from(settings.translation_scale_percent),
        color: colour(settings.use_custom_color, &settings.custom_color, false),
        effect: effect(settings.text_effect),
        show_translation: settings.show_translation,
        opacity_percent: f64::from(settings.opacity_percent),
        animation: animation(settings.animation),
    }
}

pub fn apply_floating_chrome(app: &AppHandle, settings: &FloatingLyricsSettings) {
    let Some(r) = renderer(app) else { return };
    let (lock, unlock) = TIPS
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| ("鎖定位置（滑鼠穿透）".into(), "解除鎖定（可移動）".into()));
    r.set_chrome(
        crate::floating_lyrics::WINDOW_LABEL,
        Chrome {
            locked: settings.locked,
            lock_tip: lock,
            unlock_tip: unlock,
        },
    );
}
