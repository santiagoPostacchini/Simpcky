//! Menú flotante propio, al estilo de los de Windows 11: esquinas
//! redondeadas, íconos, una fila de colores y una fila de botones de
//! formato. Reemplaza a los menús nativos en las notas, que no saben
//! mostrar ni muestras de color ni botones lado a lado (por eso el color
//! era una lista de nombres, y además estaba repetido en dos botones).
//!
//! No toma el foco (`WS_EX_NOACTIVATE`): la nota sigue activa y su texto
//! sigue seleccionado mientras se elige el formato, y una nota "asomada"
//! no vuelve al escritorio por abrir un menú. A cambio, el mouse se
//! captura (un clic afuera lo cierra, como un menú de verdad) y el
//! teclado le llega desde el bucle de mensajes (ver `handle_key`).

use std::cell::RefCell;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::{rgb, wide};

const CLASS: &str = "SimpckyFlyout";
// Medidas a 100 % (96 ppp): se escalan con `px` al ppp del monitor.
const PAD: i32 = 4;
const ITEM_H: i32 = 32;
const SEP_H: i32 = 9;
const SWATCH_H: i32 = 44;
const TOOL_H: i32 = 40;
const TOOL_W: i32 = 36;
const ICON_COL: i32 = 36;
const MIN_W: i32 = 220;
const TIMER_WATCH: usize = 1;

/// Un botón de la fila de formato.
pub struct Tool {
    pub id: u32,
    pub glyph: Glyph,
    pub active: bool,
}

pub enum Glyph {
    /// Una letra con el estilo que representa: B negrita, I cursiva, U
    /// subrayada, S tachada.
    Letter(char, usize),
    Icon(u16),
}

