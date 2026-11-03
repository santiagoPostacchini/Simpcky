//! Cuadros al ritmo de la pantalla, para las transiciones y el arrastre.
//!
//! Con `SetTimer` las transiciones avanzaban cada ~15,6 ms (Windows
//! redondea): 10 pasos por animación, saltos grandes al enrollar y algún
//! tirón. Acá un hilo espera a que el compositor termine cada cuadro
//! (`DwmFlush`) y avisa a una ventana de mensajes de la interfaz; solo
//! mientras alguien los pida. Si el aviso anterior no se atendió todavía,
//! no se manda otro (no se acumulan). Medido en las demos (monitor de
//! 400 Hz): 56 pasos al enrollar en vez de 10, y sin tirones.
//!
//! Llegan también dentro de los bucles modales de Windows (redimensionar
//! una nota anclada, menús), porque van a una ventana y no al hilo.

use std::cell::Cell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::wide;

/// Quién pide cuadros.
#[derive(Clone, Copy)]
pub enum Who {
    /// Las transiciones de `anim.rs`.
    Anim = 1,
    /// El arrastre y las capas de `snap.rs`.
    Snap = 2,
    /// El marco del borde de `edges.rs`.
    Edges = 4,
}

const WM_FRAME: u32 = WM_APP + 0x51;

static WANTED: AtomicU32 = AtomicU32::new(0);
static TARGET: AtomicIsize = AtomicIsize::new(0);
static PENDING: AtomicBool = AtomicBool::new(false);

struct Gate {
    on: Mutex<bool>,
    cv: Condvar,
}

fn gate() -> &'static Gate {
    static G: OnceLock<Gate> = OnceLock::new();
    G.get_or_init(|| {
        std::thread::Builder::new().name("cuadros".into()).spawn(run).ok();
        Gate { on: Mutex::new(false), cv: Condvar::new() }
    })
}

fn run() {
    loop {
        {
            let g = gate();
            let mut on = g.on.lock().unwrap();
            while !*on {
                on = g.cv.wait(on).unwrap();
            }
        }
        // Sin compositor (no debería pasar en Windows 10/11): ~60 por segundo.
        if unsafe { DwmFlush() } < 0 {
            std::thread::sleep(Duration::from_millis(16));
        }
        let target = TARGET.load(Ordering::Relaxed);
        if target != 0 && !PENDING.swap(true, Ordering::AcqRel) && unsafe { PostMessageW(target as HWND, WM_FRAME, 0, 0) } == 0 {
            PENDING.store(false, Ordering::Release);
        }
    }
}

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
}

/// La ventana (solo de mensajes) que recibe los cuadros, en el hilo de la interfaz.
fn window() -> HWND {
    let h = WINDOW.get();
    if !h.is_null() {
        return h;
    }
    unsafe {
        let class = wide("SimpckyFrames");
        let hinstance = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(proc_),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let h = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), hinstance, std::ptr::null());
        WINDOW.set(h);
        TARGET.store(h as isize, Ordering::Relaxed);
        h
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_FRAME {
        PENDING.store(false, Ordering::Release);
        let w = WANTED.load(Ordering::Relaxed);
        if w & Who::Anim as u32 != 0 {
            crate::anim::frame();
        }
        if w & Who::Snap as u32 != 0 {
            crate::snap::frame();
        }
        if w & Who::Edges as u32 != 0 {
            crate::edges::frame();
        }
        return 0;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Prende o apaga los cuadros para `who` (apagados, el hilo duerme).
pub fn want(who: Who, on: bool) {
    if on {
        window();
    }
    let bit = who as u32;
    let all = if on { WANTED.fetch_or(bit, Ordering::Relaxed) | bit } else { WANTED.fetch_and(!bit, Ordering::Relaxed) & !bit };
    let g = gate();
    let mut cur = g.on.lock().unwrap();
    if *cur != (all != 0) {
        *cur = all != 0;
        g.cv.notify_all();
    }
}
