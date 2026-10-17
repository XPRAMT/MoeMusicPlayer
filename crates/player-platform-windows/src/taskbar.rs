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
}

#[cfg(windows)]
pub use imp::{taskbar_uses_light_theme, TaskbarProber};

#[cfg(windows)]
mod imp {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, RECT};
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
        FindWindowExW, FindWindowW, GetWindowRect, IsWindowVisible,
    };

    use super::{Rect, TaskbarProbe};

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

        pub fn probe(&self) -> Result<TaskbarProbe, String> {
            let tray: HWND = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }
                .map_err(|_| "找不到工作列".to_owned())?;
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
            let covered = matches!(
                unsafe { SHQueryUserNotificationState() },
                Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
            );
            let visible = unsafe { IsWindowVisible(tray) }.as_bool();
            let vertical = bounds.height() > bounds.width();

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
