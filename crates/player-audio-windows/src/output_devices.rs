//! Output devices (ADR 0020, phase 0): the list of available render endpoints, the per-device settings (volume, delay,
//! channel mapping) and the pure channel-mixing rules. Several devices can play the same track at once; the first
//! enabled device is the primary (it keeps the existing playback path), the others are mirrors (`mirror.rs`).

/// Longest delay a device can be given (the user aligns Bluetooth earbuds, a few hundred milliseconds at most).
pub const MAX_DELAY_MS: u32 = 2000;

/// One available output endpoint as the settings page lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputDeviceInfo {
    /// Stable endpoint id (CPAL's `wasapi:{…}`), what the settings store.
    pub id: String,
    pub name: String,
    /// The Windows default render endpoint right now.
    pub is_default: bool,
    /// Channels of the endpoint's shared-mode mix format.
    pub channels: u16,
    pub sample_rate: u32,
}

/// What a chosen device is doing right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceState {
    Playing,
    /// Being opened (or reopened after it dropped out).
    Opening,
    /// Could not be opened or stopped responding; opening is retried every few seconds.
    Lost,
}

/// The settings page's view of one chosen device (`id: None` is the system default).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputDeviceStatus {
    pub id: Option<String>,
    pub state: DeviceState,
    /// Why the device is not playing, or a note about it.
    pub message: Option<String>,
    /// The rate and channel count the device's stream runs at.
    pub device_rate: Option<u32>,
    pub channels: Option<u16>,
    /// A mirror device whose rate differs from the track's (the rate is converted for it).
    pub converted: bool,
}

/// Where one output channel of a device takes its sound from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelRoute {
    /// The source channel (0 = first channel of the track), or `None` for silence.
    pub source: Option<u8>,
    /// Gain of that channel, 0 to 100 percent.
    pub gain_percent: u8,
}

/// One device's settings. A device that is not in the list plays nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputDeviceSetting {
    /// `None` is "the system default": it follows whichever endpoint Windows has as default.
    pub id: Option<String>,
    pub enabled: bool,
    pub delay_ms: u32,
    /// `None` is automatic (a sensible downmix or upmix); otherwise one route per output channel of the device.
    pub channel_map: Option<Vec<ChannelRoute>>,
}

impl Default for OutputDeviceSetting {
    fn default() -> Self {
        Self {
            id: None,
            enabled: true,
            delay_ms: 0,
            channel_map: None,
        }
    }
}

impl OutputDeviceSetting {
    pub fn normalized(mut self) -> Self {
        self.delay_ms = self.delay_ms.min(MAX_DELAY_MS);
        if let Some(routes) = self.channel_map.as_mut() {
            routes.truncate(32);
            for route in routes.iter_mut() {
                route.gain_percent = route.gain_percent.min(100);
            }
        }
        self
    }
}

/// The devices that play, in order: the first is the primary. Disabled entries are dropped, and so is the system
/// default when a specific device is enabled. With no entry at all the
/// system default plays, as before this feature existed.
pub fn active_devices(settings: &[OutputDeviceSetting]) -> Vec<OutputDeviceSetting> {
    let mut active: Vec<OutputDeviceSetting> = Vec::new();
    for setting in settings.iter().filter(|setting| setting.enabled) {
        if !active.iter().any(|existing| existing.id == setting.id) {
            active.push(setting.clone().normalized());
        }
    }
    // A chosen device replaces the system default: with both, the same endpoint would play the music twice.
    if active.iter().any(|setting| setting.id.is_some()) {
        active.retain(|setting| setting.id.is_some());
    }
    if active.is_empty() {
        active.push(OutputDeviceSetting::default());
    }
    active
}

/// Names of the channels of a standard layout with `count` channels (what a shared-mode mix format uses).
pub fn channel_labels(count: usize) -> Vec<&'static str> {
    match count {
        0 => Vec::new(),
        1 => vec!["C"],
        2 => vec!["FL", "FR"],
        3 => vec!["FL", "FR", "C"],
        4 => vec!["FL", "FR", "BL", "BR"],
        5 => vec!["FL", "FR", "C", "BL", "BR"],
        6 => vec!["FL", "FR", "C", "LFE", "BL", "BR"],
        7 => vec!["FL", "FR", "C", "LFE", "BC", "SL", "SR"],
        8 => vec!["FL", "FR", "C", "LFE", "BL", "BR", "SL", "SR"],
        _ => (0..count).map(|_| "").collect(),
    }
}

/// For every output channel, the `(source channel, gain)` terms that are summed into it.
pub type MixTerms = Vec<Vec<(usize, f32)>>;

