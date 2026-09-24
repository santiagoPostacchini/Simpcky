//! Selector de notas: con un atajo global (Ctrl+Alt+N, o el que se
//! elija en Configuración) aparece la lista de notas, la última usada
//! primero. Se escribe para filtrar, flechas y Enter (o clic), y la
//! elegida viene al frente **asomada** (ver `note::open_note`): sin
//! "siempre encima", y al hacer clic en otra cosa vuelve sola a su lugar
//! en el escritorio. También trae las ocultas.
//!
//! Una ventana emergente propia, sin controles nativos (el texto de la
//! búsqueda lo lleva ella misma), dibujada con los colores del tema.

use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::app;
use crate::flyout;
use crate::theme;
use crate::win::wide;

const CLASS_NAME: &str = "SimpckyPicker";
/// Medidas a 100 %.
const W: i32 = 460;
const SEARCH_H: i32 = 50;
const ROW_H: i32 = 50;
const PAD: i32 = 8;
const MAX_ROWS: usize = 7;

struct Item {
    id: u32,
    color: u8,
    title: String,
    preview: String,
    pinned: bool,
    hidden: bool,
}

struct State {
    hwnd: isize,
    query: String,
    items: Vec<Item>,
    sel: usize,
    /// Primera fila a la vista.
    top: usize,
    dpi: i32,
}

static STATE: Mutex<State> = Mutex::new(State { hwnd: 0, query: String::new(), items: Vec::new(), sel: 0, top: 0, dpi: 96 });

