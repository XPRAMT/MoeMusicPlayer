//! Where lyrics can go on the Windows taskbar (ADR 0018): the taskbar's bounds, the horizontal stretches its
//! buttons and the notification area do not use, and the state that decides whether lyrics may be drawn at all.
//! Everything is in physical pixels.

/// A rectangle in physical screen pixels (`right` and `bottom` exclusive).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Which free stretch to use when there are several.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AreaChoice {
    Widest,
    Leftmost,
    Rightmost,
}

impl Rect {
    /// `self` reaches over all of `inner` (equal edges count).
    pub fn covers(&self, inner: Rect) -> bool {
        self.left <= inner.left
            && self.top <= inner.top
            && self.right >= inner.right
            && self.bottom >= inner.bottom
    }
}

/// The stretches of `bounds` along the x axis that `occupied` (x intervals) leaves free, left to right. Each is
/// shrunk by `margin` on both sides and dropped when it is then narrower than `min_width`.
pub fn free_stretches(
    bounds: Rect,
    occupied: &[(i32, i32)],
    margin: i32,
    min_width: i32,
) -> Vec<(i32, i32)> {
    let mut intervals: Vec<(i32, i32)> = occupied
        .iter()
        .map(|&(from, to)| (from.max(bounds.left), to.min(bounds.right)))
        .filter(|&(from, to)| to > from)
        .collect();
    intervals.sort_unstable();
    let mut stretches = Vec::new();
    let mut cursor = bounds.left;
    for (from, to) in intervals {
        if from > cursor {
            stretches.push((cursor, from));
        }
        cursor = cursor.max(to);
    }
    if cursor < bounds.right {
        stretches.push((cursor, bounds.right));
    }
    stretches
        .into_iter()
        .map(|(from, to)| (from + margin, to - margin))
        .filter(|&(from, to)| to - from >= min_width)
        .collect()
}

/// The rectangle (full taskbar height) lyrics should occupy, or `None` when no stretch is wide enough.
pub fn pick_free_area(
    bounds: Rect,
    occupied: &[(i32, i32)],
    choice: AreaChoice,
    margin: i32,
    min_width: i32,
) -> Option<Rect> {
    let stretches = free_stretches(bounds, occupied, margin, min_width);
    let (left, right) = match choice {
        AreaChoice::Widest => stretches
            .iter()
            .copied()
            .max_by_key(|&(from, to)| (to - from, -from))?,
        AreaChoice::Leftmost => *stretches.first()?,
        AreaChoice::Rightmost => *stretches.last()?,
    };
    Some(Rect {
        left,
        top: bounds.top,
        right,
        bottom: bounds.bottom,
    })
}

/// What the taskbar looks like right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskbarProbe {
    pub bounds: Rect,
    /// x intervals taken by buttons and the notification area.
    pub occupied: Vec<(i32, i32)>,
    pub dpi: u32,
    /// The taskbar hides itself when the pointer leaves it.
    pub auto_hide: bool,
    /// A full-screen app, game or presentation is in front, or the taskbar is not visible: nothing should be drawn.
    pub covered: bool,
    /// Left or right edge: not supported.
    pub vertical: bool,
    /// The taskbar window (an `HWND` value: `Shell_TrayWnd` or a `Shell_SecondaryTrayWnd`), so the lyrics window
    /// can be made its owned window.
    pub tray_window: isize,
}

/// One taskbar: the primary `Shell_TrayWnd` or a `Shell_SecondaryTrayWnd` on another monitor (ADR 0031).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarWindow {
    /// The taskbar window (an `HWND` value).
    pub hwnd: isize,
    /// `Shell_TrayWnd`: the taskbar on the primary monitor, the one with the notification area.
    pub primary: bool,
    /// GDI device name of the monitor it is on (`\\.\DISPLAY6`), matching [`DisplayMonitor::device`].
    pub monitor_device: String,
}

