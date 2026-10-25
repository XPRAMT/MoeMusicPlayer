//! Bridge between the app and the native lyrics renderer (ADR 0034, Windows): one renderer thread for every taskbar
//! window and the floating window. The main window stays the single source of the lyrics, the playback samples, the
//! per-track delay, the accent colour and the translated padlock tooltips: it broadcasts them as events
//! (`taskbar-lyrics-data`, `taskbar-lyrics-playback`, `lyrics-accent-color`, `lyrics-native-delay`,
//! `floating-lyrics-tips`) and this module listens on the Rust side. The main and secondary opacity are not events:
//! each window's own settings carry them into its `Look`. When the renderer starts it asks the main
//! window to send everything once (`taskbar-lyrics-request`, `taskbar-lyrics-playback-request`).

use std::sync::{Arc, OnceLock};

use player_lyrics_native::{
    Align, Animation, Chrome, Events, Look, LyricLine, LyricWord, LyricsData, NativeLyrics,
    PlayState, PlaybackSample, TextEffect,
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
    fn drag_ended(&self, id: &str) {
        if crate::floating_lyrics::is_window_label(id) {
            crate::floating_lyrics::settle_after_drag();
        }
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
    /// The line's own end (`LyricLineDto.endMs`), when its source gives one.
    #[serde(default)]
    end_ms: Option<f64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    translation: Option<String>,
    #[serde(default)]
    words: Vec<WordPayload>,
}

/// A word of a word-timed line (`LyricLineDto.words`, ADR 0041): start, end, text.
#[derive(Deserialize)]
struct WordPayload {
    s: f64,
    e: f64,
    t: String,
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

/// The audio engine's clock for the lyrics glide: the backend's played position and when it was read (shared
/// mode: rodio's consumed samples minus the delay-line silence; exclusive WASAPI and DoP: frames taken from the ring
/// minus the endpoint latency; DSD converts to PCM before either). Read from the player's snapshot, which the audio
/// worker refreshes about every 40 ms; the renderer averages the readings, so their steps do not move the glide.
fn audio_clock(app: AppHandle) -> player_lyrics_native::ClockSource {
    Arc::new(move || {
        let state = app.try_state::<crate::AppState>()?;
        let snapshot = state.playback.as_ref()?.player.snapshot();
        Some(player_lyrics_native::AudioClock {
            position_ms: snapshot.position.as_secs_f64() * 1000.0,
            at: snapshot.position_at?,
            playing: snapshot.state == player_audio_windows::PlaybackState::Playing,
        })
    })
}

/// The renderer, started on first use (None when it could not start).
pub fn renderer(app: &AppHandle) -> Option<&'static NativeLyrics> {
    RENDERER
        .get_or_init(|| {
            let renderer = NativeLyrics::start(Arc::new(AppEvents(app.clone())))
                .map_err(|error| eprintln!("lyrics renderer: {error}"))
                .ok()?;
            listen(app, renderer.clone());
            renderer.set_clock_source(audio_clock(app.clone()));
            // Ask the main window for the current state now that we listen.
            let _ = app.emit("taskbar-lyrics-request", ());
            let _ = app.emit("taskbar-lyrics-playback-request", ());
            Some(renderer)
        })
        .as_ref()
}

/// A `taskbar-lyrics-data` payload as the renderer's lyrics (`None` when it does not parse; `Some(None)`: no track).
/// Every time (start, own end, word times) stays in lyrics time; the renderer applies the offset to all alike.
fn lyrics_data(payload: &str) -> Option<Option<LyricsData>> {
    let data = serde_json::from_str::<DataPayload>(payload).ok()?;
    Some(data.track_id.map(|track_id| {
        LyricsData {
            track_id,
            offset_ms: data.offset_ms,
            lines: data
                .lines
                .into_iter()
                .map(|line| LyricLine {
                    start_ms: line.start_ms,
                    end_ms: line.end_ms.filter(|end| end.is_finite()),
                    text: line.text.unwrap_or_default(),
                    translation: line.translation.unwrap_or_default(),
                    words: line
                        .words
                        .into_iter()
                        .map(|w| LyricWord {
                            start_ms: w.s,
                            end_ms: w.e,
                            text: w.t,
                        })
                        .collect(),
                })
                .collect(),
        }
    }))
}

fn listen(app: &AppHandle, renderer: NativeLyrics) {
    let r = renderer.clone();
    app.listen_any("taskbar-lyrics-data", move |event| {
        if let Some(data) = lyrics_data(event.payload()) {
            r.set_lyrics(data);
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
                activity: player_lyrics_native::pause_hide::Activity::from_state(
                    &p.state, p.playing,
                ),
            });
        }
    });
    let r = renderer.clone();
    app.listen_any("lyrics-accent-color", move |event| {
        if let Some(rgb) = serde_json::from_str::<String>(event.payload())
            .ok()
            .and_then(|v| hex(&v))
        {
            r.set_accent(rgb);
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
        main_opacity_percent: f64::from(settings.main_opacity_percent),
        secondary_opacity_percent: f64::from(settings.secondary_opacity_percent),
        font_weight: settings.font_weight,
        animation: animation(settings.animation),
        hide_when_paused: settings.hide_when_paused,
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
        main_opacity_percent: f64::from(settings.main_opacity_percent),
        secondary_opacity_percent: f64::from(settings.secondary_opacity_percent),
        font_weight: settings.font_weight,
        animation: animation(settings.animation),
        hide_when_paused: settings.hide_when_paused,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_data_carries_each_lines_own_end() {
        let data = lyrics_data(
            r#"{"trackId":"t","offsetMs":250,"lines":[{"startMs":1000,"endMs":3500,"text":"a"},{"startMs":4000,"text":"b","words":[{"s":4000,"e":4500,"t":"b"}]}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(data.offset_ms, Some(250.0));
        assert_eq!(data.lines[0].end_ms, Some(3500.0));
        assert_eq!(data.lines[1].end_ms, None);
        assert_eq!(data.lines[1].words[0].end_ms, 4500.0);
        assert_eq!(lyrics_data(r#"{"trackId":null}"#), Some(None));
        assert_eq!(lyrics_data("nonsense"), None);
    }
}