pub fn register_class(hinstance: HINSTANCE) {
    unsafe {
        let class_name = wide(CLASS_NAME);
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
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

/// El atajo: abre el selector, o lo cierra si ya estaba abierto.
pub fn toggle() {
    let open = STATE.lock().unwrap().hwnd;
    if open != 0 {
        unsafe { DestroyWindow(open as HWND) };
    } else {
        show();
    }
}

fn px(v: i32) -> i32 {
    v * STATE.lock().unwrap().dpi / 96
}

/// Las notas que coinciden con la búsqueda, la última usada primero.
fn collect(query: &str) -> Vec<Item> {
    let q = query.to_lowercase();
    let a = app().lock().unwrap();
    let mut list: Vec<(u64, u32, Item)> = a
        .notes
        .values()
        .map(|nr| {
            let d = &nr.data;
            let item = Item {
                id: d.id,
                color: d.color,
                title: crate::note::display_title(d),
                preview: crate::allnotes::preview_of(d),
                pinned: d.layer == crate::persist::Layer::AlwaysOnTop,
                hidden: d.hidden,
            };
            (nr.last_active, d.id, item)
        })
        .filter(|(_, _, it)| q.is_empty() || it.title.to_lowercase().contains(&q) || it.preview.to_lowercase().contains(&q))
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    list.into_iter().map(|(_, _, it)| it).collect()
}

fn size(rows: usize) -> (i32, i32) {
    (px(W), px(SEARCH_H) + rows.clamp(1, MAX_ROWS) as i32 * px(ROW_H) + px(PAD) * 2)
}

fn show() {
    let hinstance = { app().lock().unwrap().hinstance } as HINSTANCE;
    unsafe {
        // En el monitor donde está el mouse, centrado y un poco arriba
        // (como la búsqueda de Windows).
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        {
            let mut s = STATE.lock().unwrap();
            s.dpi = flyout::dpi_at(pt.x, pt.y);
            s.query.clear();
            s.sel = 0;
            s.top = 0;
        }
        let items = collect("");
        let (w, h) = size(items.len());
        STATE.lock().unwrap().items = items;
        let work = info.rcWork;
        let x = work.left + ((work.right - work.left) - w) / 2;
        let y = work.top + (work.bottom - work.top) / 5;
        let class_name = wide(CLASS_NAME);
        let title = wide("Traer una nota");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        STATE.lock().unwrap().hwnd = hwnd as isize;
        let pref: i32 = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const i32 as *const _, 4);
        ShowWindow(hwnd, SW_SHOW);
        // El atajo global le da permiso a la app para pasar adelante.
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
}

/// Cambió la búsqueda: se vuelve a filtrar y la ventana se ajusta a lo
/// que haya.
fn refilter(hwnd: HWND) {
    let query = STATE.lock().unwrap().query.clone();
    let items = collect(&query);
    let (w, h) = size(items.len());
    {
        let mut s = STATE.lock().unwrap();
        s.items = items;
        s.sel = 0;
        s.top = 0;
    }
    unsafe {
        SetWindowPos(hwnd, null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        InvalidateRect(hwnd, null(), 0);
    }
}

fn move_sel(hwnd: HWND, delta: i32) {
    {
        let mut s = STATE.lock().unwrap();
        if s.items.is_empty() {
            return;
        }
        let last = s.items.len() as i32 - 1;
        s.sel = (s.sel as i32 + delta).clamp(0, last) as usize;
        if s.sel < s.top {
            s.top = s.sel;
        } else if s.sel >= s.top + MAX_ROWS {
            s.top = s.sel + 1 - MAX_ROWS;
        }
    }
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

/// La fila bajo (`x`, `y`) (coordenadas de cliente).
fn row_at(y: i32) -> Option<usize> {
    let top_y = px(SEARCH_H) + px(PAD);
    if y < top_y {
        return None;
    }
    let s = STATE.lock().unwrap();
    let i = s.top + ((y - top_y) / (ROW_H * s.dpi / 96)) as usize;
    (i < s.items.len() && i < s.top + MAX_ROWS).then_some(i)
}

/// Trae la nota elegida y cierra el selector. Primero la nota (así pasa
/// adelante mientras la app todavía es la de primer plano).
fn pick(hwnd: HWND) {
    let id = {
        let s = STATE.lock().unwrap();
        s.items.get(s.sel).map(|it| it.id)
    };
    if let Some(id) = id {
        crate::note::open_by_id(id);
    }
    unsafe { DestroyWindow(hwnd) };
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

unsafe fn round_rect(g: *mut GpGraphics, r: &RECT, radius: f32, color: u32) {
    flyout::fill_round(g, r, radius, color);
}

fn on_paint(hwnd: HWND) {
    let c = theme::chrome();
    let (query, dpi, sel, top) = {
        let s = STATE.lock().unwrap();
        (s.query.clone(), s.dpi, s.sel, s.top)
    };
    let p = |v: i32| v * dpi / 96;
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right.max(1), rc.bottom.max(1));
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, bmp);
        let bg = CreateSolidBrush(c.window);
        FillRect(mem, &rc, bg);
        DeleteObject(bg);
        SetBkMode(mem, TRANSPARENT as i32);

        // La búsqueda: lupa, lo escrito (o qué hacer) y una rayita debajo.
        let search = RECT { left: p(PAD) + p(8), top: 0, right: w - p(PAD) - p(8), bottom: p(SEARCH_H) };
        let icon = RECT { left: search.left, right: search.left + p(24), ..search };
        flyout::draw_glyph(mem, flyout::icon_font(-p(16)), 0xE721, &icon, c.muted);
        let mut text_rc = RECT { left: icon.right + p(8), ..search };
        let old = SelectObject(mem, flyout::font(-p(16), FW_NORMAL as i32, 0, "Segoe UI"));
        let (shown, color) = if query.is_empty() {
            ("Traer una nota al frente: escribí para buscar".to_string(), c.muted)
        } else {
            (format!("{query}|"), c.text)
        };
        SetTextColor(mem, color);
        let t = wide(&shown);
        DrawTextW(mem, t.as_ptr(), -1, &mut text_rc, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX);
        SelectObject(mem, old);
        let line = CreateSolidBrush(c.surface_hover);
        FillRect(mem, &RECT { left: 0, top: p(SEARCH_H) - 1, right: w, bottom: p(SEARCH_H) }, line);
        DeleteObject(line);

        let mut g: *mut GpGraphics = null_mut();
        GdipCreateFromHDC(mem, &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        let s = STATE.lock().unwrap();
        if s.items.is_empty() {
            let mut r = RECT { left: p(PAD) * 3, top: p(SEARCH_H) + p(PAD), right: w - p(PAD) * 3, bottom: h - p(PAD) };
            let old = SelectObject(mem, flyout::font(-p(14), FW_NORMAL as i32, 0, "Segoe UI"));
            SetTextColor(mem, c.muted);
            let msg = wide(if query.is_empty() { "Todavía no hay notas." } else { "Ninguna nota coincide." });
            DrawTextW(mem, msg.as_ptr(), -1, &mut r, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            SelectObject(mem, old);
        }
        for (i, it) in s.items.iter().enumerate().skip(top).take(MAX_ROWS) {
            let y = p(SEARCH_H) + p(PAD) + (i - top) as i32 * p(ROW_H);
            let row = RECT { left: p(PAD), top: y, right: w - p(PAD), bottom: y + p(ROW_H) };
            let row_bg = if i == sel { c.surface_hover } else { c.window };
            if i == sel {
                round_rect(g, &row, p(8) as f32, row_bg);
            }
            // El color de la nota, en un cuadradito.
            let (header, _, _) = theme::note_colors(it.color);
            let cy = (row.top + row.bottom) / 2;
            let dot = RECT { left: row.left + p(12), top: cy - p(8), right: row.left + p(28), bottom: cy + p(8) };
            round_rect(g, &dot, p(4) as f32, header);
            // Marcas: siempre encima, oculta.
            let mut right = row.right - p(10);
            for (on, glyph) in [(it.hidden, 0xED1Au16), (it.pinned, 0xE718)] {
                if on {
                    let r = RECT { left: right - p(20), top: row.top, right, bottom: row.bottom };
                    flyout::draw_glyph(mem, flyout::icon_font(-p(13)), glyph, &r, c.muted);
                    right -= p(22);
                }
            }
            let text_left = dot.right + p(12);
            let title_rc = RECT { left: text_left, top: row.top + p(6), right, bottom: cy + p(2) };
            let ink = if it.hidden { c.muted } else { c.text };
            if !crate::d2d::draw_title(mem, title_rc, &it.title, (ink, 0xff), crate::d2d::Bg::Solid(row_bg), p(14)) {
                let old = SelectObject(mem, flyout::font(-p(14), FW_SEMIBOLD as i32, 0, "Segoe UI"));
                SetTextColor(mem, ink);
                let mut r = title_rc;
                let t = wide(&it.title);
                DrawTextW(mem, t.as_ptr(), -1, &mut r, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX);
                SelectObject(mem, old);
            }
            let preview = if it.hidden {
                format!("Oculta · {}", it.preview)
            } else {
                it.preview.clone()
            };
            if !preview.is_empty() {
                let mut r = RECT { left: text_left, top: cy + p(2), right, bottom: row.bottom - p(4) };
                let old = SelectObject(mem, flyout::font(-p(12), FW_NORMAL as i32, 0, "Segoe UI"));
                SetTextColor(mem, c.muted);
                let t = wide(&preview);
                DrawTextW(mem, t.as_ptr(), -1, &mut r, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX);
                SelectObject(mem, old);
            }
        }
        drop(s);
        GdipDeleteGraphics(g);

        BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_KEYDOWN => {
            match wparam as u16 {
                VK_ESCAPE => {
                    DestroyWindow(hwnd);
                }
                VK_RETURN => pick(hwnd),
                VK_DOWN => move_sel(hwnd, 1),
                VK_UP => move_sel(hwnd, -1),
                VK_NEXT => move_sel(hwnd, MAX_ROWS as i32),
                VK_PRIOR => move_sel(hwnd, -(MAX_ROWS as i32)),
                VK_BACK => {
                    let changed = STATE.lock().unwrap().query.pop().is_some();
                    if changed {
                        refilter(hwnd);
                    }
                }
                _ => {}
            }
            0
        }
        WM_CHAR => {
            // Enter, Esc y Retroceso ya se atendieron en WM_KEYDOWN.
            if let Some(ch) = char::from_u32(wparam as u32).filter(|c| !c.is_control()) {
                STATE.lock().unwrap().query.push(ch);
                refilter(hwnd);
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some(i) = row_at(y) {
                let changed = {
                    let mut s = STATE.lock().unwrap();
                    std::mem::replace(&mut s.sel, i) != i
                };
                if changed {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(i) = row_at(y) {
                STATE.lock().unwrap().sel = i;
                pick(hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as i16 as i32;
            move_sel(hwnd, if delta > 0 { -1 } else { 1 });
            0
        }
        // Se fue a otra cosa: se cierra solo, como un menú.
        WM_ACTIVATE => {
            if (wparam & 0xffff) as u32 == WA_INACTIVE {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            0
        }
        WM_DESTROY => {
            let mut s = STATE.lock().unwrap();
            if s.hwnd == hwnd as isize {
                s.hwnd = 0;
                s.items.clear();
                s.query.clear();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
