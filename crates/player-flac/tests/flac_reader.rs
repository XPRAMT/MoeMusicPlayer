//! The FLAC reader and its WAV stream against fixtures encoded by the reference `flac` encoder (sine tones; see
//! `tests/fixtures`). Decoding is checked against the MD5 signature in STREAMINFO.

use std::fs::File;
use std::path::PathBuf;

use player_flac::wav::{FlacWavStream, WAV_HEADER_LEN};
use player_flac::FlacReader;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::default::codecs::FlacDecoder;
use symphonia::default::formats::FlacReader as FlacDemuxer;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Every sample of the file, decoded by Symphonia with the STREAMINFO MD5 check turned on.
fn verified_samples(name: &str) -> Vec<f32> {
    let stream = MediaSourceStream::new(
        Box::new(File::open(fixture(name)).unwrap()),
        MediaSourceStreamOptions::default(),
    );
    let mut demuxer = FlacDemuxer::try_new(stream, &FormatOptions::default()).unwrap();
    let params = demuxer.default_track().unwrap().codec_params.clone();
    let mut decoder = FlacDecoder::try_new(&params, &DecoderOptions { verify: true }).unwrap();
    let mut samples = Vec::new();
    while let Ok(packet) = demuxer.next_packet() {
        let decoded = decoder.decode(&packet).unwrap();
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    assert_eq!(decoder.finalize().verify_ok, Some(true), "{name}: MD5");
    samples
}

fn read_all(reader: &mut FlacReader) -> Vec<f32> {
    let mut samples = Vec::new();
    while reader.read_frames(&mut samples).unwrap() > 0 {}
    samples
}

const FIXTURES: &[(&str, u32, u16, u32, u64)] = &[
    // name, rate, channels, bits, frames
    ("s16_stereo_44100.flac", 44_100, 2, 16, 44_100),
    ("s24_stereo_96000.flac", 96_000, 2, 24, 24_000),
    ("s24_mono_48000.flac", 48_000, 1, 24, 14_400),
    ("s16_6ch_48000.flac", 48_000, 6, 16, 9_600),
];

#[test]
fn every_fixture_decodes_to_the_verified_samples_with_its_stream_info() {
    for &(name, rate, channels, bits, frames) in FIXTURES {
        let mut reader = FlacReader::open_path(&fixture(name)).unwrap();
        let info = *reader.info();
        assert_eq!(info.sample_rate, rate, "{name}");
        assert_eq!(info.channels, channels, "{name}");
        assert_eq!(info.bits_per_sample, Some(bits), "{name}");
        assert_eq!(info.total_frames, Some(frames), "{name}");
        let samples = read_all(&mut reader);
        assert_eq!(samples.len() as u64, frames * u64::from(channels), "{name}");
        assert_eq!(samples, verified_samples(name), "{name}");
        assert!(
            samples.iter().any(|sample| sample.abs() > 0.1),
            "{name}: not silent"
        );
        assert_eq!(
            reader.read_frames(&mut Vec::new()).unwrap(),
            0,
            "{name}: stays at the end"
        );
    }
}

#[test]
fn samples_keep_their_full_bit_depth() {
    let full_scale = |bits: u32| f64::from(1_u32 << (bits - 1));
    for &(name, _, _, bits, _) in FIXTURES {
        let samples = read_all(&mut FlacReader::open_path(&fixture(name)).unwrap());
        let scale = full_scale(bits);
        for sample in &samples {
            let value = f64::from(*sample) * scale;
            assert_eq!(
                value,
                value.round(),
                "{name}: {sample} is not a {bits}-bit value"
            );
        }
        if bits == 24 {
            // A 24-bit file has samples that a 16-bit decode would have lost.
            assert!(
                samples
                    .iter()
                    .any(|sample| (f64::from(*sample) * 32_768.0).fract() != 0.0),
                "{name}: only 16-bit values"
            );
        }
    }
}

#[test]
fn seeking_anywhere_gives_the_same_samples_as_reading_in_order() {
    for &(name, _, channels, _, frames) in FIXTURES {
        let whole = verified_samples(name);
        let channels = usize::from(channels);
        let mut reader = FlacReader::open_path(&fixture(name)).unwrap();
        // Frame boundaries of the 1152-sample blocks, positions inside blocks, backwards and forwards.
        let mut targets = vec![
            frames / 2,
            0,
            1,
            1_151,
            1_152,
            1_153,
            frames / 3,
            frames - 1,
            4_097,
            7,
        ];
        targets.retain(|target| *target < frames);
        for target in targets {
            reader.seek_to_frame(target).unwrap();
            assert_eq!(reader.position(), target);
            let mut got = Vec::new();
            while got.len() < 3_000 * channels && reader.read_frames(&mut got).unwrap() > 0 {}
            let start = target as usize * channels;
            let end = (start + got.len()).min(whole.len());
            assert!(!got.is_empty(), "{name} at {target}");
            let first_difference = got[..end - start]
                .iter()
                .zip(&whole[start..end])
                .position(|(a, b)| a != b);
            assert_eq!(
                first_difference,
                None,
                "{name} at {target} (got {})",
                got.len()
            );
        }
        reader.seek_to_frame(frames + 1_000).unwrap();
        assert_eq!(reader.position(), frames, "{name}: clamped to the end");
        assert_eq!(reader.read_frames(&mut Vec::new()).unwrap(), 0);
        reader.seek_to_frame(0).unwrap();
        assert_eq!(
            read_all(&mut reader),
            whole,
            "{name}: again from the start after the end"
        );
    }
}

#[test]
fn a_leading_id3v2_tag_is_skipped() {
    let mut bytes = b"ID3\x03\x00\x00\x00\x00\x00\x20".to_vec();
    bytes.extend(std::iter::repeat_n(0_u8, 32));
    bytes.extend(std::fs::read(fixture("s24_mono_48000.flac")).unwrap());
    let mut reader = FlacReader::open(std::io::Cursor::new(bytes)).unwrap();
    let whole = read_all(&mut reader);
    assert_eq!(whole, verified_samples("s24_mono_48000.flac"));
    reader.seek_to_frame(10_000).unwrap();
    let mut got = Vec::new();
    reader.read_frames(&mut got).unwrap();
    assert_eq!(got[..], whole[10_000..10_000 + got.len()]);
}

fn read_wav(stream: &mut FlacWavStream, chunk: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buffer = vec![0_u8; chunk];
    loop {
        let count = stream.read_at(out.len() as u64, &mut buffer).unwrap();
        if count == 0 {
            return out;
        }
        out.extend_from_slice(&buffer[..count]);
    }
}

#[test]
fn the_wav_stream_has_the_right_header_and_the_decoded_samples() {
    for &(name, rate, channels, _, frames) in FIXTURES {
        let mut wav = FlacWavStream::new(FlacReader::open_path(&fixture(name)).unwrap()).unwrap();
        let bytes_per_frame = u64::from(channels) * 4;
        assert_eq!(
            wav.total_len(),
            WAV_HEADER_LEN + frames * bytes_per_frame,
            "{name}"
        );
        assert_eq!(wav.total_frames(), frames);
        assert_eq!(wav.sample_rate(), rate);
        assert_eq!(wav.channels(), usize::from(channels));
        let bytes = read_wav(&mut wav, 1_000);
        assert_eq!(bytes.len() as u64, wav.total_len(), "{name}");
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32_at(4) as u64, wav.total_len() - 8);
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16_at(20), 3, "IEEE float");
        assert_eq!(u16_at(22), channels);
        assert_eq!(u32_at(24), rate);
        assert_eq!(u32_at(28) as u64, u64::from(rate) * bytes_per_frame);
        assert_eq!(u16_at(32) as u64, bytes_per_frame);
        assert_eq!(u16_at(34), 32);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32_at(40) as u64, frames * bytes_per_frame);
        let served: Vec<f32> = bytes[44..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(served, verified_samples(name), "{name}");
    }
}

