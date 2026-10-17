//! Opus files get a spectrogram through the project's own Opus decoder (ADR 0017), not Symphonia.

use std::sync::atomic::AtomicBool;

use player_opus::testing::write_opus;
use player_tags::{render_spectrogram, SpectrogramOptions};

#[test]
fn an_opus_tone_shows_up_at_its_frequency() {
    let directory =
        std::env::temp_dir().join(format!("moe-opus-spectrogram-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    // The same bytes under an Opus extension and under `.ogg` (detected by content).
    for name in ["tone.opus", "tone.ogg"] {
        let path = directory.join(name);
        std::fs::write(&path, write_opus(96_000, 2, 1_000.0, 0, 0)).unwrap();
        let options = SpectrogramOptions {
            columns: 64,
            height: 512,
            ..SpectrogramOptions::default()
        };
        let image = render_spectrogram(&path, options, &AtomicBool::new(false)).unwrap();
        assert_eq!((image.sample_rate, image.channels), (48_000, 2), "{name}");
        assert_eq!(image.duration_ms, 2_000, "{name}");
        // Linear axis up to 24 kHz: the brightest row of a middle column is the one holding 1 kHz.
        let column = image.columns / 2;
        let brightest = (0..image.height)
            .max_by_key(|row| image.levels[(row * image.columns + column) as usize])
            .unwrap();
        let hz = (1.0 - (f64::from(brightest) + 0.5) / f64::from(image.height)) * 24_000.0;
        assert!((hz - 1_000.0).abs() < 150.0, "{name}: peak at {hz} Hz");
    }
    let _ = std::fs::remove_dir_all(&directory);
}