pub enum Entry {
    /// Los colores de nota: `base + i` es el comando del color `i`.
    Swatches { base: u32, current: u8 },
    /// Botones de formato: no cierran el menú (se puede poner negrita y
    /// cursiva de una), se prenden y apagan en el lugar.
    Tools(Vec<Tool>),
    Item { id: u32, icon: u16, label: String, shortcut: &'static str, checked: bool, enabled: bool, danger: bool },
    Separator,
}

impl Entry {
    pub fn item(id: u32, icon: u16, label: &str, shortcut: &'static str) -> Entry {
        Entry::Item { id, icon, label: label.to_string(), shortcut, checked: false, enabled: true, danger: false }
    }
    pub fn toggle(id: u32, icon: u16, label: &str, shortcut: &'static str, checked: bool) -> Entry {
        Entry::Item { id, icon, label: label.to_string(), shortcut, checked, enabled: true, danger: false }
    }
    pub fn enabled(mut self, on: bool) -> Entry {
        if let Entry::Item { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
    pub fn danger(mut self) -> Entry {
        if let Entry::Item { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
}

/// Dónde abrirlo.
pub enum Anchor {
    /// Esquina superior izquierda en ese punto (clic derecho).
    Point(i32, i32),
    /// Debajo de un botón (en coordenadas de pantalla), alineado a su
    /// borde derecho.
    Below(RECT),
}

/// Qué se toca con el mouse o el teclado: (índice de entrada, índice
/// dentro de la fila).
type Spot = Option<(usize, usize)>;

struct State {
    hwnd: HWND,
    /// Puntos por pulgada del monitor donde se abrió.
    dpi: i32,
    owner: HWND,
    entries: Vec<Entry>,
    width: i32,
    hot: Spot,
    pressed: bool,
    foreground: HWND,
    /// Ya se soltaron todos los botones del mouse desde que se abrió
    /// (el clic que lo abrió no cuenta como "clic afuera").
    armed: bool,
}

thread_local! {
    static OPEN: RefCell<Option<State>> = const { RefCell::new(None) };
    static FONTS: RefCell<Vec<(i32, i32, u8, &'static str, isize)>> = const { RefCell::new(Vec::new()) };
}

pub fn register_class(hinstance: HINSTANCE) {
    unsafe {
        let name = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: name.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

pub fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

/// Cierra el menú, si hay uno abierto.
pub fn close() {
    let hwnd = OPEN.with(|o| o.borrow_mut().take().map(|s| s.hwnd));
    if let Some(h) = hwnd {
        unsafe {
            KillTimer(h, TIMER_WATCH);
            if GetCapture() == h {
                ReleaseCapture();
            }
            DestroyWindow(h);
        }
    }
}

/// Abre el menú. Cada opción elegida le llega a `owner` como un
/// `WM_COMMAND` con su id, igual que con un menú nativo.
pub fn show(owner: HWND, entries: Vec<Entry>, anchor: Anchor) {
    close();
    let (ax, ay) = match &anchor {
        Anchor::Point(x, y) => (*x, *y),
        Anchor::Below(r) => (r.left, r.bottom),
    };
    let dpi = dpi_at(ax, ay);
    let width = measure_width(&entries, dpi);
    let height = px(dpi, PAD) * 2 + entries.iter().map(|e| entry_height(e, dpi)).sum::<i32>();
    let (x, y) = place(anchor, width, height);
    unsafe {
        let class = wide(CLASS);
        let hinstance = crate::app::app().lock().unwrap().hinstance;
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class.as_ptr(),
            null(),
            WS_POPUP,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            hinstance as HINSTANCE,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let pref: i32 = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const i32 as *const _, 4);
        let border: u32 = if crate::theme::is_dark() { rgb(0x3A, 0x3D, 0x44) } else { rgb(0xE2, 0xE4, 0xE8) };
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &border as *const u32 as *const _, 4);
        OPEN.with(|o| {
            *o.borrow_mut() =
                Some(State { hwnd, dpi, owner, entries, width, hot: None, pressed: false, foreground: GetForegroundWindow(), armed: false })
        });
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetCapture(hwnd);
        SetTimer(hwnd, TIMER_WATCH, 100, None);
    }
}

/// Teclas mientras el menú está abierto (llamado desde el bucle de
/// mensajes, antes de que la tecla le llegue al texto de la nota).
/// `true` si la usó.
pub fn handle_key(vk: u32) -> bool {
    if !is_open() {
        return false;
    }
    match vk as u16 {
        VK_ESCAPE => close(),
        VK_UP | VK_DOWN => {
            OPEN.with(|o| {
                let mut o = o.borrow_mut();
                let Some(s) = o.as_mut() else { return };
                let items: Vec<usize> = s
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| matches!(e, Entry::Item { enabled: true, .. }))
                    .map(|(i, _)| i)
                    .collect();
                if items.is_empty() {
                    return;
                }
                let cur = s.hot.and_then(|(i, _)| items.iter().position(|&x| x == i));
                let next = match (cur, vk as u16 == VK_DOWN) {
                    (None, true) => 0,
                    (None, false) => items.len() - 1,
                    (Some(c), true) => (c + 1) % items.len(),
                    (Some(c), false) => (c + items.len() - 1) % items.len(),
                };
                s.hot = Some((items[next], 0));
                unsafe { InvalidateRect(s.hwnd, null(), 0) };
            });
        }
        VK_RETURN | VK_SPACE => {
            let hot = OPEN.with(|o| o.borrow().as_ref().and_then(|s| s.hot));
            if let Some(spot) = hot {
                activate(spot);
            }
        }
        // Cualquier otra tecla: el menú se va y la tecla sigue su curso
        // (como con un menú nativo, escribir lo cierra).
        VK_SHIFT | VK_CONTROL | VK_MENU => return true,
        _ => {
            close();
            return false;
        }
    }
    true
}

// -----------------------------------------------------------------
// Medidas
// -----------------------------------------------------------------

/// `v` píxeles a 96 ppp, llevados a `dpi`.
pub fn px(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

/// Los puntos por pulgada del monitor que contiene (`x`, `y`).
pub fn dpi_at(x: i32, y: i32) -> i32 {
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) < 0 {
            return 96;
        }
        (dx as i32).max(96)
    }
}

fn entry_height(e: &Entry, dpi: i32) -> i32 {
    px(
        dpi,
        match e {
            Entry::Swatches { .. } => SWATCH_H,
            Entry::Tools(_) => TOOL_H,
            Entry::Item { .. } => ITEM_H,
            Entry::Separator => SEP_H,
        },
    )
}

fn measure_width(entries: &[Entry], dpi: i32) -> i32 {
    let p = |v| px(dpi, v);
    let mut w = p(MIN_W);
    unsafe {
        let dc = GetDC(null_mut());
        for e in entries {
            let need = match e {
                Entry::Item { label, shortcut, checked, .. } => {
                    let mut need = p(12 + ICON_COL) + text_width(dc, font(-p(14), 400, 0, "Segoe UI"), label) + p(32);
                    if !shortcut.is_empty() {
                        need += text_width(dc, font(-p(12), 400, 0, "Segoe UI"), shortcut) + p(12);
                    }
                    if *checked {
                        need += p(24);
                    }
                    need
                }
                Entry::Tools(tools) => p(12 + tools.len() as i32 * TOOL_W + 12),
                Entry::Swatches { .. } => p(12 + crate::theme::PALETTE_LEN as i32 * 32 + 12),
                Entry::Separator => 0,
            };
            w = w.max(need + p(PAD) * 2);
        }
        ReleaseDC(null_mut(), dc);
    }
    w
}

unsafe fn text_width(dc: HDC, f: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, f);
    let w = wide(s);
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(dc, w.as_ptr(), (w.len() - 1) as i32, &mut size);
    SelectObject(dc, old);
    size.cx
}

/// Que entre en el monitor: si no hay lugar abajo, se abre hacia
/// arriba; si no hay lugar a la derecha, hacia la izquierda.
fn place(anchor: Anchor, w: i32, h: i32) -> (i32, i32) {
    let (mut x, mut y, flip_y) = match anchor {
        Anchor::Point(x, y) => (x, y, y),
        Anchor::Below(r) => (r.right - w, r.bottom + 2, r.top - 2),
    };
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            let r = info.rcWork;
            if y + h > r.bottom {
                y = (flip_y - h).max(r.top);
            }
            if x + w > r.right {
                x = r.right - w;
            }
            x = x.max(r.left);
        }
    }
    (x, y)
}

/// Qué hay en el punto (`x`, `y`) del menú.
fn hit(s: &State, x: i32, y: i32) -> Spot {
    let p = |v| px(s.dpi, v);
    if x < p(PAD) || x >= s.width - p(PAD) {
        return None;
    }
    let mut top = p(PAD);
    let left = p(PAD + 12);
    for (i, e) in s.entries.iter().enumerate() {
        let h = entry_height(e, s.dpi);
        if y >= top && y < top + h {
            return match e {
                Entry::Item { enabled: true, .. } => Some((i, 0)),
                Entry::Swatches { .. } => {
                    let k = (x - left) / p(32);
                    (x >= left && (k as usize) < crate::theme::PALETTE_LEN).then_some((i, k as usize))
                }
                Entry::Tools(tools) => {
                    let k = (x - left) / p(TOOL_W);
                    (x >= left && (k as usize) < tools.len()).then_some((i, k as usize))
                }
                _ => None,
            };
        }
        top += h;
    }
    None
}

/// Usar lo que hay en `spot`: manda el comando al dueño y (salvo en la
/// fila de formato) cierra el menú.
fn activate(spot: (usize, usize)) {
    let action = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let s = o.as_mut()?;
        match s.entries.get_mut(spot.0)? {
            Entry::Item { id, enabled: true, .. } => Some((s.owner, *id, true)),
            Entry::Swatches { base, .. } => Some((s.owner, *base + spot.1 as u32, true)),
            Entry::Tools(tools) => {
                let t = tools.get_mut(spot.1)?;
                t.active = !t.active;
                unsafe { InvalidateRect(s.hwnd, null(), 0) };
                Some((s.owner, t.id, false))
            }
            _ => None,
        }
    });
    let Some((owner, id, closes)) = action else { return };
    if closes {
        close();
    }
    unsafe { SendMessageW(owner, WM_COMMAND, id as usize, 0) };
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

