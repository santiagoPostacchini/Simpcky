//! Tema claro / oscuro.
//!
//! Un solo interruptor global (menú de la bandeja, o el botón sol/luna
//! de "Todas las notas") que cambia a la vez:
//! - los colores de cada nota (encabezado, cuerpo, tinta),
//! - la ventana "Todas las notas" (fondo, buscador, texto),
//! - la barra de título de esa ventana (`DWMWA_USE_IMMERSIVE_DARK_MODE`),
//! - las barras de desplazamiento (tema "DarkMode_Explorer"),
//! - y los menús nativos (la API no documentada de `uxtheme.dll` que
//!   usan el Bloc de notas, Notepad++, etc.; si no existe en esta
//!   versión de Windows simplemente se queda en claro).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Controls::SetWindowTheme;

use crate::win::{rgb, wide};

static DARK: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// Fija el tema al arrancar (sin repintar nada: todavía no hay ventanas).
pub fn init(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    apply_menus(dark);
}

/// Cambia el tema en caliente: guarda la preferencia y repinta todo.
pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    apply_menus(dark);
    crate::app::save_settings();
    crate::note::apply_theme_all();
    crate::allnotes::apply_theme();
    crate::welcome::refresh();
}

/// Lo que tiene elegido Windows para las apps (Configuración →
/// Personalización → Colores). Solo se usa la primera vez, para
/// arrancar como el resto del sistema.
pub fn system_prefers_dark() -> bool {
    unsafe {
        let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let value = wide("AppsUseLightTheme");
        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let ok = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut c_void,
            &mut size,
        );
        ok == ERROR_SUCCESS && data == 0
    }
}

// -----------------------------------------------------------------
// Paletas de las notas: (encabezado, cuerpo, tinta)
// -----------------------------------------------------------------

pub const PALETTE_LEN: usize = 6;

const LIGHT: [(u32, u32, u32); PALETTE_LEN] = [
    (rgb(0xFD, 0xE6, 0x8A), rgb(0xFE, 0xF9, 0xC3), rgb(0x78, 0x35, 0x0F)), // amarillo
    (rgb(0xFB, 0xCF, 0xE8), rgb(0xFC, 0xE7, 0xF3), rgb(0x83, 0x18, 0x43)), // rosa
    (rgb(0xBB, 0xF7, 0xD0), rgb(0xDC, 0xFC, 0xE7), rgb(0x06, 0x5F, 0x46)), // verde
    (rgb(0xBF, 0xDB, 0xFE), rgb(0xDB, 0xEA, 0xFE), rgb(0x1E, 0x3A, 0x8A)), // azul
    (rgb(0xE9, 0xD5, 0xFF), rgb(0xF3, 0xE8, 0xFF), rgb(0x58, 0x1C, 0x87)), // morado
    (rgb(0xE2, 0xE8, 0xF0), rgb(0xF1, 0xF5, 0xF9), rgb(0x33, 0x41, 0x55)), // gris
];

/// En oscuro el color de la nota vive en el encabezado (un tono
/// profundo del mismo color), el cuerpo es casi negro con apenas un
/// tinte, y la tinta es el pastel claro de la paleta clara — así cada
/// nota se sigue reconociendo por su color, con contraste de sobra
/// (≥ 5:1 en el encabezado, ≥ 10:1 en el cuerpo).
const DARK_PALETTE: [(u32, u32, u32); PALETTE_LEN] = [
    (rgb(0x6B, 0x55, 0x14), rgb(0x2A, 0x25, 0x15), rgb(0xFD, 0xE6, 0x8A)), // amarillo
    (rgb(0x74, 0x28, 0x4F), rgb(0x2B, 0x18, 0x22), rgb(0xFB, 0xCF, 0xE8)), // rosa
    (rgb(0x1D, 0x5E, 0x3E), rgb(0x15, 0x26, 0x1C), rgb(0xBB, 0xF7, 0xD0)), // verde
    (rgb(0x23, 0x4A, 0x80), rgb(0x15, 0x1F, 0x30), rgb(0xBF, 0xDB, 0xFE)), // azul
    (rgb(0x52, 0x30, 0x7F), rgb(0x20, 0x18, 0x30), rgb(0xE9, 0xD5, 0xFF)), // morado
    (rgb(0x3E, 0x46, 0x52), rgb(0x1E, 0x22, 0x27), rgb(0xE2, 0xE8, 0xF0)), // gris
];

pub const COLOR_NAMES: [&str; PALETTE_LEN] = ["Amarillo", "Rosa", "Verde", "Azul", "Morado", "Gris"];

/// (encabezado, cuerpo, tinta) de un color de nota, en el tema actual.
pub fn note_colors(idx: u8) -> (u32, u32, u32) {
    let i = (idx as usize) % PALETTE_LEN;
    if is_dark() {
        DARK_PALETTE[i]
    } else {
        LIGHT[i]
    }
}

// -----------------------------------------------------------------
// Colores de ventanas propias ("Todas las notas", fantasma de arrastre)
// -----------------------------------------------------------------

pub struct Chrome {
    pub window: u32,
    /// Superficies apoyadas sobre el fondo: el buscador, el botón de tema.
    pub surface: u32,
    pub surface_hover: u32,
    pub text: u32,
    pub muted: u32,
}

pub fn chrome() -> Chrome {
    if is_dark() {
        Chrome {
            window: rgb(0x1F, 0x20, 0x23),
            surface: rgb(0x2C, 0x2E, 0x33),
            surface_hover: rgb(0x38, 0x3B, 0x41),
            text: rgb(0xE5, 0xE7, 0xEB),
            muted: rgb(0x9C, 0xA3, 0xAF),
        }
    } else {
        Chrome {
            window: rgb(0xFF, 0xFF, 0xFF),
            surface: rgb(0xF1, 0xF5, 0xF9),
            surface_hover: rgb(0xE2, 0xE8, 0xF0),
            text: rgb(0x1F, 0x29, 0x37),
            muted: rgb(0x6B, 0x72, 0x80),
        }
    }
}

// -----------------------------------------------------------------
// Aplicar el tema a ventanas nativas
// -----------------------------------------------------------------

/// Barra de título clara u oscura (Windows 10 20H1+ / 11).
pub fn apply_title_bar(hwnd: HWND) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    let on: BOOL = is_dark() as BOOL;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &on as *const BOOL as *const c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

/// Barras de desplazamiento (y demás partes temables) de una ventana.
pub fn apply_scrollbars(hwnd: HWND) {
    let name = wide(if is_dark() { "DarkMode_Explorer" } else { "Explorer" });
    unsafe {
        SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
    }
}

/// Menús nativos oscuros. `SetPreferredAppMode` (ordinal 135) y
/// `FlushMenuThemes` (136) de `uxtheme.dll` no están documentados,
/// pero existen desde Windows 10 1903 y los usa medio mundo; si no
/// aparecen, los menús se quedan en claro y listo.
fn apply_menus(dark: bool) {
    unsafe {
        let lib = wide("uxtheme.dll");
        let module = LoadLibraryW(lib.as_ptr());
        if module.is_null() {
            return;
        }
        if let Some(set_mode) = GetProcAddress(module, 135 as *const u8) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            // 2 = ForceDark, 3 = ForceLight
            set_mode(if dark { 2 } else { 3 });
        }
        if let Some(flush) = GetProcAddress(module, 136 as *const u8) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}
