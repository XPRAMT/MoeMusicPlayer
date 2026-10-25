//! The Windows renderer thread (see the crate docs).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use player_lyrics_rules as rules;
use windows::core::{implement, w, Interface, Ref, Result, BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::{Size, TypedEventHandler};
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DwmFlush, DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::Composition::{
    ICompositionDrawingSurfaceInterop, ICompositorDesktopInterop, ICompositorInterop,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT,
};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Matrix3x2, Vector2, Vector3};

use crate::plan;
use crate::sampling::{self, Curve};
use crate::{
    dwrite_weight, effect_rgb, fit_font_size, floating_area_height_px, floating_lock_css,
    floating_lock_top_px, floating_main_slot_css, floating_margin_css, floating_top_room_px,
    translation_weight, Chrome, Events, Kind, Look, LyricsData, PlayState, PlaybackSample,
    FLOATING_LINE_HEIGHT, TASKBAR_LINE_HEIGHT, TASKBAR_SIDE_PADDING_CSS,
};

const WM_WAKE: u32 = WM_APP + 41;

/// Diagnostics: with `MOE_LYRICS_NATIVE_LOG=<file>` every re-plan and its reason is appended there.
fn debug_log(line: &str) {
    use std::io::Write;
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    if let Some(path) = PATH.get_or_init(|| std::env::var("MOE_LYRICS_NATIVE_LOG").ok()) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{:?} {line}", std::time::SystemTime::now());
        }
    }
}
/// How often an idle renderer checks that its GPU device still exists.
const DEVICE_CHECK: Duration = Duration::from_secs(2);

enum Cmd {
    Create {
        id: String,
        kind: Kind,
        owner: Option<isize>,
        reply: Sender<Option<isize>>,
    },
    Destroy(String),
    Look(String, Option<Look>),
    Chrome(String, Chrome),
    /// Fixed mode: the cursor is over the window (the watcher tells, the window itself ignores the mouse).
    Hover(String, bool),
    ClickThrough(String, bool),
    Lyrics(Option<LyricsData>),
    Playback(PlaybackSample),
    /// The theme accent (word highlight in the none mode, ADR 0041).
    Accent([u8; 3]),
    Delay {
        track_id: String,
        delay_sec: f64,
    },
    Shutdown,
    Clock(crate::ClockSource),
}

/// Handle to the renderer thread. Cheap to clone; every call only posts a command.
#[derive(Clone)]
pub struct NativeLyrics {
    tx: Sender<Cmd>,
    thread: u32,
}

impl NativeLyrics {
    pub fn start(events: Arc<dyn Events>) -> std::io::Result<Self> {
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel();
        std::thread::Builder::new()
            .name("moe-lyrics-native".into())
            .spawn(move || {
                let thread = unsafe { GetCurrentThreadId() };
                // Make the thread's message queue exist before anyone posts to it.
                let mut msg = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                }
                let _ = ready_tx.send(thread);
                run(rx, events);
            })?;
        let thread = ready_rx
            .recv()
            .map_err(|_| std::io::Error::other("lyrics renderer did not start"))?;
        Ok(Self { tx, thread })
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok() {
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Creates a hidden window; returns its HWND (the caller places and shows it with SetWindowPos).
    pub fn create(&self, id: &str, kind: Kind, owner: Option<isize>) -> Option<isize> {
        let (reply, rx) = channel();
        self.send(Cmd::Create {
            id: id.to_owned(),
            kind,
            owner,
            reply,
        });
        rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
    }
    pub fn destroy(&self, id: &str) {
        self.send(Cmd::Destroy(id.to_owned()));
    }
    /// `None` hides the text (e.g. disabled settings) without destroying the window.
    pub fn set_look(&self, id: &str, look: Option<Look>) {
        self.send(Cmd::Look(id.to_owned(), look));
    }
    pub fn set_chrome(&self, id: &str, chrome: Chrome) {
        self.send(Cmd::Chrome(id.to_owned(), chrome));
    }
    pub fn set_hover(&self, id: &str, hover: bool) {
        self.send(Cmd::Hover(id.to_owned(), hover));
    }
    pub fn set_click_through(&self, id: &str, through: bool) {
        self.send(Cmd::ClickThrough(id.to_owned(), through));
    }
    pub fn set_lyrics(&self, lyrics: Option<LyricsData>) {
        self.send(Cmd::Lyrics(lyrics));
    }
    pub fn set_playback(&self, sample: PlaybackSample) {
        self.send(Cmd::Playback(sample));
    }
    pub fn set_accent(&self, rgb: [u8; 3]) {
        self.send(Cmd::Accent(rgb));
    }
    pub fn set_timing_delay(&self, track_id: &str, delay_sec: f64) {
        self.send(Cmd::Delay {
            track_id: track_id.to_owned(),
            delay_sec,
        });
    }
    /// The audio engine's clock: while it is set, the glide follows it (page samples then only carry the track,
    /// state and duration).
    pub fn set_clock_source(&self, source: crate::ClockSource) {
        self.send(Cmd::Clock(source));
    }
    pub fn shutdown(&self) {
        self.send(Cmd::Shutdown);
    }
}

// ---------------------------------------------------------------- devices

struct Gfx {
    d3d: ID3D11Device,
    d2d_factory: ID2D1Factory1,
    _d2d_device: ID2D1Device,
    dwrite: IDWriteFactory2,
    fallback: IDWriteFontFallback,
    compositor: Compositor,
    graphics: CompositionGraphicsDevice,
    /// Set by `CompositionGraphicsDevice.RenderingDeviceReplaced`: every drawing surface must be drawn again.
    replaced: Arc<AtomicBool>,
    /// Segoe UI ascent and descent in em: the line box of every row (CSS uses the primary font's metrics).
    ascent_em: f32,
    descent_em: f32,
}

/// Windows.UI.Composition with `CompositionVisualSurface` (the edge-fade mask) needs Windows 10 1809
/// (UniversalApiContract 7); the desktop interop itself needs 1803.
fn composition_supported() -> bool {
    ApiInformation::IsApiContractPresentByMajor(
        &HSTRING::from("Windows.Foundation.UniversalApiContract"),
        7,
    )
    .unwrap_or(false)
}

/// The thread's dispatcher queue: Windows.UI.Composition on a desktop thread needs one (it commits implicitly
/// through it while the thread pumps messages).
fn create_dispatcher_queue() -> Result<DispatcherQueueController> {
    unsafe {
        CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })
    }
}

fn create_gfx() -> Result<Gfx> {
    unsafe {
        let mut d3d: Option<ID3D11Device> = None;
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let hardware = D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d),
            None,
            None,
        );
        if hardware.is_err() {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
        }
        let d3d = d3d.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let d2d_factory: ID2D1Factory1 =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let d2d_device = d2d_factory.CreateDevice(&dxgi)?;
        let compositor = Compositor::new()?;
        let graphics = compositor
            .cast::<ICompositorInterop>()?
            .CreateGraphicsDevice(&d2d_device)?;
        let replaced = Arc::new(AtomicBool::new(false));
        let flag = replaced.clone();
        let thread = GetCurrentThreadId();
        graphics.RenderingDeviceReplaced(&TypedEventHandler::new(move |_, _| {
            flag.store(true, Ordering::SeqCst);
            let _ = PostThreadMessageW(thread, WM_WAKE, WPARAM(0), LPARAM(0));
            Ok(())
        }))?;
        let dwrite: IDWriteFactory2 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        // 'Segoe UI', 'Microsoft JhengHei', sans-serif (app.css): CJK and kana go to JhengHei, then the system
        // fallback (Segoe UI Emoji for colour emoji).
        let builder = dwrite.CreateFontFallbackBuilder()?;
        let cjk = [
            DWRITE_UNICODE_RANGE {
                first: 0x2E80,
                last: 0x9FFF,
            },
            DWRITE_UNICODE_RANGE {
                first: 0xF900,
                last: 0xFAFF,
            },
            DWRITE_UNICODE_RANGE {
                first: 0xFF00,
                last: 0xFFEF,
            },
            DWRITE_UNICODE_RANGE {
                first: 0x20000,
                last: 0x2FFFF,
            },
        ];
        let family = HSTRING::from("Microsoft JhengHei");
        builder.AddMapping(
            &cjk,
            &[family.as_ptr()],
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            1.0,
        )?;
        builder.AddMappings(&dwrite.GetSystemFontFallback()?)?;
        let fallback = builder.CreateFontFallback()?;
        let (ascent_em, descent_em) = segoe_metrics(&dwrite).unwrap_or((1.0, 0.25));
        Ok(Gfx {
            d3d,
            d2d_factory,
            _d2d_device: d2d_device,
            dwrite,
            fallback,
            compositor,
            graphics,
            replaced,
            ascent_em,
            descent_em,
        })
    }
}

/// A Direct2D-drawn composition surface and its size in device pixels.
#[derive(Clone)]
struct Surface {
    surface: CompositionDrawingSurface,
    width: u32,
    height: u32,
}

/// Creates a `width` x `height` px drawing surface and draws it once with `draw` (the device context is translated
/// to the surface's update offset by `base`, cleared to transparent, at 96 DPI so one unit is one device pixel).
fn draw_surface(
    gfx: &Gfx,
    width: u32,
    height: u32,
    draw: impl FnOnce(&ID2D1DeviceContext, Matrix3x2) -> Result<()>,
) -> Result<Surface> {
    let (width, height) = (width.max(1), height.max(1));
    let surface = gfx.graphics.CreateDrawingSurface(
        Size {
            Width: width as f32,
            Height: height as f32,
        },
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        DirectXAlphaMode::Premultiplied,
    )?;
    let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
    unsafe {
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
        dc.SetDpi(96.0, 96.0);
        let base = Matrix3x2::translation(offset.x as f32, offset.y as f32);
        dc.SetTransform(&base);
        dc.Clear(Some(&rgba([0, 0, 0], 0.0)));
        let drawn = draw(&dc, base);
        let ended = interop.EndDraw();
        drawn?;
        ended?;
    }
    Ok(Surface {
        surface,
        width,
        height,
    })
}

/// A sprite showing `surface` 1:1 at its pixel size, sampled with linear filtering (the glide moves in sub-pixel
/// steps, `sampling::OFFSET_STEP_PX`).
fn sprite(gfx: &Gfx, surface: &Surface) -> Result<SpriteVisual> {
    let brush = gfx
        .compositor
        .CreateSurfaceBrushWithSurface(&surface.surface.cast::<ICompositionSurface>()?)?;
    brush.SetStretch(CompositionStretch::None)?;
    brush.SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::Linear)?;
    let visual = gfx.compositor.CreateSpriteVisual()?;
    visual.SetBrush(&brush)?;
    visual.SetSize(Vector2 {
        X: surface.width as f32,
        Y: surface.height as f32,
    })?;
    Ok(visual)
}

fn set_offset(visual: &Visual, x: f64, y: f64) -> Result<()> {
    visual.SetOffset(Vector3 {
        X: x as f32,
        Y: y as f32,
        Z: 0.0,
    })
}

fn segoe_metrics(dwrite: &IDWriteFactory2) -> Result<(f32, f32)> {
    unsafe {
        let mut collection = None;
        dwrite.GetSystemFontCollection(&mut collection, false)?;
        let collection = collection.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let mut index = 0;
        let mut exists = FALSE;
        collection.FindFamilyName(w!("Segoe UI"), &mut index, &mut exists)?;
        if !exists.as_bool() {
            return Err(E_FAIL.into());
        }
        let font = collection.GetFontFamily(index)?.GetFirstMatchingFont(
            DWRITE_FONT_WEIGHT_MEDIUM,
            DWRITE_FONT_STRETCH_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
        )?;
        let mut m = DWRITE_FONT_METRICS::default();
        font.GetMetrics(&mut m);
        let em = m.designUnitsPerEm as f32;
        Ok((m.ascent as f32 / em, m.descent as f32 / em))
    }
}