/// Fuente cacheada: (alto, peso, estilo: 1 cursiva, 2 subrayada, 4
/// tachada, cara).
pub fn font(height: i32, weight: i32, style: u8, face: &'static str) -> HFONT {
    FONTS.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(&(.., h)) = f.iter().find(|(ht, wt, st, fc, _)| *ht == height && *wt == weight && *st == style && *fc == face) {
            return h as HFONT;
        }
        let name = wide(face);
        let h = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight,
                (style & 1) as u32,
                ((style >> 1) & 1) as u32,
                ((style >> 2) & 1) as u32,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                name.as_ptr(),
            )
        };
        f.push((height, weight, style, face, h as isize));
        h
    })
}

/// La fuente de íconos del sistema: "Segoe Fluent Icons" en Windows 11,
/// "Segoe MDL2 Assets" (mismos códigos) en Windows 10.
pub fn icon_font(height: i32) -> HFONT {
    static FACE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    let face = *FACE.get_or_init(|| unsafe {
        let dc = GetDC(null_mut());
        let f = font(-16, 400, 0, "Segoe Fluent Icons");
        let old = SelectObject(dc, f);
        let mut buf = [0u16; 64];
        let n = GetTextFaceW(dc, 64, buf.as_mut_ptr());
        SelectObject(dc, old);
        ReleaseDC(null_mut(), dc);
        let got = String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize]);
        if got.eq_ignore_ascii_case("Segoe Fluent Icons") {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        }
    });
    font(height, 400, 0, face)
}

