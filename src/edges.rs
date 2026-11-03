//! El borde para cambiar el tamaño, a la vista y más fácil de agarrar.
//!
//! La nota no tiene marco: el borde que redimensiona era una franja
//! invisible de 8 px hacia adentro, y no se notaba dónde empezaba ni
//! dónde terminaba. Con el mouse sobre una nota aparece un marco (una
//! ventana en capas, una sola para todas las notas) que:
//! - se extiende `OUT` px afuera de la nota: el borde se agarra desde los
//!   dos lados (sus píxeles tienen alfa 1, invisibles pero no dejan pasar
//!   el mouse; los de adentro tienen alfa 0 y el mouse llega a la nota);
//! - dibuja el contorno de la nota y resalta el borde o la esquina que se
//!   va a mover, con un fundido corto;
//! - al apretar, le pasa el clic a la nota, que redimensiona como siempre
//!   (Windows en las ancladas, `note::begin_sizing` en las de primer nivel).
//!
//! Sigue a la nota en cada cuadro (`frames.rs`) mientras el mouse esté
//! cerca o se esté cambiando el tamaño, y después se va.

use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::time::Instant;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::wide;

/// Cuánto sale el marco afuera de la nota, y el radio de sus esquinas (a 100 %).
const OUT: i32 = 6;
const RADIUS: i32 = 8;

#[derive(Default)]
struct Ring {
    hwnd: isize,
    note: isize,
    /// El borde resaltado (`HTLEFT`…`HTBOTTOMRIGHT`), o 0.
    hit: u32,
    /// Qué tan a la vista está el resaltado (0 a 1).
    a: f32,
    last: Option<Instant>,
    /// Lo último que se dibujó: rectángulo de la nota, borde y alfa.
    drawn: Option<([i32; 4], u32, u8, bool)>,
    /// Cómo estaba la nota al ponerle el marco encima: (hija del escritorio, siempre encima).
    layer: (bool, bool),
    /// Lo que se miró en el cuadro anterior (mouse, nota, Ctrl): si no cambió y
    /// no hay fundido a medias, no hay nada que hacer.
    seen: Option<([i32; 2], [i32; 4], bool)>,
}

thread_local! {
    static RING: RefCell<Ring> = RefCell::new(Ring::default());
}

fn px(v: i32) -> i32 {
    v * crate::note::note_dpi(null_mut()) / 96
}

fn cursor() -> POINT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    p
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// El mouse está sobre `note`: el marco la acompaña mientras siga cerca.
pub fn track(note: HWND) {
    if crate::snap::moving() {
        return;
    }
    let changed = RING.with(|r| {
        let mut r = r.borrow_mut();
        // El marco es una ventana poseída: Windows lo destruye con su dueña
        // (una nota borrada, Explorer reiniciado). Entonces se hace otro.
        if r.hwnd == 0 || unsafe { IsWindow(r.hwnd as HWND) } == 0 {
            r.hwnd = create() as isize;
            r.note = 0;
        }
        let changed = r.note != note as isize;
        if changed {
            r.note = note as isize;
            r.hit = 0;
            r.a = 0.0;
            r.drawn = None;
            r.seen = None;
        }
        changed
    });
    if changed {
        place_layer(note);
    }
    crate::frames::want(crate::frames::Who::Edges, true);
}

