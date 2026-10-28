//! Turning the PCM Media3 hands over into the samples a USB audio streaming alt carries (ADR 0047).
//!
//! Media3's sink gives this output float PCM for every track the app plays (the FLAC, WavPack and DSD converters serve
//! float WAV, and `FloatAwareAudioProcessorChain` turns 16-bit decoder output into float); 16-, 24- and 32-bit integer
//! PCM are accepted too. A float sample of an integer source of n ≤ 24 bits is that integer divided by 2^(n−1), exactly
//! (`f32` has a 24-bit significand), so multiplying by 2^(bits−1) of the alt and rounding gives the original integer
//! back bit for bit, also into a wider container. A 32-bit integer source that went through float is not exact.
//!
//! Samples sit left-justified in their subslot (UAC2 Type I formats: the unused low bits are zero; a 4-byte subslot
//! with 24 valid bits is what Linux calls S32_LE), little-endian. The gain is the app volume; only exactly 1.0 leaves
//! the samples untouched.

/// The encoding of the PCM written to the output (`C.ENCODING_PCM_*`, little-endian, interleaved).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEncoding {
    F32,
    S16,
    /// 24-bit packed in 3 bytes.
    S24,
    S32,
}

impl InputEncoding {
    /// Android's `AudioFormat.ENCODING_PCM_*` (the same numbers as Media3's `C.ENCODING_PCM_*`).
    pub fn from_android(encoding: i32) -> Option<Self> {
        match encoding {
            2 => Some(Self::S16),
            4 => Some(Self::F32),
            21 => Some(Self::S24),
            22 => Some(Self::S32),
            _ => None,
        }
    }

    pub fn bytes(self) -> usize {
        match self {
            Self::S16 => 2,
            Self::S24 => 3,
            Self::F32 | Self::S32 => 4,
        }
    }
}

/// How one sample sits on the bus: the subslot size and the valid bits (bSubslotSize, bBitResolution).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub bytes: u8,
    pub bits: u8,
}

/// The sample as a signed integer of `bits` bits, rounded and clamped.
fn quantise(value: f64, bits: u8) -> i64 {
    let scale = (1_i64 << (bits - 1)) as f64;
    let max = (1_i64 << (bits - 1)) - 1;
    ((value * scale).round() as i64).clamp(-max - 1, max)
}

/// An integer sample of `from` bits as `to` bits: shifted when no gain applies (exact when widening).
fn rescale(sample: i64, from: u8, to: u8) -> i64 {
    if to >= from {
        sample << (to - from)
    } else {
        sample >> (from - to)
    }
}

fn read_sample(encoding: InputEncoding, bytes: &[u8]) -> (i64, u8, Option<f32>) {
    match encoding {
        InputEncoding::F32 => (
            0,
            0,
            Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        ),
        InputEncoding::S16 => (
            i64::from(i16::from_le_bytes([bytes[0], bytes[1]])),
            16,
            None,
        ),
        InputEncoding::S24 => (
            i64::from(i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8),
            24,
            None,
        ),
        InputEncoding::S32 => (
            i64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            32,
            None,
        ),
    }
}

/// One sample at the container's valid bits.
fn convert(encoding: InputEncoding, bytes: &[u8], container: Container, gain: f32) -> i64 {
    let (integer, integer_bits, float) = read_sample(encoding, bytes);
    let unity = gain == 1.0;
    match float {
        Some(value) => quantise(f64::from(value) * f64::from(gain), container.bits),
        None if unity => rescale(integer, integer_bits, container.bits),
        None => {
            let value = integer as f64 / (1_i64 << (integer_bits - 1)) as f64;
            quantise(value * f64::from(gain), container.bits)
        }
    }
}

