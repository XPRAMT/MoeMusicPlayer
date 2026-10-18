//! The endpoint behind exclusive mode. Everything above this file talks to the two small traits, so negotiation,
//! buffering, seek and pause are tested with a fake endpoint; `WasapiEndpoint` is the real thing (the `wasapi` crate,
//! event-driven exclusive mode).

use std::time::Duration;

use super::format::{FormatProbe, StreamFormat};

/// An opened exclusive stream, driven by the render thread.
pub(crate) trait RenderStream {
    /// Frames the endpoint buffer holds (one period in event-driven exclusive mode).
    fn buffer_frames(&self) -> usize;
    /// Wait until the device wants more audio (`true`) or the timeout passes (`false`).
    fn wait(&mut self, timeout: Duration) -> Result<bool, String>;
    /// Frames that can be written now.
    fn free_frames(&mut self) -> Result<usize, String>;
    /// Write `frames` frames; `bytes` holds exactly that many.
    fn write(&mut self, frames: usize, bytes: &[u8]) -> Result<(), String>;
    fn start(&mut self) -> Result<(), String>;
}

/// One render endpoint: says which formats it takes and opens streams.
pub(crate) trait Endpoint: FormatProbe {
    fn open(&mut self, format: &StreamFormat) -> Result<Box<dyn RenderStream>, String>;
}

#[cfg(windows)]
pub(crate) use real::WasapiEndpoint;

#[cfg(windows)]
mod real {
    use std::collections::HashMap;
    use std::time::Duration;

    use wasapi::{
        AudioClient, AudioRenderClient, DeviceEnumerator, Direction, Handle, SampleType,
        StreamMode, WasapiError, WaveFormat,
    };

    use super::{Endpoint, FormatProbe, RenderStream, StreamFormat};

    /// HRESULTs the render path can run into (audioclient.h).
    const AUDCLNT_E_UNSUPPORTED_FORMAT: u32 = 0x8889_0008;
    const AUDCLNT_E_DEVICE_IN_USE: u32 = 0x8889_000A;
    const AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED: u32 = 0x8889_000E;
    const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED: u32 = 0x8889_0019;
    const AUDCLNT_E_DEVICE_INVALIDATED: u32 = 0x8889_0004;

    /// A message the user can act on.
    pub(crate) fn error_text(error: &WasapiError) -> String {
        if let WasapiError::Windows(inner) = error {
            let code = inner.code().0 as u32;
            return match code {
                AUDCLNT_E_DEVICE_IN_USE => "這個裝置正被其他程式使用，請關閉占用它的程式後再試。".to_owned(),
                AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => {
                    "這個裝置沒有允許應用程式取得獨佔控制：請在 Windows 音效設定的裝置內容 → 進階，勾選「允許應用程式取得這個裝置的獨佔控制權」。"
                        .to_owned()
                }
                AUDCLNT_E_UNSUPPORTED_FORMAT => "這個裝置不接受這個格式。".to_owned(),
                AUDCLNT_E_DEVICE_INVALIDATED => "裝置已中斷連線。".to_owned(),
                _ => format!("{error}"),
            };
        }
        format!("{error}")
    }

    fn windows_code(error: &WasapiError) -> Option<u32> {
        match error {
            WasapiError::Windows(inner) => Some(inner.code().0 as u32),
            _ => None,
        }
    }

    fn wave_format(format: &StreamFormat, mask: Option<u32>) -> WaveFormat {
        WaveFormat::new(
            usize::from(format.container.container_bits()),
            usize::from(format.container.valid_bits()),
            &SampleType::Int,
            format.rate as usize,
            usize::from(format.channels),
            mask,
        )
    }

    pub(crate) struct WasapiEndpoint {
        device: wasapi::Device,
        /// Used only to ask which formats the device takes.
        probe_client: AudioClient,
        /// The wave format (channel mask quirks included) each accepted `StreamFormat` was accepted as.
        accepted: HashMap<(u32, u16, u16, u16), WaveFormat>,
    }