fn device_lost(gfx: &Gfx) -> bool {
    unsafe { gfx.d3d.GetDeviceRemovedReason().is_err() }
}

fn is_device_loss(error: &windows::core::Error) -> bool {
    let code = error.code();
    code == DXGI_ERROR_DEVICE_REMOVED
        || code == DXGI_ERROR_DEVICE_RESET
        || code == D2DERR_RECREATE_TARGET
}

// ---------------------------------------------------------------- glyph outlines (for the stroke effect)

#[implement(IDWriteTextRenderer)]
struct OutlineCollector {
    factory: ID2D1Factory1,
    geometries: Rc<RefCell<Vec<ID2D1Geometry>>>,
}

impl IDWritePixelSnapping_Impl for OutlineCollector_Impl {
    fn IsPixelSnappingDisabled(&self, _: *const core::ffi::c_void) -> Result<BOOL> {
        Ok(TRUE)
    }
    fn GetCurrentTransform(
        &self,
        _: *const core::ffi::c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> Result<()> {
        unsafe {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m12: 0.0,
                m21: 0.0,
                m22: 1.0,
                dx: 0.0,
                dy: 0.0,
            };
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
            let face = run
                .fontFace
                .as_ref()
                .ok_or(windows::core::Error::from(E_POINTER))?;
            let path = self.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            face.GetGlyphRunOutline(
                run.fontEmSize,
                run.glyphIndices,
                Some(run.glyphAdvances),
                if run.glyphOffsets.is_null() {
                    None
                } else {
                    Some(run.glyphOffsets)
                },
                run.glyphCount,
                run.isSideways.as_bool(),
                run.bidiLevel % 2 == 1,
                &sink,
            )?;
            sink.Close()?;
            let moved = self
                .factory
                .CreateTransformedGeometry(&path, &Matrix3x2::translation(x, y))?;
            self.geometries.borrow_mut().push(moved.cast()?);
        }
        Ok(())
    }
    fn DrawUnderline(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_UNDERLINE,
        _: Ref<windows::core::IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
    fn DrawStrikethrough(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_STRIKETHROUGH,
        _: Ref<windows::core::IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
    fn DrawInlineObject(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: Ref<IDWriteInlineObject>,
        _: BOOL,
        _: BOOL,
        _: Ref<windows::core::IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- text

#[derive(Clone, Copy, PartialEq)]
struct TextStyle {
    color: [u8; 3],
    effect: rules::TextEffect,
    /// Line box in em.
    line_height: f32,
    /// The colour the shadow / outline colour is taken from (the text colour; the accent copy of a line keeps the
    /// white line's effect, so the two surfaces' shadows are identical and a clip between them shows no seam).
    effect_from: [u8; 3],
    /// Weight of the original text; translation rows carry `translation_weight` of it in their row.
    weight: u16,
}

fn rgba(c: [u8; 3], a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c[0] as f32 / 255.0,
        g: c[1] as f32 / 255.0,
        b: c[2] as f32 / 255.0,
        a,
    }
}

fn text_layout(
    gfx: &Gfx,
    text: &str,
    size: f32,
    weight: u16,
    line_height: f32,
) -> Result<IDWriteTextLayout> {
    unsafe {
        let format = gfx.dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT(dwrite_weight(weight)),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("zh-TW"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        // CSS line box: the content box (ascent + descent) centred in line-height x font size.
        let box_h = line_height * size;
        let baseline =
            (box_h - (gfx.ascent_em + gfx.descent_em) * size) / 2.0 + gfx.ascent_em * size;
        format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, box_h, baseline)?;
        let f1: IDWriteTextFormat1 = format.cast()?;
        f1.SetFontFallback(&gfx.fallback)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        gfx.dwrite.CreateTextLayout(&wide, &format, 1.0e6, box_h)
    }
}

fn text_width(layout: &IDWriteTextLayout) -> f32 {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = layout.GetMetrics(&mut m);
    }
    m.widthIncludingTrailingWhitespace.ceil()
}

/// The centre of the first grapheme of a row (px from its left edge): the continuous strip's anchor (ADR 0041 D10).
fn first_glyph_centre(layout: &IDWriteTextLayout) -> f32 {
    let mut x = 0.0;
    let mut y = 0.0;
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
        if layout
            .HitTestTextPosition(0, false, &mut x, &mut y, &mut m)
            .is_ok()
        {
            return m.left + m.width / 2.0;
        }
    }
    0.0
}

/// The centre of the last grapheme of a row (px from its left edge), for the terminal glide.
fn last_glyph_centre(layout: &IDWriteTextLayout, text: &str) -> f32 {
    use unicode_segmentation::UnicodeSegmentation;
    let prefix = text
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(i, _)| i);
    let start = text[..prefix].encode_utf16().count();
    let mut x = 0.0;
    let mut y = 0.0;
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
        if layout
            .HitTestTextPosition(start as u32, false, &mut x, &mut y, &mut m)
            .is_ok()
        {
            return m.left + m.width / 2.0;
        }
    }
    text_width(layout)
}

/// Rows drawn into one surface, stacked: `(text, font px)`.
struct RasterRequest<'a> {
    rows: &'a [(&'a str, f32, u16)],
    style: TextStyle,
}

struct Raster {
    surface: Surface,
    /// Room around the text for the effect (px on every side).
    margin: f32,
    row_widths: Vec<f32>,
}

fn effect_margin(font_px: f32) -> f32 {
    // 3 sigma of the 0.3 em shadow (covers the outline too).
    (font_px * 0.9).ceil().max(1.0)
}

fn rasterise(gfx: &Gfx, request: &RasterRequest) -> Result<Raster> {
    unsafe {
        let font = request.rows.first().map(|r| r.1).unwrap_or(1.0);
        let margin = effect_margin(font);
        let layouts: Vec<IDWriteTextLayout> = request
            .rows
            .iter()
            .map(|(text, px, weight)| {
                text_layout(gfx, text, *px, *weight, request.style.line_height)
            })
            .collect::<Result<_>>()?;
        let row_widths: Vec<f32> = layouts.iter().map(text_width).collect();
        let width = row_widths.iter().copied().fold(1.0f32, f32::max);
        let height: f32 = request
            .rows
            .iter()
            .map(|r| r.1 * request.style.line_height)
            .sum();
        let sw = (width + 2.0 * margin).ceil().max(1.0) as u32;
        let sh = (height + 2.0 * margin).ceil().max(1.0) as u32;
        let surface = draw_surface(gfx, sw, sh, |dc, base| {
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let mut origins = Vec::new();
            let mut y = margin;
            for (_, px, _) in request.rows {
                origins.push(Vector2 { X: margin, Y: y });
                y += px * request.style.line_height;
            }
            let text_brush = dc.CreateSolidColorBrush(&rgba(request.style.color, 1.0), None)?;
            let effect_color = effect_rgb(request.style.effect_from);
            let options = D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_NO_SNAP;
            match request.style.effect {
                rules::TextEffect::None => {}
                rules::TextEffect::Shadow => {
                    // text-shadow: 0 0 0.6em rgba(effect, 0.75) — a CSS blur radius is 2 sigma, so sigma = 0.3 em.
                    let list = dc.CreateCommandList()?;
                    let target = dc.GetTarget()?;
                    dc.SetTarget(&list);
                    dc.SetTransform(&Matrix3x2::identity());
                    for (layout, origin) in layouts.iter().zip(&origins) {
                        dc.DrawTextLayout(*origin, layout, &text_brush, options);
                    }
                    list.Close()?;
                    dc.SetTarget(&target);
                    let shadow = dc.CreateEffect(&CLSID_D2D1Shadow)?;
                    shadow.SetInput(0, &list, true);
                    let sigma = font * 0.3;
                    shadow.SetValue(
                        D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32,
                        D2D1_PROPERTY_TYPE_FLOAT,
                        &sigma.to_le_bytes(),
                    )?;
                    let colour = rgba(effect_color, 0.75);
                    let mut bytes = Vec::new();
                    for v in [colour.r, colour.g, colour.b, colour.a] {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                    shadow.SetValue(
                        D2D1_SHADOW_PROP_COLOR.0 as u32,
                        D2D1_PROPERTY_TYPE_VECTOR4,
                        &bytes,
                    )?;
                    let output = shadow.GetOutput()?;
                    dc.SetTransform(&base);
                    dc.DrawImage(
                        &output,
                        None,
                        None,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        D2D1_COMPOSITE_MODE_SOURCE_OVER,
                    );
                }
                rules::TextEffect::Stroke => {
                    // -webkit-text-stroke: 0.08em rgba(effect, 0.5), stroke under the fill.
                    let geometries = Rc::new(RefCell::new(Vec::new()));
                    let collector: IDWriteTextRenderer = OutlineCollector {
                        factory: gfx.d2d_factory.clone(),
                        geometries: geometries.clone(),
                    }
                    .into();
                    for (layout, origin) in layouts.iter().zip(&origins) {
                        layout.Draw(None, &collector, origin.X, origin.Y)?;
                    }
                    let stroke = dc.CreateSolidColorBrush(&rgba(effect_color, 0.5), None)?;
                    for g in geometries.borrow().iter() {
                        dc.DrawGeometry(g, &stroke, font * 0.08, None);
                    }
                }
            }
            for (layout, origin) in layouts.iter().zip(&origins) {
                dc.DrawTextLayout(*origin, layout, &text_brush, options);
            }
            Ok(())
        })?;
        Ok(Raster {
            surface,
            margin,
            row_widths,
        })
    }
}

/// The padlock button (floating window): a dark circle with a thin light ring and the lock glyph.
fn rasterise_lock(
    gfx: &Gfx,
    size: u32,
    scale: f32,
    locked: bool,
    pressed_hover: bool,
) -> Result<Surface> {
    draw_surface(gfx, size, size, |dc, base| unsafe {
        let offset = POINT {
            x: base.M31 as i32,
            y: base.M32 as i32,
        };
        let s = size.max(1) as f32;
        let r = s / 2.0;
        let fill = dc.CreateSolidColorBrush(
            &rgba([0, 0, 0], if pressed_hover { 0.75 } else { 0.55 }),
            None,
        )?;
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: r, Y: r },
            radiusX: r,
            radiusY: r,
        };
        dc.FillEllipse(&ellipse, &fill);
        let ring = dc.CreateSolidColorBrush(&rgba([255, 255, 255], 0.35), None)?;
        let border = scale;
        let inner = D2D1_ELLIPSE {
            point: Vector2 { X: r, Y: r },
            radiusX: r - border / 2.0,
            radiusY: r - border / 2.0,
        };
        dc.DrawEllipse(&inner, &ring, border, None);
        // The 24x24 icon at 62 % of the button, centred.
        let icon = s * 0.62;
        let unit = icon / 24.0;
        let origin = (s - icon) / 2.0;
        dc.SetTransform(
            &(Matrix3x2 {
                M11: unit,
                M12: 0.0,
                M21: 0.0,
                M22: unit,
                M31: origin + offset.x as f32,
                M32: origin + offset.y as f32,
            }),
        );
        let white = dc.CreateSolidColorBrush(&rgba([255, 255, 255], 1.0), None)?;
        let body = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: 5.0,
                top: 11.0,
                right: 19.0,
                bottom: 21.0,
            },
            radiusX: 2.0,
            radiusY: 2.0,
        };
        dc.FillRoundedRectangle(&body, &white);
        // M8 11V8a4 4 0 0 1 8 0v3 (closed) / M8 11V8a4 4 0 0 1 7.6-1.7 (open), floating-lyrics-lock-icon.js.
        let path = gfx.d2d_factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(Vector2 { X: 8.0, Y: 11.0 }, D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLine(Vector2 { X: 8.0, Y: 8.0 });
        let end = if locked {
            Vector2 { X: 16.0, Y: 8.0 }
        } else {
            Vector2 { X: 15.6, Y: 6.3 }
        };
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: end,
            size: D2D_SIZE_F {
                width: 4.0,
                height: 4.0,
            },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: D2D1_ARC_SIZE_SMALL,
        });
        if locked {
            sink.AddLine(Vector2 { X: 16.0, Y: 11.0 });
        }
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        let props = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
            transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
        };
        let style = gfx.d2d_factory.CreateStrokeStyle(&props, None)?;
        dc.DrawGeometry(&path, &white, 2.0, &style);
        Ok(())
    })
}

