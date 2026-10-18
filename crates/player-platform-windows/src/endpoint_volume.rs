//! The real volume of Windows playback endpoints (the same one Windows' volume mixer and the device's own buttons
//! change), read and set through `IAudioEndpointVolume`. This is a property of the device, not a gain applied to our
//! audio: changing it also changes what other apps sound like on that device.

/// A device's volume as Windows reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EndpointVolume {
    /// 0.0 to 1.0.
    pub volume: f32,
    pub muted: bool,
}

#[cfg(windows)]
mod imp {
    use super::EndpointVolume;
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    /// CPAL's id is "wasapi:" plus the endpoint id Windows uses.
    fn endpoint_id(id: &str) -> &str {
        id.strip_prefix("wasapi:").unwrap_or(id)
    }

    fn endpoint(id: Option<&str>) -> Result<IAudioEndpointVolume, String> {
        // Already initialised on this thread is fine.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|error| format!("無法列舉音訊裝置：{error}"))?;
        let device = match id {
            Some(id) => {
                let wide: Vec<u16> = endpoint_id(id).encode_utf16().chain(Some(0)).collect();
                unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
            }
            None => unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) },
        }
        .map_err(|error| format!("找不到這個音訊裝置：{error}"))?;
        unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }
            .map_err(|error| format!("無法讀取裝置音量：{error}"))
    }

    pub fn read(id: Option<&str>) -> Result<EndpointVolume, String> {
        let endpoint = endpoint(id)?;
        let volume = unsafe { endpoint.GetMasterVolumeLevelScalar() }
            .map_err(|error| format!("無法讀取裝置音量：{error}"))?;
        let muted = unsafe { endpoint.GetMute() }
            .map_err(|error| format!("無法讀取靜音狀態：{error}"))?
            .as_bool();
        Ok(EndpointVolume { volume, muted })
    }

    pub fn set_volume(id: Option<&str>, volume: f32) -> Result<(), String> {
        let endpoint = endpoint(id)?;
        unsafe { endpoint.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), std::ptr::null()) }
            .map_err(|error| format!("無法設定裝置音量：{error}"))
    }

    pub fn set_muted(id: Option<&str>, muted: bool) -> Result<(), String> {
        let endpoint = endpoint(id)?;
        unsafe { endpoint.SetMute(muted, std::ptr::null()) }
            .map_err(|error| format!("無法設定靜音：{error}"))
    }
}

/// Read the volume of the endpoint with this CPAL id (`None`: the default playback device).
#[cfg(windows)]
pub fn read_endpoint_volume(id: Option<&str>) -> Result<EndpointVolume, String> {
    imp::read(id)
}

/// Set the volume (0.0 to 1.0) of an endpoint.
#[cfg(windows)]
pub fn set_endpoint_volume(id: Option<&str>, volume: f32) -> Result<(), String> {
    imp::set_volume(id, volume)
}

/// Mute or unmute an endpoint.
#[cfg(windows)]
pub fn set_endpoint_muted(id: Option<&str>, muted: bool) -> Result<(), String> {
    imp::set_muted(id, muted)
}