#[test]
fn the_wav_stream_reads_any_range_like_a_file() {
    let mut wav =
        FlacWavStream::new(FlacReader::open_path(&fixture("s16_stereo_44100.flac")).unwrap())
            .unwrap();
    let whole = read_wav(&mut wav, 777);
    let total = wav.total_len();
    for &(position, length) in &[
        (0_u64, 44_usize),
        (30, 100),
        (total / 2, 4_096),
        (1_000, 4_096),
        (77_777, 513),
        (total - 100, 5_000),
        (44 + 1_152 * 4 - 3, 10),
    ] {
        let mut buffer = vec![0_u8; length];
        let count = wav.read_at(position, &mut buffer).unwrap();
        let end = (position as usize + length).min(whole.len());
        assert_eq!(
            buffer[..count],
            whole[position as usize..end],
            "at {position}"
        );
    }
    let mut buffer = [0_u8; 16];
    assert_eq!(wav.read_at(total, &mut buffer).unwrap(), 0);
    assert_eq!(wav.read_at(total + 100, &mut buffer).unwrap(), 0);
}

/// A seek near the end (ADR 0036) jumps straight into the last FLAC frame: it gives the right samples, and the end
/// itself reads as the end, never as an error.
#[test]
fn the_wav_stream_jumps_into_the_last_frame_and_to_the_end() {
    for &(name, _, channels, _, frames) in FIXTURES {
        let bytes_per_frame = u64::from(channels) * 4;
        let whole = read_wav(
            &mut FlacWavStream::new(FlacReader::open_path(&fixture(name)).unwrap()).unwrap(),
            4_096,
        );
        for back in [1_u64, 2, 20, 500] {
            let mut wav =
                FlacWavStream::new(FlacReader::open_path(&fixture(name)).unwrap()).unwrap();
            let position = WAV_HEADER_LEN + (frames - back) * bytes_per_frame;
            let mut buffer = vec![0_u8; 4_096];
            let count = wav.read_at(position, &mut buffer).unwrap();
            assert_eq!(
                count as u64,
                (back * bytes_per_frame).min(4_096),
                "{name} -{back}"
            );
            assert_eq!(
                buffer[..count],
                whole[position as usize..position as usize + count],
                "{name} -{back}"
            );
        }
        let mut wav = FlacWavStream::new(FlacReader::open_path(&fixture(name)).unwrap()).unwrap();
        let mut buffer = [0_u8; 16];
        assert_eq!(
            wav.read_at(wav.total_len(), &mut buffer).unwrap(),
            0,
            "{name}"
        );
    }
}