    fn key(format: &StreamFormat) -> (u32, u16, u16, u16) {
        (
            format.rate,
            format.channels,
            format.container.container_bits(),
            format.container.valid_bits(),
        )
    }

    impl WasapiEndpoint {
        /// `id` is CPAL's endpoint id ("wasapi:{...}"), or `None` for the default render device.
        pub(crate) fn new(id: Option<&str>) -> Result<Self, String> {
            let _ = wasapi::initialize_mta();
            let enumerator =
                DeviceEnumerator::new().map_err(|error| super::real::error_text(&error))?;
            let device = match id {
                Some(id) => enumerator.get_device(id.strip_prefix("wasapi:").unwrap_or(id)),
                None => enumerator.get_default_device(&Direction::Render),
            }
            .map_err(|error| format!("找不到播放裝置：{}", error_text(&error)))?;
            let probe_client = device
                .get_iaudioclient()
                .map_err(|error| error_text(&error))?;
            Ok(Self {
                device,
                probe_client,
                accepted: HashMap::new(),
            })
        }
    }

    impl FormatProbe for WasapiEndpoint {
        fn supports(&mut self, format: &StreamFormat) -> bool {
            let wanted = wave_format(format, None);
            match self
                .probe_client
                .is_supported_exclusive_with_quirks(&wanted)
            {
                Ok(accepted) => {
                    self.accepted.insert(key(format), accepted);
                    true
                }
                Err(_) => false,
            }
        }
    }

    impl Endpoint for WasapiEndpoint {
        fn open(&mut self, format: &StreamFormat) -> Result<Box<dyn RenderStream>, String> {
            let wave = match self.accepted.get(&key(format)) {
                Some(accepted) => accepted.clone(),
                None => wave_format(format, None),
            };
            // The period is the device's own default, exactly as it reports it. Do not round it to a "nice" buffer
            // size: some drivers (the Bluetooth one) raise the event every period however many frames the buffer
            // holds, so a buffer shorter than the period makes playback run slow by that ratio (measured: 384 frames
            // for a 10 ms period at 44.1 kHz played at 87 % speed). Drivers that need an aligned buffer (HD Audio)
            // say so with AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED and give the size to use, handled below.
            let (default_period, min_period) = self
                .probe_client
                .get_device_period()
                .map_err(|error| error_text(&error))?;
            let mut period = default_period.max(min_period);
            // A failed Initialize leaves the client unusable, so every attempt takes a fresh one.
            for attempt in 0..2 {
                let mut client = self
                    .device
                    .get_iaudioclient()
                    .map_err(|error| error_text(&error))?;
                match client.initialize_client(
                    &wave,
                    &Direction::Render,
                    &StreamMode::EventsExclusive { period_hns: period },
                ) {
                    Ok(()) => return finish_open(client, format),
                    Err(error)
                        if attempt == 0
                            && windows_code(&error) == Some(AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED) =>
                    {
                        // The driver tells the buffer size it wants; ask again with a period of exactly that.
                        let frames = i64::from(
                            client
                                .get_buffer_size()
                                .map_err(|error| error_text(&error))?,
                        );
                        period = wasapi::calculate_period_100ns(frames, i64::from(format.rate));
                    }
                    Err(error) => return Err(error_text(&error)),
                }
            }
            Err("無法以對齊的緩衝區開啟裝置。".to_owned())
        }
    }

    fn finish_open(
        client: AudioClient,
        format: &StreamFormat,
    ) -> Result<Box<dyn RenderStream>, String> {
        let event = client
            .set_get_eventhandle()
            .map_err(|error| error_text(&error))?;
        let render = client
            .get_audiorenderclient()
            .map_err(|error| error_text(&error))?;
        let buffer_frames = client
            .get_buffer_size()
            .map_err(|error| error_text(&error))? as usize;
        Ok(Box::new(WasapiStream {
            client,
            render,
            event,
            buffer_frames,
            frame_bytes: format.frame_bytes(),
        }))
    }