pub unsafe fn draw_glyph(dc: HDC, f: HFONT, code: u16, rc: &RECT, color: u32) {
    let old = SelectObject(dc, f);
    SetTextColor(dc, color);
    let mut r = *rc;
    DrawTextW(dc, [code, 0].as_ptr(), 1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
}

pub unsafe fn fill_round(g: *mut GpGraphics, r: &RECT, radius: f32, color: u32) {
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    let (x, y, w, h) = (r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32);
    let d = radius * 2.0;
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(crate::note::colorref_to_argb(color), &mut brush);
    GdipFillPath(g, brush as *mut GpBrush, path);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeletePath(path);
}

fn danger_color() -> u32 {
    if crate::theme::is_dark() {
        rgb(0xF8, 0x71, 0x71)
    } else {
        rgb(0xC4, 0x2B, 0x1C)
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetClientRect(hwnd, &mut rc);
    let mem = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
    let old_bmp = SelectObject(mem, bmp);

    let chrome = crate::theme::chrome();
    let bg = CreateSolidBrush(chrome.window);
    FillRect(mem, &rc, bg);
    DeleteObject(bg);
    SetBkMode(mem, TRANSPARENT as i32);

    let mut g: *mut GpGraphics = null_mut();
    GdipCreateFromHDC(mem, &mut g);
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);

    OPEN.with(|o| {
        let o = o.borrow();
        let Some(s) = o.as_ref() else { return };
        let p = |v| px(s.dpi, v);
        let mut top = p(PAD);
        for (i, e) in s.entries.iter().enumerate() {
            let h = entry_height(e, s.dpi);
            let row = RECT { left: p(PAD), top, right: s.width - p(PAD), bottom: top + h };
            match e {
                Entry::Separator => {
                    let line = CreateSolidBrush(if crate::theme::is_dark() { rgb(0x3A, 0x3D, 0x44) } else { rgb(0xE5, 0xE7, 0xEB) });
                    let r = RECT { left: row.left + p(4), top: top + h / 2, right: row.right - p(4), bottom: top + h / 2 + p(1).max(1) };
                    FillRect(mem, &r, line);
                    DeleteObject(line);
                }
                Entry::Item { icon, label, shortcut, checked, enabled, danger, .. } => {
                    if s.hot == Some((i, 0)) && *enabled {
                        fill_round(g, &RECT { left: row.left, top: row.top + 1, right: row.right, bottom: row.bottom - 1 }, p(4) as f32, chrome.surface_hover);
                    }
                    let color = if !*enabled {
                        chrome.muted
                    } else if *danger {
                        danger_color()
                    } else {
                        chrome.text
                    };
                    let icon_rc = RECT { left: row.left + p(8), top: row.top, right: row.left + p(8 + 24), bottom: row.bottom };
                    draw_glyph(mem, icon_font(-p(16)), *icon, &icon_rc, color);
                    let mut right = row.right - p(12);
                    if *checked {
                        let check = RECT { left: right - p(18), top: row.top, right, bottom: row.bottom };
                        draw_glyph(mem, icon_font(-p(14)), 0xE73E, &check, chrome.text);
                        right -= p(24);
                    }
                    if !shortcut.is_empty() {
                        let old = SelectObject(mem, font(-p(12), 400, 0, "Segoe UI"));
                        SetTextColor(mem, chrome.muted);
                        let t = wide(shortcut);
                        let mut r = RECT { left: row.left, top: row.top, right, bottom: row.bottom };
                        DrawTextW(mem, t.as_ptr(), -1, &mut r, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                        SelectObject(mem, old);
                    }
                    let old = SelectObject(mem, font(-p(14), 400, 0, "Segoe UI"));
                    SetTextColor(mem, color);
                    let t = wide(label);
                    let mut r = RECT { left: row.left + p(8 + ICON_COL), top: row.top, right, bottom: row.bottom };
                    DrawTextW(mem, t.as_ptr(), -1, &mut r, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
                    SelectObject(mem, old);
                }
                Entry::Swatches { current, .. } => {
                    for k in 0..crate::theme::PALETTE_LEN {
                        let cx = row.left + p(12 + k as i32 * 32 + 16);
                        let cy = row.top + h / 2;
                        let (header, _, ink) = crate::theme::note_colors(k as u8);
                        let (halo, dot) = (p(15), p(11));
                        if s.hot == Some((i, k)) {
                            fill_round(g, &RECT { left: cx - halo, top: cy - halo, right: cx + halo, bottom: cy + halo }, halo as f32, chrome.surface_hover);
                        }
                        let mut brush: *mut GpSolidFill = null_mut();
                        GdipCreateSolidFill(crate::note::colorref_to_argb(header), &mut brush);
                        GdipFillEllipseI(g, brush as *mut GpBrush, cx - dot, cy - dot, dot * 2, dot * 2);
                        GdipDeleteBrush(brush as *mut GpBrush);
                        // Un borde apenas marcado: el gris y el amarillo
                        // claros se perderían contra el fondo blanco.
                        let mut pen: *mut GpPen = null_mut();
                        GdipCreatePen1(0x22000000 | (crate::note::colorref_to_argb(chrome.text) & 0x00ff_ffff), 1.0, UnitPixel, &mut pen);
                        GdipDrawEllipseI(g, pen, cx - dot, cy - dot, dot * 2, dot * 2);
                        GdipDeletePen(pen);
                        if *current as usize == k {
                            let r = RECT { left: cx - dot, top: cy - dot, right: cx + dot, bottom: cy + dot };
                            draw_glyph(mem, icon_font(-p(12)), 0xE73E, &r, ink);
                        }
                    }
                }
                Entry::Tools(tools) => {
                    for (k, t) in tools.iter().enumerate() {
                        let x = row.left + p(12 + k as i32 * TOOL_W);
                        let r = RECT { left: x + p(2), top: row.top + p(4), right: x + p(TOOL_W - 2), bottom: row.bottom - p(4) };
                        if t.active {
                            fill_round(g, &r, p(4) as f32, chrome.surface_hover);
                        } else if s.hot == Some((i, k)) {
                            fill_round(g, &r, p(4) as f32, chrome.surface);
                        }
                        match t.glyph {
                            Glyph::Icon(code) => draw_glyph(mem, icon_font(-p(16)), code, &r, chrome.text),
                            Glyph::Letter(c, style) => {
                                let (weight, bits, face) = match style {
                                    crate::richtext::BOLD => (700, 0, "Segoe UI"),
                                    crate::richtext::ITALIC => (400, 1, "Georgia"),
                                    crate::richtext::UNDERLINE => (400, 2, "Segoe UI"),
                                    _ => (400, 4, "Segoe UI"),
                                };
                                let old = SelectObject(mem, font(-p(17), weight, bits, face));
                                SetTextColor(mem, chrome.text);
                                let mut rr = r;
                                let txt = [c as u16, 0];
                                DrawTextW(mem, txt.as_ptr(), 1, &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                                SelectObject(mem, old);
                            }
                        }
                    }
                }
            }
            top += h;
        }
    });

    GdipDeleteGraphics(g);
    BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    DeleteObject(bmp);
    DeleteDC(mem);
    EndPaint(hwnd, &ps);
}

// -----------------------------------------------------------------
// Mensajes
// -----------------------------------------------------------------

fn point(lparam: LPARAM) -> (i32, i32) {
    ((lparam & 0xffff) as i16 as i32, ((lparam >> 16) & 0xffff) as i16 as i32)
}

fn inside(hwnd: HWND, x: i32, y: i32) -> bool {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    x >= 0 && y >= 0 && x < rc.right && y < rc.bottom
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            OPEN.with(|o| {
                let mut o = o.borrow_mut();
                if let Some(s) = o.as_mut().filter(|s| s.hwnd == hwnd) {
                    let hot = hit(s, x, y);
                    if hot != s.hot {
                        s.hot = hot;
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            });
            0
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            let (x, y) = point(lparam);
            if !inside(hwnd, x, y) {
                // Clic afuera: se cierra y el clic se pierde, como en
                // cualquier menú.
                close();
            } else {
                OPEN.with(|o| {
                    if let Some(s) = o.borrow_mut().as_mut() {
                        s.pressed = true;
                    }
                });
            }
            0
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let (x, y) = point(lparam);
            // Solo cuenta si el clic también empezó adentro: el que abrió
            // el menú (en el botón "⋯") se suelta afuera.
            let spot = OPEN.with(|o| {
                let mut o = o.borrow_mut();
                let s = o.as_mut()?;
                let was = std::mem::replace(&mut s.pressed, false);
                if was && inside(hwnd, x, y) {
                    hit(s, x, y)
                } else {
                    None
                }
            });
            if let Some(spot) = spot {
                activate(spot);
            }
            0
        }
        WM_CAPTURECHANGED => {
            if lparam as HWND != hwnd {
                close();
            }
            0
        }
        WM_TIMER if wparam == TIMER_WATCH => {
            // Otra app pasó al frente (Alt+Tab), o un clic afuera que la
            // captura no alcanzó a ver (en otra aplicación): el menú no
            // queda colgado.
            let any_down = GetAsyncKeyState(VK_LBUTTON as i32) < 0 || GetAsyncKeyState(VK_RBUTTON as i32) < 0;
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut wr);
            let outside = pt.x < wr.left || pt.x >= wr.right || pt.y < wr.top || pt.y >= wr.bottom;
            let gone = OPEN.with(|o| {
                let mut o = o.borrow_mut();
                let Some(s) = o.as_mut() else { return false };
                if !any_down {
                    s.armed = true;
                }
                GetForegroundWindow() != s.foreground || (s.armed && any_down && outside)
            });
            if gone {
                close();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
