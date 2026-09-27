use std::fs::File;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use crate::{AudioBackend, AudioError};

pub(super) struct RodioBackend {
    // The device sink owns CPAL's WASAPI stream and must outlive the player.
    _device_sink: MixerDeviceSink,
    player: Player,
    loaded: bool,
    runtime_errors: Receiver<String>,
}

impl RodioBackend {
    pub(super) fn new() -> Result<Self, AudioError> {
        let (runtime_error_tx, runtime_errors) = mpsc::sync_channel(1);
        let mut device_sink = DeviceSinkBuilder::from_default_device()
            .map_err(|error| AudioError::OutputDevice(error.to_string()))?
            .with_error_callback(runtime_error_callback(runtime_error_tx))
            .open_sink_or_fallback()
            .map_err(|error| AudioError::OutputDevice(error.to_string()))?;
        device_sink.log_on_drop(false);
        let player = Player::connect_new(device_sink.mixer());
        Ok(Self {
            _device_sink: device_sink,
            player,
            loaded: false,
            runtime_errors,
        })
    }
}

impl AudioBackend for RodioBackend {
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let file = File::open(path).map_err(|error| AudioError::FileOpen {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let decoder = Decoder::try_from(file).map_err(|error| match error {
            rodio::decoder::DecoderError::UnrecognizedFormat => AudioError::UnsupportedFormat {
                path: path.to_path_buf(),
            },
            _ => AudioError::Decode {
                path: path.to_path_buf(),
                message: error.to_string(),
            },
        })?;
        let duration = decoder.total_duration();

        self.player.pause();
        self.player.stop();
        self.player.append(decoder);
        self.player.pause();
        self.loaded = true;
        Ok(duration)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        if !self.loaded {
            return Err(AudioError::NoTrackLoaded);
        }
        self.player.play();
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        self.player.pause();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        self.player.stop();
        self.loaded = false;
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
        self.player
            .try_seek(position)
            .map_err(|error| AudioError::Seek(error.to_string()))?;
        Ok(self.player.get_pos())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        self.player.set_volume(volume);
        Ok(())
    }

    fn position(&self) -> Duration {
        self.player.get_pos()
    }

    fn is_empty(&self) -> bool {
        self.player.empty()
    }

    fn take_error(&mut self) -> Option<AudioError> {
        match self.runtime_errors.try_recv() {
            Ok(message) => Some(AudioError::Backend(format!(
                "output stream failed: {message}"
            ))),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

fn runtime_error_callback(
    sender: SyncSender<String>,
) -> impl FnMut(rodio::cpal::StreamError) + Clone + Send + 'static {
    move |error| {
        let _ = sender.try_send(error.to_string());
    }
}
