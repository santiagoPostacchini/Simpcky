//! Ícono de la app: el mismo que lleva el `.exe` (`assets/simpcky.ico`,
//! embebido por `build.rs` como recurso 1), para que la bandeja, las
//! ventanas y el archivo en el Explorador sean exactamente iguales.
//!
//! Windows elige del `.ico` el cuadro que mejor le queda a cada tamaño
//! (16 px en la bandeja, 32 en Alt+Tab, etc.), así que nada se escala
//! a mano.

use std::sync::OnceLock;

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Id del `RT_GROUP_ICON` que genera `build.rs`.
const ICON_ID: usize = 1;

static SMALL: OnceLock<isize> = OnceLock::new();
static LARGE: OnceLock<isize> = OnceLock::new();

fn load(size_metric: SYSTEM_METRICS_INDEX) -> isize {
    unsafe {
        let size = GetSystemMetrics(size_metric);
        let module = GetModuleHandleW(std::ptr::null());
        // MAKEINTRESOURCE(1): el id va en el puntero mismo.
        LoadImageW(module, ICON_ID as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) as isize
    }
}

/// Ícono chico (bandeja, barra de título).
pub fn app_icon() -> HICON {
    (*SMALL.get_or_init(|| load(SM_CXSMICON))) as HICON
}

/// Ícono grande (Alt+Tab, barra de tareas).
pub fn app_icon_large() -> HICON {
    (*LARGE.get_or_init(|| load(SM_CXICON))) as HICON
}
