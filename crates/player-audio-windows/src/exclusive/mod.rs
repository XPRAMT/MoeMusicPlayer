//! WASAPI exclusive output (ADR 0020, phase 1): bit-perfect PCM straight to one endpoint.

pub(crate) mod backend;
pub(crate) mod endpoint;
pub(crate) mod format;
pub(crate) mod session;

/// Ask Windows (MMCSS) to schedule the calling thread like a pro-audio thread; the guard undoes it. Best effort.
pub(crate) struct RenderThreadBoost(windows::Win32::Foundation::HANDLE);

pub(crate) fn boost_render_thread() -> Option<RenderThreadBoost> {
    use windows::core::w;
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    let mut index = 0_u32;
    unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }
        .ok()
        .map(RenderThreadBoost)
}

impl Drop for RenderThreadBoost {
    fn drop(&mut self) {
        let _ =
            unsafe { windows::Win32::System::Threading::AvRevertMmThreadCharacteristics(self.0) };
    }
}
