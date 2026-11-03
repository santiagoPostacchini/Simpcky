//! Cuadros al ritmo de la pantalla.
//!
//! Simpcky avanza las transiciones con un `SetTimer` de 10 ms, que Windows
//! redondea a ~15,6 ms: con una pantalla de 60 Hz a veces cae dos pasos en
//! un cuadro y a veces ninguno (se nota como "tironcitos"). Acá un hilo
//! espera a que el compositor termine cada cuadro (`DwmFlush`) y avisa a la
//! ventana principal con un mensaje; solo mientras haya algo moviéndose.
//! Si el aviso anterior todavía no se atendió, no se manda otro.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::DwmFlush;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// El mensaje que llega a la ventana en cada cuadro.
pub const WM_FRAME: u32 = WM_APP + 0x51;

static TARGET: AtomicIsize = AtomicIsize::new(0);
static PENDING: AtomicBool = AtomicBool::new(false);

struct Gate {
    on: Mutex<bool>,
    cv: Condvar,
}

fn gate() -> &'static Gate {
    static G: OnceLock<Gate> = OnceLock::new();
    G.get_or_init(|| {
        std::thread::spawn(run);
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
        unsafe { DwmFlush() };
        let target = TARGET.load(Ordering::Relaxed);
        if target != 0 && !PENDING.swap(true, Ordering::AcqRel) {
            unsafe { PostMessageW(target as HWND, WM_FRAME, 0, 0) };
        }
    }
}

/// Adónde llegan los cuadros (una ventana del hilo de la interfaz).
pub fn set_target(hwnd: HWND) {
    TARGET.store(hwnd as isize, Ordering::Relaxed);
}

/// Prende o apaga los cuadros (apagados no gastan nada).
pub fn run_frames(on: bool) {
    let g = gate();
    let mut cur = g.on.lock().unwrap();
    if *cur != on {
        *cur = on;
        g.cv.notify_all();
    }
}

/// Llamar al atender `WM_FRAME`.
pub fn frame_taken() {
    PENDING.store(false, Ordering::Release);
}