/// The move-mode highlight behind the lyric area: rgba(0,0,0,0.28), 10 CSS px corners.
fn rasterise_area(gfx: &Gfx, width: u32, height: u32, radius: f32) -> Result<Surface> {
    draw_surface(gfx, width, height, |dc, _| unsafe {
        let brush = dc.CreateSolidColorBrush(&rgba([0, 0, 0], 0.28), None)?;
        let rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: width as f32,
                bottom: height as f32,
            },
            radiusX: radius,
            radiusY: radius,
        };
        dc.FillRoundedRectangle(&rect, &brush);
        Ok(())
    })
}

// ---------------------------------------------------------------- animations

/// The none mode's karaoke (ADR 0041): a word-timed main row is two sprites of the same position, the white line
/// and its accent copy, with complementary clips: the accent one from the row's left to the karaoke edge
/// (`rules::karaoke_glide`: one smooth curve per line, hard at the first word's start and the last word's end, the
/// words guiding it within the speed band), the white one from the edge on. The
/// edge wipes with the audio clock and is written only when it moved at least 1/8 device px, so a still line commits
/// nothing and nothing is rasterised per frame.
struct WordClip {
    /// `[lit, dim]`.
    clips: [InsetClip; 2],
    /// The edge (surface px) over seconds from the commit.
    glide: Vec<rules::GlidePiece>,
    times: Vec<(f64, f64)>,
    width: f64,
    /// The edge last written (NaN before the first write).
    shown: f64,
}

impl WordClip {
    fn sample(&mut self, t: f64) -> Result<bool> {
        let x = rules::glide_value(&self.glide, t).clamp(0.0, self.width);
        if (x - self.shown).abs() < 0.125 {
            return Ok(false);
        }
        let [lit, dim] = &self.clips;
        lit.SetRightInset((self.width - x) as f32)?;
        dim.SetLeftInset(x as f32)?;
        self.shown = x;
        Ok(true)
    }
}

/// Each word's x range `(left, right)` in a row's layout (px from the text's left edge), from DirectWrite's hit
/// testing on the whole line (so kerning and CJK spacing are the line's); an empty word is a point.
fn word_spans(layout: &IDWriteTextLayout, ranges: &[(usize, usize)]) -> Vec<(f32, f32)> {
    ranges
        .iter()
        .map(|&(start, len)| unsafe {
            if len > 0 {
                let mut metrics = [DWRITE_HIT_TEST_METRICS::default(); 16];
                let mut count = 0u32;
                if layout
                    .HitTestTextRange(
                        start as u32,
                        len as u32,
                        0.0,
                        0.0,
                        Some(&mut metrics),
                        &mut count,
                    )
                    .is_ok()
                    && count > 0
                {
                    let used = &metrics[..(count as usize).min(metrics.len())];
                    let l = used.iter().map(|m| m.left).fold(f32::INFINITY, f32::min);
                    let r = used
                        .iter()
                        .map(|m| m.left + m.width)
                        .fold(f32::NEG_INFINITY, f32::max);
                    return (l, r);
                }
            }
            let (mut x, mut y) = (0.0, 0.0);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = layout.HitTestTextPosition(start as u32, false, &mut x, &mut y, &mut m);
            (x, x)
        })
        .collect()
}

/// One property the renderer thread samples every frame (`sampling`): the strip's or a row's Offset.X, or a
/// cell's Opacity, from its plan's curve. `written` is the device value last set (NaN before the first write).
struct Live {
    visual: Visual,
    kind: sampling::Kind,
    curve: Curve,
    /// Offset.Y kept with every Offset write.
    y: f64,
    written: f64,
}

impl Live {
    /// Writes the value at `t` seconds after the plan's instant if its device value changed; true when it wrote.
    fn sample(&mut self, t: f64) -> Result<bool> {
        let Some(v) = sampling::changed(self.kind, &self.curve, t, self.written) else {
            return Ok(false);
        };
        match self.kind {
            sampling::Kind::OffsetX => set_offset(&self.visual, v, self.y)?,
            sampling::Kind::Opacity => self.visual.SetOpacity(v as f32)?,
        }
        self.written = v;
        Ok(true)
    }
}

// ---------------------------------------------------------------- windows

/// State the window procedure reads and writes (never the renderer state, which may be borrowed).
struct Shared {
    id: String,
    kind: Kind,
    locked: Cell<bool>,
    /// Client rect of the padlock (floating), empty when hidden.
    lock_rect: Cell<RECT>,
    mouse_inside: Cell<bool>,
    over_lock: Cell<bool>,
    /// A mouse drag in move mode: (cursor at the button press, window top-left then), screen px.
    drag: Cell<Option<(POINT, POINT)>>,
}

enum Pending {
    Dirty(String),
    LockClicked(String),
    Moved(String, i32, i32),
    Close(String),
    DragEnded(String),
}

thread_local! {
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
}

fn push(p: Pending) {
    PENDING.with(|q| q.borrow_mut().push(p));
}

fn shared(hwnd: HWND) -> Option<&'static Shared> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared;
        ptr.as_ref()
    }
}

fn in_rect(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn cursor_in_window(hwnd: HWND) -> bool {
    unsafe {
        let mut p = POINT::default();
        let mut r = RECT::default();
        GetCursorPos(&mut p).is_ok() && GetWindowRect(hwnd, &mut r).is_ok() && in_rect(r, p.x, p.y)
    }
}

fn track_leave(hwnd: HWND, nonclient: bool) {
    let mut t = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: if nonclient {
            TME_LEAVE | TME_NONCLIENT
        } else {
            TME_LEAVE
        },
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut t);
    }
}

fn mouse_at(hwnd: HWND, s: &Shared, client: POINT, nonclient: bool) {
    if !s.mouse_inside.get() {
        s.mouse_inside.set(true);
        push(Pending::Dirty(s.id.clone()));
    }
    let over = in_rect(s.lock_rect.get(), client.x, client.y);
    if over != s.over_lock.get() {
        s.over_lock.set(over);
        push(Pending::Dirty(s.id.clone()));
    }
    track_leave(hwnd, nonclient);
}

