//! Exclusive-mode formats (ADR 0020): the sample containers an endpoint can take, the order they are tried in, the
//! sample rates tried when the endpoint refuses the track's own rate, and the `f32` to integer encoding. All pure, so
//! it is tested without a device.

/// How one sample sits in the buffer the endpoint reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Container {
    Int16,
    /// 24 valid bits left-justified in a 32-bit word.
    Int24In32,
    Int24Packed,
    Int32,
}

impl Container {
    /// Tried in this order: 24-in-32 holds every source of 24 bits or less exactly and is what most DACs want, then
    /// packed 24, then 32, and 16 last (a deeper source would lose bits there).
    pub(crate) const PREFERENCE: [Container; 4] = [
        Container::Int24In32,
        Container::Int24Packed,
        Container::Int32,
        Container::Int16,
    ];

    pub(crate) fn bytes_per_sample(self) -> usize {
        match self {
            Container::Int16 => 2,
            Container::Int24Packed => 3,
            Container::Int24In32 | Container::Int32 => 4,
        }
    }

    /// `wBitsPerSample` of the format.
    pub(crate) fn container_bits(self) -> u16 {
        (self.bytes_per_sample() * 8) as u16
    }

    /// `wValidBitsPerSample` of the format.
    pub(crate) fn valid_bits(self) -> u16 {
        match self {
            Container::Int16 => 16,
            Container::Int24In32 | Container::Int24Packed => 24,
            Container::Int32 => 32,
        }
    }
}

/// What the endpoint stream is opened with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamFormat {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
    pub(crate) container: Container,
}

impl StreamFormat {
    pub(crate) fn frame_bytes(self) -> usize {
        self.container.bytes_per_sample() * usize::from(self.channels)
    }
}

/// Anything that can say whether the endpoint accepts a format (the real endpoint asks WASAPI; tests answer from a
/// list).
pub(crate) trait FormatProbe {
    fn supports(&mut self, format: &StreamFormat) -> bool;
}

const FAMILY_44: [u32; 5] = [44_100, 88_200, 176_400, 352_800, 705_600];
const FAMILY_48: [u32; 4] = [48_000, 96_000, 192_000, 384_000];

/// The rates to try for a track at `native`, best first: the track's own rate, then integer multiples of it in the
/// same family, then the other family (at or above the native rate, closest first), then lower rates (closest first).
pub(crate) fn candidate_rates(native: u32) -> Vec<u32> {
    let (same, other): (&[u32], &[u32]) = if native.is_multiple_of(44_100) {
        (&FAMILY_44, &FAMILY_48)
    } else {
        (&FAMILY_48, &FAMILY_44)
    };
    let mut rates = vec![native];
    let push = |rate: u32, rates: &mut Vec<u32>| {
        if !rates.contains(&rate) {
            rates.push(rate);
        }
    };
    for &rate in same
        .iter()
        .filter(|&&rate| rate > native && rate % native == 0)
    {
        push(rate, &mut rates);
    }
    let mut higher: Vec<u32> = same
        .iter()
        .chain(other.iter())
        .copied()
        .filter(|&rate| rate > native)
        .collect();
    higher.sort_unstable();
    for rate in higher {
        push(rate, &mut rates);
    }
    let mut lower: Vec<u32> = same
        .iter()
        .chain(other.iter())
        .copied()
        .filter(|&rate| rate < native)
        .collect();
    lower.sort_unstable_by(|a, b| b.cmp(a));
    for rate in lower {
        push(rate, &mut rates);
    }
    rates
}

/// The outcome of negotiating a track with an endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Negotiated {
    pub(crate) format: StreamFormat,
    pub(crate) source_rate: u32,
    pub(crate) source_channels: u16,
    /// The stream carries DSD as DoP words (rate = DSD rate / 16, stereo, nothing may touch the samples).
    pub(crate) dop: bool,
}

impl Negotiated {
    /// The track's rate had to be converted.
    pub(crate) fn resampled(&self) -> bool {
        self.format.rate != self.source_rate
    }

    /// The channel layout had to be changed (the device does not take the track's channel count).
    pub(crate) fn remixed(&self) -> bool {
        self.format.channels != self.source_channels
    }
}

/// Find the format to open the endpoint with for a track: its own channel count first (then stereo), and for each
/// rate in `candidate_rates` the containers in `Container::PREFERENCE`. The rate is the outer choice (a refused rate
/// is resampled to the nearest accepted one; the container only changes how the same samples are laid out).
pub(crate) fn negotiate(
    probe: &mut (impl FormatProbe + ?Sized),
    source_rate: u32,
    source_channels: u16,
) -> Option<Negotiated> {
    let mut channel_options = vec![source_channels.max(1)];
    if source_channels != 2 {
        channel_options.push(2);
    }
    for rate in candidate_rates(source_rate) {
        for &channels in &channel_options {
            for container in Container::PREFERENCE {
                let format = StreamFormat {
                    rate,
                    channels,
                    container,
                };
                if probe.supports(&format) {
                    return Some(Negotiated {
                        format,
                        source_rate,
                        source_channels,
                        dop: false,
                    });
                }
            }
        }
    }
    None
}

