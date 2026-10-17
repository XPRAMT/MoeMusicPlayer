//! Synthetic-signal helpers for tests (enabled for other crates by the `testing` feature): a sigma-delta modulator,
//! a DSF writer, a full-file converter and a sine fit.

use std::f64::consts::PI;

use crate::convert::{ConversionPlan, DsdPcmConverter};
use crate::dsf::{DATA_HEADER_SIZE, DATA_OFFSET, DSD_CHUNK_SIZE, FMT_CHUNK_SIZE};
use crate::stream::DSD_IDLE_BYTE;

/// Second-order sigma-delta modulator: `signal(i)` in [−0.7, 0.7] → MSB-first DSD bytes, `samples` bits.
pub fn modulate(samples: usize, signal: impl Fn(usize) -> f64) -> Vec<u8> {
    let (mut first, mut second, mut previous) = (0.0_f64, 0.0_f64, 1.0_f64);
    let mut bytes = vec![0_u8; samples.div_ceil(8)];
    for index in 0..samples {
        first += signal(index) - previous;
        second += first - previous;
        let bit = second >= 0.0;
        previous = if bit { 1.0 } else { -1.0 };
        if bit {
            bytes[index / 8] |= 0x80 >> (index % 8);
        }
    }
    bytes
}

pub fn tone(rate: u32, frequency: f64, amplitude: f64) -> impl Fn(usize) -> f64 {
    move |index| amplitude * (2.0 * PI * frequency * index as f64 / f64::from(rate)).sin()
}

/// Convert a mono byte stream completely, in chunks, with delay compensation and the exact frame count.
pub fn convert_all(plan: &ConversionPlan, bytes: &[u8], samples: usize, chunk: usize) -> Vec<f32> {
    let mut converter = DsdPcmConverter::new(plan.clone(), 1, 0.0);
    let total_frames = samples / plan.ratio;
    let mut discard = plan.discard_frames();
    let mut frames: Vec<f32> = Vec::new();
    let mut output = vec![Vec::new()];
    let mut position = 0;
    while frames.len() < total_frames {
        let input: Vec<u8> = if position < bytes.len() {
            let end = (position + chunk).min(bytes.len());
            let slice = bytes[position..end].to_vec();
            position = end;
            slice
        } else {
            vec![DSD_IDLE_BYTE; chunk]
        };
        converter.process(&[input], &mut output);
        for &sample in &output[0] {
            if discard > 0 {
                discard -= 1;
            } else if frames.len() < total_frames {
                frames.push(sample);
            }
        }
    }
    frames
}

/// Least-squares amplitude and delay of `frequency` (Hz) in `frames` sampled at `rate`, with a DC term. The
/// first element is frame number `first_index` of the stream, so delays are measured against absolute time.
/// Returns (amplitude, delay in frames: positive = the signal arrives late).
pub fn fit_sine(frames: &[f32], first_index: usize, rate: u32, frequency: f64) -> (f64, f64) {
    let omega = 2.0 * PI * frequency / f64::from(rate);
    let (mut cc, mut ss, mut cs, mut c1, mut s1, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut yc, mut ys, mut y1) = (0.0, 0.0, 0.0);
    for (index, &sample) in frames.iter().enumerate() {
        let value = f64::from(sample);
        let (sin, cos) = (omega * (first_index + index) as f64).sin_cos();
        cc += cos * cos;
        ss += sin * sin;
        cs += cos * sin;
        c1 += cos;
        s1 += sin;
        n += 1.0;
        yc += value * cos;
        ys += value * sin;
        y1 += value;
    }
    // Solve [cc cs c1; cs ss s1; c1 s1 n] · [a b c] = [yc ys y1].
    let determinant = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let matrix = [[cc, cs, c1], [cs, ss, s1], [c1, s1, n]];
    let d = determinant(matrix);
    let replace = |column: usize, vector: [f64; 3]| {
        let mut m = matrix;
        for row in 0..3 {
            m[row][column] = vector[row];
        }
        determinant(m)
    };
    let vector = [yc, ys, y1];
    let a = replace(0, vector) / d;
    let b = replace(1, vector) / d;
    // y ≈ a·cos(ωi) + b·sin(ωi) = A·sin(ωi + φ) with A·sin φ = a, A·cos φ = b.
    let amplitude = a.hypot(b);
    let phase = a.atan2(b);
    (amplitude, -phase / omega)
}

/// Build a DSF image. `channel_bytes[c]` are the bytes of channel `c` in **file order** (LSB-first when
/// `lsb_first`); each is zero-padded to whole blocks.
pub fn build_dsf(
    sample_rate: u32,
    samples_per_channel: u64,
    block_size: u32,
    lsb_first: bool,
    channel_bytes: &[Vec<u8>],
) -> Vec<u8> {
    let channels = channel_bytes.len() as u32;
    let per_channel = samples_per_channel.div_ceil(8) as usize;
    let blocks = per_channel.div_ceil(block_size as usize);
    let mut data = Vec::new();
    for block in 0..blocks {
        for channel in channel_bytes {
            let start = block * block_size as usize;
            for index in 0..block_size as usize {
                data.push(
                    *channel
                        .get(start + index)
                        .filter(|_| start + index < per_channel)
                        .unwrap_or(&0),
                );
            }
        }
    }
    let data_chunk_size = DATA_HEADER_SIZE + data.len() as u64;
    let total = DATA_OFFSET + data.len() as u64;
    let mut file = Vec::new();
    file.extend_from_slice(b"DSD ");
    file.extend_from_slice(&DSD_CHUNK_SIZE.to_le_bytes());
    file.extend_from_slice(&total.to_le_bytes());
    file.extend_from_slice(&0_u64.to_le_bytes());
    file.extend_from_slice(b"fmt ");
    file.extend_from_slice(&FMT_CHUNK_SIZE.to_le_bytes());
    file.extend_from_slice(&1_u32.to_le_bytes());
    file.extend_from_slice(&0_u32.to_le_bytes());
    file.extend_from_slice(&(if channels == 1 { 1_u32 } else { 2_u32 }).to_le_bytes());
    file.extend_from_slice(&channels.to_le_bytes());
    file.extend_from_slice(&sample_rate.to_le_bytes());
    file.extend_from_slice(&(if lsb_first { 1_u32 } else { 8_u32 }).to_le_bytes());
    file.extend_from_slice(&samples_per_channel.to_le_bytes());
    file.extend_from_slice(&block_size.to_le_bytes());
    file.extend_from_slice(&0_u32.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&data_chunk_size.to_le_bytes());
    file.extend_from_slice(&data);
    file
}
