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
pub(super) mod imp {
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

    pub(super) fn endpoint(id: Option<&str>) -> Result<IAudioEndpointVolume, String> {
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

/// A volume change Windows reported for a playback endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeChange {
    /// CPAL's endpoint id ("wasapi:{...}"), as the device list uses.
    pub id: String,
    /// 0.0 to 1.0.
    pub volume: f32,
    pub muted: bool,
}

/// Which endpoint ids to start and stop watching to go from `registered` to `wanted`.
pub fn plan_registrations(registered: &[String], wanted: &[String]) -> (Vec<String>, Vec<String>) {
    let add =
        wanted
            .iter()
            .filter(|id| !registered.contains(id))
            .fold(Vec::new(), |mut list, id| {
                if !list.contains(id) {
                    list.push(id.clone());
                }
                list
            });
    let remove = registered
        .iter()
        .filter(|id| !wanted.contains(id))
        .cloned()
        .collect();
    (add, remove)
}

#[cfg(windows)]
pub use watch::VolumeWatcher;

/// Windows tells us when an endpoint's volume or mute changes (the Windows volume mixer, a headset button, another
/// program), the way the reference channel mapper does with `RegisterControlChangeNotify`: no polling.
#[cfg(windows)]
mod watch {
    use std::collections::HashMap;
    use std::sync::{mpsc, Arc};
    use std::thread;

    use windows::core::implement;
    use windows::Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    };
    use windows::Win32::Media::Audio::AUDIO_VOLUME_NOTIFICATION_DATA;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    use super::{imp, plan_registrations, VolumeChange};

    type Sink = Arc<dyn Fn(VolumeChange) + Send + Sync>;

    /// The COM object Windows calls. It runs on a Windows thread, so it only hands the change on.
    #[implement(IAudioEndpointVolumeCallback)]
    struct Callback {
        id: String,
        sink: Sink,
    }

    impl IAudioEndpointVolumeCallback_Impl for Callback_Impl {
        fn OnNotify(
            &self,
            notify: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
        ) -> windows::core::Result<()> {
            // SAFETY: Windows passes a valid notification for the duration of the call.
            if let Some(data) = unsafe { notify.as_ref() } {
                (self.sink)(VolumeChange {
                    id: self.id.clone(),
                    volume: data.fMasterVolume,
                    muted: data.bMuted.as_bool(),
                });
            }
            Ok(())
        }
    }

    /// One endpoint being watched; dropping it stops the notifications.
    struct Registration {
        volume: IAudioEndpointVolume,
        callback: IAudioEndpointVolumeCallback,
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            let _ = unsafe { self.volume.UnregisterControlChangeNotify(&self.callback) };
        }
    }

    fn register(id: &str, sink: &Sink) -> Result<Registration, String> {
        let volume = imp::endpoint(Some(id))?;
        let callback: IAudioEndpointVolumeCallback = Callback {
            id: id.to_owned(),
            sink: Arc::clone(sink),
        }
        .into();
        unsafe { volume.RegisterControlChangeNotify(&callback) }
            .map_err(|error| format!("無法監看裝置音量：{error}"))?;
        Ok(Registration { volume, callback })
    }

    /// Owns a thread that keeps the notifications of the chosen endpoints registered.
    pub struct VolumeWatcher {
        commands: mpsc::Sender<Vec<String>>,
    }

    impl VolumeWatcher {
        /// `sink` is called (on a Windows thread) for every change of a watched endpoint.
        pub fn start(sink: impl Fn(VolumeChange) + Send + Sync + 'static) -> Self {
            let (commands, requests) = mpsc::channel::<Vec<String>>();
            let sink: Sink = Arc::new(sink);
            let _ = thread::Builder::new()
                .name("moe-volume-watch".to_owned())
                .spawn(move || {
                    // The COM objects live on this thread; Windows calls the callbacks from its own threads.
                    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                    let mut registered: HashMap<String, Registration> = HashMap::new();
                    while let Ok(mut wanted) = requests.recv() {
                        // Only the newest request matters.
                        while let Ok(newer) = requests.try_recv() {
                            wanted = newer;
                        }
                        let current: Vec<String> = registered.keys().cloned().collect();
                        let (add, remove) = plan_registrations(&current, &wanted);
                        for id in remove {
                            registered.remove(&id);
                        }
                        for id in add {
                            // A device that cannot be watched now (not plugged in) is tried again on the next request.
                            if let Ok(registration) = register(&id, &sink) {
                                registered.insert(id, registration);
                            }
                        }
                    }
                });
            Self { commands }
        }

        /// Watch exactly these endpoints (ids as the device list reports them).
        pub fn watch(&self, ids: Vec<String>) {
            let _ = self.commands.send(ids);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::plan_registrations;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn only_the_difference_is_registered_or_removed() {
        let (add, remove) = plan_registrations(&ids(&["a", "b"]), &ids(&["b", "c"]));
        assert_eq!((add, remove), (ids(&["c"]), ids(&["a"])));
        let (add, remove) = plan_registrations(&[], &ids(&["a", "a", "b"]));
        assert_eq!(
            (add, remove),
            (ids(&["a", "b"]), Vec::new()),
            "a repeated id is registered once"
        );
        let (add, remove) = plan_registrations(&ids(&["a"]), &ids(&["a"]));
        assert!(add.is_empty() && remove.is_empty());
        let (add, remove) = plan_registrations(&ids(&["a", "b"]), &[]);
        assert_eq!((add, remove), (Vec::new(), ids(&["a", "b"])));
    }
}