const SQRT_HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The automatic mapping from `source` channels to a device with `device` channels.
pub fn auto_terms(source: usize, device: usize) -> MixTerms {
    if device == 0 {
        return Vec::new();
    }
    if source == 0 {
        return vec![Vec::new(); device];
    }
    let mut terms: MixTerms = vec![Vec::new(); device];
    match (source, device) {
        (s, d) if s == d => {
            for (channel, slot) in terms.iter_mut().enumerate() {
                slot.push((channel, 1.0));
            }
        }
        // Mono goes to the two front channels (a mono track on 5.1 is not sent to the subwoofer).
        (1, d) => {
            terms[0].push((0, 1.0));
            if d >= 2 {
                terms[1].push((0, 1.0));
            }
        }
        // Anything to a mono device: the front pair summed at equal power.
        (s, 1) => {
            terms[0].push((0, SQRT_HALF));
            terms[0].push((1.min(s - 1), SQRT_HALF));
        }
        // 5.1 and 7.1 to stereo: the usual ITU downmix, the low-frequency channel left out.
        (6, 2) | (8, 2) => {
            terms[0] = vec![(0, 1.0), (2, SQRT_HALF), (4, SQRT_HALF)];
            terms[1] = vec![(1, 1.0), (2, SQRT_HALF), (5, SQRT_HALF)];
            if source == 8 {
                terms[0].push((6, SQRT_HALF));
                terms[1].push((7, SQRT_HALF));
            }
        }
        // Everything else: channel for channel as far as both go.
        (s, d) => {
            for (channel, term) in terms.iter_mut().enumerate().take(s.min(d)) {
                term.push((channel, 1.0));
            }
        }
    }
    terms
}

/// The terms of a custom channel map: one route per output channel; a missing or out-of-range route is silent.
pub fn route_terms(routes: &[ChannelRoute], source: usize, device: usize) -> MixTerms {
    (0..device)
        .map(|channel| match routes.get(channel) {
            Some(ChannelRoute {
                source: Some(index),
                gain_percent,
            }) if usize::from(*index) < source && *gain_percent > 0 => {
                vec![(usize::from(*index), f32::from(*gain_percent) / 100.0)]
            }
            _ => Vec::new(),
        })
        .collect()
}

/// Mixes one source frame into one device frame.
#[derive(Clone, Debug)]
pub struct ChannelMixer {
    terms: MixTerms,
    source_channels: usize,
}

impl ChannelMixer {
    pub fn new(map: Option<&[ChannelRoute]>, source: usize, device: usize) -> Self {
        let terms = match map {
            Some(routes) => route_terms(routes, source, device),
            None => auto_terms(source, device),
        };
        Self {
            terms,
            source_channels: source,
        }
    }

    pub fn device_channels(&self) -> usize {
        self.terms.len()
    }

    pub fn source_channels(&self) -> usize {
        self.source_channels
    }

    /// True when every output channel is the same source channel at full gain (nothing to mix).
    pub fn is_identity(&self) -> bool {
        self.terms.len() == self.source_channels
            && self.terms.iter().enumerate().all(|(channel, slot)| {
                slot.len() == 1 && slot[0].0 == channel && (slot[0].1 - 1.0).abs() < f32::EPSILON
            })
    }

    pub fn mix(&self, source: &[f32], out: &mut [f32]) {
        for (slot, terms) in out.iter_mut().zip(&self.terms) {
            *slot = terms
                .iter()
                .map(|&(channel, gain)| source.get(channel).copied().unwrap_or(0.0) * gain)
                .sum();
        }
    }
}

/// Every active render endpoint (Windows enumerates only the ones that are plugged in and enabled).
#[cfg(windows)]
pub fn list_output_devices() -> Vec<OutputDeviceInfo> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    let host = rodio::cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    let mut list: Vec<OutputDeviceInfo> = devices
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let name = device
                .description()
                // The name Windows shows in its sound settings ("耳機 (WF-1000XM6)"); CPAL keeps it as the first
                // extended line when it differs from the short device description ("耳機").
                .map(|description| {
                    description
                        .extended()
                        .first()
                        .cloned()
                        .unwrap_or_else(|| description.name().to_owned())
                })
                .unwrap_or_else(|_| id.clone());
            let config = device.default_output_config().ok();
            Some(OutputDeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                channels: config.as_ref().map_or(2, |config| config.channels()),
                sample_rate: config.as_ref().map_or(0, |config| config.sample_rate()),
                id,
                name,
            })
        })
        .collect();
    list.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    list
}

