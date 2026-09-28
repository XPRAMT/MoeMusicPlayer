#![cfg(windows)]

use std::fs::File;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{mixer, Decoder, Player, Source};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
#[ignore = "read-only endpoint diagnostic; run manually when investigating playback rate"]
fn reports_default_output_endpoint_config_without_opening_a_stream() {
    let host = rodio::cpal::default_host();
    let device = host.default_output_device().expect("default output device");
    let config = device.default_output_config().unwrap();
    eprintln!(
        "default output sample_rate={} Hz; channels={}; format={:?}",
        config.sample_rate(),
        config.channels(),
        config.sample_format()
    );
}

#[test]
fn supported_file_decoders_report_duration_and_seek_with_audio_timebase() {
    for name in [
        "seek-tone.mp3",
        "seek-tone.flac",
        "seek-tone.ogg",
        "seek-tone.wav",
    ] {
        let decoder = Decoder::try_from(File::open(fixture(name)).unwrap())
            .unwrap_or_else(|error| panic!("could not decode {name}: {error}"));
        let duration = decoder
            .total_duration()
            .unwrap_or_else(|| panic!("{name} should have a decoded duration"));
        assert!(
            (Duration::from_millis(1_900)..=Duration::from_millis(2_200)).contains(&duration),
            "expected the synthetic two-second duration for {name}, got {duration:?}"
        );

        let mut tracked = decoder.track_position();
        tracked
            .try_seek(Duration::from_millis(700))
            .unwrap_or_else(|error| panic!("{name} should seek: {error}"));
        let actual = tracked.get_pos();
        assert!(
            (Duration::from_millis(650)..=Duration::from_millis(850)).contains(&actual),
            "seeked {name} position should remain near 700 ms, got {actual:?}"
        );
    }
}

#[test]
fn aac_lc_m4a_decoder_reports_duration_and_decodes_frames_without_output_device() {
    let fixture_path = fixture("aac-lc.m4a");
    let mut inputs = vec![(fixture_path, Some(Duration::from_secs(1)))];
    if let Some(path) = std::env::var_os("MOEMUSICPLAYER_AAC_M4A_PATH") {
        inputs.push((PathBuf::from(path), None));
    }

    for (path, expected_duration) in inputs {
        assert_aac_lc_m4a_decodes(&path, expected_duration);
    }
}

fn assert_aac_lc_m4a_decodes(path: &Path, expected_duration: Option<Duration>) {
    let file = File::open(path)
        .unwrap_or_else(|error| panic!("could not open AAC-LC M4A input {path:?}: {error}"));
    let decoder = Decoder::try_from(file)
        .unwrap_or_else(|error| panic!("could not decode AAC-LC M4A input {path:?}: {error}"));
    let duration = decoder
        .total_duration()
        .unwrap_or_else(|| panic!("AAC-LC M4A input {path:?} should report a duration"));

    if let Some(expected_duration) = expected_duration {
        assert!(
            duration.abs_diff(expected_duration) <= Duration::from_millis(100),
            "expected AAC-LC M4A input {path:?} duration near {expected_duration:?}, got {duration:?}"
        );
    }

    let mut decoded_samples = 0u64;
    let mut nonzero_samples = 0u64;
    for sample in decoder {
        decoded_samples += 1;
        if sample != 0.0 {
            nonzero_samples += 1;
        }
    }
    assert!(
        decoded_samples > 0,
        "AAC-LC M4A input {path:?} should decode samples"
    );
    assert!(
        nonzero_samples > 0,
        "AAC-LC M4A input {path:?} should contain nonzero decoded samples"
    );
    eprintln!(
        "decoded AAC-LC M4A: path={path:?}, duration={duration:?}, decoded_samples={decoded_samples}, nonzero_samples={nonzero_samples}"
    );
}

#[test]
fn rodio_player_advances_one_second_per_output_second_at_default_speed() {
    let output_rate = NonZeroU32::new(96_000).unwrap();
    let channels = NonZeroU16::new(2).unwrap();
    let (mixer, mut output) = mixer::mixer(channels, output_rate);
    let player = Player::connect_new(&mixer);
    let input_rate = NonZeroU32::new(48_000).unwrap();
    let input_channels = NonZeroU16::new(2).unwrap();
    let samples = vec![0.0; input_rate.get() as usize * 2 * 2];

    player.append(SamplesBuffer::new(input_channels, input_rate, samples));
    assert_eq!(player.speed(), 1.0);
    player.play();

    for _ in 0..(output_rate.get() as usize * channels.get() as usize) {
        assert!(output.next().is_some());
    }

    let position = player.get_pos();
    assert!(
        (Duration::from_millis(950)..=Duration::from_millis(1_050)).contains(&position),
        "after one second of output frames, expected about one second of position; got {position:?}"
    );
}

#[test]
fn mp3_player_position_advances_at_the_mixer_output_rate() {
    let output_rate = NonZeroU32::new(96_000).unwrap();
    let channels = NonZeroU16::new(2).unwrap();
    let (mixer, mut output) = mixer::mixer(channels, output_rate);
    let player = Player::connect_new(&mixer);
    let decoder = Decoder::try_from(File::open(fixture("seek-tone.mp3")).unwrap()).unwrap();

    player.append(decoder);
    assert_eq!(player.speed(), 1.0);
    player.play();

    for _ in 0..(output_rate.get() as usize * channels.get() as usize) {
        assert!(output.next().is_some());
    }

    let position = player.get_pos();
    assert!(
        (Duration::from_millis(950)..=Duration::from_millis(1_050)).contains(&position),
        "after one second of 96 kHz mixer output, expected about one second of MP3 position; got {position:?}"
    );
}