/// The containers a DoP stream is tried in: 24 valid bits left-justified in 32 first (what DACs expect), then packed 24,
/// then a full 32-bit container with the 24 bits at the top. Never 16 bit: the marker byte would be lost.
pub(crate) const DOP_CONTAINERS: [Container; 3] = [
    Container::Int24In32,
    Container::Int24Packed,
    Container::Int32,
];

/// Find the format for DoP at exactly `rate` (the DSD rate / 16): stereo, the first accepted container. DoP cannot be
/// resampled, so a refused rate means no DoP.
pub(crate) fn negotiate_dop(
    probe: &mut (impl FormatProbe + ?Sized),
    rate: u32,
) -> Option<Negotiated> {
    DOP_CONTAINERS.into_iter().find_map(|container| {
        let format = StreamFormat {
            rate,
            channels: 2,
            container,
        };
        probe.supports(&format).then_some(Negotiated {
            format,
            source_rate: rate,
            source_channels: 2,
            dop: true,
        })
    })
}

/// Scale between a 24-bit integer and the `f32` the sample pipeline carries it as.
const INT24_SCALE: f32 = 8_388_608.0;

/// The `f32` that carries a 24-bit DoP word through the sample ring (exact: 24 bits fit the mantissa).
pub(crate) fn dop_word_to_sample(word: u32) -> f32 {
    ((word << 8) as i32 >> 8) as f32 / INT24_SCALE
}

/// The `f32` for the DoP idle pattern (the marker is stamped by [`encode_dop`]).
pub(crate) fn dop_idle_sample() -> f32 {
    f32::from(player_dsd::dop::DOP_IDLE_BITS) / INT24_SCALE
}

/// Append DoP frames to `out`: the 16 DSD bits of every sample (the low 16 bits of the carried word) under a marker
/// that alternates frame by frame. The marker comes from `phase`, which the render thread owns, so it stays unbroken
/// across seeks, track changes, pauses and idle fill, whatever markers the sources wrote.
pub(crate) fn encode_dop(
    samples: &[f32],
    channels: usize,
    container: Container,
    phase: &mut usize,
    out: &mut Vec<u8>,
) {
    out.reserve(samples.len() * container.bytes_per_sample());
    for frame in samples.chunks(channels.max(1)) {
        let marker = u32::from(player_dsd::dop::DOP_MARKERS[*phase & 1]);
        for &sample in frame {
            let bits = ((sample * INT24_SCALE).round() as i32 as u32) & 0xFFFF;
            let word = (marker << 16) | bits;
            match container {
                Container::Int24Packed => out.extend_from_slice(&word.to_le_bytes()[..3]),
                _ => out.extend_from_slice(&(word << 8).to_le_bytes()),
            }
        }
        *phase ^= 1;
    }
}