/// One active monitor (ADR 0031).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayMonitor {
    /// Stays the same across restarts and while the monitor is unplugged: the monitor's device interface path
    /// (`\\?\DISPLAY#GSM5B7F#...`), or the GDI device name when Windows reports no path for it.
    pub id: String,
    /// 1-based, in the order Windows reports the active display paths (`QueryDisplayConfig`): the closest match to
    /// the numbers in Settings → Display that can be read without the Settings app.
    pub number: u32,
    /// GDI device name (`\\.\DISPLAY6`).
    pub device: String,
    /// The monitor's own name (from its EDID); empty when unknown.
    pub name: String,
    pub primary: bool,
    /// Physical pixels.
    pub bounds: Rect,
}

/// A monitor as GDI sees it: device name, bounds, primary.
pub type GdiMonitor = (String, Rect, bool);
/// An active display path in `QueryDisplayConfig` order: GDI device name, monitor device path, friendly name.
pub type DisplayPath = (String, String, String);

/// Number the GDI monitors by the order of their display paths. A source shown on several monitors (duplicate mode)
/// is one GDI monitor and keeps its first path; a monitor without a path comes last and is known by its device name.
pub fn number_monitors(gdi: &[GdiMonitor], paths: &[DisplayPath]) -> Vec<DisplayMonitor> {
    let mut monitors: Vec<DisplayMonitor> = Vec::with_capacity(gdi.len());
    let mut push = |device: &str, id: &str, name: &str| {
        if monitors.iter().any(|monitor| monitor.device == device) {
            return;
        }
        let Some((_, bounds, primary)) = gdi.iter().find(|(name, ..)| name == device) else {
            return;
        };
        monitors.push(DisplayMonitor {
            id: if id.is_empty() {
                device.to_owned()
            } else {
                id.to_owned()
            },
            number: 0,
            device: device.to_owned(),
            name: name.to_owned(),
            primary: *primary,
            bounds: *bounds,
        });
    };
    for (device, id, name) in paths {
        push(device, id, name);
    }
    for (device, ..) in gdi {
        push(device, "", "");
    }
    for (index, monitor) in monitors.iter_mut().enumerate() {
        monitor.number = index as u32 + 1;
    }
    monitors
}

/// The rectangle lyrics should use on one taskbar, or `None` when they must be hidden there (covered, auto-hide,
/// vertical, or no stretch wide enough). The margin and the narrowest stretch are CSS px scaled by that taskbar's own
/// DPI: monitors differ (240 and 192 DPI on the maintainer's two).
pub fn lyrics_area(
    probe: &TaskbarProbe,
    margin_css_px: i32,
    min_width_css_px: i32,
) -> Option<Rect> {
    if probe.covered || probe.auto_hide || probe.vertical {
        return None;
    }
    let scale = |css_px: i32| css_px * probe.dpi.max(96) as i32 / 96;
    pick_free_area(
        probe.bounds,
        &probe.occupied,
        AreaChoice::Widest,
        scale(margin_css_px),
        scale(min_width_css_px),
    )
}

#[cfg(windows)]
pub use imp::{
    list_monitors, list_taskbars, monitor_device_of_window, taskbar_uses_light_theme, TaskbarProber,
};