/// The CPAL device with this id, if it is plugged in.
#[cfg(windows)]
pub(crate) fn find_output_device(id: &str) -> Option<rodio::cpal::Device> {
    use rodio::cpal::traits::HostTrait;
    use std::str::FromStr;

    let device_id = rodio::cpal::DeviceId::from_str(id).ok()?;
    rodio::cpal::default_host().device_by_id(&device_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(source: Option<u8>, gain: u8) -> ChannelRoute {
        ChannelRoute {
            source,
            gain_percent: gain,
        }
    }

    #[test]
    fn a_chosen_device_replaces_the_system_default() {
        let default = OutputDeviceSetting::default();
        let chosen = OutputDeviceSetting {
            id: Some("a".into()),
            ..OutputDeviceSetting::default()
        };
        let active = active_devices(&[default, chosen.clone()]);
        assert_eq!(active, vec![chosen]);
    }

    #[test]
    fn no_entry_means_the_system_default_and_duplicates_and_disabled_entries_drop_out() {
        assert_eq!(active_devices(&[]), vec![OutputDeviceSetting::default()]);
        let disabled = OutputDeviceSetting {
            id: Some("a".into()),
            enabled: false,
            ..OutputDeviceSetting::default()
        };
        assert_eq!(
            active_devices(std::slice::from_ref(&disabled)),
            vec![OutputDeviceSetting::default()]
        );
        let one = OutputDeviceSetting {
            id: Some("a".into()),
            ..OutputDeviceSetting::default()
        };
        let two = OutputDeviceSetting {
            id: Some("b".into()),
            delay_ms: 120,
            ..OutputDeviceSetting::default()
        };
        let active = active_devices(&[one.clone(), disabled, two.clone(), one.clone()]);
        assert_eq!(
            active,
            vec![one, two],
            "order kept, the first is the primary, duplicates dropped"
        );
    }

    #[test]
    fn settings_are_clamped() {
        let setting = OutputDeviceSetting {
            delay_ms: 99_999,
            channel_map: Some(vec![route(Some(0), 200)]),
            ..OutputDeviceSetting::default()
        }
        .normalized();
        assert_eq!(setting.delay_ms, MAX_DELAY_MS);
        assert_eq!(setting.channel_map.unwrap()[0].gain_percent, 100);
    }

    #[test]
    fn standard_layouts_have_their_channel_names() {
        assert_eq!(channel_labels(2), ["FL", "FR"]);
        assert_eq!(channel_labels(6), ["FL", "FR", "C", "LFE", "BL", "BR"]);
        assert_eq!(channel_labels(8).len(), 8);
    }

    #[test]
    fn automatic_mapping_follows_the_usual_rules() {
        let mix = |source: usize, device: usize, frame: &[f32]| {
            let mixer = ChannelMixer::new(None, source, device);
            let mut out = vec![9.0; device];
            mixer.mix(frame, &mut out);
            out
        };
        assert_eq!(mix(2, 2, &[0.5, -0.5]), [0.5, -0.5]);
        assert_eq!(mix(1, 2, &[0.25]), [0.25, 0.25]);
        assert_eq!(mix(1, 4, &[0.25]), [0.25, 0.25, 0.0, 0.0]);
        let mono = mix(2, 1, &[0.5, 0.5]);
        assert!(
            (mono[0] - 2.0 * 0.5 * SQRT_HALF).abs() < 1e-6,
            "equal-power sum of the front pair"
        );
        assert_eq!(mix(2, 6, &[0.5, -0.5]), [0.5, -0.5, 0.0, 0.0, 0.0, 0.0]);
        // 5.1 on stereo: centre and rear channels fold in, the LFE does not.
        let stereo = mix(6, 2, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(stereo, [1.0, 0.0], "LFE alone is dropped");
        let stereo = mix(6, 2, &[0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        assert!((stereo[0] - SQRT_HALF).abs() < 1e-6 && (stereo[1] - SQRT_HALF).abs() < 1e-6);
        assert!(ChannelMixer::new(None, 2, 2).is_identity());
        assert!(!ChannelMixer::new(None, 1, 2).is_identity());
    }

    #[test]
    fn a_custom_map_picks_one_source_channel_per_output_channel_with_a_gain() {
        let routes = [
            route(Some(1), 100),
            route(Some(1), 50),
            route(None, 100),
            route(Some(7), 100),
        ];
        let mixer = ChannelMixer::new(Some(&routes), 2, 4);
        let mut out = [9.0; 4];
        mixer.mix(&[0.8, -0.4], &mut out);
        assert_eq!(
            out,
            [-0.4, -0.2, 0.0, 0.0],
            "right goes to the first two, silent, out-of-range is silent"
        );
        // Routes shorter than the device leave the remaining channels silent.
        let mixer = ChannelMixer::new(Some(&[route(Some(0), 100)]), 2, 2);
        let mut out = [9.0; 2];
        mixer.mix(&[0.3, 0.6], &mut out);
        assert_eq!(out, [0.3, 0.0]);
        assert_eq!(mixer.device_channels(), 2);
        assert_eq!(mixer.source_channels(), 2);
    }
}
