//! Synthetic Ogg Opus files for tests (enabled for other crates by the `testing` feature).

use std::f64::consts::PI;
use std::io::Cursor;

use ogg::writing::{PacketWriteEndInfo, PacketWriter};

use crate::OPUS_RATE;

const SERIAL: u32 = 0x4d4f_4531;

/// An Ogg Opus file of `frames` frames (stereo or mono): `tone_hz` on the left, twice that on the right. The
    /// OpusTags packet carries `tags_padding` extra bytes (to force a packet across several pages).
pub fn write_opus(frames: usize, channels: u8, tone_hz: f64, gain_q8: i16, tags_padding: usize) -> Vec<u8> {
        let channels_enum = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };
        let mut encoder = opus::Encoder::new(OPUS_RATE, channels_enum, opus::Application::Audio).unwrap();
        encoder.set_bitrate(opus::Bitrate::Bits(128_000)).unwrap();
        let pre_skip = encoder.get_lookahead().unwrap() as u16;
        let mut bytes = Vec::new();
        {
            let mut writer = PacketWriter::new(Cursor::new(&mut bytes));
            let mut head = b"OpusHead".to_vec();
            head.push(1);
            head.push(channels);
            head.extend_from_slice(&pre_skip.to_le_bytes());
            head.extend_from_slice(&OPUS_RATE.to_le_bytes());
            head.extend_from_slice(&gain_q8.to_le_bytes());
            head.push(0);
            writer.write_packet(head, SERIAL, PacketWriteEndInfo::EndPage, 0).unwrap();
            let mut tags = b"OpusTags".to_vec();
            let vendor = b"moe-test";
            tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
            tags.extend_from_slice(vendor);
            tags.extend_from_slice(&1_u32.to_le_bytes());
            let comment = format!("TITLE={}", "x".repeat(tags_padding));
            tags.extend_from_slice(&(comment.len() as u32).to_le_bytes());
            tags.extend_from_slice(comment.as_bytes());
            writer.write_packet(tags, SERIAL, PacketWriteEndInfo::EndPage, 0).unwrap();

            const PACKET_FRAMES: usize = 960;
            let packets = (frames + usize::from(pre_skip)).div_ceil(PACKET_FRAMES);
            let mut sample_index = 0_usize;
            let mut output = vec![0_u8; 4000];
            for packet in 0..packets {
                let mut pcm = Vec::with_capacity(PACKET_FRAMES * usize::from(channels));
                for _ in 0..PACKET_FRAMES {
                    // The first `pre_skip` frames are lead-in silence so the tone starts at frame 0 of the audio.
                    let value = if sample_index < usize::from(pre_skip) {
                        0.0
                    } else {
                        sample_index as f64 - f64::from(pre_skip)
                    };
                    let live = sample_index >= usize::from(pre_skip) && (value as usize) < frames;
                    let t = value / f64::from(OPUS_RATE);
                    let left = if live { 0.4 * (2.0 * PI * tone_hz * t).sin() } else { 0.0 };
                    let right = if live { 0.4 * (2.0 * PI * tone_hz * 2.0 * t).sin() } else { 0.0 };
                    pcm.push(left as f32);
                    if channels == 2 {
                        pcm.push(right as f32);
                    }
                    sample_index += 1;
                }
                let size = encoder.encode_float(&pcm, &mut output).unwrap();
                let granule = (sample_index).min(frames + usize::from(pre_skip)) as u64;
                let last = packet + 1 == packets;
                let end = if last {
                    PacketWriteEndInfo::EndStream
                } else if packet % 8 == 7 {
                    PacketWriteEndInfo::EndPage
                } else {
                    PacketWriteEndInfo::NormalPacket
                };
                writer.write_packet(output[..size].to_vec(), SERIAL, end, granule).unwrap();
            }
        }
        bytes
    }

