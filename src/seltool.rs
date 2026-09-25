//! Barra de formato al seleccionar: negrita, cursiva, subrayado y tachado
//! arriba del texto recién seleccionado con el mouse. No se lleva el foco
//! ni el mouse (el texto sigue seleccionado, se puede seguir escribiendo)
//! y se va sola al escribir, hacer clic en otro lado, desplazar el texto
//! o salir de la nota.

use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::flyout;
use crate::richtext::{BOLD, ITALIC, STRIKE, UNDERLINE};
use crate::theme;
use crate::win::wide;

const CLASS_NAME: &str = "SimpckySelTool";
/// Medidas a 100 %.
const BTN: i32 = 30;
const PAD: i32 = 4;
const GAP: i32 = 2;
const STYLES: [usize; 4] = [BOLD, ITALIC, UNDERLINE, STRIKE];

struct State {
    hwnd: isize,
    /// El RichEdit al que le da formato.
    edit: isize,
    hover: Option<usize>,
    dpi: i32,
    active: [bool; 4],
}

static STATE: Mutex<State> = Mutex::new(State { hwnd: 0, edit: 0, hover: None, dpi: 96, active: [false; 4] });

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

fn px(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

fn size(dpi: i32) -> (i32, i32) {
    (px(dpi, PAD * 2 + BTN * 4 + GAP * 3), px(dpi, PAD * 2 + BTN))
}

/// El botón `k` (en coordenadas de la barra).
fn button(dpi: i32, k: usize) -> RECT {
    let left = px(dpi, PAD + k as i32 * (BTN + GAP));
    RECT { left, top: px(dpi, PAD), right: left + px(dpi, BTN), bottom: px(dpi, PAD + BTN) }
}

/// Se terminó de seleccionar con el mouse en `edit`: la barra arriba de
/// la selección (o abajo, si arriba no entra en la nota). Si no hay nada
/// seleccionado, se va.
pub fn show_for(edit: HWND) {
    let Some((sel, styles)) = crate::editor::selection_box(edit) else {
        hide();
        return;
    };
    let active = STYLES.map(|k| styles[k]);
    let note = unsafe { GetParent(edit) };
    let dpi = unsafe { GetDpiForWindow(edit) }.max(96) as i32;
    let (w, h) = size(dpi);
    let mut note_rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(note, &mut note_rc) };
    let gap = px(dpi, 6);
    let x = sel.left.clamp(note_rc.left + gap, (note_rc.right - gap - w).max(note_rc.left + gap));
    let mut y = sel.top - gap - h;
    if y < note_rc.top + gap {
        y = sel.bottom + gap;
    }

    let existing = STATE.lock().unwrap().hwnd as HWND;
    let hwnd = if existing.is_null() { create() } else { existing };
    if hwnd.is_null() {
        return;
    }
    {
        let mut s = STATE.lock().unwrap();
        s.edit = edit as isize;
        s.dpi = dpi;
        s.active = active;
        s.hover = None;
    }
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        InvalidateRect(hwnd, null(), 0);
    }
}

fn create() -> HWND {
    let hinstance = crate::app::app().lock().unwrap().hinstance as HINSTANCE;
    let class_name = wide(CLASS_NAME);
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );
        if !hwnd.is_null() {
            let pref: i32 = DWMWCP_ROUND;
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const i32 as *const _, 4);
            STATE.lock().unwrap().hwnd = hwnd as isize;
        }
        hwnd
    }
}

pub fn hide() {
    let hwnd = {
        let mut s = STATE.lock().unwrap();
        s.edit = 0;
        s.hwnd as HWND
    };
    if !hwnd.is_null() {
        unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
}

/// Se cierra si es la de `edit` (el texto se va, se bloquea…).
pub fn hide_for(edit: HWND) {
    if STATE.lock().unwrap().edit == edit as isize {
        hide();
    }
}

/// El formato cambió por otro lado (un atajo): la barra, si está, al día.
pub fn refresh(edit: HWND) {
    if STATE.lock().unwrap().edit == edit as isize {
        show_for(edit);
    }
}

/// ¿`w` es la barra, abierta sobre el texto de `note`? (El mouse encima
/// de la barra cuenta como encima de la nota: si no, una nota que se
/// enrolla sola se enrollaba al ir a tocar un botón.)
pub fn is_over(w: HWND, note: HWND) -> bool {
    let s = STATE.lock().unwrap();
    s.hwnd != 0 && w as isize == s.hwnd && s.edit != 0 && unsafe { GetParent(s.edit as HWND) } == note
}

fn hit(x: i32, y: i32) -> Option<usize> {
    let dpi = STATE.lock().unwrap().dpi;
    (0..4).find(|&k| {
        let r = button(dpi, k);
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
}

fn on_paint(hwnd: HWND) {
    let (dpi, hover, active) = {
        let s = STATE.lock().unwrap();
        (s.dpi, s.hover, s.active)
    };
    let c = theme::chrome();
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
        let old_bmp = SelectObject(mem, bmp);
        let bg = CreateSolidBrush(c.window);
        FillRect(mem, &rc, bg);
        DeleteObject(bg);
        SetBkMode(mem, TRANSPARENT as i32);

        let mut g: *mut GpGraphics = null_mut();
        GdipCreateFromHDC(mem, &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        for (k, &style) in STYLES.iter().enumerate() {
            let r = button(dpi, k);
            if active[k] {
                flyout::fill_round(g, &r, px(dpi, 5) as f32, c.surface_hover);
            } else if hover == Some(k) {
                flyout::fill_round(g, &r, px(dpi, 5) as f32, c.surface);
            }
            // Las mismas letras que el menú del clic derecho.
            let (weight, bits, face) = match style {
                BOLD => (700, 0, "Segoe UI"),
                ITALIC => (400, 1, "Georgia"),
                UNDERLINE => (400, 2, "Segoe UI"),
                _ => (400, 4, "Segoe UI"),
            };
            let old = SelectObject(mem, flyout::font(-px(dpi, 16), weight, bits, face));
            SetTextColor(mem, c.text);
            let mut rr = r;
            let letter = [['B', 'I', 'U', 'S'][k] as u16, 0];
            DrawTextW(mem, letter.as_ptr(), 1, &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(mem, old);
        }
        GdipDeleteGraphics(g);
        BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    match msg {
        // Tocarla no activa nada: el foco sigue en el texto.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let h = hit(x, y);
            let changed = {
                let mut s = STATE.lock().unwrap();
                std::mem::replace(&mut s.hover, h) != h
            };
            if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            0
        }
        0x02A3 /* WM_MOUSELEAVE */ => {
            STATE.lock().unwrap().hover = None;
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONDOWN => {
            let edit = STATE.lock().unwrap().edit as HWND;
            if let (Some(k), false) = (hit(x, y), edit.is_null()) {
                crate::editor::toggle_style(edit, STYLES[k]);
                refresh(edit);
            }
            0
        }
        WM_DESTROY => {
            let mut s = STATE.lock().unwrap();
            if s.hwnd == hwnd as isize {
                s.hwnd = 0;
                s.edit = 0;
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