/// Append `samples` (interleaved `f32`, -1.0 to 1.0) to `out` as the container's little-endian integers, rounded to
/// nearest and clamped, without dither. Sources of 24 bits or less come back exactly: the decoders divide integers by
/// 2^(N-1), which `f32` holds exactly up to 24 bits, and this multiplies by the same power of two.
pub(crate) fn encode(samples: &[f32], container: Container, out: &mut Vec<u8>) {
    out.reserve(samples.len() * container.bytes_per_sample());
    for &sample in samples {
        let sample = if sample.is_nan() { 0.0 } else { sample };
        match container {
            Container::Int16 => {
                let value = (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
                out.extend_from_slice(&value.to_le_bytes());
            }
            Container::Int24In32 => {
                let value = (sample * 8_388_608.0)
                    .round()
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend_from_slice(&(value << 8).to_le_bytes());
            }
            Container::Int24Packed => {
                let value = (sample * 8_388_608.0)
                    .round()
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend_from_slice(&value.to_le_bytes()[..3]);
            }
            Container::Int32 => {
                let value = (f64::from(sample) * 2_147_483_648.0)
                    .round()
                    .clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
}

/// Silence for `frames` frames of the format.
pub(crate) fn silence(format: StreamFormat, frames: usize, out: &mut Vec<u8>) {
    out.resize(out.len() + frames * format.frame_bytes(), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Accepts(Vec<StreamFormat>);

    impl FormatProbe for Accepts {
        fn supports(&mut self, format: &StreamFormat) -> bool {
            self.0.contains(format)
        }
    }

    fn format(rate: u32, channels: u16, container: Container) -> StreamFormat {
        StreamFormat {
            rate,
            channels,
            container,
        }
    }

    #[test]
    fn dop_negotiates_the_exact_rate_with_24_bit_containers_in_order_and_never_resamples() {
        let all = [
            format(176_400, 2, Container::Int16),
            format(176_400, 2, Container::Int32),
            format(176_400, 2, Container::Int24Packed),
            format(176_400, 2, Container::Int24In32),
        ];
        let negotiated = negotiate_dop(&mut Accepts(all.to_vec()), 176_400).unwrap();
        assert_eq!(negotiated.format, format(176_400, 2, Container::Int24In32));
        assert!(negotiated.dop && !negotiated.resampled() && !negotiated.remixed());
        let packed = negotiate_dop(&mut Accepts(all[..3].to_vec()), 176_400).unwrap();
        assert_eq!(packed.format.container, Container::Int24Packed);
        let wide = negotiate_dop(&mut Accepts(all[..2].to_vec()), 176_400).unwrap();
        assert_eq!(wide.format.container, Container::Int32);
        assert_eq!(
            negotiate_dop(&mut Accepts(all[..1].to_vec()), 176_400),
            None,
            "16 bit cannot carry DoP"
        );
        assert_eq!(
            negotiate_dop(
                &mut Accepts(vec![format(352_800, 2, Container::Int24In32)]),
                176_400
            ),
            None,
            "another rate is no DoP"
        );
    }

    #[test]
    fn dop_words_survive_the_f32_ring_and_get_the_render_threads_marker() {
        // The source carries any marker; encoding replaces it with the unbroken alternation and keeps the 16 bits.
        let words = [0x05_1234_u32, 0xFA_9ABC, 0x05_0000, 0xFA_FFFF];
        let samples: Vec<f32> = words.iter().map(|&word| dop_word_to_sample(word)).collect();
        let mut phase = 1; // the previous frame ended on 0x05, so this one must be 0xFA
        let mut out = Vec::new();
        encode_dop(&samples, 2, Container::Int24In32, &mut phase, &mut out);
        let decoded: Vec<i32> = out
            .chunks(4)
            .map(|chunk| i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) >> 8)
            .collect();
        assert_eq!(
            decoded,
            [
                0xFA_1234_u32 as i32 - 0x0100_0000,
                0xFA_9ABC_u32 as i32 - 0x0100_0000,
                0x05_0000,
                0x05_FFFF
            ]
        );
        assert_eq!(phase, 1);
    }

    #[test]
    fn dop_encoding_fits_every_container() {
        let samples = [dop_word_to_sample(0x05_ABCD), dop_word_to_sample(0x05_1357)];
        let mut phase = 0;
        let mut packed = Vec::new();
        encode_dop(&samples, 2, Container::Int24Packed, &mut phase, &mut packed);
        assert_eq!(packed, [0xCD, 0xAB, 0x05, 0x57, 0x13, 0x05]);
        let mut phase = 0;
        let mut wide = Vec::new();
        encode_dop(&samples, 2, Container::Int32, &mut phase, &mut wide);
        assert_eq!(wide, [0x00, 0xCD, 0xAB, 0x05, 0x00, 0x57, 0x13, 0x05]);
        assert!((dop_idle_sample() * 8_388_608.0).round() as u32 == 0x6969);
    }

    #[test]
    fn rates_are_tried_native_first_then_same_family_multiples_then_the_other_family_then_lower() {
        assert_eq!(
            candidate_rates(44_100),
            [44_100, 88_200, 176_400, 352_800, 705_600, 48_000, 96_000, 192_000, 384_000]
        );
        assert_eq!(
            candidate_rates(96_000),
            [96_000, 192_000, 384_000, 176_400, 352_800, 705_600, 88_200, 48_000, 44_100]
        );
        // A rate outside both families is tried first, then an exact multiple (3x), then the standard rates above it.
        let odd = candidate_rates(32_000);
        assert_eq!(odd[0], 32_000);
        assert_eq!(odd[..3], [32_000, 96_000, 192_000]);
        assert!(!odd.contains(&0));
        assert_eq!(candidate_rates(22_050)[1], 44_100);
    }

    #[test]
    fn a_device_that_takes_the_native_rate_gets_it_in_the_first_container_it_accepts() {
        let mut device = Accepts(vec![
            format(44_100, 2, Container::Int32),
            format(44_100, 2, Container::Int16),
        ]);
        let negotiated = negotiate(&mut device, 44_100, 2).unwrap();
        assert_eq!(negotiated.format, format(44_100, 2, Container::Int32));
        assert!(!negotiated.resampled());
        assert!(!negotiated.remixed());
    }

    #[test]
    fn a_refused_rate_goes_to_the_nearest_accepted_rate_and_is_marked_resampled() {
        // A DAC that only takes 96 kHz and 192 kHz: 44.1 kHz material is converted to the closest rate it takes
        // (176.4 kHz is not accepted, so the other family at or above the native rate comes next: 48, then 96).
        let mut device = Accepts(vec![
            format(96_000, 2, Container::Int24In32),
            format(192_000, 2, Container::Int24In32),
        ]);
        let negotiated = negotiate(&mut device, 44_100, 2).unwrap();
        assert_eq!(negotiated.format.rate, 96_000);
        assert!(negotiated.resampled());
        // Material that fits the device's family keeps an integer ratio.
        let mut device = Accepts(vec![
            format(88_200, 2, Container::Int24In32),
            format(48_000, 2, Container::Int24In32),
        ]);
        assert_eq!(
            negotiate(&mut device, 44_100, 2).unwrap().format.rate,
            88_200
        );
    }

    #[test]
    fn lower_rates_are_the_last_resort() {
        let mut device = Accepts(vec![format(44_100, 2, Container::Int16)]);
        let negotiated = negotiate(&mut device, 96_000, 2).unwrap();
        assert_eq!(negotiated.format, format(44_100, 2, Container::Int16));
        assert!(negotiated.resampled());
    }

    #[test]
    fn a_track_with_more_channels_than_the_device_takes_falls_back_to_stereo() {
        let mut device = Accepts(vec![format(48_000, 2, Container::Int24In32)]);
        let negotiated = negotiate(&mut device, 48_000, 6).unwrap();
        assert_eq!(negotiated.format.channels, 2);
        assert!(negotiated.remixed());
        assert!(!negotiated.resampled());
    }

    #[test]
    fn a_device_that_takes_nothing_gives_no_format() {
        assert_eq!(negotiate(&mut Accepts(Vec::new()), 44_100, 2), None);
    }

    #[test]
    fn sixteen_and_twenty_four_bit_samples_round_trip_exactly() {
        // The decoders produce n / 2^(bits-1); encoding must give n back for every value.
        let sixteen: Vec<i16> = [-32_768, -32_767, -12_345, -1, 0, 1, 777, 32_766, 32_767].into();
        let samples: Vec<f32> = sixteen
            .iter()
            .map(|&value| f32::from(value) / 32_768.0)
            .collect();
        let mut out = Vec::new();
        encode(&samples, Container::Int16, &mut out);
        let decoded: Vec<i16> = out
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(decoded, sixteen);

        let twenty_four: Vec<i32> = [
            -8_388_608, -8_388_607, -1_234_567, -1, 0, 1, 7_654_321, 8_388_607,
        ]
        .into();
        let samples: Vec<f32> = twenty_four
            .iter()
            .map(|&value| value as f32 / 8_388_608.0)
            .collect();
        let mut packed = Vec::new();
        encode(&samples, Container::Int24Packed, &mut packed);
        let decoded: Vec<i32> = packed
            .chunks(3)
            .map(|bytes| {
                i32::from_le_bytes([
                    bytes[0],
                    bytes[1],
                    bytes[2],
                    if bytes[2] & 0x80 != 0 { 0xFF } else { 0 },
                ])
            })
            .collect();
        assert_eq!(decoded, twenty_four);
        let mut word = Vec::new();
        encode(&samples, Container::Int24In32, &mut word);
        let decoded: Vec<i32> = word
            .chunks(4)
            .map(|bytes| i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 8)
            .collect();
        assert_eq!(
            decoded, twenty_four,
            "24 valid bits, left-justified in the 32-bit word"
        );
        assert!(
            word.chunks(4).all(|bytes| bytes[0] == 0),
            "the low byte of a 24-in-32 word is zero"
        );
    }

    #[test]
    fn full_scale_and_out_of_range_samples_clamp_and_nan_is_silence() {
        let mut out = Vec::new();
        encode(&[1.0, -1.0, 2.0, f32::NAN], Container::Int16, &mut out);
        let values: Vec<i16> = out
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(values, [32_767, -32_768, 32_767, 0]);
        let mut out = Vec::new();
        encode(&[1.0, -1.0], Container::Int32, &mut out);
        let values: Vec<i32> = out
            .chunks(4)
            .map(|word| i32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        assert_eq!(values, [i32::MAX, i32::MIN]);
    }

    #[test]
    fn silence_is_zero_bytes_of_the_right_length() {
        let mut out = Vec::new();
        silence(format(48_000, 2, Container::Int24Packed), 10, &mut out);
        assert_eq!(out.len(), 10 * 2 * 3);
        assert!(out.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn container_sizes_match_the_wave_format_fields() {
        assert_eq!(
            Container::PREFERENCE.map(|c| (c.container_bits(), c.valid_bits())),
            [(32, 24), (24, 24), (32, 32), (16, 16)]
        );
    }
}