/// Pack `input` (whole frames of `encoding`) into `out` as `container` samples, with `gain` applied. `out` must hold
/// exactly as many samples as `input`. Returns the bytes written.
pub fn pack(
    encoding: InputEncoding,
    input: &[u8],
    container: Container,
    gain: f32,
    out: &mut [u8],
) -> usize {
    let in_bytes = encoding.bytes();
    let out_bytes = usize::from(container.bytes);
    let shift = u32::from(container.bytes) * 8 - u32::from(container.bits);
    let samples = input.len() / in_bytes;
    debug_assert!(out.len() >= samples * out_bytes);
    for (sample, slot) in input
        .chunks_exact(in_bytes)
        .zip(out.chunks_exact_mut(out_bytes))
    {
        let value = convert(encoding, sample, container, gain) << shift;
        slot.copy_from_slice(&value.to_le_bytes()[..out_bytes]);
    }
    samples * out_bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use md5::{Digest, Md5};
    use player_flac::FlacReader;
    use std::path::{Path, PathBuf};

    const C16: Container = Container { bytes: 2, bits: 16 };
    const C24: Container = Container { bytes: 3, bits: 24 };
    const C4_24: Container = Container { bytes: 4, bits: 24 };
    const C32: Container = Container { bytes: 4, bits: 32 };

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn packed(encoding: InputEncoding, input: &[u8], container: Container, gain: f32) -> Vec<u8> {
        let mut out = vec![0; input.len() / encoding.bytes() * usize::from(container.bytes)];
        assert_eq!(pack(encoding, input, container, gain, &mut out), out.len());
        out
    }

    #[test]
    fn every_16_bit_sample_through_float_comes_back_exactly_in_each_container() {
        let input: Vec<f32> = (i16::MIN..=i16::MAX)
            .map(|s| f32::from(s) / 32_768.0)
            .collect();
        let bytes = floats(&input);
        let out = packed(InputEncoding::F32, &bytes, C16, 1.0);
        for (index, pair) in out.chunks_exact(2).enumerate() {
            assert_eq!(
                i32::from(i16::from_le_bytes([pair[0], pair[1]])),
                i32::from(i16::MIN) + index as i32
            );
        }
        // Wider containers: the same sample, shifted up (zero low bits).
        let out = packed(InputEncoding::F32, &bytes, C24, 1.0);
        for (index, triple) in out.chunks_exact(3).enumerate() {
            let value = i32::from_le_bytes([0, triple[0], triple[1], triple[2]]) >> 8;
            assert_eq!(value, (i32::from(i16::MIN) + index as i32) << 8);
        }
        let out = packed(InputEncoding::F32, &bytes, C32, 1.0);
        for (index, quad) in out.chunks_exact(4).enumerate() {
            let value = i32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]);
            assert_eq!(value, (i32::from(i16::MIN) + index as i32) << 16);
        }
    }

    #[test]
    fn every_24_bit_sample_through_float_comes_back_exactly() {
        let min = -(1_i32 << 23);
        let max = (1_i32 << 23) - 1;
        let mut out3 = [0_u8; 3];
        let mut out4 = [0_u8; 4];
        for sample in min..=max {
            let float = (sample as f32) / 8_388_608.0;
            let input = float.to_le_bytes();
            pack(InputEncoding::F32, &input, C24, 1.0, &mut out3);
            assert_eq!(
                i32::from_le_bytes([0, out3[0], out3[1], out3[2]]) >> 8,
                sample
            );
            // 4-byte subslot with 24 valid bits: left-justified, low byte zero.
            pack(InputEncoding::F32, &input, C4_24, 1.0, &mut out4);
            assert_eq!(out4[0], 0);
            assert_eq!(i32::from_le_bytes(out4) >> 8, sample);
        }
    }

    #[test]
    fn full_scale_is_clamped_and_integers_pass_through() {
        let out = packed(
            InputEncoding::F32,
            &floats(&[1.0, -1.0, 2.0, -2.0]),
            C16,
            1.0,
        );
        assert_eq!(out, [0xFF, 0x7F, 0x00, 0x80, 0xFF, 0x7F, 0x00, 0x80]);
        let s16: Vec<u8> = [-32_768_i16, 1, 32_767]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(packed(InputEncoding::S16, &s16, C16, 1.0), s16);
        let s24 = [0x01, 0x02, 0x83];
        assert_eq!(packed(InputEncoding::S24, &s24, C24, 1.0), s24);
        assert_eq!(
            packed(InputEncoding::S24, &s24, C4_24, 1.0),
            [0, 0x01, 0x02, 0x83]
        );
        let s32 = 0x1234_5678_i32.to_le_bytes();
        assert_eq!(packed(InputEncoding::S32, &s32, C32, 1.0), s32);
        // Narrowing a 32-bit integer drops its low bits.
        assert_eq!(
            packed(InputEncoding::S32, &s32, C24, 1.0),
            [0x56, 0x34, 0x12]
        );
    }

    #[test]
    fn a_gain_below_one_scales_and_one_leaves_the_samples_alone() {
        let input = floats(&[0.5, -0.25]);
        let half = packed(InputEncoding::F32, &input, C16, 0.5);
        assert_eq!(half, [0x00, 0x20, 0x00, 0xF0]);
        let s16 = 16_384_i16.to_le_bytes();
        assert_eq!(
            packed(InputEncoding::S16, &s16, C16, 0.5),
            8_192_i16.to_le_bytes()
        );
        assert_eq!(packed(InputEncoding::F32, &input, C16, 0.0), [0, 0, 0, 0]);
    }

    #[test]
    fn android_encodings_map_to_inputs() {
        assert_eq!(InputEncoding::from_android(2), Some(InputEncoding::S16));
        assert_eq!(InputEncoding::from_android(4), Some(InputEncoding::F32));
        assert_eq!(InputEncoding::from_android(21), Some(InputEncoding::S24));
        assert_eq!(InputEncoding::from_android(22), Some(InputEncoding::S32));
        assert_eq!(InputEncoding::from_android(5), None);
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../player-flac/tests/fixtures")
            .join(name)
    }

    /// The MD5 signature in STREAMINFO (the last 16 of its 34 bytes, right after "fLaC" and the block header).
    fn streaminfo_md5(path: &Path) -> [u8; 16] {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"fLaC");
        bytes[26..42].try_into().unwrap()
    }

    /// Decode with `player-flac` (float, as the Android converter serves it) and pack: the MD5 of the packed samples is
    /// the one the encoder signed over the original integers.
    fn assert_bit_exact(name: &str, container: Container) {
        let path = fixture(name);
        let mut reader = FlacReader::open_path(&path).unwrap();
        let mut samples = Vec::new();
        let mut packed_all = Vec::new();
        loop {
            samples.clear();
            if reader.read_frames(&mut samples).unwrap() == 0 {
                break;
            }
            packed_all.extend(packed(
                InputEncoding::F32,
                &floats(&samples),
                container,
                1.0,
            ));
        }
        let digest: [u8; 16] = Md5::digest(&packed_all).into();
        assert_eq!(digest, streaminfo_md5(&path), "{name}");
    }

    #[test]
    fn decoded_flac_packs_to_exactly_the_samples_its_md5_signs() {
        assert_bit_exact("s16_stereo_44100.flac", C16);
        assert_bit_exact("s24_stereo_96000.flac", C24);
    }
}
