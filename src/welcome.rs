//! Pantalla de bienvenida: se abre la primera vez que Simpcky arranca en
//! una compu (todavía no hay `settings.json`). Reúne lo que conviene
//! decidir de entrada — iniciar con Windows, el clic derecho del
//! escritorio, el tema — y ofrece la sincronización con Google, que es
//! opcional. Todo se puede cambiar después desde el menú de la bandeja.
//!
//! Dibujada a mano como "Todas las notas" (GDI+ con doble buffer), con
//! los colores del tema actual.

use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::app;
use crate::theme;
use crate::win::wide;

const CLASS_NAME: &str = "SimpckyWelcome";
const W: i32 = 460;
const H: i32 = 604;
const PAD: i32 = 28;
const ROW_H: i32 = 44;
const ROWS_TOP: i32 = 186;
const ROWS: i32 = 4;
const CARD_TOP: i32 = ROWS_TOP + ROWS * ROW_H + 22;
const CARD_H: i32 = 118;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Autostart,
    DesktopMenu,
    Dark,
    AutoUpdate,
    Connect,
    Privacy,
    Start,
    None,
}

struct State {
    hwnd: isize,
    hover: Hit,
    fonts: [isize; 4], // título, texto, etiqueta, chico
}

static STATE: Mutex<State> = Mutex::new(State { hwnd: 0, hover: Hit::None, fonts: [0; 4] });

pub fn register_class(hinstance: HINSTANCE) {
    unsafe {
        let class_name = wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: crate::icon::app_icon_large(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: crate::icon::app_icon(),
        };
        RegisterClassExW(&wc);
    }
}

/// Abre la bienvenida (o la trae al frente si ya está abierta).
pub fn show() {
    let existing = STATE.lock().unwrap().hwnd;
    if existing != 0 {
        crate::win::show_normal(existing as HWND);
        return;
    }
    let hinstance = { app().lock().unwrap().hinstance } as HINSTANCE;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    unsafe {
        // Tamaño de ventana para que el área de cliente mida W×H.
        let mut rc = RECT { left: 0, top: 0, right: W, bottom: H };
        AdjustWindowRectEx(&mut rc, style, 0, WS_EX_APPWINDOW);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        // Centrada en el área de trabajo del monitor principal.
        let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut _, 0);
        let x = work.left + ((work.right - work.left) - ww) / 2;
        let y = work.top + ((work.bottom - work.top) - wh) / 2;

        let class_name = wide(CLASS_NAME);
        let title = wide("Bienvenido a Simpcky");
        let hwnd = CreateWindowExW(WS_EX_APPWINDOW, class_name.as_ptr(), title.as_ptr(), style, x, y, ww, wh, null_mut(), null_mut(), hinstance, null());
        if hwnd.is_null() {
            return;
        }
        STATE.lock().unwrap().hwnd = hwnd as isize;
        theme::apply_title_bar(hwnd);
        crate::win::show_normal(hwnd);
    }
}

