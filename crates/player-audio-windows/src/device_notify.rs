//! Windows tells us when playback devices come, go, change state or when the default device moves
//! (`IMMNotificationClient`), so nothing has to poll for it. Every notification bumps a generation counter that the
//! audio backends compare with the value they last saw, and calls the sink the app installed (the settings panel
//! re-reads its device list from that).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};

use windows::core::{implement, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, DEVICE_STATE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

type Sink = Arc<dyn Fn() + Send + Sync>;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static START: Once = Once::new();

/// Counts the device notifications so far; a changed value means "look at the devices again".
pub fn device_change_generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Called (on a Windows thread) after every device notification. Replaces an earlier sink.
pub fn set_device_change_sink(sink: impl Fn() + Send + Sync + 'static) {
    *SINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(sink));
}

fn notify() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    let sink = SINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(sink) = sink {
        sink();
    }
}

#[implement(IMMNotificationClient)]
struct Client;

impl IMMNotificationClient_Impl for Client_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        notify();
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        notify();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        notify();
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        _role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        // Only playback matters here.
        if flow == eRender {
            notify();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        // Fires for every property write (volume meters, names); the generation is for real changes only.
        Ok(())
    }
}

/// Start listening (once per process). The registration lives on its own thread for the rest of the process.
pub fn start_device_watch() {
    START.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("moe-device-notify".to_owned())
            .spawn(|| {
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                let Ok(enumerator) = (unsafe {
                    CoCreateInstance::<_, IMMDeviceEnumerator>(
                        &MMDeviceEnumerator,
                        None,
                        CLSCTX_ALL,
                    )
                }) else {
                    return;
                };
                let client: IMMNotificationClient = Client.into();
                if unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }.is_err() {
                    return;
                }
                // Keep the enumerator and the client alive; Windows calls the client from its own threads.
                loop {
                    std::thread::park();
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_notification_bumps_the_generation_and_calls_the_sink() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        set_device_change_sink(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let before = device_change_generation();
        notify();
        notify();
        assert!(device_change_generation() >= before + 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