#[cfg(windows)]
mod imp {
    use windows::core::{w, BOOL};
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
        DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };
    use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
        MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_ButtonControlTypeId,
        UIA_ControlTypePropertyId,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, SHQueryUserNotificationState, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA,
        QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindowRect, IsIconic,
        IsWindowVisible,
    };

    use super::{number_monitors, DisplayMonitor, GdiMonitor, Rect, TaskbarProbe, TaskbarWindow};

    /// At most this many secondary taskbars are looked for (one per monitor).
    const MAX_SECONDARY_TASKBARS: usize = 16;

    fn wide(text: &[u16]) -> String {
        let end = text
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(text.len());
        String::from_utf16_lossy(&text[..end])
    }

    fn monitor_info(monitor: HMONITOR) -> Option<GdiMonitor> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }
            .as_bool()
            .then(|| {
                (
                    wide(&info.szDevice),
                    rect_of(info.monitorInfo.rcMonitor),
                    // MONITORINFOF_PRIMARY
                    info.monitorInfo.dwFlags & 1 != 0,
                )
            })
    }

    /// GDI device name of the monitor a window is on (nearest when it is off-screen; a minimized window counts where
    /// it was before it was minimized).
    pub fn monitor_device_of_window(window: isize) -> Option<String> {
        let monitor =
            unsafe { MonitorFromWindow(HWND(window as *mut _), MONITOR_DEFAULTTONEAREST) };
        monitor_info(monitor).map(|(device, ..)| device)
    }

    /// Every taskbar: `Shell_TrayWnd` first, then each `Shell_SecondaryTrayWnd`.
    pub fn list_taskbars() -> Vec<TaskbarWindow> {
        let entry = |tray: HWND, primary: bool| TaskbarWindow {
            hwnd: tray.0 as isize,
            primary,
            monitor_device: monitor_device_of_window(tray.0 as isize).unwrap_or_default(),
        };
        let mut found = Vec::new();
        if let Ok(tray) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
            found.push(entry(tray, true));
        }
        let mut after: Option<HWND> = None;
        for _ in 0..MAX_SECONDARY_TASKBARS {
            let Ok(tray) =
                (unsafe { FindWindowExW(None, after, w!("Shell_SecondaryTrayWnd"), None) })
            else {
                break;
            };
            found.push(entry(tray, false));
            after = Some(tray);
        }
        found
    }

    /// The active monitors, numbered (see [`DisplayMonitor::number`]).
    pub fn list_monitors() -> Vec<DisplayMonitor> {
        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            let list = unsafe { &mut *(data.0 as *mut Vec<GdiMonitor>) };
            if let Some(entry) = monitor_info(monitor) {
                list.push(entry);
            }
            true.into()
        }
        let mut gdi: Vec<GdiMonitor> = Vec::new();
        let _ = unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut gdi as *mut Vec<GdiMonitor> as isize),
            )
        };
        number_monitors(&gdi, &display_paths())
    }

    /// The active display paths in the order `QueryDisplayConfig` reports them.
    fn display_paths() -> Vec<super::DisplayPath> {
        let mut path_count = 0_u32;
        let mut mode_count = 0_u32;
        if unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        } != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        if unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        } != ERROR_SUCCESS
        {
            return Vec::new();
        }
        paths.truncate(path_count as usize);
        paths
            .iter()
            .filter_map(|path| {
                let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                        size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                        adapterId: path.sourceInfo.adapterId,
                        id: path.sourceInfo.id,
                    },
                    ..Default::default()
                };
                if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
                    return None;
                }
                let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                    header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                        size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                        adapterId: path.targetInfo.adapterId,
                        id: path.targetInfo.id,
                    },
                    ..Default::default()
                };
                let (id, name) = if unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0 {
                    (
                        wide(&target.monitorDevicePath),
                        wide(&target.monitorFriendlyDeviceName),
                    )
                } else {
                    (String::new(), String::new())
                };
                Some((wide(&source.viewGdiDeviceName), id, name))
            })
            .collect()
    }

    fn rect_of(rect: RECT) -> Rect {
        Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    /// Reads the taskbar through UI Automation. Create it on the thread that uses it: it initialises COM there.
    pub struct TaskbarProber {
        automation: IUIAutomation,
    }

    impl TaskbarProber {
        pub fn new() -> Result<Self, String> {
            // Already initialised on this thread is fine.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let automation: IUIAutomation =
                unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
                    .map_err(|error| format!("無法啟動 UI Automation：{error}"))?;
            Ok(Self { automation })
        }

        /// The primary taskbar.
        pub fn probe(&self) -> Result<TaskbarProbe, String> {
            let tray: HWND = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }
                .map_err(|_| "找不到工作列".to_owned())?;
            self.probe_tray(tray, true)
        }

        /// One taskbar from [`list_taskbars`].
        pub fn probe_taskbar(&self, taskbar: &TaskbarWindow) -> Result<TaskbarProbe, String> {
            self.probe_tray(HWND(taskbar.hwnd as *mut _), taskbar.primary)
        }

        fn probe_tray(&self, tray: HWND, primary: bool) -> Result<TaskbarProbe, String> {
            let mut raw = RECT::default();
            unsafe { GetWindowRect(tray, &mut raw) }.map_err(|error| error.to_string())?;
            let bounds = rect_of(raw);
            let dpi = unsafe { GetDpiForWindow(tray) };

            let mut state = APPBARDATA {
                cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                ..Default::default()
            };
            let auto_hide =
                unsafe { SHAppBarMessage(ABM_GETSTATE, &mut state) } & (ABS_AUTOHIDE as usize) != 0;
            // The notification state is one value for the whole desktop: a full-screen game on the primary monitor
            // must not hide the lyrics on the others, so a secondary taskbar only asks whether a window covers its
            // own monitor.
            let covered = if !primary {
                foreground_covers_monitor_of(tray)
            } else {
                match unsafe { SHQueryUserNotificationState() } {
                    Ok(QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE) => true,
                    // "Busy" is also reported for a maximized window on another monitor, or right after another
                    // window took over (the player was minimized): believe it only when a window really covers the
                    // taskbar's own monitor.
                    Ok(QUNS_BUSY) => foreground_covers_monitor_of(tray),
                    _ => false,
                }
            };
            let visible = unsafe { IsWindowVisible(tray) }.as_bool();
            let vertical = bounds.height() > bounds.width();

            // The secondary taskbars have no TrayNotifyWnd; their clock is a UI Automation Button
            // (`SystemTray.AccentButton`), so the buttons below cover it.
            let mut occupied = Vec::new();
            if let Ok(notify) =
                unsafe { FindWindowExW(Some(tray), None, w!("TrayNotifyWnd"), None) }
            {
                let mut notify_rect = RECT::default();
                if unsafe { GetWindowRect(notify, &mut notify_rect) }.is_ok() {
                    occupied.push((notify_rect.left, notify_rect.right));
                }
            }
            for button in self.button_rects(tray)? {
                // Offscreen or collapsed buttons report empty or far-away rectangles.
                if button.width() > 0 && button.bottom > bounds.top && button.top < bounds.bottom {
                    occupied.push((button.left, button.right));
                }
            }

            Ok(TaskbarProbe {
                bounds,
                occupied,
                dpi,
                auto_hide,
                covered: covered || !visible,
                vertical,
                tray_window: tray.0 as isize,
            })
        }

        fn button_rects(&self, tray: HWND) -> Result<Vec<Rect>, String> {
            let root = unsafe { self.automation.ElementFromHandle(tray) }
                .map_err(|error| error.to_string())?;
            let condition = unsafe {
                self.automation.CreatePropertyCondition(
                    UIA_ControlTypePropertyId,
                    &VARIANT::from(UIA_ButtonControlTypeId.0),
                )
            }
            .map_err(|error| error.to_string())?;
            let found = unsafe { root.FindAll(TreeScope_Descendants, &condition) }
                .map_err(|error| error.to_string())?;
            let count = unsafe { found.Length() }.map_err(|error| error.to_string())?;
            let mut rects = Vec::with_capacity(count as usize);
            for index in 0..count {
                let Ok(element) = (unsafe { found.GetElement(index) }) else {
                    continue;
                };
                if let Ok(rect) = unsafe { element.CurrentBoundingRectangle() } {
                    rects.push(rect_of(rect));
                }
            }
            Ok(rects)
        }
    }

    /// The foreground window is a real window (not the desktop or the taskbar itself) that reaches over the whole
    /// monitor the taskbar is on, taskbar area included: a full-screen video, a game, a borderless presentation. A
    /// maximized window stops at the work area and does not count.
    fn foreground_covers_monitor_of(tray: HWND) -> bool {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() || foreground == tray || unsafe { IsIconic(foreground) }.as_bool()
        {
            return false;
        }
        let mut class = [0_u16; 64];
        let length = unsafe { GetClassNameW(foreground, &mut class) }.max(0) as usize;
        let class = String::from_utf16_lossy(&class[..length.min(class.len())]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let monitor_rect = |window: HWND| {
            let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            unsafe { GetMonitorInfoW(monitor, &mut info) }
                .as_bool()
                .then(|| rect_of(info.rcMonitor))
        };
        let (Some(taskbar_monitor), Some(window_monitor)) =
            (monitor_rect(tray), monitor_rect(foreground))
        else {
            return false;
        };
        if taskbar_monitor != window_monitor {
            return false;
        }
        let mut raw = RECT::default();
        unsafe { GetWindowRect(foreground, &mut raw) }.is_ok()
            && rect_of(raw).covers(taskbar_monitor)
    }

    /// The taskbar follows `SystemUsesLightTheme`, not the apps theme.
    pub fn taskbar_uses_light_theme() -> bool {
        let mut value = 0_u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("SystemUsesLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        status.is_ok() && value != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The measured Windows 11 taskbar: widgets 15..395, tasks 1506..2334, notification area 3105..3840.
    const BOUNDS: Rect = Rect {
        left: 0,
        top: 2080,
        right: 3840,
        bottom: 2160,
    };
    const OCCUPIED: [(i32, i32); 4] = [(15, 395), (1506, 1584), (1584, 2334), (3105, 3840)];

    #[test]
    fn a_window_covers_a_monitor_only_when_it_reaches_over_all_of_it() {
        let monitor = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let work_area = Rect {
            left: -8,
            top: -8,
            right: 1928,
            bottom: 1040,
        };
        assert!(
            !work_area.covers(monitor),
            "a maximized window stops above the taskbar"
        );
        assert!(
            Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080
            }
            .covers(monitor),
            "full screen"
        );
        assert!(
            Rect {
                left: -8,
                top: -8,
                right: 1928,
                bottom: 1088
            }
            .covers(monitor),
            "borderless, a little over"
        );
        assert!(!Rect {
            left: 0,
            top: 0,
            right: 1919,
            bottom: 1080
        }
        .covers(monitor));
    }

    #[test]
    fn the_measured_taskbar_has_a_stretch_on_each_side_of_the_tasks() {
        assert_eq!(
            free_stretches(BOUNDS, &OCCUPIED, 20, 300),
            vec![(415, 1486), (2354, 3085)]
        );
    }

    #[test]
    fn the_choice_picks_the_widest_leftmost_or_rightmost_stretch() {
        let pick = |choice| pick_free_area(BOUNDS, &OCCUPIED, choice, 20, 300).unwrap();
        let span = |area: Rect| (area.left, area.right);
        assert_eq!(span(pick(AreaChoice::Widest)), (415, 1486));
        assert_eq!(span(pick(AreaChoice::Leftmost)), (415, 1486));
        assert_eq!(span(pick(AreaChoice::Rightmost)), (2354, 3085));
        let area = pick(AreaChoice::Rightmost);
        assert_eq!((area.top, area.bottom), (2080, 2160));
    }

    #[test]
    fn a_left_aligned_taskbar_has_one_stretch_before_the_tray() {
        let occupied = [(0, 1200), (3105, 3840)];
        assert_eq!(
            free_stretches(BOUNDS, &occupied, 20, 300),
            vec![(1220, 3085)]
        );
    }

    fn probe(bounds: Rect, occupied: &[(i32, i32)], dpi: u32) -> TaskbarProbe {
        TaskbarProbe {
            bounds,
            occupied: occupied.to_vec(),
            dpi,
            ..Default::default()
        }
    }

    #[test]
    fn each_taskbar_uses_its_own_dpi_for_the_margin_and_the_narrowest_stretch() {
        // The maintainer's monitors: primary 3840×2160 at 240 DPI, secondary 2560×1440 at 192 DPI to its right.
        let primary = probe(BOUNDS, &OCCUPIED, 240);
        // 8 CSS px = 20 physical px at 240 DPI.
        assert_eq!(
            lyrics_area(&primary, 8, 120),
            Some(Rect {
                left: 415,
                top: 2080,
                right: 1486,
                bottom: 2160
            })
        );
        // Measured secondary taskbar (UI Automation): start/search/tasks 4849..5391, clock 6283..6392.
        let secondary_bounds = Rect {
            left: 3840,
            top: 2088,
            right: 6400,
            bottom: 2152,
        };
        let secondary = probe(
            secondary_bounds,
            &[(4849, 4911), (4911, 4971), (4971, 5391), (6283, 6392)],
            192,
        );
        // 8 CSS px = 16 physical px at 192 DPI; the 8 px after the clock is far too narrow.
        assert_eq!(
            lyrics_area(&secondary, 8, 120),
            Some(Rect {
                left: 3856,
                top: 2088,
                right: 4833,
                bottom: 2152
            })
        );
        // 120 CSS px is 240 physical px at 192 DPI but 300 at 240 DPI: a 260 px gap fits only the first.
        let gap = [(3840, 4000), (4260, 6400)];
        assert!(lyrics_area(&probe(secondary_bounds, &gap, 96), 0, 120).is_some());
        assert!(lyrics_area(&probe(secondary_bounds, &gap, 192), 0, 120).is_some());
        assert!(lyrics_area(&probe(secondary_bounds, &gap, 240), 0, 120).is_none());
    }

    #[test]
    fn a_covered_auto_hidden_or_vertical_taskbar_shows_nothing() {
        let base = probe(BOUNDS, &OCCUPIED, 240);
        assert!(lyrics_area(
            &TaskbarProbe {
                covered: true,
                ..base.clone()
            },
            8,
            120
        )
        .is_none());
        assert!(lyrics_area(
            &TaskbarProbe {
                auto_hide: true,
                ..base.clone()
            },
            8,
            120
        )
        .is_none());
        assert!(lyrics_area(
            &TaskbarProbe {
                vertical: true,
                ..base
            },
            8,
            120
        )
        .is_none());
    }

    #[test]
    fn monitors_are_numbered_in_display_path_order() {
        let primary = Rect {
            left: 0,
            top: 0,
            right: 3840,
            bottom: 2160,
        };
        let side = Rect {
            left: 3840,
            top: 712,
            right: 6400,
            bottom: 2152,
        };
        let gdi = vec![
            (r"\\.\DISPLAY1".to_owned(), primary, true),
            (r"\\.\DISPLAY6".to_owned(), side, false),
        ];
        let paths = vec![
            (
                r"\\.\DISPLAY6".to_owned(),
                r"\\?\DISPLAY#B#2".to_owned(),
                "Side".to_owned(),
            ),
            (
                r"\\.\DISPLAY1".to_owned(),
                r"\\?\DISPLAY#A#1".to_owned(),
                "Main".to_owned(),
            ),
            // The same source on a second monitor (duplicate mode) stays one monitor.
            (
                r"\\.\DISPLAY1".to_owned(),
                r"\\?\DISPLAY#C#3".to_owned(),
                "Copy".to_owned(),
            ),
        ];
        let numbered = number_monitors(&gdi, &paths);
        let summary: Vec<_> = numbered
            .iter()
            .map(|monitor| {
                (
                    monitor.number,
                    monitor.device.as_str(),
                    monitor.id.as_str(),
                    monitor.primary,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (1, r"\\.\DISPLAY6", r"\\?\DISPLAY#B#2", false),
                (2, r"\\.\DISPLAY1", r"\\?\DISPLAY#A#1", true),
            ]
        );
        // Without display paths the GDI order is used and the device name is the id.
        let fallback = number_monitors(&gdi, &[]);
        assert_eq!(fallback[0].id, r"\\.\DISPLAY1");
        assert_eq!(fallback[1].number, 2);
        assert_eq!(fallback[1].bounds, side);
    }

    #[test]
    fn stretches_that_are_too_narrow_are_dropped_and_overlaps_merge() {
        let occupied = [(0, 1000), (900, 1200), (1250, 3840)];
        // 1200..1250 is only 50 px wide.
        assert!(free_stretches(BOUNDS, &occupied, 0, 100).is_empty());
        assert_eq!(
            pick_free_area(BOUNDS, &occupied, AreaChoice::Widest, 0, 100),
            None
        );
        // Intervals outside the bounds are clipped away.
        assert_eq!(
            free_stretches(BOUNDS, &[(-500, -10), (4000, 5000)], 0, 100),
            vec![(0, 3840)]
        );
    }
}