fn lparam_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xffff) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(s) = shared(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_NCHITTEST => {
            if s.kind == Kind::Taskbar {
                return LRESULT(HTTRANSPARENT as isize);
            }
            // Move mode drags with our own capture (`WM_LBUTTONDOWN`), not the system caption drag: that one keeps
            // the window on screen and stops before the transparent margins can leave it (ADR 0032 placement).
            LRESULT(HTCLIENT as isize)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SETCURSOR => {
            let cursor = if s.over_lock.get() {
                IDC_HAND
            } else if s.kind == Kind::Floating && !s.locked.get() {
                IDC_SIZEALL
            } else {
                IDC_ARROW
            };
            if let Ok(c) = LoadCursorW(None, cursor) {
                SetCursor(Some(c));
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            if let Some((cursor, origin)) = s.drag.get() {
                let mut now = POINT::default();
                if GetCursorPos(&mut now).is_ok() {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        origin.x + now.x - cursor.x,
                        origin.y + now.y - cursor.y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                return LRESULT(0);
            }
            mouse_at(hwnd, s, lparam_point(lparam), false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN if s.kind == Kind::Floating && !s.locked.get() => {
            let p = lparam_point(lparam);
            let mut cursor = POINT::default();
            let mut r = RECT::default();
            if !in_rect(s.lock_rect.get(), p.x, p.y)
                && GetCursorPos(&mut cursor).is_ok()
                && GetWindowRect(hwnd, &mut r).is_ok()
            {
                s.drag.set(Some((
                    cursor,
                    POINT {
                        x: r.left,
                        y: r.top,
                    },
                )));
                SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            if s.drag.take().is_some() {
                push(Pending::DragEnded(s.id.clone()));
            }
            LRESULT(0)
        }
        WM_NCMOUSEMOVE => {
            let mut p = lparam_point(lparam);
            let _ = ScreenToClient(hwnd, &mut p);
            mouse_at(hwnd, s, p, true);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => {
            if !cursor_in_window(hwnd) {
                s.mouse_inside.set(false);
                s.over_lock.set(false);
                push(Pending::Dirty(s.id.clone()));
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if s.drag.get().is_some() {
                // WM_CAPTURECHANGED ends the drag.
                let _ = ReleaseCapture();
                return LRESULT(0);
            }
            let p = lparam_point(lparam);
            if in_rect(s.lock_rect.get(), p.x, p.y) {
                push(Pending::LockClicked(s.id.clone()));
            }
            LRESULT(0)
        }
        WM_MOVE if s.kind == Kind::Floating => {
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_ok() {
                push(Pending::Moved(s.id.clone(), r.left, r.top));
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            if s.kind == Kind::Floating {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            push(Pending::Dirty(s.id.clone()));
            LRESULT(0)
        }
        WM_SIZE => {
            push(Pending::Dirty(s.id.clone()));
            LRESULT(0)
        }
        WM_CLOSE => {
            push(Pending::Close(s.id.clone()));
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Shared;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

const CLASS: PCWSTR = w!("MoeLyricsNative");

fn register_class() {
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
    }
}

/// Lines beyond the drawn ones that must be rasterised already, and how many are drawn ahead in one batch.
const PREWARM_NEEDED: usize = 2;
const PREWARM_LINES: usize = 100;
/// Later batches (while the strip glides) stay small: each line costs about 1 ms of CPU on top of the commit wait,
/// and 100 at once stalled about 270 ms in the demo.
const PREWARM_LATER_LINES: usize = 16;
/// The theme's default accent (`theme.js`), until the main window sends the chosen one.
const DEFAULT_ACCENT: [u8; 3] = [0xA9, 0xC3, 0xFE];

/// (font, translation font, line count), main widths, translation widths, last glyph centre, and per line the
/// word centres (px from the text's left edge; empty for a line without words).
/// Per lyrics, size and look: main and translation widths, the last glyph centre of the last line, each line's first
/// glyph centre and each line's word centres.
/// Measurement key: main px, translation px, line count, weight.
type MeasureKey = (u32, u32, usize, u16);
type MeasuredLines = (MeasureKey, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>);

/// Owned raster rows: `(text, font px, weight)`.
type OwnedRows = Vec<(String, f32, u16)>;

struct Win {
    hwnd: HWND,
    kind: Kind,
    target: Option<DesktopWindowTarget>,
    root: Option<ContainerVisual>,
    /// The edge-fade mask host, rebuilt only on a size, DPI or area change.
    fade: Option<FadeHost>,
    look: Option<Look>,
    chrome: Chrome,
    /// Fixed mode hover from the watcher.
    hover: bool,
    tooltip: Option<HWND>,
    tooltip_text: Vec<u16>,
    cache: HashMap<String, Rc<Raster>>,
    lock_cache: HashMap<(u32, bool, bool), Surface>,
    area_cache: HashMap<(u32, u32, u32), Surface>,
    wake_at: Option<Instant>,
    has_translation: Option<bool>,
    /// The properties sampled every frame and the instant their plan's time 0 stands for.
    live: Vec<Live>,
    /// The none mode's sung-word clips, sampled every frame with `live`.
    words: Vec<WordClip>,
    plan_at: Instant,
    /// Line widths of the continuous strip: (font, translation font, line count), main and translation widths,
    /// the last glyph's centre. Cleared with the raster cache (new lyrics or look).
    measured: Option<MeasuredLines>,
    /// Lines to rasterise ahead of their entry (rows, style).
    prewarm: Vec<(OwnedRows, TextStyle)>,
    /// Until when any of them still moves (`None` = nothing moves).
    moving_until: Option<Instant>,
}

fn create_window(id: &str, kind: Kind, owner: Option<isize>) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let mut ex = WS_EX_NOREDIRECTIONBITMAP
            | WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_NOACTIVATE
            | WS_EX_LAYERED;
        if kind == Kind::Taskbar {
            ex |= WS_EX_TRANSPARENT;
        }
        let owner = owner.map(|h| HWND(h as *mut _));
        let title = HSTRING::from(id);
        let hwnd = CreateWindowExW(
            ex,
            CLASS,
            &title,
            WS_POPUP,
            0,
            0,
            1,
            1,
            owner,
            None,
            Some(instance.into()),
            None,
        )?;
        let shared = Box::new(Shared {
            id: id.to_owned(),
            kind,
            locked: Cell::new(false),
            lock_rect: Cell::new(RECT::default()),
            mouse_inside: Cell::new(false),
            over_lock: Cell::new(false),
            drag: Cell::new(None),
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(shared) as isize);
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        Ok(hwnd)
    }
}

fn create_tooltip(hwnd: HWND) -> Option<HWND> {
    unsafe {
        let tip = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            TOOLTIPS_CLASSW,
            None,
            WINDOW_STYLE(WS_POPUP.0 | TTS_ALWAYSTIP | TTS_NOPREFIX),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(hwnd),
            None,
            GetModuleHandleW(None).ok().map(|m| m.into()),
            None,
        )
        .ok()?;
        let mut info = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_SUBCLASS,
            hwnd,
            uId: 1,
            ..Default::default()
        };
        info.lpszText = PWSTR(w!("").as_ptr() as *mut _);
        SendMessageW(
            tip,
            TTM_ADDTOOLW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
        Some(tip)
    }
}

fn update_tooltip(win: &mut Win, rect: RECT, text: &str) {
    let Some(tip) = win.tooltip else { return };
    win.tooltip_text = text.encode_utf16().chain(Some(0)).collect();
    let mut info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_SUBCLASS,
        hwnd: win.hwnd,
        uId: 1,
        rect,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tip,
            TTM_NEWTOOLRECTW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
        info.lpszText = PWSTR(win.tooltip_text.as_mut_ptr());
        SendMessageW(
            tip,
            TTM_UPDATETIPTEXTW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
    }
}

// ---------------------------------------------------------------- renderer state

struct Clock {
    track_id: Option<String>,
    state: PlayState,
    motion: plan::Motion,
    at: Instant,
    duration_ms: Option<f64>,
}

impl Clock {
    fn dt(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.at).as_secs_f64() * 1000.0
    }

    /// The drift correction still running at `now`: (rate, lyrics ms until it ends).
    fn slew(&self, now: Instant) -> Option<(f64, f64)> {
        if self.state != PlayState::Playing {
            return None;
        }
        self.motion.slew(self.dt(now))
    }

    fn position(&self, now: Instant) -> f64 {
        let p = if self.state == PlayState::Playing {
            self.motion.position(self.dt(now))
        } else {
            self.motion.base_ms
        };
        match self.duration_ms {
            Some(d) if d > 0.0 => p.clamp(0.0, d),
            _ => p.max(0.0),
        }
    }
}

struct Renderer {
    gfx: Option<Gfx>,
    windows: HashMap<String, Win>,
    lyrics: Option<LyricsData>,
    clock: Clock,
    accent: [u8; 3],
    delay: Option<(String, f64)>,
    events: Arc<dyn Events>,
    dirty: Vec<String>,
    source: Option<crate::ClockSource>,
    sync: plan::AudioSync,
    last_reading: Option<Instant>,
    /// False on Windows older than 10 1809, or without a dispatcher queue: no lyrics windows at all.
    supported: bool,
}

fn run(rx: Receiver<Cmd>, events: Arc<dyn Events>) {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    register_class();
    // Kept alive for the thread's lifetime: Windows.UI.Composition commits through it.
    let queue = if composition_supported() {
        create_dispatcher_queue()
            .map_err(|e| eprintln!("lyrics renderer: no dispatcher queue: {e}"))
            .ok()
    } else {
        eprintln!(
            "lyrics renderer: needs Windows 10 version 1809 or later; lyrics windows are off"
        );
        None
    };
    let supported = queue.is_some();
    let mut r = Renderer {
        gfx: if supported {
            create_gfx()
                .map_err(|e| eprintln!("lyrics renderer: {e}"))
                .ok()
        } else {
            None
        },
        supported,
        windows: HashMap::new(),
        lyrics: None,
        clock: Clock {
            track_id: None,
            state: PlayState::Stopped,
            motion: plan::Motion::at(0.0),
            at: Instant::now(),
            duration_ms: None,
        },
        accent: DEFAULT_ACCENT,
        delay: None,
        events,
        dirty: Vec::new(),
        source: None,
        sync: plan::AudioSync::default(),
        last_reading: None,
    };
    let mut last_check = Instant::now();
    loop {
        while let Ok(cmd) = rx.try_recv() {
            if !r.command(cmd) {
                r.destroy_all();
                return;
            }
        }
        pump_messages();
        r.pending();
        if r.gfx
            .as_ref()
            .is_some_and(|g| g.replaced.swap(false, Ordering::SeqCst))
        {
            // CompositionGraphicsDevice.RenderingDeviceReplaced: the drawing surfaces lost their content.
            r.forget_surfaces();
            r.mark_all();
        }
        let now = Instant::now();
        for (id, win) in &r.windows {
            if win.wake_at.is_some_and(|at| at <= now) {
                r.dirty.push(id.clone());
            }
        }
        if now.duration_since(last_check) >= DEVICE_CHECK {
            last_check = now;
            if r.supported && r.gfx.as_ref().is_none_or(device_lost) {
                r.rebuild_devices();
            }
        }
        r.audio_sync();
        r.refresh_dirty();
        if r.windows.values().any(|w| w.moving_until.is_some()) {
            // Per-frame commit: sample every moving window for the next frame, commit (the message pump runs the
            // implicit commit), then wait for DWM's next composition. One loop for all windows, paced by DWM.
            r.sample_frame();
            pump_messages();
            r.prewarm_batch();
            let waited = Instant::now();
            unsafe {
                let _ = DwmFlush();
            }
            // DwmFlush returns at once while DWM has nothing to compose: never spin, wait about a frame instead.
            if waited.elapsed() < Duration::from_millis(1) {
                let lead = frame_lead().max(Duration::from_millis(4));
                unsafe {
                    let _ = MsgWaitForMultipleObjects(
                        None,
                        false,
                        lead.as_millis() as u32,
                        QS_ALLINPUT,
                    );
                }
            }
            continue;
        }
        // Idle (a still line): the next line's rasters are drawn now, long before they are needed.
        r.prewarm_batch();
        let next = r.windows.values().filter_map(|w| w.wake_at).min();
        let mut timeout = DEVICE_CHECK.saturating_sub(last_check.elapsed());
        if let Some(at) = next {
            timeout = timeout.min(at.saturating_duration_since(Instant::now()));
        }
        unsafe {
            let _ = MsgWaitForMultipleObjects(
                None,
                false,
                timeout.as_millis().min(u32::MAX as u128) as u32,
                QS_ALLINPUT,
            );
        }
    }
}

/// Diagnostics: with `MOE_LYRICS_FRAME_LOG=<file>` every sampled frame appends
/// `<ms> frame <id> <wake_ms> <sample_ms> <clock_ms> <offset> <committed 0|1>` (the renderer's playback clock at the sample time and
/// the first per-frame property, the strip or the first row, as written) and every new audio-clock reading
/// `<ms> audio <dt_ms> <audio_ms> <predicted_ms> <error_ms>`; ms since the first logged line.
fn frame_log(id: &str, woke: Instant, frame: Instant, clock_ms: f64, win: &Win, changed: bool) {
    let Some(first) = win.live.first() else {
        return;
    };
    let written = first.written;
    frame_log_write(|ms| {
        format!(
            "{:.3} frame {id} {:.3} {:.3} {clock_ms:.3} {written} {}
",
            ms(woke),
            ms(woke),
            ms(frame),
            changed as u8
        )
    });
}

fn frame_log_write(line: impl FnOnce(&dyn Fn(Instant) -> f64) -> String) {
    use std::io::Write;
    use std::sync::Mutex;
    type FrameLog = Mutex<(std::io::BufWriter<std::fs::File>, Instant)>;
    static LOG: std::sync::OnceLock<Option<FrameLog>> = std::sync::OnceLock::new();
    let Some(log) = LOG.get_or_init(|| {
        let path = std::env::var("MOE_LYRICS_FRAME_LOG").ok()?;
        let file = std::fs::File::create(path).ok()?;
        Some(Mutex::new((std::io::BufWriter::new(file), Instant::now())))
    }) else {
        return;
    };
    if let Ok(mut g) = log.lock() {
        let start = g.1;
        let text = line(&|t: Instant| t.saturating_duration_since(start).as_secs_f64() * 1000.0);
        let _ = g.0.write_all(text.as_bytes());
        let _ = g.0.flush();
    }
}

/// A free-form line in the frame log (`MOE_LYRICS_FRAME_LOG`).
fn frame_log_line(line: &str) {
    frame_log_write(|ms| format!("{} {line}\n", ms(Instant::now())));
}

/// Dispatches the thread's messages (window messages, the dispatcher queue and so the composition commit).
fn pump_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message != WM_WAKE {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// How long after a value is set it reaches the screen: the implicit commit, DWM's composition of the window's
/// visual tree and the present. Measured with per-frame screen captures against the DirectComposition renderer
/// (ADR 0034, 2026-10-24): about 39 ms on a 120 Hz display and 22 ms on a 60 Hz one; 32 ms keeps line arrivals
/// within a frame of the playback clock on both. Values are sampled for that moment.
const PRESENT_LATENCY: Duration = Duration::from_millis(32);

impl Renderer {
    /// Rasterises the queued upcoming lines of every window together (after this frame's commit).
    fn prewarm_batch(&mut self) {
        let Some(gfx) = self.gfx.as_ref() else { return };
        let started = Instant::now();
        let mut count = 0;
        for win in self.windows.values_mut() {
            for (rows, style) in std::mem::take(&mut win.prewarm) {
                let rows: Vec<(&str, f32, u16)> =
                    rows.iter().map(|(t, f, w)| (t.as_str(), *f, *w)).collect();
                if let Err(e) = cached(gfx, win, &rows, style) {
                    eprintln!("lyrics prewarm: {e}");
                }
                count += 1;
            }
        }
        if count > 0 {
            let bytes = shared_bytes();
            frame_log_line(&format!(
                "prewarm {count} lines took {:.3} ms, all caches {:.1} MiB",
                started.elapsed().as_secs_f64() * 1000.0,
                bytes as f64 / 1048576.0
            ));
        }
    }

    /// Writes the changed per-frame values of every moving window for the coming frame.
    fn sample_frame(&mut self) {
        let woke = Instant::now();
        let frame = present_time();
        let mut lost = false;
        for (id, win) in self.windows.iter_mut() {
            if win.moving_until.is_none() {
                continue;
            }
            let result = sample_window(win, frame);
            let changed = matches!(result, Ok(true));
            frame_log(id, woke, frame, self.clock.position(frame), win, changed);
            match result {
                Ok(_) => {}
                Err(e) if is_device_loss(&e) => lost = true,
                Err(e) => eprintln!("lyrics window {id}: {e}"),
            }
        }
        if lost {
            self.rebuild_devices();
        }
    }

    /// Compares each new audio-clock reading with the renderer's clock; a window's mean error corrects the clock
    /// gently (or jumps on a seek or stall).
    fn audio_sync(&mut self) {
        if self.clock.state != PlayState::Playing {
            return;
        }
        let Some(reading) = self.source.as_ref().and_then(|s| s()) else {
            return;
        };
        if !reading.playing || self.last_reading == Some(reading.at) {
            return;
        }
        self.last_reading = Some(reading.at);
        let dt = reading
            .at
            .saturating_duration_since(self.clock.at)
            .as_secs_f64()
            * 1000.0;
        if reading.at < self.clock.at {
            return;
        }
        let predicted = self.clock.motion.position(dt);
        let error = reading.position_ms - predicted;
        frame_log_line(&format!(
            "audio {:.3} {:.3} {:.3} {:.3}",
            dt, reading.position_ms, predicted, error
        ));
        let wall = reading
            .at
            .saturating_duration_since(self.clock.at)
            .as_secs_f64()
            * 1000.0;
        let Some(mean) = self.sync.observe(wall, error) else {
            return;
        };
        match self.clock.motion.audio_correct(dt, mean) {
            plan::Correction::Keep => {}
            plan::Correction::Slew(motion) => {
                debug_log(&format!(
                    "audio slew: mean error {mean:.1} rate {:.5}",
                    motion.rate
                ));
                self.clock.motion = motion;
                self.clock.at = reading.at;
                self.sync.reset();
                self.mark_all();
            }
            plan::Correction::Jump => {
                let now = Instant::now();
                let position = reading.position_ms
                    + now.saturating_duration_since(reading.at).as_secs_f64() * 1000.0;
                debug_log(&format!(
                    "audio jump: mean error {mean:.1} to {position:.0}"
                ));
                self.clock.motion = plan::Motion::at(position);
                self.clock.at = now;
                self.sync.reset();
                self.mark_all();
            }
        }
    }

    fn mark_all(&mut self) {
        self.dirty.extend(self.windows.keys().cloned());
    }

    fn command(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Create {
                id,
                kind,
                owner,
                reply,
            } => {
                let created = self.create(&id, kind, owner);
                let _ = reply.send(created.map(|h| h.0 as isize));
            }
            Cmd::Destroy(id) => {
                if let Some(win) = self.windows.remove(&id) {
                    destroy(win);
                }
            }
            Cmd::Look(id, look) => {
                if let Some(win) = self.windows.get_mut(&id) {
                    if win.look != look {
                        win.look = look;
                        win.cache.clear();
                        win.measured = None;
                        self.dirty.push(id);
                    }
                }
            }
            Cmd::Chrome(id, chrome) => {
                if let Some(win) = self.windows.get_mut(&id) {
                    if let Some(s) = shared(win.hwnd) {
                        s.locked.set(chrome.locked);
                    }
                    win.chrome = chrome;
                    self.dirty.push(id);
                }
            }
            Cmd::Hover(id, hover) => {
                if let Some(win) = self.windows.get_mut(&id) {
                    if win.hover != hover {
                        win.hover = hover;
                        self.dirty.push(id);
                    }
                }
            }
            Cmd::ClickThrough(id, through) => {
                if let Some(win) = self.windows.get(&id) {
                    unsafe {
                        let ex = GetWindowLongPtrW(win.hwnd, GWL_EXSTYLE);
                        let flag = WS_EX_TRANSPARENT.0 as isize;
                        let next = if through { ex | flag } else { ex & !flag };
                        if next != ex {
                            SetWindowLongPtrW(win.hwnd, GWL_EXSTYLE, next);
                        }
                    }
                }
            }
            Cmd::Lyrics(lyrics) => {
                if self.lyrics != lyrics {
                    self.lyrics = lyrics;
                    for win in self.windows.values_mut() {
                        win.cache.clear();
                        win.measured = None;
                    }
                    self.mark_all();
                }
            }
            Cmd::Playback(sample) => {
                let now = Instant::now();
                let same = self.clock.track_id == sample.track_id
                    && self.clock.state == sample.state
                    && self.clock.duration_ms == sample.duration_ms;
                // The audio engine's clock, when there is one, is the position (the page's sample is up to
                // 500 ms old and arrives with a variable latency).
                let reading = self.source.as_ref().and_then(|s| s());
                let mut sample = sample;
                if let Some(r) = reading {
                    sample.position_ms = r.position_ms
                        + if r.playing && sample.state == PlayState::Playing {
                            now.saturating_duration_since(r.at).as_secs_f64() * 1000.0
                        } else {
                            0.0
                        };
                }
                let correction = if !same {
                    plan::Correction::Jump
                } else if sample.state == PlayState::Playing && reading.is_some() {
                    // Playing on: the audio clock keeps the glide in sync (`audio_sync`).
                    plan::Correction::Keep
                } else if sample.state == PlayState::Playing {
                    // Small drift is absorbed by a rate change from the current position (never a step).
                    self.clock
                        .motion
                        .correct(self.clock.dt(now), sample.position_ms)
                } else if (self.clock.position(now) - sample.position_ms).abs()
                    < plan::CLOCK_TOLERANCE_MS
                {
                    plan::Correction::Keep
                } else {
                    plan::Correction::Jump
                };
                match correction {
                    plan::Correction::Keep => {}
                    plan::Correction::Slew(motion) => {
                        debug_log(&format!(
                            "slew: predicted {:.1} sample {:.1} rate {:.4}",
                            motion.base_ms, sample.position_ms, motion.rate
                        ));
                        self.clock.motion = motion;
                        self.clock.at = now;
                        self.mark_all();
                    }
                    plan::Correction::Jump => {
                        debug_log(&format!(
                            "rebase: predicted {:.0} sample {:.0} state {:?}->{:?} track same {}",
                            self.clock.position(now),
                            sample.position_ms,
                            self.clock.state,
                            sample.state,
                            self.clock.track_id == sample.track_id
                        ));
                        self.sync.reset();
                        self.clock = Clock {
                            track_id: sample.track_id,
                            state: sample.state,
                            motion: plan::Motion::at(sample.position_ms),
                            at: now,
                            duration_ms: sample.duration_ms,
                        };
                        self.mark_all();
                    }
                }
            }
            Cmd::Accent(rgb) => {
                if self.accent != rgb {
                    self.accent = rgb;
                    for win in self.windows.values_mut() {
                        win.cache.clear();
                    }
                    self.mark_all();
                }
            }
            Cmd::Delay {
                track_id,
                delay_sec,
            } => {
                let next = Some((track_id, rules::clamp_timing_delay_sec(delay_sec)));
                if self.delay != next {
                    self.delay = next;
                    self.mark_all();
                }
            }
            Cmd::Shutdown => return false,
            Cmd::Clock(source) => {
                self.source = Some(source);
                self.sync.reset();
            }
        }
        true
    }

    fn create(&mut self, id: &str, kind: Kind, owner: Option<isize>) -> Option<HWND> {
        if let Some(old) = self.windows.remove(id) {
            destroy(old);
        }
        if !self.supported {
            return None;
        }
        let hwnd = create_window(id, kind, owner)
            .map_err(|e| eprintln!("lyrics window {id}: {e}"))
            .ok()?;
        let tooltip = if kind == Kind::Floating {
            create_tooltip(hwnd)
        } else {
            None
        };
        let mut win = Win {
            hwnd,
            kind,
            target: None,
            root: None,
            fade: None,
            look: None,
            chrome: Chrome::default(),
            hover: false,
            tooltip,
            tooltip_text: Vec::new(),
            cache: HashMap::new(),
            lock_cache: HashMap::new(),
            area_cache: HashMap::new(),
            wake_at: None,
            has_translation: None,
            live: Vec::new(),
            plan_at: Instant::now(),
            measured: None,
            words: Vec::new(),
            prewarm: Vec::new(),
            moving_until: None,
        };
        if let Some(gfx) = &self.gfx {
            attach(gfx, &mut win);
        }
        self.windows.insert(id.to_owned(), win);
        Some(hwnd)
    }

    fn destroy_all(&mut self) {
        for (_, win) in self.windows.drain() {
            destroy(win);
        }
    }

    fn pending(&mut self) {
        let items: Vec<Pending> = PENDING.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for item in items {
            match item {
                Pending::Dirty(id) => self.dirty.push(id),
                Pending::LockClicked(id) => self.events.lock_clicked(&id),
                Pending::Moved(id, x, y) => self.events.moved(&id, x, y),
                Pending::Close(id) => self.events.close_requested(&id),
                Pending::DragEnded(id) => self.events.drag_ended(&id),
            }
        }
    }

    fn forget_surfaces(&mut self) {
        SHARED.with(|s| s.borrow_mut().clear());
        for win in self.windows.values_mut() {
            win.cache.clear();
            win.measured = None;
            win.lock_cache.clear();
            win.area_cache.clear();
        }
    }

    fn rebuild_devices(&mut self) {
        self.forget_surfaces();
        for win in self.windows.values_mut() {
            win.fade = None;
            win.root = None;
            if let Some(target) = win.target.take() {
                let _ = target.Close();
            }
        }
        self.gfx = None;
        match create_gfx() {
            Ok(gfx) => {
                for win in self.windows.values_mut() {
                    attach(&gfx, win);
                }
                self.gfx = Some(gfx);
                self.mark_all();
            }
            Err(e) => eprintln!("lyrics renderer devices: {e}"),
        }
    }

    fn refresh_dirty(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        let mut ids = std::mem::take(&mut self.dirty);
        ids.sort();
        ids.dedup();
        let started = Instant::now();
        debug_log(&format!("refresh {ids:?}"));
        let Some(gfx) = self.gfx.as_ref() else { return };
        let mut lost = false;
        let ctx = Context {
            lyrics: self.lyrics.as_ref(),
            clock: &self.clock,
            accent: self.accent,
            delay: self.delay.as_ref(),
            now: Instant::now(),
            frame: present_time(),
        };
        let mut reports = Vec::new();
        for id in &ids {
            if let Some(win) = self.windows.get_mut(id) {
                match refresh(gfx, win, &ctx) {
                    Ok(has) => {
                        if win.kind == Kind::Floating && win.has_translation != Some(has) {
                            win.has_translation = Some(has);
                            reports.push((id.clone(), has));
                        }
                    }
                    Err(e) if is_device_loss(&e) => lost = true,
                    Err(e) => eprintln!("lyrics window {id}: {e}"),
                }
            }
        }
        // Windows.UI.Composition commits implicitly when the thread returns to its message loop.
        frame_log_line(&format!(
            "refresh {} took {:.3} ms",
            ids.join(","),
            started.elapsed().as_secs_f64() * 1000.0
        ));
        for (id, has) in reports {
            self.events.has_translation(&id, has);
        }
        if lost {
            self.rebuild_devices();
        }
    }
}

/// Values set now are committed when the thread returns to its message loop and composed at the next DWM frame:
/// sample them for that frame's vblank, so the strip shows where the playback clock is when it reaches the screen.
/// When values set now reach the screen, on DWM's vblank grid: the last vblank DWM reports (advanced by whole
/// periods if DWM has been idle) plus `PRESENT_LATENCY` rounded to whole periods. Every frame's sample time is then
/// exactly one refresh period after the previous one, however late the thread woke after `DwmFlush`, so the deltas
/// follow the curve without wake-time noise. Falls back to now + `PRESENT_LATENCY` without DWM timing.
fn present_time() -> Instant {
    let now_instant = Instant::now();
    let fallback = now_instant + PRESENT_LATENCY;
    unsafe {
        let mut info = DWM_TIMING_INFO {
            cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        let (mut now, mut freq) = (0i64, 0i64);
        if DwmGetCompositionTimingInfo(HWND::default(), &mut info).is_err()
            || QueryPerformanceCounter(&mut now).is_err()
            || QueryPerformanceFrequency(&mut freq).is_err()
            || freq <= 0
            || info.qpcRefreshPeriod == 0
        {
            return fallback;
        }
        let period = info.qpcRefreshPeriod as i64;
        let vblank = info.qpcVBlank as i64;
        let last = vblank + (now - vblank).max(0) / period * period;
        let frames = (PRESENT_LATENCY.as_secs_f64() * freq as f64 / period as f64)
            .round()
            .max(1.0) as i64;
        let target = last + frames * period;
        let ahead = (target - now) as f64 / freq as f64;
        if !(0.0..=0.2).contains(&ahead) {
            return fallback;
        }
        now_instant + Duration::from_secs_f64(ahead)
    }
}

fn frame_lead() -> Duration {
    unsafe {
        let mut info = DWM_TIMING_INFO {
            cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        let (mut now, mut freq) = (0i64, 0i64);
        if DwmGetCompositionTimingInfo(HWND::default(), &mut info).is_err()
            || QueryPerformanceCounter(&mut now).is_err()
            || QueryPerformanceFrequency(&mut freq).is_err()
            || freq <= 0
            || info.qpcRefreshPeriod == 0
        {
            return Duration::ZERO;
        }
        let period = info.qpcRefreshPeriod as i64;
        let vblank = info.qpcVBlank as i64;
        let behind = (now - vblank).max(0);
        let next = vblank + (behind / period + 1) * period;
        let ticks = (next - now).clamp(0, freq / 20);
        Duration::from_secs_f64(ticks as f64 / freq as f64)
    }
}

fn attach(gfx: &Gfx, win: &mut Win) {
    let made = (|| -> Result<(DesktopWindowTarget, ContainerVisual)> {
        let target = unsafe {
            gfx.compositor
                .cast::<ICompositorDesktopInterop>()?
                .CreateDesktopWindowTarget(win.hwnd, true)?
        };
        let root = gfx.compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        Ok((target, root))
    })();
    match made {
        Ok((target, root)) => {
            win.target = Some(target);
            win.root = Some(root);
        }
        Err(e) => eprintln!("lyrics window target: {e}"),
    }
}

fn destroy(win: Win) {
    if let Some(target) = &win.target {
        let _ = target.Close();
    }
    unsafe {
        if let Some(tip) = win.tooltip {
            let _ = DestroyWindow(tip);
        }
        let _ = DestroyWindow(win.hwnd);
    }
}

struct Context<'a> {
    lyrics: Option<&'a LyricsData>,
    clock: &'a Clock,
    accent: [u8; 3],
    delay: Option<&'a (String, f64)>,
    /// The plan's time 0 (when the plan is made).
    now: Instant,
    /// When the values this refresh sets reach the screen (`PRESENT_LATENCY`).
    frame: Instant,
}

/// Top of the first row. Floating: the main row's slot (from the chosen font alone) so the first row never moves when
/// a translation row comes or goes; the translation hangs below it. Taskbar: the rows centred in the area.
fn rows_top(
    area: Area,
    floating: bool,
    look: &Look,
    scale: f64,
    rows_h: f64,
    main_row_h: f64,
) -> f64 {
    if floating {
        area.top + (floating_main_slot_css(look.font_size_px) * scale - main_row_h) / 2.0
    } else {
        area.top + (area.height - rows_h) / 2.0
    }
}

#[derive(Clone, Copy)]
struct Area {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

/// Rebuilds one window's visual tree; returns whether it draws a translation line.
/// Registers a per-frame property of this plan and writes its value for the coming frame.
fn live(
    win: &mut Win,
    ctx: &Context,
    visual: Visual,
    kind: sampling::Kind,
    curve: Curve,
    y: f64,
) -> Result<()> {
    let end = curve.end();
    let mut l = Live {
        visual,
        kind,
        curve,
        y,
        written: f64::NAN,
    };
    l.sample(ctx.frame.saturating_duration_since(ctx.now).as_secs_f64())?;
    if ctx.clock.state == PlayState::Playing && end > 0.0 {
        let until = ctx.now + Duration::from_secs_f64(end);
        win.moving_until = Some(win.moving_until.map_or(until, |u| u.max(until)));
    }
    win.live.push(l);
    Ok(())
}

/// Samples every per-frame property of a window for the frame presented at `frame`; true when any changed.
fn sample_window(win: &mut Win, frame: Instant) -> Result<bool> {
    let t = frame.saturating_duration_since(win.plan_at).as_secs_f64();
    let mut any = false;
    for l in &mut win.live {
        any |= l.sample(t)?;
    }
    for word in &mut win.words {
        any |= word.sample(t)?;
    }
    if win.moving_until.is_some_and(|u| frame > u) {
        win.moving_until = None;
    }
    Ok(any)
}

fn refresh(gfx: &Gfx, win: &mut Win, ctx: &Context) -> Result<bool> {
    // Nothing rasterised yet (new lyrics, look or size): the up-front batch is drawn in this refresh.
    let fresh = win.cache.is_empty();
    win.wake_at = None;
    win.live.clear();
    win.words.clear();
    win.moving_until = None;
    win.plan_at = ctx.now;
    let Some(root) = win.root.clone() else {
        return Ok(false);
    };
    root.Children()?.RemoveAll()?;
    let mut client = RECT::default();
    unsafe {
        GetClientRect(win.hwnd, &mut client)?;
    }
    let (w, h) = (
        (client.right - client.left) as f64,
        (client.bottom - client.top) as f64,
    );
    let scale = unsafe { GetDpiForWindow(win.hwnd) }.max(96) as f64 / 96.0;
    let Some(look) = win.look.clone() else {
        return Ok(false);
    };
    if w < 1.0 || h < 1.0 {
        return Ok(false);
    }
    let floating = win.kind == Kind::Floating;
    let line_height = if floating {
        FLOATING_LINE_HEIGHT
    } else {
        TASKBAR_LINE_HEIGHT
    };

    // The lyric area (physical px) inside the window.
    let area = if floating {
        let margin = floating_margin_css(look.font_size_px) * scale;
        let lock = floating_lock_css(look.font_size_px) * scale;
        // The room above is from the main row only (the translation row hangs below it): ADR 0032.
        let top = f64::from(floating_top_room_px(look.font_size_px, scale));
        Area {
            left: margin,
            top,
            width: (w - margin * 2.0 - lock).max(1.0),
            height: floating_area_height_px(h, top),
        }
    } else {
        let pad = TASKBAR_SIDE_PADDING_CSS * scale;
        Area {
            left: pad,
            top: 0.0,
            width: (w - 2.0 * pad).max(1.0),
            height: h,
        }
    };

    root.SetSize(Vector2 {
        X: w as f32,
        Y: h as f32,
    })?;
    // Animation "none": the shown line at the main opacity. "continuous": the container is opaque, the static mask
    // carries the main and the secondary opacity.
    let opacity = plan::container_opacity(look.animation, look.main_opacity_percent);
    let (mask_main, mask_secondary) =
        plan::mask_percents(look.main_opacity_percent, look.secondary_opacity_percent);

    if floating {
        chrome(gfx, win, &root, area, w, scale, &look)?;
    }

    let lyrics = ctx
        .lyrics
        .filter(|l| ctx.clock.track_id.as_deref() == Some(l.track_id.as_str()));
    let state = ctx.clock.state;
    let Some(lyrics) = lyrics.filter(|_| state.shows_lyrics()) else {
        return Ok(false);
    };
    let delay = ctx
        .delay
        .filter(|(t, _)| *t == lyrics.track_id)
        .map_or(0.0, |d| d.1);
    let offset = rules::display_offset_ms(lyrics.offset_ms, delay);
    let position = ctx.clock.position(ctx.now);
    let playing = state == PlayState::Playing;
    let style = TextStyle {
        color: look.color,
        effect: look.effect,
        line_height: line_height as f32,
        effect_from: look.color,
        weight: look.font_weight,
    };
    let trans_weight = translation_weight(look.font_weight);
    let clip_room = match look.effect {
        rules::TextEffect::Shadow => 0.6,
        rules::TextEffect::Stroke => 0.04,
        rules::TextEffect::None => 0.0,
    };

    let has_translation;
    if look.animation == rules::Animation::Continuous {
        let lines: Vec<_> = lyrics
            .lines
            .iter()
            .filter_map(|l| {
                let line = (
                    l.start_ms,
                    l.text.trim(),
                    if look.show_translation {
                        l.translation.trim()
                    } else {
                        ""
                    },
                );
                (!line.1.is_empty() || !line.2.is_empty()).then_some((line, l))
            })
            .collect();
        let sources: Vec<&crate::LyricLine> = lines.iter().map(|l| l.1).collect();
        let lines: Vec<(f64, &str, &str)> = lines.into_iter().map(|l| l.0).collect();
        // Word-timed lines (ADR 0041 D10), each judged on its own: every line start is a knot, a word-timed line adds
        // its last word's end, a last line without word times the terminal estimate. Line- and word-timed alike see
        // the static continuous mask (experimental) instead of edge fades and line fades. Translations have no word
        // effects: they follow the main row.
        let word_ranges: Vec<Option<Vec<(usize, usize)>>> =
            sources.iter().map(|l| l.word_ranges()).collect();
        // Without any word-timed line the knots are exactly the line glide's (`strip_cubic`, tested in
        // `without_word_times_the_knot_glide_is_the_line_glide`), so such a song keeps the line path bit for bit.
        let any_word_line = word_ranges.iter().any(Option::is_some);
        has_translation = lines.iter().any(|l| !l.2.is_empty());
        if lines.is_empty() {
            return Ok(false);
        }
        let font_css = fit_font_size(
            look.font_size_px,
            area.height / scale,
            if has_translation {
                look.translation_scale_percent
            } else {
                0.0
            },
            line_height,
        );
        let font = (font_css * scale) as f32;
        let trans_font = font * (look.translation_scale_percent / 100.0) as f32;
        // Measure every line once per lyrics, size and look (DirectWrite layouts; a few hundred of them take tens of
        // milliseconds, far too long to redo at every line change).
        let key = (
            font.to_bits(),
            trans_font.to_bits(),
            lines.len(),
            look.font_weight,
        );
        if win.measured.as_ref().is_none_or(|m| m.0 != key) {
            let mut main_widths = Vec::new();
            let mut trans_widths = Vec::new();
            let mut last_centres = Vec::new();
            let mut first_centres = Vec::new();
            for (_, text, tr) in lines.iter() {
                let main = text_layout(gfx, text, font, style.weight, style.line_height)?;
                main_widths.push(if text.is_empty() {
                    0.0
                } else {
                    text_width(&main) as f64
                });
                first_centres.push(if text.is_empty() {
                    0.0
                } else {
                    first_glyph_centre(&main) as f64
                });
                last_centres.push(if text.is_empty() {
                    0.0
                } else {
                    last_glyph_centre(&main, text) as f64
                });
                trans_widths.push(if tr.is_empty() {
                    0.0
                } else {
                    text_width(&text_layout(
                        gfx,
                        tr,
                        trans_font,
                        trans_weight,
                        style.line_height,
                    )?) as f64
                });
            }
            win.measured = Some((key, main_widths, trans_widths, last_centres, first_centres));
        }
        let (_, main_widths, trans_widths, last_centres, first_centres) =
            win.measured.clone().unwrap();
        let last_centre = last_centres.last().copied().unwrap_or(0.0);
        let starts: Vec<f64> = lines.iter().map(|l| l.0).collect();
        let mut layout = rules::layout_strip(&main_widths, &trans_widths, font as f64);
        // Every line, word-timed or not, sits with its first glyph's centre on the area's centre at its start.
        layout.anchors = first_centres;
        let last = lines.last().unwrap();
        let arrival =
            rules::final_arrival_ms(last.0, last.1, ctx.clock.duration_ms.map(|d| d + offset));
        let knots = any_word_line.then(|| {
            let knot_lines: Vec<rules::KnotLine> = lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    // Only the line's start and, with its own word times, the end of its last word: the words
                    // themselves never shape the glide.
                    let end_ms = word_ranges[i]
                        .as_ref()
                        .and_then(|_| sources[i].words.last())
                        .map(|w| w.end_ms);
                    rules::KnotLine {
                        start_ms: l.0,
                        cell_start: layout.starts[i],
                        anchor: layout.anchors[i],
                        end_ms,
                        last_centre: last_centres[i],
                    }
                })
                .collect();
            rules::word_strip_knots(&knot_lines, Some(arrival))
        });
        let input = plan::StripInput {
            word_glide: knots.as_deref(),
            starts: &starts,
            layout: &layout,
            view_width: area.width,
            final_line: Some(rules::FinalLine {
                end_ms: Some(arrival),
                last_glyph_center: Some(last_centre),
            }),
            at_ms: position + offset,
            playing,
            rate: ctx.clock.slew(ctx.now).map_or(1.0, |s| s.0),
            slew_until_ms: ctx.clock.slew(ctx.now).map(|s| position + offset + s.1),
        };
        let planned = plan::plan_strip(&input);
        let rows_h = font as f64 * line_height
            + if has_translation {
                trans_font as f64 * line_height
            } else {
                0.0
            };
        let top = rows_top(
            area,
            floating,
            &look,
            scale,
            rows_h,
            font as f64 * line_height,
        );
        let mut cells = Vec::new();
        for (index, keys) in &planned.opacity {
            let (_, text, tr) = lines[*index];
            let rows: Vec<(&str, f32, u16)> = if has_translation {
                vec![(text, font, style.weight), (tr, trans_font, trans_weight)]
            } else {
                vec![(text, font, style.weight)]
            };
            cells.push((cached(gfx, win, &rows, style)?, *index, keys));
        }
        // The lines that enter next are rasterised ahead, so a line change never rasterises on its own frame.
        // Drawing composition surfaces makes the next commit wait about 30 ms (3–4 frames at 120 Hz) however many
        // are drawn in that frame, so they are drawn in large batches: right after the lyrics load (in this refresh,
        // the same frame as the drawn cells: one stall, before or as the strip starts) up to `PREWARM_LINES` lines;
        // later, when the next `PREWARM_NEEDED` lines are not cached, the next `PREWARM_LATER_LINES` together after a
        // frame's commit (`prewarm_batch`).
        win.prewarm.clear();
        if let Some((_, last)) = planned.cells {
            let rows_of = |text: &str, tr: &str| {
                let mut rows = vec![(text.to_string(), font, style.weight)];
                if has_translation {
                    rows.push((tr.to_string(), trans_font, trans_weight));
                }
                rows
            };
            let needed = lines
                .iter()
                .skip(last + 1)
                .take(PREWARM_NEEDED)
                .any(|(_, text, tr)| !is_cached(win, &rows_of(text, tr), style));
            if needed {
                let started = Instant::now();
                let mut drawn = 0;
                let batch = if fresh {
                    PREWARM_LINES
                } else {
                    PREWARM_LATER_LINES
                };
                for (_, text, tr) in lines.iter().skip(last + 1).take(batch) {
                    let rows = rows_of(text, tr);
                    if is_cached(win, &rows, style) {
                        continue;
                    }
                    if fresh {
                        let rows: Vec<(&str, f32, u16)> =
                            rows.iter().map(|(t, f, w)| (t.as_str(), *f, *w)).collect();
                        cached(gfx, win, &rows, style)?;
                        drawn += 1;
                    } else {
                        win.prewarm.push((rows, style));
                    }
                }
                if drawn > 0 {
                    frame_log_line(&format!(
                        "load batch {drawn} lines took {:.3} ms, window holds {} surfaces {:.1} MiB, all                          windows {:.1} MiB (shared counted once)",
                        started.elapsed().as_secs_f64() * 1000.0,
                        win.cache.len(),
                        cache_bytes(win) as f64 / 1048576.0,
                        shared_bytes() as f64 / 1048576.0
                    ));
                }
            }
        }
        let content = content_host(
            gfx,
            win,
            &root,
            area,
            w,
            h,
            opacity,
            font as f64 * clip_room,
            Mask::Centre {
                em: font as f64,
                main: mask_main,
                secondary: mask_secondary,
            },
        )?;
        // The strip glides (opacity is the static mask only): sampled per frame from the plan (`Live`), in 1/8 device px steps
        // and 1/255 opacity steps.
        let strip = gfx.compositor.CreateContainerVisual()?;
        let strip_visual: Visual = strip.cast()?;
        live(
            win,
            ctx,
            strip_visual,
            sampling::Kind::OffsetX,
            Curve::Cubic {
                keys: planned.offset.clone(),
                add: area.left,
            },
            0.0,
        )?;
        content.Children()?.InsertAtTop(&strip)?;
        for (raster, index, keys) in &cells {
            let cell: Visual = sprite(gfx, &raster.surface)?.cast()?;
            set_offset(
                &cell,
                layout.starts[*index] - raster.margin as f64,
                (top - raster.margin as f64).round(),
            )?;
            strip.Children()?.InsertAtTop(&cell)?;
            live(
                win,
                ctx,
                cell,
                sampling::Kind::Opacity,
                Curve::Linear {
                    keys: (*keys).clone(),
                    add: 0.0,
                    factor: 1.0,
                },
                0.0,
            )?;
        }
        win.wake_at = planned
            .wake_ms
            .map(|ms| ctx.now + Duration::from_secs_f64(ms.max(1.0) / 1000.0));
    } else {
        let starts: Vec<f64> = lyrics.lines.iter().map(|l| l.start_ms).collect();
        let planned = plan::plan_line(
            &starts,
            position,
            offset,
            ctx.clock.duration_ms,
            true,
            playing,
        );
        win.wake_at = planned
            .wake_ms
            .map(|ms| ctx.now + Duration::from_secs_f64(ms.max(1.0) / 1000.0));
        let Some(index) = planned.index else {
            return Ok(false);
        };
        let line = &lyrics.lines[index];
        let text = line.text.trim();
        let tr = if look.show_translation {
            line.translation.trim()
        } else {
            ""
        };
        has_translation = !tr.is_empty();
        if text.is_empty() {
            return Ok(has_translation);
        }
        let font_css = fit_font_size(
            look.font_size_px,
            area.height / scale,
            if has_translation {
                look.translation_scale_percent
            } else {
                0.0
            },
            line_height,
        );
        let font = (font_css * scale) as f32;
        let trans_font = font * (look.translation_scale_percent / 100.0) as f32;
        let mut rows = vec![(text, font, style.weight)];
        if has_translation {
            rows.push((tr, trans_font, trans_weight));
        }
        let rows_h: f64 = rows.iter().map(|r| r.1 as f64 * line_height).sum();
        let mut y = rows_top(
            area,
            floating,
            &look,
            scale,
            rows_h,
            font as f64 * line_height,
        );
        let mut placed = Vec::new();
        for row in rows {
            let raster = cached(gfx, win, &[row], style)?;
            let width = raster.row_widths[0] as f64;
            let overflow = (width - area.width).max(0.0).ceil();
            let base = if overflow > 0.0 {
                area.left
            } else {
                match look.align {
                    rules::Align::Left => area.left,
                    rules::Align::Center => area.left + (area.width - width) / 2.0,
                    rules::Align::Right => area.left + area.width - width,
                }
            };
            placed.push((raster, base, overflow, y));
            y += row.1 as f64 * line_height;
        }
        let content = content_host(
            gfx,
            win,
            &root,
            area,
            w,
            h,
            opacity,
            font as f64 * clip_room,
            Mask::Edge,
        )?;
        // Word-timed main row (ADR 0041): karaoke, sung words and the sung part of the current word in the theme
        // accent. The line is rasterised twice (white and accent, same layout and the same shadow / outline colour)
        // and shown as two sprites of the same position with complementary clips (`WordClip`), so the shadow has no
        // seam, kerning and CJK spacing are the line's own, and nothing is drawn per frame.
        // Translations keep the whole-line display.
        let spans = line.word_ranges().and_then(|ranges| {
            let layout = text_layout(gfx, text, font, style.weight, style.line_height).ok()?;
            Some(word_spans(&layout, &ranges))
        });
        let accent_style = TextStyle {
            color: ctx.accent,
            ..style
        };
        for (row, (raster, base, overflow, y)) in placed.iter().enumerate() {
            let words = spans.as_ref().filter(|_| row == 0);
            let accent = match words {
                Some(_) => Some(cached(
                    gfx,
                    win,
                    &[(text, font, style.weight)],
                    accent_style,
                )?),
                None => None,
            };
            // `[white]`, or `[accent (lit), white (dim)]` for a karaoke row.
            let mut visuals: Vec<Visual> = Vec::new();
            if let Some(accent) = &accent {
                visuals.push(sprite(gfx, &accent.surface)?.cast()?);
            }
            visuals.push(sprite(gfx, &raster.surface)?.cast()?);
            for visual in &visuals {
                content.Children()?.InsertAtTop(visual)?;
                live(
                    win,
                    ctx,
                    visual.clone(),
                    sampling::Kind::OffsetX,
                    Curve::Linear {
                        keys: planned.scroll.clone(),
                        add: base - raster.margin as f64,
                        factor: -overflow,
                    },
                    (y - raster.margin as f64).round(),
                )?;
            }
            if let (Some(spans), Some(_)) = (words, &accent) {
                let at = position + offset;
                let margin = raster.margin as f64;
                let times: Vec<(f64, f64)> = line
                    .words
                    .iter()
                    .map(|w| ((w.start_ms - at) / 1000.0, (w.end_ms - at) / 1000.0))
                    .collect();
                let clips = [
                    gfx.compositor.CreateInsetClip()?,
                    gfx.compositor.CreateInsetClip()?,
                ];
                for (visual, clip) in visuals.iter().zip(&clips) {
                    visual.SetClip(clip)?;
                }
                let spans: Vec<(f64, f64)> = spans
                    .iter()
                    .map(|&(l, r)| (l as f64 + margin, r as f64 + margin))
                    .collect();
                let mut word = WordClip {
                    clips,
                    glide: rules::karaoke_glide(&spans, &times),
                    times,
                    width: raster.surface.width as f64,
                    shown: f64::NAN,
                };
                word.sample(ctx.frame.saturating_duration_since(ctx.now).as_secs_f64())?;
                if playing {
                    if let Some(end) = word
                        .times
                        .iter()
                        .map(|t| t.1)
                        .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))))
                    {
                        if end > 0.0 {
                            let until = ctx.now + Duration::from_secs_f64(end);
                            win.moving_until =
                                Some(win.moving_until.map_or(until, |u| u.max(until)));
                        }
                    }
                }
                win.words.push(word);
            }
        }
        // The next line's rasters (and its accent copy) are drawn ahead, so a line change never rasterises.
        win.prewarm.clear();
        if let Some(next) = lyrics.lines.get(index + 1) {
            let next_text = next.text.trim();
            let next_tr = if look.show_translation {
                next.translation.trim()
            } else {
                ""
            };
            let next_font = (fit_font_size(
                look.font_size_px,
                area.height / scale,
                if next_tr.is_empty() {
                    0.0
                } else {
                    look.translation_scale_percent
                },
                line_height,
            ) * scale) as f32;
            if !next_text.is_empty() {
                win.prewarm.push((
                    vec![(next_text.to_string(), next_font, style.weight)],
                    style,
                ));
                if next.word_ranges().is_some() {
                    win.prewarm.push((
                        vec![(next_text.to_string(), next_font, style.weight)],
                        accent_style,
                    ));
                }
                if !next_tr.is_empty() {
                    let trans = next_font * (look.translation_scale_percent / 100.0) as f32;
                    win.prewarm
                        .push((vec![(next_tr.to_string(), trans, trans_weight)], style));
                }
            }
        }
    }
    Ok(has_translation)
}

/// Diagnostics: `MOE_LYRICS_NO_FADE=1` draws the lyrics without the edge fade (to measure what it costs).
fn edge_fade_enabled() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    !*OFF.get_or_init(|| std::env::var("MOE_LYRICS_NO_FADE").is_ok_and(|v| v == "1"))
}

/// The edge-fade host of one window: the lyric visuals go into `content`, which a `CompositionVisualSurface` renders
/// offscreen; `shown` paints that surface through a `CompositionMaskBrush` whose mask is a static horizontal
/// `CompositionLinearGradientBrush` (`sampling::edge_fade_stops`, the rule `edge_fade_opacity`). One copy of the
/// content per window, and the mask is a single pass in DWM.
struct FadeHost {
    key: [i64; 5],
    content: ContainerVisual,
    shown: SpriteVisual,
}

/// Which static mask a window's lyrics are seen through.
#[derive(Clone, Copy, PartialEq)]
enum Mask {
    /// Line-timed lyrics: the square edge fade (`sampling::edge_fade_stops`).
    Edge,
    /// Continuous mode, line- and word-timed (experimental, ADR 0041 D10): the static mask
    /// (`rules::centre_mask_stops`) at the main row's em and the main and secondary opacity (whole percents), the
    /// same for the translation row so both fade at the same places.
    Centre { em: f64, main: u32, secondary: u32 },
}

impl Mask {
    fn stops(self, area: Area) -> Vec<(f64, f64)> {
        match self {
            Mask::Edge => sampling::edge_fade_stops(area.width, area.height),
            Mask::Centre {
                em,
                main,
                secondary,
            } => rules::centre_mask_stops(
                area.width,
                em,
                f64::from(main) / 100.0,
                f64::from(secondary) / 100.0,
            ),
        }
    }
    fn key(self) -> i64 {
        match self {
            Mask::Edge => 0,
            Mask::Centre {
                em,
                main,
                secondary,
            } => {
                ((em * 64.0).round() as i64)
                    | i64::from(main) << 24
                    | i64::from(secondary) << 32
                    | 1 << 48
            }
        }
    }
}

fn build_fade_host(
    gfx: &Gfx,
    area: Area,
    w: f64,
    h: f64,
    mask: Mask,
) -> Result<(ContainerVisual, SpriteVisual)> {
    let c = &gfx.compositor;
    let size = Vector2 {
        X: w as f32,
        Y: h as f32,
    };
    let content = c.CreateContainerVisual()?;
    content.SetSize(size)?;
    let surface = c.CreateVisualSurface()?;
    surface.SetSourceVisual(&content)?;
    surface.SetSourceSize(size)?;
    let source = c.CreateSurfaceBrushWithSurface(&surface)?;
    source.SetStretch(CompositionStretch::None)?;
    source.SetHorizontalAlignmentRatio(0.0)?;
    source.SetVerticalAlignmentRatio(0.0)?;
    let gradient = c.CreateLinearGradientBrush()?;
    gradient.SetMappingMode(CompositionMappingMode::Absolute)?;
    gradient.SetStartPoint(Vector2 {
        X: area.left as f32,
        Y: 0.0,
    })?;
    gradient.SetEndPoint(Vector2 {
        X: (area.left + area.width) as f32,
        Y: 0.0,
    })?;
    // Clamp: transparent beyond the lyric area on both sides (as the zones end there).
    gradient.SetExtendMode(CompositionGradientExtendMode::Clamp)?;
    let stops = gradient.ColorStops()?;
    for (offset, alpha) in mask.stops(area) {
        let stop = c.CreateColorGradientStopWithOffsetAndColor(
            offset as f32,
            Color {
                A: (alpha * 255.0).round() as u8,
                R: 255,
                G: 255,
                B: 255,
            },
        )?;
        stops.Append(&stop)?;
    }
    let mask = c.CreateMaskBrush()?;
    mask.SetSource(&source)?;
    mask.SetMask(&gradient)?;
    let shown = c.CreateSpriteVisual()?;
    shown.SetSize(size)?;
    shown.SetBrush(&mask)?;
    Ok((content, shown))
}

/// The container the lyric visuals of this refresh go into, already in the window's tree with the window opacity.
/// With the edge fade it is the (cached) masked host; without it a plain container clipped to the area plus `clip`.
#[allow(clippy::too_many_arguments)]
fn content_host(
    gfx: &Gfx,
    win: &mut Win,
    root: &ContainerVisual,
    area: Area,
    w: f64,
    h: f64,
    opacity: f32,
    clip: f64,
    mask: Mask,
) -> Result<ContainerVisual> {
    if !edge_fade_enabled() {
        let content = gfx.compositor.CreateContainerVisual()?;
        content.SetSize(Vector2 {
            X: w as f32,
            Y: h as f32,
        })?;
        let left = (area.left - clip).max(0.0);
        let right = (w - (area.left + area.width + clip)).max(0.0);
        let inset =
            gfx.compositor
                .CreateInsetClipWithInsets(left as f32, 0.0, right as f32, 0.0)?;
        content.SetClip(&inset)?;
        content.SetOpacity(opacity)?;
        root.Children()?.InsertAtTop(&content)?;
        return Ok(content);
    }
    let key = [
        w.round() as i64,
        h.round() as i64,
        (area.left * 64.0).round() as i64,
        (area.width * 64.0).round() as i64 ^ ((area.height * 64.0).round() as i64) << 32,
        mask.key(),
    ];
    if win.fade.as_ref().is_none_or(|f| f.key != key) {
        let (content, shown) = build_fade_host(gfx, area, w, h, mask)?;
        win.fade = Some(FadeHost {
            key,
            content,
            shown,
        });
    }
    let host = win.fade.as_ref().unwrap();
    host.content.Children()?.RemoveAll()?;
    host.shown.SetOpacity(opacity)?;
    root.Children()?.InsertAtTop(&host.shown)?;
    Ok(host.content.clone())
}

fn raster_key<T: std::fmt::Debug>(rows: &[(T, f32, u16)], style: TextStyle) -> String {
    plan::raster_key(
        rows,
        style.color,
        style.effect_from,
        style.effect as u8,
        style.line_height,
    )
}

fn is_cached(win: &Win, rows: &[(String, f32, u16)], style: TextStyle) -> bool {
    let rows: Vec<(&str, f32, u16)> = rows.iter().map(|(t, f, w)| (t.as_str(), *f, *w)).collect();
    win.cache.contains_key(&raster_key(&rows, style))
}

/// Bytes of the window's cached line surfaces (BGRA, 4 bytes per pixel).
fn cache_bytes(win: &Win) -> u64 {
    win.cache
        .values()
        .map(|r| r.surface.width as u64 * r.surface.height as u64 * 4)
        .sum()
}

thread_local! {
    /// Line rasters shared by every window on the renderer thread (`shared_cache`): a key that matches another
    /// window's (same text, font px at its DPI, colour, effect, line height) reuses its surface.
    static SHARED: RefCell<crate::shared_cache::SharedCache<Raster>> =
        RefCell::new(crate::shared_cache::SharedCache::default());
}

/// Bytes of all line surfaces some window still holds, each counted once.
fn shared_bytes() -> u64 {
    SHARED.with(|s| {
        s.borrow()
            .live()
            .map(|r| r.surface.width as u64 * r.surface.height as u64 * 4)
            .sum()
    })
}

fn cached(
    gfx: &Gfx,
    win: &mut Win,
    rows: &[(&str, f32, u16)],
    style: TextStyle,
) -> Result<Rc<Raster>> {
    let key = raster_key(rows, style);
    if let Some(r) = win.cache.get(&key) {
        return Ok(r.clone());
    }
    if let Some(r) = SHARED.with(|s| s.borrow_mut().get(&key)) {
        win.cache.insert(key, r.clone());
        return Ok(r);
    }
    let raster = Rc::new(rasterise(gfx, &RasterRequest { rows, style })?);
    SHARED.with(|s| s.borrow_mut().insert(key.clone(), &raster));
    frame_log_line(&format!(
        "raster {}x{}",
        raster.surface.width, raster.surface.height
    ));
    win.cache.insert(key, raster.clone());
    Ok(raster)
}

/// The floating window's padlock (and its tooltip) and the move-mode highlight.
#[allow(clippy::too_many_arguments)]
fn chrome(
    gfx: &Gfx,
    win: &mut Win,
    root: &ContainerVisual,
    area: Area,
    w: f64,
    scale: f64,
    look: &Look,
) -> Result<()> {
    let Some(s) = shared(win.hwnd) else {
        return Ok(());
    };
    let locked = win.chrome.locked;
    let show = if locked {
        win.hover
    } else {
        s.mouse_inside.get()
    };
    // floating_lyrics::unlock_button_rect.
    let size = (floating_lock_css(look.font_size_px) * scale)
        .round()
        .max(1.0) as i32;
    let right = w as i32 - (floating_margin_css(look.font_size_px) * scale).round() as i32;
    let top = floating_lock_top_px(area.top as i32, area.height as i32, size);
    let rect = RECT {
        left: right - size,
        top,
        right,
        bottom: top + size,
    };
    s.lock_rect.set(if show { rect } else { RECT::default() });
    let tip = if locked {
        win.chrome.unlock_tip.clone()
    } else {
        win.chrome.lock_tip.clone()
    };
    update_tooltip(win, if show { rect } else { RECT::default() }, &tip);
    if !locked && show {
        let (aw, ah) = (area.width.round() as u32, area.height.round() as u32);
        let radius = (10.0 * scale) as f32;
        let key = (aw, ah, radius as u32);
        let surface = match win.area_cache.get(&key) {
            Some(s) => s.clone(),
            None => {
                let s = rasterise_area(gfx, aw, ah, radius)?;
                win.area_cache.insert(key, s.clone());
                s
            }
        };
        let v = sprite(gfx, &surface)?;
        set_offset(&v.cast()?, area.left.round(), area.top.round())?;
        root.Children()?.InsertAtBottom(&v)?;
    }
    if show {
        let over = s.over_lock.get();
        let key = (size as u32, locked, over);
        let surface = match win.lock_cache.get(&key) {
            Some(s) => s.clone(),
            None => {
                let s = rasterise_lock(gfx, size as u32, scale as f32, locked, over)?;
                win.lock_cache.insert(key, s.clone());
                s
            }
        };
        let v = sprite(gfx, &surface)?;
        set_offset(&v.cast()?, rect.left as f64, rect.top as f64)?;
        root.Children()?.InsertAtTop(&v)?;
    }
    Ok(())
}