/// Pone el marco justo encima de la nota en el orden de las ventanas:
/// poseído por ella si es de primer nivel; si está anclada (hija del
/// escritorio), poseído por el escritorio, arriba de las notas y debajo
/// de las aplicaciones (como el modo vidrio).
fn layer_of(note: HWND) -> (bool, bool) {
    unsafe { (GetWindowLongPtrW(note, GWL_STYLE) as u32 & WS_CHILD != 0, GetWindowLongPtrW(note, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0) }
}

fn place_layer(note: HWND) {
    let ring = RING.with(|r| r.borrow().hwnd) as HWND;
    if ring.is_null() {
        return;
    }
    let (child, topmost) = layer_of(note);
    RING.with(|r| r.borrow_mut().layer = (child, topmost));
    unsafe {
        let owner = if child { crate::desktop::find_host() } else { note };
        SetWindowLongPtrW(ring, GWLP_HWNDPARENT, owner as isize);
        if !topmost && GetWindowLongPtrW(ring, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0 {
            SetWindowPos(ring, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        // Justo arriba de la nota: debajo de la ventana que la tapa (si hay).
        let above = if child { null_mut() } else { GetWindow(note, GW_HWNDPREV) };
        let after = if topmost {
            HWND_TOPMOST
        } else if child {
            HWND_BOTTOM
        } else if above.is_null() {
            HWND_TOP
        } else {
            above
        };
        SetWindowPos(ring, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

fn hide() {
    let ring = RING.with(|r| {
        let mut r = r.borrow_mut();
        r.note = 0;
        r.hit = 0;
        r.a = 0.0;
        r.drawn = None;
        r.last = None;
        r.seen = None;
        r.hwnd
    });
    if ring != 0 {
        unsafe { ShowWindow(ring as HWND, SW_HIDE) };
    }
    crate::frames::want(crate::frames::Who::Edges, false);
}

/// Un cuadro: sigue a la nota, decide qué borde resaltar y se va si el
/// mouse se alejó.
pub fn frame() {
    let (ring, note) = RING.with(|r| {
        let r = r.borrow();
        (r.hwnd as HWND, r.note as HWND)
    });
    // (Mientras se arrastra una nota, no; mientras se le cambia el tamaño, sí.)
    if ring.is_null() || note.is_null() || unsafe { IsWindow(ring) == 0 || IsWindow(note) == 0 || IsWindowVisible(note) == 0 } || crate::snap::moving() {
        hide();
        return;
    }
    // Pasó a anclada, suelta o siempre encima con el mouse encima: el marco la sigue.
    if RING.with(|r| r.borrow().layer) != layer_of(note) {
        place_layer(note);
    }
    let out = px(OUT);
    let rc = window_rect(note);
    let sizing = unsafe { GetCapture() } == note;
    let p = cursor();
    let ctrl = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) } < 0;
    let key = ([p.x, p.y], q(rc), ctrl);
    let idle = RING.with(|r| {
        let mut r = r.borrow_mut();
        let same = r.seen == Some(key) && (r.a == 0.0 || r.a == 1.0);
        r.seen = Some(key);
        if same {
            r.last = Some(Instant::now());
        }
        same
    });
    if idle {
        return;
    }
    let near = p.x >= rc.left - out && p.x < rc.right + out && p.y >= rc.top - out && p.y < rc.bottom + out;
    let hit = if sizing { RING.with(|r| r.borrow().hit) } else { crate::note::edge_hit(note, p.x, p.y, out) };
    let now = Instant::now();
    let (a, show_hit, gone) = RING.with(|r| {
        let mut r = r.borrow_mut();
        let s = r.last.replace(now).map_or(1.0, |t| (now - t).as_secs_f32() * 1000.0 / 16.0).min(4.0);
        let to = if hit != 0 && (near || sizing) { 1.0 } else { 0.0 };
        r.a += (to - r.a) * (1.0 - (1.0f32 - 0.45).powf(s));
        if (to - r.a).abs() < 0.02 {
            r.a = to;
        }
        if hit != 0 {
            r.hit = hit;
        }
        (r.a, r.hit, !near && !sizing && r.a == 0.0)
    });
    if gone {
        hide();
        return;
    }
    draw(ring, rc, show_hit, a, near || sizing);
}

/// Dibuja el marco: la franja de afuera (alfa 1, para agarrar), el
/// contorno y el borde resaltado.
fn draw(ring: HWND, rc: RECT, hit: u32, a: f32, near: bool) {
    let alpha = (a * 255.0).round() as u8;
    let same = RING.with(|r| r.borrow().drawn == Some((q(rc), hit, alpha, near)));
    if same {
        return;
    }
    let out = px(OUT);
    let (nw, nh) = (rc.right - rc.left, rc.bottom - rc.top);
    let (w, h) = (nw + 2 * out, nh + 2 * out);
    if w <= 0 || h <= 0 {
        return;
    }
    let mut px_buf = vec![0u32; (w * h) as usize];
    let r = px(RADIUS) as f32;
    // Lo de afuera de la nota (y las esquinas recortadas) se agarra.
    let inside = |x: f32, y: f32| -> bool {
        let (l, t, ri, b) = (out as f32, out as f32, (out + nw) as f32, (out + nh) as f32);
        if x < l || x >= ri || y < t || y >= b {
            return false;
        }
        let cx = if x < l + r { Some(l + r) } else if x >= ri - r { Some(ri - r) } else { None };
        let cy = if y < t + r { Some(t + r) } else if y >= b - r { Some(b - r) } else { None };
        match (cx, cy) {
            (Some(cx), Some(cy)) => ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() <= r,
            _ => true,
        }
    };
    if near {
        let ri = r.ceil() as i32;
        for y in 0..h {
            // Entre los redondeos, solo las franjas de los costados; arriba y abajo, la fila entera.
            let middle = y >= out + ri && y < out + nh - ri;
            for x in 0..w {
                if middle && x >= out && x < out + nw {
                    continue;
                }
                if !inside(x as f32 + 0.5, y as f32 + 0.5) {
                    px_buf[(y * w + x) as usize] = 0x0100_0000;
                }
            }
        }
    }
    if alpha > 0 {
        let accent: u32 = if crate::theme::is_dark() { 0x60a5fa } else { 0x3b82f6 };
        // Contorno suave de toda la nota, y el borde elegido más fuerte.
        let faint = 0.35 * a;
        stroke(&mut px_buf, w, h, out, nw, nh, r, 0, px(1).max(1) as f32 + 0.5, accent, faint);
        stroke(&mut px_buf, w, h, out, nw, nh, r, hit, px(3) as f32, accent, 0.95 * a);
    }
    present(ring, rc.left - out, rc.top - out, w, h, &px_buf);
    RING.with(|r| r.borrow_mut().drawn = Some((q(rc), hit, alpha, near)));
}

/// Pinta sobre el borde de la nota (centrado en él) un trazo de grosor `t`:
/// todo el contorno si `hit` es 0, o solo ese lado o esa esquina.
#[allow(clippy::too_many_arguments)]
fn stroke(buf: &mut [u32], w: i32, h: i32, out: i32, nw: i32, nh: i32, r: f32, hit: u32, t: f32, rgb: u32, a: f32) {
    let (l, top, ri, b) = (out as f32, out as f32, (out + nw) as f32, (out + nh) as f32);
    // Qué tramos van: lados (izq, der, arriba, abajo) y esquinas (ai, ad, bi, bd).
    let corner_len = px(18) as f32;
    let (sides, corners): ([bool; 4], [bool; 4]) = match hit {
        0 => ([true; 4], [true; 4]),
        HTLEFT => ([true, false, false, false], [false; 4]),
        HTRIGHT => ([false, true, false, false], [false; 4]),
        HTTOP => ([false, false, true, false], [false; 4]),
        HTBOTTOM => ([false, false, false, true], [false; 4]),
        HTTOPLEFT => ([false; 4], [true, false, false, false]),
        HTTOPRIGHT => ([false; 4], [false, true, false, false]),
        HTBOTTOMLEFT => ([false; 4], [false, false, true, false]),
        _ => ([false; 4], [false, false, false, true]),
    };
    let half = t / 2.0;
    let cov_line = |d: f32| (half + 0.5 - d.abs()).clamp(0.0, 1.0);
    let ink = |buf: &mut [u32], x: i32, y: i32, c: f32| {
        if c <= 0.0 || x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let k = (c * a).clamp(0.0, 1.0);
        let i = (y * w + x) as usize;
        let d = buf[i];
        let (da, dr, dg, db) = ((d >> 24) as f32, ((d >> 16) & 255) as f32, ((d >> 8) & 255) as f32, (d & 255) as f32);
        let (sr, sg, sb) = (((rgb >> 16) & 255) as f32, ((rgb >> 8) & 255) as f32, (rgb & 255) as f32);
        let mix = |s: f32, d: f32| (s * k + d * (1.0 - k)).round() as u32;
        let na = (255.0 * k + da * (1.0 - k)).round().max(1.0) as u32;
        buf[i] = (na << 24) | (mix(sr, dr) << 16) | (mix(sg, dg) << 8) | mix(sb, db);
    };
    let reach = (half + 1.0).ceil() as i32;
    // Lados: entre los redondeos de las esquinas.
    for (k, on) in sides.iter().enumerate() {
        if !on {
            continue;
        }
        let vertical = k < 2;
        let pos = [l, ri, top, b][k];
        let (from, to) = if vertical { (top + r, b - r) } else { (l + r, ri - r) };
        for s in from.floor() as i32..to.ceil() as i32 {
            for o in -reach..=reach {
                let q = pos.round() as i32 + o;
                let d = q as f32 + 0.5 - pos;
                let (x, y) = if vertical { (q, s) } else { (s, q) };
                ink(buf, x, y, cov_line(d));
            }
        }
    }
    // Esquinas: el arco, y en las elegidas un tramo de cada lado.
    let centers = [(l + r, top + r), (ri - r, top + r), (l + r, b - r), (ri - r, b - r)];
    for (k, on) in corners.iter().enumerate() {
        if !on {
            continue;
        }
        let (cx, cy) = centers[k];
        let (sx, sy) = ([-1.0, 1.0, -1.0, 1.0][k], [-1.0, -1.0, 1.0, 1.0][k]);
        let box_ = r + half + 1.0;
        for y in (cy - box_).floor() as i32..=(cy + box_).ceil() as i32 {
            for x in (cx - box_).floor() as i32..=(cx + box_).ceil() as i32 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * sx < 0.0 || dy * sy < 0.0 {
                    continue;
                }
                ink(buf, x, y, cov_line((dx * dx + dy * dy).sqrt() - r));
            }
        }
        if hit != 0 {
            // Tramos rectos a los dos lados de la esquina.
            let leg = (corner_len - r).max(0.0);
            let (vx, hy) = (if sx < 0.0 { l } else { ri }, if sy < 0.0 { top } else { b });
            let ys = if sy < 0.0 { (top + r, top + r + leg) } else { (b - r - leg, b - r) };
            let xs = if sx < 0.0 { (l + r, l + r + leg) } else { (ri - r - leg, ri - r) };
            for s in ys.0.floor() as i32..ys.1.ceil() as i32 {
                for o in -reach..=reach {
                    let q = vx.round() as i32 + o;
                    ink(buf, q, s, cov_line(q as f32 + 0.5 - vx));
                }
            }
            for s in xs.0.floor() as i32..xs.1.ceil() as i32 {
                for o in -reach..=reach {
                    let q = hy.round() as i32 + o;
                    ink(buf, s, q, cov_line(q as f32 + 0.5 - hy));
                }
            }
        }
    }
}

/// Lleva los píxeles (BGRA premultiplicado) a la ventana en capas.
fn present(ring: HWND, x: i32, y: i32, w: i32, h: i32, pixels: &[u32]) {
    unsafe {
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        let mut bits: *mut core::ffi::c_void = null_mut();
        let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bmp.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
            let old = SelectObject(mem, bmp as HGDIOBJ);
            let dst = POINT { x, y };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            UpdateLayeredWindow(ring, screen, &dst, &size, mem, &src, 0, &blend, ULW_ALPHA);
            SelectObject(mem, old);
            DeleteObject(bmp as HGDIOBJ);
            if IsWindowVisible(ring) == 0 {
                ShowWindow(ring, SW_SHOWNOACTIVATE);
            }
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
    }
}

fn create() -> HWND {
    unsafe {
        let class = wide("SimpckyEdges");
        let hinstance = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null());
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(proc_),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
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
        )
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let note = RING.with(|r| r.try_borrow().map(|r| r.note).unwrap_or(0)) as HWND;
    match msg {
        // Qué borde hay bajo el mouse: el cursor de cambiar el tamaño lo pone Windows.
        WM_NCHITTEST if !note.is_null() => {
            let (x, y) = ((lparam & 0xffff) as i16 as i32, ((lparam >> 16) & 0xffff) as i16 as i32);
            match crate::note::edge_hit(note, x, y, px(OUT)) {
                0 => HTTRANSPARENT as LRESULT,
                hit => hit as LRESULT,
            }
        }
        // El clic va a la nota, que cambia de tamaño como siempre.
        WM_NCLBUTTONDOWN if !note.is_null() && (HTLEFT..=HTBOTTOMRIGHT).contains(&(wparam as u32)) => {
            SendMessageW(note, WM_NCLBUTTONDOWN, wparam, lparam);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn q(r: RECT) -> [i32; 4] {
    [r.left, r.top, r.right, r.bottom]
}