/// Algo cambió afuera (el tema, la cuenta de Google): repintar.
pub fn refresh() {
    let hwnd = STATE.lock().unwrap().hwnd as HWND;
    if !hwnd.is_null() {
        unsafe {
            theme::apply_title_bar(hwnd);
            SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

// -----------------------------------------------------------------
// Layout
// -----------------------------------------------------------------

fn row_rect(i: i32) -> RECT {
    RECT { left: PAD, top: ROWS_TOP + i * ROW_H, right: W - PAD, bottom: ROWS_TOP + (i + 1) * ROW_H }
}

fn card_rect() -> RECT {
    RECT { left: PAD, top: CARD_TOP, right: W - PAD, bottom: CARD_TOP + CARD_H }
}

fn connect_rect() -> RECT {
    let c = card_rect();
    RECT { left: c.left + 16, top: c.bottom - 16 - 32, right: c.left + 16 + 210, bottom: c.bottom - 16 }
}

fn start_rect() -> RECT {
    RECT { left: W - PAD - 132, top: H - PAD - 38, right: W - PAD, bottom: H - PAD }
}

fn privacy_rect() -> RECT {
    RECT { left: PAD, top: H - PAD - 30, right: PAD + 170, bottom: H - PAD - 8 }
}

fn in_rect(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Qué muestra la tarjeta de sincronización.
enum SyncView {
    Hidden,
    Offer,
    Waiting,
    Connected(String),
}

fn sync_view() -> SyncView {
    if !crate::sync::is_configured() {
        SyncView::Hidden
    } else if crate::sync::is_connected() {
        SyncView::Connected(crate::sync::status().0)
    } else if crate::sync::is_signing_in() {
        SyncView::Waiting
    } else {
        SyncView::Offer
    }
}

fn hit_test(x: i32, y: i32) -> Hit {
    for (i, hit) in [Hit::Autostart, Hit::DesktopMenu, Hit::Dark, Hit::AutoUpdate].into_iter().enumerate() {
        if in_rect(&row_rect(i as i32), x, y) {
            return hit;
        }
    }
    if matches!(sync_view(), SyncView::Offer) && in_rect(&connect_rect(), x, y) {
        return Hit::Connect;
    }
    if in_rect(&start_rect(), x, y) {
        return Hit::Start;
    }
    if in_rect(&privacy_rect(), x, y) {
        return Hit::Privacy;
    }
    Hit::None
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

fn argb(a: u8, c: u32) -> u32 {
    (crate::note::colorref_to_argb(c) & 0x00ff_ffff) | ((a as u32) << 24)
}

unsafe fn round_rect(g: *mut GpGraphics, r: &RECT, radius: i32, color: u32) {
    let (x, y, w, h) = (r.left, r.top, r.right - r.left, r.bottom - r.top);
    let d = (radius * 2).min(w).min(h);
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    GdipAddPathArcI(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArcI(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArcI(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArcI(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    GdipFillPath(g, brush as *mut GpBrush, path);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeletePath(path);
}

unsafe fn text(hdc: HDC, font: isize, color: u32, s: &str, mut rc: RECT, flags: u32) {
    let old = SelectObject(hdc, font as HGDIOBJ);
    SetTextColor(hdc, color);
    let w = wide(s);
    DrawTextW(hdc, w.as_ptr(), -1, &mut rc, flags);
    SelectObject(hdc, old);
}

/// Ámbar del ícono: el color de "prendido" en los dos temas (con la
/// perilla blanca encima se distingue bien de "apagado").
const ON_COLOR: u32 = crate::win::rgb(0xF5, 0x9E, 0x0B);

/// Un interruptor (como el del diseño, en el menú de la nota).
unsafe fn switch(g: *mut GpGraphics, right: i32, cy: i32, on: bool, off_color: u32) {
    let track = RECT { left: right - 40, top: cy - 11, right, bottom: cy + 11 };
    round_rect(g, &track, 11, argb(0xff, if on { ON_COLOR } else { off_color }));
    let kx = if on { track.right - 20 } else { track.left + 2 };
    // Un borde apenas más oscuro, para que la perilla se vea también
    // sobre una pista clara.
    round_rect(g, &RECT { left: kx - 1, top: cy - 10, right: kx + 19, bottom: cy + 10 }, 10, 0x30000000);
    round_rect(g, &RECT { left: kx, top: cy - 9, right: kx + 18, bottom: cy + 9 }, 9, 0xFFFF_FFFF);
}

fn fonts() -> [isize; 4] {
    let mut s = STATE.lock().unwrap();
    if s.fonts[0] == 0 {
        let face = wide("Segoe UI");
        let make = |h: i32, weight: i32| unsafe {
            CreateFontW(
                h,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH as u32) | (FF_DONTCARE as u32),
                face.as_ptr(),
            ) as isize
        };
        s.fonts = [make(-24, FW_SEMIBOLD as i32), make(-15, FW_NORMAL as i32), make(-15, FW_NORMAL as i32), make(-13, FW_NORMAL as i32)];
    }
    s.fonts
}

fn on_paint(hwnd: HWND) {
    let [f_title, f_text, f_label, f_small] = fonts();
    let hover = STATE.lock().unwrap().hover;
    let c = theme::chrome();
    let dark = theme::is_dark();
    let (yellow_header, _, yellow_ink) = theme::note_colors(0);
    let autostart = crate::shell::is_autostart_enabled();
    let desktop_menu = app().lock().unwrap().settings.desktop_menu;
    let view = sync_view();

    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, W, H);
        let old_bmp = SelectObject(mem, bmp as HGDIOBJ);
        let bg = CreateSolidBrush(c.window);
        FillRect(mem, &RECT { left: 0, top: 0, right: W, bottom: H }, bg);
        DeleteObject(bg as HGDIOBJ);
        SetBkMode(mem, TRANSPARENT as i32);

        // Encabezado: ícono + título + explicación.
        DrawIconEx(mem, PAD, PAD, crate::icon::app_icon_large(), 48, 48, 0, null_mut(), DI_NORMAL);
        text(mem, f_title, c.text, "¡Bienvenido a Simpcky!", RECT { left: PAD + 64, top: PAD + 6, right: W - PAD, bottom: PAD + 44 }, DT_SINGLELINE | DT_LEFT);
        text(
            mem,
            f_text,
            c.muted,
            "Tus notas viven en el escritorio. Creá una con un clic en el ícono de la bandeja, con Ctrl+N, o con clic derecho en el escritorio.",
            RECT { left: PAD, top: PAD + 66, right: W - PAD, bottom: ROWS_TOP - 30 },
            DT_WORDBREAK | DT_LEFT,
        );
        text(mem, f_small, c.muted, "PREFERENCIAS", RECT { left: PAD, top: ROWS_TOP - 24, right: W - PAD, bottom: ROWS_TOP }, DT_SINGLELINE | DT_LEFT);

        let mut g: *mut GpGraphics = null_mut();
        GdipCreateFromHDC(mem, &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);

        let rows = [
            (Hit::Autostart, "Iniciar Simpcky con Windows", autostart),
            (Hit::DesktopMenu, "\"Nueva nota\" en el clic derecho del escritorio", desktop_menu),
            (Hit::Dark, "Modo oscuro", dark),
            (Hit::AutoUpdate, "Buscar actualizaciones automáticamente", crate::update::is_auto()),
        ];
        for (i, (hit, label, on)) in rows.iter().enumerate() {
            let r = row_rect(i as i32);
            if hover == *hit {
                round_rect(g, &RECT { left: r.left - 10, top: r.top + 2, right: r.right + 10, bottom: r.bottom - 2 }, 8, argb(0xff, c.surface));
            }
            text(mem, f_label, c.text, label, RECT { left: r.left, top: r.top, right: r.right - 56, bottom: r.bottom }, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS);
            switch(g, r.right, (r.top + r.bottom) / 2, *on, c.surface_hover);
        }

        // Tarjeta de sincronización (opcional).
        if !matches!(view, SyncView::Hidden) {
            let card = card_rect();
            round_rect(g, &card, 12, argb(0xff, c.surface));
            let inner = RECT { left: card.left + 16, top: card.top + 14, right: card.right - 16, bottom: card.bottom };
            text(mem, f_label, c.text, "Tus notas en todas tus compus", RECT { bottom: inner.top + 22, ..inner }, DT_SINGLELINE | DT_LEFT);
            let blurb = match &view {
                SyncView::Connected(email) => format!("Sincronizando con Google Drive ({email})."),
                SyncView::Waiting => "Terminá de iniciar sesión en el navegador…".to_string(),
                _ => "Opcional: con tu cuenta de Google, en una carpeta oculta de tu Drive que solo ve Simpcky.".to_string(),
            };
            text(mem, f_small, c.muted, &blurb, RECT { top: inner.top + 26, bottom: inner.top + 62, ..inner }, DT_WORDBREAK | DT_LEFT);
            if matches!(view, SyncView::Offer) {
                let b = connect_rect();
                round_rect(g, &b, 8, argb(0xff, if hover == Hit::Connect { c.surface_hover } else { c.window }));
                text(mem, f_small, c.text, "Conectar con Google…", b, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            }
        }

        // Pie: política de privacidad y "Empezar".
        let s = start_rect();
        round_rect(g, &s, 10, argb(0xff, yellow_header));
        if hover == Hit::Start {
            round_rect(g, &s, 10, argb(0x22, if dark { 0xFFFFFF } else { 0x000000 }));
        }
        GdipDeleteGraphics(g);
        text(mem, f_label, yellow_ink, "Empezar", s, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
        let p = privacy_rect();
        text(mem, f_small, c.muted, "Política de privacidad", p, DT_SINGLELINE | DT_VCENTER | DT_LEFT);
        if hover == Hit::Privacy {
            let pen = CreatePen(PS_SOLID, 1, c.muted);
            let old = SelectObject(mem, pen as HGDIOBJ);
            MoveToEx(mem, p.left, p.bottom - 3, null_mut());
            LineTo(mem, p.left + 142, p.bottom - 3);
            SelectObject(mem, old);
            DeleteObject(pen as HGDIOBJ);
        }

        BitBlt(hdc, 0, 0, W, H, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp as HGDIOBJ);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

// -----------------------------------------------------------------
// Interacción
// -----------------------------------------------------------------

fn open_privacy() {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let url = wide(concat!(env!("CARGO_PKG_HOMEPAGE"), "privacidad.html"));
    let verb = wide("open");
    unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

fn on_click(hwnd: HWND, hit: Hit) {
    match hit {
        Hit::Autostart => crate::shell::set_autostart(!crate::shell::is_autostart_enabled()),
        Hit::DesktopMenu => crate::tray::toggle_desktop_menu(),
        Hit::Dark => crate::theme::set_dark(!crate::theme::is_dark()),
        Hit::AutoUpdate => crate::update::toggle_auto(),
        Hit::Connect => crate::sync::begin_sign_in(),
        Hit::Privacy => open_privacy(),
        Hit::Start => unsafe {
            DestroyWindow(hwnd);
            return;
        },
        Hit::None => return,
    }
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let hit = hit_test(x, y);
            let changed = {
                let mut s = STATE.lock().unwrap();
                std::mem::replace(&mut s.hover, hit) != hit
            };
            if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            0
        }
        0x02A3 /* WM_MOUSELEAVE */ => {
            STATE.lock().unwrap().hover = Hit::None;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SETCURSOR => {
            let hover = STATE.lock().unwrap().hover;
            if hover != Hit::None && (lparam & 0xffff) as u32 == HTCLIENT {
                SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                1
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_LBUTTONUP => {
            on_click(hwnd, hit_test(x, y));
            0
        }
        WM_KEYDOWN if wparam as u16 == VK_RETURN || wparam as u16 == VK_ESCAPE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            let mut s = STATE.lock().unwrap();
            s.hwnd = 0;
            s.hover = Hit::None;
            for f in s.fonts.iter_mut() {
                if *f != 0 {
                    DeleteObject(*f as HGDIOBJ);
                    *f = 0;
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