    struct WasapiStream {
        client: AudioClient,
        render: AudioRenderClient,
        event: Handle,
        buffer_frames: usize,
        frame_bytes: usize,
    }

    impl RenderStream for WasapiStream {
        fn buffer_frames(&self) -> usize {
            self.buffer_frames
        }

        fn wait(&mut self, timeout: Duration) -> Result<bool, String> {
            match self
                .event
                .wait_for_event(timeout.as_millis().min(u128::from(u32::MAX)) as u32)
            {
                Ok(()) => Ok(true),
                Err(WasapiError::EventTimeout) => Ok(false),
                Err(error) => Err(error_text(&error)),
            }
        }

        fn free_frames(&mut self) -> Result<usize, String> {
            self.client
                .get_available_space_in_frames()
                .map(|frames| frames as usize)
                .map_err(|error| error_text(&error))
        }

        fn write(&mut self, frames: usize, bytes: &[u8]) -> Result<(), String> {
            debug_assert_eq!(bytes.len(), frames * self.frame_bytes);
            self.render
                .write_to_device(frames, bytes, None)
                .map_err(|error| error_text(&error))
        }

        fn start(&mut self) -> Result<(), String> {
            self.client
                .start_stream()
                .map_err(|error| error_text(&error))
        }
    }

    impl Drop for WasapiStream {
        fn drop(&mut self) {
            let _ = self.client.stop_stream();
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::{Endpoint, FormatProbe, RenderStream, StreamFormat};

    /// What the fake device received.
    #[derive(Default)]
    pub(crate) struct Capture {
        pub(crate) bytes: Mutex<Vec<u8>>,
        pub(crate) opened: Mutex<Vec<StreamFormat>>,
        /// Make the stream fail on the next write (a device that disappeared).
        pub(crate) fail: AtomicBool,
    }

    pub(crate) struct FakeEndpoint {
        pub(crate) accepts: Vec<StreamFormat>,
        pub(crate) capture: Arc<Capture>,
        /// Frames the fake device takes per tick.
        pub(crate) period_frames: usize,
        /// Time one tick takes (the fake device plays much faster than real time to keep tests short).
        pub(crate) tick: Duration,
    }

    impl FormatProbe for FakeEndpoint {
        fn supports(&mut self, format: &StreamFormat) -> bool {
            self.accepts.contains(format)
        }
    }

    impl Endpoint for FakeEndpoint {
        fn open(&mut self, format: &StreamFormat) -> Result<Box<dyn RenderStream>, String> {
            if !self.accepts.contains(format) {
                return Err("unsupported format".to_owned());
            }
            self.capture.opened.lock().unwrap().push(*format);
            Ok(Box::new(FakeStream {
                capture: Arc::clone(&self.capture),
                period_frames: self.period_frames,
                tick: self.tick,
                frame_bytes: format.frame_bytes(),
            }))
        }
    }

    struct FakeStream {
        capture: Arc<Capture>,
        period_frames: usize,
        tick: Duration,
        frame_bytes: usize,
    }

    impl RenderStream for FakeStream {
        fn buffer_frames(&self) -> usize {
            self.period_frames
        }

        fn wait(&mut self, _timeout: Duration) -> Result<bool, String> {
            thread::sleep(self.tick);
            Ok(true)
        }

        fn free_frames(&mut self) -> Result<usize, String> {
            if self.capture.fail.load(Ordering::Relaxed) {
                return Err("裝置已中斷連線。".to_owned());
            }
            Ok(self.period_frames)
        }

        fn write(&mut self, frames: usize, bytes: &[u8]) -> Result<(), String> {
            assert_eq!(bytes.len(), frames * self.frame_bytes);
            self.capture.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }

        fn start(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
}
