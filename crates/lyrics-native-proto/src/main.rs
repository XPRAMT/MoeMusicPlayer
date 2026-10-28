//! Research prototype for ADR 0034 (draft): a native lyrics strip drawn with Direct2D/DirectWrite into
//! DirectComposition surfaces, with the continuous glide and the per-line opacity fades either run by DWM
//! (`--mode anim`, IDCompositionAnimation, no per-frame work in this process), set by this process once per
//! compositor frame (`--mode commit`), or fully re-rasterised every frame (`--mode redraw`, the WebView-like
//! worst case). Not part of the app; nothing here is wired to the player.
//!
//! Run: `cargo run --release -- --windows 3 --mode anim --seconds 20 --effect shadow --owner-taskbar`

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}

#[cfg(windows)]
fn main() {
    if let Err(error) = win::run() {
        eprintln!("error: {error:?}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use windows::core::{implement, w, Interface, Ref, Result, BOOL, HSTRING, PCWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    use windows::Win32::Graphics::Direct3D::*;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::DirectComposition::*;
    use windows::Win32::Graphics::DirectWrite::*;
    use windows::Win32::Graphics::Dwm::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    use windows::Win32::UI::HiDpi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows_numerics::{Matrix3x2, Vector2};

    // ---------------------------------------------------------------- options

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Mode {
        Anim,
        Commit,
        Redraw,
    }
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Effect {
        None,
        Shadow,
        Outline,
    }

    struct Options {
        windows: usize,
        mode: Mode,
        seconds: f64,
        effect: Effect,
        owner_taskbar: bool,
        linear: bool,
        speed_scale: f32,
        shot: Option<String>,
    }

    fn parse_options() -> Options {
        let mut o = Options {
            windows: 1,
            mode: Mode::Anim,
            seconds: 15.0,
            effect: Effect::Shadow,
            owner_taskbar: false,
            linear: false,
            speed_scale: 1.0,
            shot: None,
        };
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            let next = |i: usize| args.get(i + 1).cloned().unwrap_or_default();
            match args[i].as_str() {
                "--windows" => {
                    o.windows = next(i).parse().unwrap_or(1).clamp(0, 16);
                    i += 1;
                }
                "--mode" => {
                    o.mode = match next(i).as_str() {
                        "commit" => Mode::Commit,
                        "redraw" => Mode::Redraw,
                        _ => Mode::Anim,
                    };
                    i += 1;
                }
                "--seconds" => {
                    o.seconds = next(i).parse().unwrap_or(15.0);
                    i += 1;
                }
                "--effect" => {
                    o.effect = match next(i).as_str() {
                        "none" => Effect::None,
                        "outline" => Effect::Outline,
                        _ => Effect::Shadow,
                    };
                    i += 1;
                }
                "--speed" => {
                    o.speed_scale = next(i).parse().unwrap_or(1.0);
                    i += 1;
                }
                "--shot" => {
                    o.shot = Some(next(i));
                    i += 1;
                }
                "--owner-taskbar" => o.owner_taskbar = true,
                "--linear" => o.linear = true,
                other => eprintln!("ignored argument {other}"),
            }
            i += 1;
        }
        o
    }

    // ---------------------------------------------------------------- lyric content and timing

    /// (main, translation, seconds this line is current)
    const LINES: &[(&str, &str, f64)] = &[
        ("夜空に浮かぶ星を数えて", "數著夜空中漂浮的星星", 2.8),
        ("君の声が聞こえた気がした", "彷彿聽見了你的聲音", 3.4),
        ("我們在風裡奔跑 🌙", "We ran through the wind", 2.6),
        ("消えない光を抱きしめて ✨", "擁抱永不熄滅的光", 3.9),
        ("明日もきっと", "明天也一定", 2.0),
        ("沿著河岸的燈火慢慢走回家", "Walking home along the river lights", 4.2),
        ("ありふれた言葉でいい", "平凡的話語就好", 3.1),
        ("Hold me tight 🎧 銀色の夢", "抱緊我 銀色的夢", 2.7),
        ("この歌が届くまで", "直到這首歌傳達給你", 3.6),
        ("いつまでも 一緒に", "永遠 在一起", 3.0),
    ];
    const LEAD: f64 = 0.2; // the 200 ms fade lead (docs/status/lyrics-windows.md: 連續動畫與透明度)
    const INACTIVE: f32 = 0.5; // inactiveOpacityPercent default 50

    struct Timeline {
        starts: Vec<f64>,
        total: f64,
    }

    fn timeline(scale: f32) -> Timeline {
        let mut starts = Vec::new();
        let mut t = 0.0;
        for (_, _, d) in LINES {
            starts.push(t);
            t += d / scale as f64;
        }
        Timeline { starts, total: t }
    }

    fn line_end(tl: &Timeline, i: usize) -> f64 {
        tl.starts.get(i + 1).copied().unwrap_or(tl.total)
    }

    /// Strip offset at time t: the current line's left edge starts at the centre and slides linearly so the next
    /// line's left edge reaches the centre when it starts (the first version of the strip rule).
    fn strip_offset(tl: &Timeline, lefts: &[f32], widths: &[f32], centre: f32, t: f64) -> f32 {
        let t = t.rem_euclid(tl.total);
        let i = tl.starts.iter().rposition(|s| *s <= t).unwrap_or(0);
        let d = line_end(tl, i) - tl.starts[i];
        centre - lefts[i] - ((t - tl.starts[i]) / d) as f32 * widths[i]
    }

    /// Opacity factor of line i at time t (1 while current, INACTIVE otherwise, 200 ms linear ramps that end
    /// exactly when the next line starts).
    fn line_opacity(tl: &Timeline, i: usize, t: f64) -> f32 {
        let t = t.rem_euclid(tl.total);
        let n = LINES.len();
        let start = tl.starts[i];
        let end = line_end(tl, i);
        let weight = if i == 0 && t >= tl.total - LEAD {
            (t - (tl.total - LEAD)) / LEAD
        } else if t < start - LEAD {
            0.0
        } else if t < start {
            (t - (start - LEAD)) / LEAD
        } else if t < end - LEAD {
            1.0
        } else if t < end {
            1.0 - (t - (end - LEAD)) / LEAD
        } else {
            0.0
        };
        let _ = n;
        INACTIVE + (1.0 - INACTIVE) * weight as f32
    }

    // ---------------------------------------------------------------- monitors

    #[derive(Clone)]
    struct Monitor {
        handle: HMONITOR,
        rect: RECT,
        work: RECT,
        dpi: u32,
        hz: u32,
        name: String,
        primary: bool,
    }

    fn monitors() -> Vec<Monitor> {
        unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
            let list = &mut *(data.0 as *mut Vec<Monitor>);
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(h, &mut info.monitorInfo).as_bool() {
                let mut dx = 0u32;
                let mut dy = 0u32;
                let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
                let _ = EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode);
                let len = info.szDevice.iter().position(|c| *c == 0).unwrap_or(32);
                list.push(Monitor {
                    handle: h,
                    rect: info.monitorInfo.rcMonitor,
                    work: info.monitorInfo.rcWork,
                    dpi: dx,
                    hz: mode.dmDisplayFrequency,
                    name: String::from_utf16_lossy(&info.szDevice[..len]),
                    primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                });
            }
            TRUE
        }
        let mut list: Vec<Monitor> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut list as *mut _ as isize));
        }
        list.sort_by_key(|m| !m.primary);
        list
    }

    fn taskbar_for(monitor: &Monitor) -> Option<HWND> {
        unsafe {
            let primary = FindWindowW(w!("Shell_TrayWnd"), None).ok();
            if let Some(p) = primary {
                if MonitorFromWindow(p, MONITOR_DEFAULTTONEAREST) == monitor.handle {
                    return Some(p);
                }
            }
            let mut after: Option<HWND> = None;
            loop {
                let found = FindWindowExW(None, after, w!("Shell_SecondaryTrayWnd"), None).ok()?;
                if MonitorFromWindow(found, MONITOR_DEFAULTTONEAREST) == monitor.handle {
                    return Some(found);
                }
                after = Some(found);
            }
        }
    }

    // ---------------------------------------------------------------- outline text renderer (glyph outlines → geometry)

    #[implement(IDWriteTextRenderer)]
    struct OutlineCollector {
        factory: ID2D1Factory1,
        geometries: Rc<RefCell<Vec<ID2D1Geometry>>>,
    }

    impl IDWritePixelSnapping_Impl for OutlineCollector_Impl {
        fn IsPixelSnappingDisabled(&self, _: *const core::ffi::c_void) -> Result<BOOL> {
            Ok(TRUE)
        }
        fn GetCurrentTransform(&self, _: *const core::ffi::c_void, transform: *mut DWRITE_MATRIX) -> Result<()> {
            unsafe {
                *transform = DWRITE_MATRIX { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0, dx: 0.0, dy: 0.0 };
            }
            Ok(())
        }
        fn GetPixelsPerDip(&self, _: *const core::ffi::c_void) -> Result<f32> {
            Ok(1.0)
        }
    }

    impl IDWriteTextRenderer_Impl for OutlineCollector_Impl {
        fn DrawGlyphRun(
            &self,
            _: *const core::ffi::c_void,
            x: f32,
            y: f32,
            _: DWRITE_MEASURING_MODE,
            run: *const DWRITE_GLYPH_RUN,
            _: *const DWRITE_GLYPH_RUN_DESCRIPTION,
            _: Ref<windows::core::IUnknown>,
        ) -> Result<()> {
            unsafe {
                let run = &*run;
                let face = run.fontFace.as_ref().ok_or(windows::core::Error::from(E_POINTER))?;
                let path = self.factory.CreatePathGeometry()?;
                let sink = path.Open()?;
                face.GetGlyphRunOutline(
                    run.fontEmSize,
                    run.glyphIndices,
                    Some(run.glyphAdvances),
                    if run.glyphOffsets.is_null() { None } else { Some(run.glyphOffsets) },
                    run.glyphCount,
                    run.isSideways.as_bool(),
                    run.bidiLevel % 2 == 1,
                    &sink,
                )?;
                sink.Close()?;
                let moved = self.factory.CreateTransformedGeometry(&path, &Matrix3x2::translation(x, y))?;
                self.geometries.borrow_mut().push(moved.cast()?);
            }
            Ok(())
        }
        fn DrawUnderline(&self, _: *const core::ffi::c_void, _: f32, _: f32, _: *const DWRITE_UNDERLINE, _: Ref<windows::core::IUnknown>) -> Result<()> {
            Ok(())
        }
        fn DrawStrikethrough(&self, _: *const core::ffi::c_void, _: f32, _: f32, _: *const DWRITE_STRIKETHROUGH, _: Ref<windows::core::IUnknown>) -> Result<()> {
            Ok(())
        }
        fn DrawInlineObject(&self, _: *const core::ffi::c_void, _: f32, _: f32, _: Ref<IDWriteInlineObject>, _: BOOL, _: BOOL, _: Ref<windows::core::IUnknown>) -> Result<()> {
            Ok(())
        }
    }

    // ---------------------------------------------------------------- shared graphics state

    struct Gfx {
        d2d_factory: ID2D1Factory1,
        d2d_device: ID2D1Device,
        dwrite: IDWriteFactory2,
        fallback: IDWriteFontFallback,
        dcomp: IDCompositionDesktopDevice,
    }

    fn create_gfx() -> Result<Gfx> {
        unsafe {
            let mut d3d: Option<ID3D11Device> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let d3d = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let d2d_factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi)?;
            let dcomp: IDCompositionDesktopDevice = DCompositionCreateDevice2(&d2d_device)?;
            let dwrite: IDWriteFactory2 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            // 'Segoe UI', 'Microsoft JhengHei', sans-serif (app.css): CJK and kana go to JhengHei, then system fallback
            // (Segoe UI Emoji for colour emoji).
            let builder = dwrite.CreateFontFallbackBuilder()?;
            let cjk = [
                DWRITE_UNICODE_RANGE { first: 0x2E80, last: 0x9FFF },
                DWRITE_UNICODE_RANGE { first: 0xF900, last: 0xFAFF },
                DWRITE_UNICODE_RANGE { first: 0xFF00, last: 0xFFEF },
                DWRITE_UNICODE_RANGE { first: 0x20000, last: 0x2FFFF },
            ];
            let family = HSTRING::from("Microsoft JhengHei");
            builder.AddMapping(&cjk, &[family.as_ptr()], None, PCWSTR::null(), PCWSTR::null(), 1.0)?;
            builder.AddMappings(&dwrite.GetSystemFontFallback()?)?;
            let fallback = builder.CreateFontFallback()?;
            Ok(Gfx { d2d_factory, d2d_device, dwrite, fallback, dcomp })
        }
    }

    struct TextStyle {
        font_px: f32,
        translation_scale: f32,
        effect: Effect,
    }

    impl TextStyle {
        fn row_h(&self) -> f32 {
            self.font_px * 1.375
        }
        fn margin(&self) -> f32 {
            // 3 sigma of the 0.3 em shadow sigma, or the half stroke
            (self.font_px * 0.9).ceil()
        }
    }

    struct CellLayout {
        main: IDWriteTextLayout,
        trans: IDWriteTextLayout,
        width: f32, // cell width including the U+3000 gap
    }

    fn layout(gfx: &Gfx, text: &str, size: f32) -> Result<IDWriteTextLayout> {
        unsafe {
            let format = gfx.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("zh-TW"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let f1: IDWriteTextFormat1 = format.cast()?;
            f1.SetFontFallback(&gfx.fallback)?;
            let wide: Vec<u16> = text.encode_utf16().collect();
            gfx.dwrite.CreateTextLayout(&wide, &format, 100000.0, size * 2.0)
        }
    }

    fn cell_layouts(gfx: &Gfx, style: &TextStyle) -> Result<Vec<CellLayout>> {
        let mut cells = Vec::new();
        for (main, trans, _) in LINES {
            let m = layout(gfx, main, style.font_px)?;
            let t = layout(gfx, trans, style.font_px * style.translation_scale)?;
            let (mw, tw) = (metrics(&m)?.widthIncludingTrailingWhitespace, metrics(&t)?.widthIncludingTrailingWhitespace);
            // lines joined by U+3000 (1 em); translation cells share the width
            cells.push(CellLayout { main: m, trans: t, width: mw.max(tw) + style.font_px });
        }
        Ok(cells)
    }

    fn metrics(l: &IDWriteTextLayout) -> Result<DWRITE_TEXT_METRICS> {
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { l.GetMetrics(&mut m)? };
        Ok(m)
    }

    fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r, g, b, a }
    }

    /// Draw one cell (main + translation row) with its effect at (x, y) = top-left of the text box.
    fn draw_cell(gfx: &Gfx, dc: &ID2D1DeviceContext, cell: &CellLayout, style: &TextStyle, x: f32, y: f32) -> Result<()> {
        unsafe {
            let text = dc.CreateSolidColorBrush(&color(1.0, 1.0, 1.0, 1.0), None)?;
            let row = style.row_h();
            let place = |l: &IDWriteTextLayout, row_index: f32| -> Result<Vector2> {
                let h = metrics(l)?.height;
                Ok(Vector2 { X: x, Y: y + row * row_index + (row - h) / 2.0 })
            };
            let pm = place(&cell.main, 0.0)?;
            let pt = place(&cell.trans, 1.0)?;
            let opts = D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_NO_SNAP;
            match style.effect {
                Effect::None => {}
                Effect::Shadow => {
                    // text-shadow: 0 0 0.6em rgba(effect, 0.75); CSS blur radius = 2 sigma → sigma 0.3 em
                    let list = dc.CreateCommandList()?;
                    let target = dc.GetTarget()?;
                    let transform = {
                        let mut m = Matrix3x2::default();
                        dc.GetTransform(&mut m);
                        m
                    };
                    dc.SetTarget(&list);
                    dc.DrawTextLayout(pm, &cell.main, &text, opts);
                    dc.DrawTextLayout(pt, &cell.trans, &text, opts);
                    list.Close()?;
                    dc.SetTarget(&target);
                    dc.SetTransform(&Matrix3x2::identity());
                    let shadow = dc.CreateEffect(&CLSID_D2D1Shadow)?;
                    shadow.SetInput(0, &list, true);
                    let sigma = style.font_px * 0.3;
                    shadow.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sigma.to_le_bytes())?;
                    let mut c = Vec::new();
                    for v in [0.0f32, 0.0, 0.0, 0.75] {
                        c.extend_from_slice(&v.to_le_bytes());
                    }
                    shadow.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &c)?;
                    let out = shadow.GetOutput()?;
                    dc.DrawImage(&out, Some(&Vector2 { X: transform.M31, Y: transform.M32 }), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                    dc.SetTransform(&transform);
                }
                Effect::Outline => {
                    // -webkit-text-stroke: 0.08em rgba(effect, 0.5); paint-order: stroke fill
                    let geometries = Rc::new(RefCell::new(Vec::new()));
                    let collector: IDWriteTextRenderer = OutlineCollector { factory: gfx.d2d_factory.clone(), geometries: geometries.clone() }.into();
                    cell.main.Draw(None, &collector, pm.X, pm.Y)?;
                    cell.trans.Draw(None, &collector, pt.X, pt.Y)?;
                    let stroke = dc.CreateSolidColorBrush(&color(0.0, 0.0, 0.0, 0.5), None)?;
                    for g in geometries.borrow().iter() {
                        dc.DrawGeometry(g, &stroke, style.font_px * 0.08, None);
                    }
                }
            }
            dc.DrawTextLayout(pm, &cell.main, &text, opts);
            dc.DrawTextLayout(pt, &cell.trans, &text, opts);
        }
        Ok(())
    }

    // ---------------------------------------------------------------- windows

    struct Cell {
        visual: IDCompositionVisual2,
        effect: IDCompositionEffectGroup,
    }

    struct LyricWindow {
        hwnd: HWND,
        _target: IDCompositionTarget,
        strip: IDCompositionVisual2,
        cells: Vec<Cell>,
        layouts: Vec<CellLayout>,
        lefts: Vec<f32>,
        widths: Vec<f32>,
        centre: f32,
        width: u32,
        height: u32,
        style: TextStyle,
        frame_surface: Option<IDCompositionSurface>,
        label: String,
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn create_window(gfx: &Gfx, opts: &Options, index: usize, mons: &[Monitor], tl: &Timeline) -> Result<LyricWindow> {
        unsafe {
            let as_taskbar = index < mons.len();
            let mon = &mons[if as_taskbar { index } else { 0 }];
            let scale = mon.dpi as f32 / 96.0;
            let mw = (mon.rect.right - mon.rect.left) as f32;
            let (x, y, w, h, font_dip, label) = if as_taskbar {
                let mut tb = (mon.rect.bottom - mon.work.bottom) as f32;
                if tb <= 0.0 {
                    tb = 48.0 * scale;
                }
                let font = (20.0f32).min(tb / scale / (2.0 * 1.375));
                (mon.rect.left as f32 + mw * 0.22, mon.rect.bottom as f32 - tb, mw * 0.33, tb, font, format!("taskbar@{}", mon.name))
            } else {
                let font = 32.0;
                let h = font * 1.375 * 2.0 * 1.4 * scale;
                (mon.rect.left as f32 + mw * 0.2, mon.work.bottom as f32 - h - 160.0 * scale, mw * 0.6, h, font, format!("floating@{}", mon.name))
            };
            let style = TextStyle { font_px: font_dip * scale, translation_scale: 1.0, effect: opts.effect };

            let class = w!("MoeLyricsNativeProto");
            let instance = GetModuleHandleW(None)?;
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() };
            RegisterClassW(&wc);
            let owner = if opts.owner_taskbar && as_taskbar { taskbar_for(mon) } else { None };
            let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT;
            let hwnd = CreateWindowExW(ex, class, w!("lyrics-native-proto"), WS_POPUP, x as i32, y as i32, w as i32, h as i32, owner, None, Some(instance.into()), None)?;
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc)?;
            let (width, height) = ((rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32);

            let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = gfx.dcomp.CreateVisual()?;
            let strip = gfx.dcomp.CreateVisual()?;
            if opts.linear {
                strip.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
            }
            root.AddVisual(&strip, false, None)?;
            target.SetRoot(&root)?;

            let layouts = cell_layouts(gfx, &style)?;
            let widths: Vec<f32> = layouts.iter().map(|c| c.width).collect();
            let mut lefts = Vec::new();
            let mut acc = 0.0;
            for wdt in &widths {
                lefts.push(acc);
                acc += wdt;
            }
            let centre = width as f32 / 2.0;
            let text_top = (height as f32 - style.row_h() * 2.0) / 2.0;
            let m = style.margin();

            let mut cells = Vec::new();
            let mut frame_surface = None;
            if opts.mode == Mode::Redraw {
                let s = gfx.dcomp.CreateSurface(width, height, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)?;
                root.SetContent(&s)?;
                frame_surface = Some(s);
            } else {
                for (i, cell) in layouts.iter().enumerate() {
                    let sw = (cell.width + 2.0 * m).ceil() as u32;
                    let sh = (style.row_h() * 2.0 + 2.0 * m).ceil() as u32;
                    let surface = gfx.dcomp.CreateSurface(sw, sh, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)?;
                    let mut offset = POINT::default();
                    let dc: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
                    dc.SetDpi(96.0, 96.0);
                    dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                    dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
                    dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
                    draw_cell(gfx, &dc, cell, &style, m, m)?;
                    surface.EndDraw()?;
                    let visual = gfx.dcomp.CreateVisual()?;
                    visual.SetContent(&surface)?;
                    visual.SetOffsetX2(lefts[i] - m)?;
                    visual.SetOffsetY2(text_top - m)?;
                    let effect = gfx.dcomp.CreateEffectGroup()?;
                    visual.SetEffect(&effect)?;
                    strip.AddVisual(&visual, true, None)?;
                    cells.push(Cell { visual, effect });
                }
            }
            let window = LyricWindow { hwnd, _target: target, strip, cells, layouts, lefts, widths, centre, width, height, style, frame_surface, label };
            if opts.mode == Mode::Anim {
                install_animations(gfx, &window, tl)?;
            }
            Ok(window)
        }
    }

    /// The whole glide and every fade as DComp animations (seconds), repeated forever: DWM evaluates them on every
    /// composed frame of the monitor showing the window; this process does nothing per frame.
    fn install_animations(gfx: &Gfx, w: &LyricWindow, tl: &Timeline) -> Result<()> {
        unsafe {
            let glide = gfx.dcomp.CreateAnimation()?;
            for i in 0..LINES.len() {
                let d = line_end(tl, i) - tl.starts[i];
                glide.AddCubic(tl.starts[i], w.centre - w.lefts[i], -(w.widths[i] / d as f32), 0.0, 0.0)?;
            }
            glide.AddRepeat(tl.total, tl.total)?;
            w.strip.SetOffsetX(&glide)?;

            let rise = (1.0 - INACTIVE) / LEAD as f32;
            for (i, cell) in w.cells.iter().enumerate() {
                let a = gfx.dcomp.CreateAnimation()?;
                let start = tl.starts[i];
                let end = line_end(tl, i);
                if i == 0 {
                    a.AddCubic(0.0, 1.0, 0.0, 0.0, 0.0)?;
                    a.AddCubic(end - LEAD, 1.0, -rise, 0.0, 0.0)?;
                    a.AddCubic(end, INACTIVE, 0.0, 0.0, 0.0)?;
                    a.AddCubic(tl.total - LEAD, INACTIVE, rise, 0.0, 0.0)?;
                } else {
                    a.AddCubic(0.0, INACTIVE, 0.0, 0.0, 0.0)?;
                    a.AddCubic(start - LEAD, INACTIVE, rise, 0.0, 0.0)?;
                    a.AddCubic(start, 1.0, 0.0, 0.0, 0.0)?;
                    a.AddCubic(end - LEAD, 1.0, -rise, 0.0, 0.0)?;
                    if end < tl.total {
                        a.AddCubic(end, INACTIVE, 0.0, 0.0, 0.0)?;
                    }
                }
                a.AddRepeat(tl.total, tl.total)?;
                cell.effect.SetOpacity(&a)?;
            }
        }
        Ok(())
    }

    fn per_frame_commit(w: &LyricWindow, tl: &Timeline, t: f64) -> Result<()> {
        unsafe {
            w.strip.SetOffsetX2(strip_offset(tl, &w.lefts, &w.widths, w.centre, t))?;
            for (i, cell) in w.cells.iter().enumerate() {
                cell.effect.SetOpacity2(line_opacity(tl, i, t))?;
            }
        }
        Ok(())
    }

    /// WebView-like worst case: clear and re-rasterise the visible cells (with blur) every frame.
    fn per_frame_redraw(gfx: &Gfx, w: &LyricWindow, tl: &Timeline, t: f64) -> Result<()> {
        unsafe {
            let surface = w.frame_surface.as_ref().unwrap();
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
            dc.SetDpi(96.0, 96.0);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            dc.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            let x0 = strip_offset(tl, &w.lefts, &w.widths, w.centre, t);
            let top = (w.height as f32 - w.style.row_h() * 2.0) / 2.0;
            for (i, cell) in w.layouts.iter().enumerate() {
                let x = x0 + w.lefts[i];
                if x + cell.width < -50.0 || x > w.width as f32 + 50.0 {
                    continue;
                }
                let layer_params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                    maskTransform: Matrix3x2::identity(),
                    opacity: line_opacity(tl, i, t),
                    ..Default::default()
                };
                dc.PushLayer(&layer_params, None);
                draw_cell(gfx, &dc, cell, &w.style, x, top)?;
                dc.PopLayer();
            }
            surface.EndDraw()?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- measurement

    struct TargetSample {
        id: COMPOSITION_TARGET_ID,
        stats: COMPOSITION_TARGET_STATS,
    }

    fn sample_targets() -> Vec<TargetSample> {
        unsafe {
            let Ok(frame) = DCompositionGetFrameId(COMPOSITION_FRAME_ID_COMPLETED) else { return Vec::new() };
            let mut fs = COMPOSITION_FRAME_STATS::default();
            let mut ids = [COMPOSITION_TARGET_ID::default(); 16];
            let mut count = 0u32;
            if DCompositionGetStatistics(frame, &mut fs, ids.len() as u32, Some(ids.as_mut_ptr()), Some(&mut count)).is_err() {
                return Vec::new();
            }
            ids[..count.min(16) as usize]
                .iter()
                .filter_map(|id| DCompositionGetTargetStatistics(frame, id).ok().map(|stats| TargetSample { id: *id, stats }))
                .collect()
        }
    }

    fn cpu_seconds() -> f64 {
        unsafe {
            let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
            let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7;
            f(k) + f(u)
        }
    }

    fn dwm_timing() -> Option<DWM_TIMING_INFO> {
        unsafe {
            let mut info = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
            DwmGetCompositionTimingInfo(HWND::default(), &mut info).ok().map(|_| info)
        }
    }

    /// Copy what DWM shows under each window (screen DC, physical px) to <prefix>-<n>.bmp.
    fn screenshot(windows: &[LyricWindow], prefix: &str) {
        unsafe {
            let screen = GetDC(None);
            for (n, w) in windows.iter().enumerate() {
                let mut rc = RECT::default();
                let _ = GetWindowRect(w.hwnd, &mut rc);
                // include some context above the window
                let pad = 40;
                let (x, y, cw, ch) = (rc.left, rc.top - pad, rc.right - rc.left, rc.bottom - rc.top + pad);
                let mem = CreateCompatibleDC(Some(screen));
                let bmp = CreateCompatibleBitmap(screen, cw, ch);
                let old = SelectObject(mem, bmp.into());
                let _ = BitBlt(mem, 0, 0, cw, ch, Some(screen), x, y, SRCCOPY);
                let mut info = BITMAPINFO::default();
                info.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: cw, biHeight: ch, biPlanes: 1, biBitCount: 32, ..Default::default() };
                let mut pixels = vec![0u8; (cw * ch * 4) as usize];
                GetDIBits(mem, bmp, 0, ch as u32, Some(pixels.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                let mut file = Vec::new();
                let size = 54 + pixels.len() as u32;
                file.extend_from_slice(b"BM");
                file.extend_from_slice(&size.to_le_bytes());
                file.extend_from_slice(&[0, 0, 0, 0]);
                file.extend_from_slice(&54u32.to_le_bytes());
                file.extend_from_slice(&40u32.to_le_bytes());
                file.extend_from_slice(&cw.to_le_bytes());
                file.extend_from_slice(&ch.to_le_bytes());
                file.extend_from_slice(&1u16.to_le_bytes());
                file.extend_from_slice(&32u16.to_le_bytes());
                file.extend_from_slice(&[0u8; 24]);
                file.extend_from_slice(&pixels);
                let _ = std::fs::write(format!("{prefix}-{n}.bmp"), file);
            }
            ReleaseDC(None, screen);
        }
    }

    // ---------------------------------------------------------------- main loop

    pub fn run() -> Result<()> {
        let opts = parse_options();
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let mons = monitors();
        for m in &mons {
            println!(
                "monitor {} primary={} rect=({},{})-({},{}) work_bottom={} dpi={} refresh={} Hz",
                m.name, m.primary, m.rect.left, m.rect.top, m.rect.right, m.rect.bottom, m.work.bottom, m.dpi, m.hz
            );
        }
        let tl = timeline(opts.speed_scale);
        let gfx = create_gfx()?;
        let mut windows = Vec::new();
        for i in 0..opts.windows {
            windows.push(create_window(&gfx, &opts, i, &mons, &tl)?);
        }
        unsafe { gfx.dcomp.Commit()? };
        for w in &windows {
            unsafe {
                let mut rc = RECT::default();
                let _ = GetWindowRect(w.hwnd, &mut rc);
                let centre = POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 };
                let hit = WindowFromPoint(centre);
                println!(
                    "window {} {}x{} px at ({},{}) dpi={} font={:.1}px click-through={} owner={:?}",
                    w.label,
                    w.width,
                    w.height,
                    rc.left,
                    rc.top,
                    GetDpiForWindow(w.hwnd),
                    w.style.font_px,
                    hit != w.hwnd,
                    GetWindow(w.hwnd, GW_OWNER).ok().map(|h| h.0)
                );
            }
        }
        println!("mode={:?} effect={:?} windows={} loop={:.1}s", opts.mode, opts.effect, windows.len(), tl.total);

        // let it settle, then measure
        let start = Instant::now();
        let settle = Duration::from_millis(1500);
        let mut measured = false;
        let mut t0 = (0.0, Vec::new(), None, Instant::now());
        let mut frames: u64 = 0;
        let mut worst_gap = 0.0f64;
        let mut last_frame = Instant::now();
        let run_for = if opts.seconds > 0.0 { Some(Duration::from_secs_f64(opts.seconds)) } else { None };
        let mut shot_pending = opts.shot.clone();
        let mut msg = MSG::default();
        loop {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return Ok(());
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            let now = Instant::now();
            if let Some(prefix) = shot_pending.as_ref() {
                if now - start >= Duration::from_millis(3000) {
                    screenshot(&windows, prefix);
                    shot_pending = None;
                }
            }
            if !measured && now - start >= settle {
                measured = true;
                t0 = (cpu_seconds(), sample_targets(), dwm_timing(), now);
                frames = 0;
                worst_gap = 0.0;
            }
            if let Some(limit) = run_for {
                if now - start >= limit + settle {
                    break;
                }
            }
            match opts.mode {
                Mode::Anim => unsafe {
                    // nothing per frame; wake a few times a second for messages and the timer
                    let _ = MsgWaitForMultipleObjects(None, false, 250, QS_ALLINPUT);
                },
                Mode::Commit | Mode::Redraw => unsafe {
                    let _ = DCompositionWaitForCompositorClock(None, 100);
                    let t = start.elapsed().as_secs_f64();
                    for w in &windows {
                        if opts.mode == Mode::Commit {
                            per_frame_commit(w, &tl, t)?;
                        } else {
                            per_frame_redraw(&gfx, w, &tl, t)?;
                        }
                    }
                    gfx.dcomp.Commit()?;
                    let f = Instant::now();
                    if measured {
                        frames += 1;
                        worst_gap = worst_gap.max((f - last_frame).as_secs_f64() * 1000.0);
                    }
                    last_frame = f;
                },
            }
        }
        let elapsed = t0.3.elapsed().as_secs_f64();
        let cpu = cpu_seconds() - t0.0;
        println!("measured {:.2}s: process CPU {:.2}% of one core", elapsed, cpu / elapsed * 100.0);
        if opts.mode != Mode::Anim {
            println!("own frames {:.1}/s (commits), worst gap {:.1} ms", frames as f64 / elapsed, worst_gap);
        }
        let end = sample_targets();
        for e in &end {
            if let Some(b) = t0.1.iter().find(|b| b.id == e.id) {
                let p = e.stats.presentedStats;
                let q = b.stats.presentedStats;
                println!(
                    "DWM target vidpn={} uid={}: presents {:.1}/s refreshes {:.1}/s (vblank {:.2} ms, outstanding {})",
                    e.id.vidPnSourceId,
                    e.id.uniqueId,
                    p.presentCount.wrapping_sub(q.presentCount) as f64 / elapsed,
                    p.refreshCount.wrapping_sub(q.refreshCount) as f64 / elapsed,
                    e.stats.vblankDuration as f64 / 1e4,
                    e.stats.outstandingPresents
                );
            }
        }
        if let (Some(a), Some(b)) = (t0.2, dwm_timing()) {
            println!(
                "DWM (global) frames {:.1}/s refreshes {:.1}/s dropped {} missed {} ",
                (b.cFrame - a.cFrame) as f64 / elapsed,
                (b.cRefresh - a.cRefresh) as f64 / elapsed,
                b.cFramesDropped - a.cFramesDropped,
                b.cFramesMissed - a.cFramesMissed
            );
        }
        for w in &windows {
            unsafe {
                let _ = DestroyWindow(w.hwnd);
            }
        }
        Ok(())
    }
}
