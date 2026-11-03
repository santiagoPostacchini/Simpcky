//! Transiciones de las notas: moverse, cambiar de tamaño, enrollarse.
//!
//! Unos 200 ms con salida suave, como en el boceto de celdas y pilas. La
//! posición y el tamaño se animan por separado, así una nota puede
//! enrollarse mientras el arrastre la lleva. Si Windows tiene apagados los
//! efectos de animación (Accesibilidad), todo pasa al instante.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::time::Instant;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const MOVE_MS: u32 = 200;
pub const ROLL_MS: u32 = 170;

#[derive(Clone, Copy)]
struct Part {
    from: (i32, i32),
    to: (i32, i32),
    start: Instant,
    ms: u32,
}

impl Part {
    /// Dónde va ahora, y si ya llegó.
    fn at(&self, now: Instant) -> ((i32, i32), bool) {
        let t = (now.duration_since(self.start).as_secs_f32() * 1000.0 / self.ms as f32).min(1.0);
        let e = 1.0 - (1.0 - t).powi(3);
        let lerp = |a: i32, b: i32| a + ((b - a) as f32 * e).round() as i32;
        ((lerp(self.from.0, self.to.0), lerp(self.from.1, self.to.1)), t >= 1.0)
    }
}

#[derive(Default)]
struct Tween {
    pos: Option<Part>,
    size: Option<Part>,
    /// Qué hacer al terminar (esconder el texto de una nota que se enrolló).
    done: Option<fn(HWND)>,
}

thread_local! {
    static TWEENS: RefCell<HashMap<isize, Tween>> = RefCell::new(HashMap::new());
}

fn enabled() -> bool {
    let mut on: BOOL = 1;
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on as *mut BOOL as *mut c_void, 0) };
    on != 0
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// Lleva la nota a `r` (coordenadas de pantalla). Lo que tuviera que
/// hacer al terminar se conserva.
pub fn to_rect(hwnd: HWND, r: RECT, ms: u32) {
    set(hwnd, Some((r.left, r.top)), Some((r.right - r.left, r.bottom - r.top)), ms, None, false);
}

/// Solo la posición (el tamaño puede estar animándose aparte).
pub fn to_pos(hwnd: HWND, x: i32, y: i32, ms: u32) {
    set(hwnd, Some((x, y)), None, ms, None, false);
}

/// Solo el tamaño (la posición la puede estar llevando el arrastre).
/// `done` reemplaza lo que hubiera que hacer al terminar.
pub fn to_size(hwnd: HWND, w: i32, h: i32, ms: u32, done: Option<fn(HWND)>) {
    set(hwnd, None, Some((w, h)), ms, done, true);
}

fn set(hwnd: HWND, pos: Option<(i32, i32)>, size: Option<(i32, i32)>, ms: u32, done: Option<fn(HWND)>, replace_done: bool) {
    let cur = window_rect(hwnd);
    let (cur_pos, cur_size) = ((cur.left, cur.top), (cur.right - cur.left, cur.bottom - cur.top));
    let now = Instant::now();
    let instant = ms == 0 || !enabled();
    let finished = TWEENS.with(|t| {
        let mut t = t.borrow_mut();
        let tw = t.entry(hwnd as isize).or_default();
        if replace_done {
            tw.done = done;
        }
        if let Some(p) = pos {
            tw.pos = (p != cur_pos && !instant).then_some(Part { from: cur_pos, to: p, start: now, ms });
        }
        if let Some(s) = size {
            tw.size = (s != cur_size && !instant).then_some(Part { from: cur_size, to: s, start: now, ms });
        }
        if tw.pos.is_none() && tw.size.is_none() {
            t.remove(&(hwnd as isize)).and_then(|tw| tw.done)
        } else {
            None
        }
    });
    let moving = TWEENS.with(|t| t.borrow().get(&(hwnd as isize)).map(|tw| (tw.pos.is_some(), tw.size.is_some())));
    // Lo que no se anima, va directo a su lugar.
    let (pos_now, size_now) = match moving {
        Some((p, s)) => (pos.filter(|_| !p), size.filter(|_| !s)),
        None => (pos, size),
    };
    place(hwnd, pos_now.filter(|p| *p != cur_pos), size_now.filter(|s| *s != cur_size));
    if let Some(f) = finished {
        f(hwnd);
    }
    if moving.is_some() {
        crate::frames::want(crate::frames::Who::Anim, true);
    }
}

/// El arrastre toma la nota: deja de ir hacia donde iba (el tamaño sigue).
pub fn stop_pos(hwnd: HWND) {
    TWEENS.with(|t| {
        if let Some(tw) = t.borrow_mut().get_mut(&(hwnd as isize)) {
            tw.pos = None;
        }
    });
}

/// ¿Está cambiando de tamaño? (Mientras tanto el texto no se reacomoda.)
pub fn sizing(hwnd: HWND) -> bool {
    TWEENS.with(|t| t.try_borrow().map(|t| t.get(&(hwnd as isize)).is_some_and(|tw| tw.size.is_some())).unwrap_or(false))
}

fn place(hwnd: HWND, pos: Option<(i32, i32)>, size: Option<(i32, i32)>) {
    if pos.is_none() && size.is_none() {
        return;
    }
    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
    let mut p = POINT { x: 0, y: 0 };
    match pos {
        Some((x, y)) => {
            p = POINT { x, y };
            unsafe {
                // Anclada: coordenadas del padre.
                if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD != 0 {
                    ScreenToClient(GetAncestor(hwnd, GA_PARENT), &mut p);
                }
            }
        }
        None => flags |= SWP_NOMOVE,
    }
    let (w, h) = size.unwrap_or((0, 0));
    if size.is_none() {
        flags |= SWP_NOSIZE;
    }
    unsafe { SetWindowPos(hwnd, null_mut(), p.x, p.y, w, h, flags) };
}

/// Un cuadro (ver `frames.rs`): cada nota da un paso hacia donde va.
pub fn frame() {
    let now = Instant::now();
    // (nota, posición, tamaño, qué hacer al terminar)
    type Step = (HWND, Option<(i32, i32)>, Option<(i32, i32)>, Option<fn(HWND)>);
    let mut steps: Vec<Step> = Vec::new();
    TWEENS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|&h, tw| {
            let hwnd = h as HWND;
            if unsafe { IsWindow(hwnd) } == 0 {
                return false;
            }
            let mut pos = None;
            if let Some(p) = tw.pos {
                let (v, end) = p.at(now);
                pos = Some(v);
                if end {
                    tw.pos = None;
                }
            }
            let mut size = None;
            if let Some(s) = tw.size {
                let (v, end) = s.at(now);
                size = Some(v);
                // Se quita antes del último paso: así ese WM_SIZE sí acomoda el texto.
                if end {
                    tw.size = None;
                }
            }
            let over = tw.pos.is_none() && tw.size.is_none();
            steps.push((hwnd, pos, size, if over { tw.done.take() } else { None }));
            !over
        });
        if t.is_empty() {
            crate::frames::want(crate::frames::Who::Anim, false);
        }
    });
    for (hwnd, pos, size, done) in steps {
        place(hwnd, pos, size);
        if let Some(f) = done {
            f(hwnd);
        }
    }
}
