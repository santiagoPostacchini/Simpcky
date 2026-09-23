//! Integración con el shell de Windows vía registro (todo en
//! `HKEY_CURRENT_USER`, sin permisos de administrador):
//!
//! - "Iniciar con Windows": `...\CurrentVersion\Run`.
//! - "Nueva nota adhesiva" en el clic derecho del escritorio: un verbo
//!   estático en `Software\Classes\DesktopBackground\Shell`. Es la
//!   forma liviana (sin DLL de extensión de shell, sin COM): al
//!   elegirlo, Windows lanza `simpcky.exe --new`, y esa segunda
//!   instancia le pasa el pedido a la que ya está corriendo (ver
//!   `main.rs`). En el menú nuevo de Windows 11 aparece dentro de
//!   "Mostrar más opciones" (o directo con Mayús + clic derecho): el
//!   menú compacto solo acepta apps empaquetadas como MSIX.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Registry::*;

use crate::win::{from_wide, wide};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Simpcky";
const DESKTOP_VERB_KEY: &str = "Software\\Classes\\DesktopBackground\\Shell\\Simpcky";

fn exe_path() -> String {
    let mut buf = vec![0u16; 1024];
    let len = unsafe { GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    buf.truncate(len as usize);
    String::from_utf16_lossy(&buf)
}

/// Crea (o abre) `HKCU\<path>` y le escribe un valor de texto. `name`
/// vacío = el valor "(Predeterminado)" de la clave.
fn write_string(path: &str, name: &str, value: &str) -> bool {
    unsafe {
        let subkey = wide(path);
        let mut hkey: HKEY = null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            null(),
            &mut hkey,
            null_mut(),
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let name_w = wide(name);
        let value_w = wide(value);
        let bytes = std::slice::from_raw_parts(value_w.as_ptr() as *const u8, value_w.len() * 2);
        let ok = RegSetValueExW(
            hkey,
            if name.is_empty() { null() } else { name_w.as_ptr() },
            0,
            REG_SZ,
            bytes.as_ptr(),
            bytes.len() as u32,
        ) == ERROR_SUCCESS;
        RegCloseKey(hkey);
        ok
    }
}

fn read_string(path: &str, name: &str) -> Option<String> {
    unsafe {
        let subkey = wide(path);
        let name_w = wide(name);
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let ok = RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            if name.is_empty() { null() } else { name_w.as_ptr() },
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size,
        );
        (ok == ERROR_SUCCESS).then(|| from_wide(&buf))
    }
}

// -----------------------------------------------------------------
// Iniciar con Windows
// -----------------------------------------------------------------

fn run_command() -> String {
    format!("\"{}\"", exe_path())
}

pub fn is_autostart_enabled() -> bool {
    read_string(RUN_KEY, RUN_VALUE).is_some()
}

pub fn set_autostart(enable: bool) {
    if enable {
        write_string(RUN_KEY, RUN_VALUE, &run_command());
    } else {
        unsafe {
            let subkey = wide(RUN_KEY);
            let mut hkey: HKEY = null_mut();
            if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_WRITE, &mut hkey) == ERROR_SUCCESS {
                let value = wide(RUN_VALUE);
                RegDeleteValueW(hkey, value.as_ptr());
                RegCloseKey(hkey);
            }
        }
    }
}

/// Si el autoarranque está activado pero apunta a otro lugar (el
/// `.exe` se movió o se recompiló en otra carpeta), lo corrige.
pub fn refresh_autostart_path() {
    if let Some(current) = read_string(RUN_KEY, RUN_VALUE) {
        if current != run_command() {
            set_autostart(true);
        }
    }
}

// -----------------------------------------------------------------
// "Nueva nota adhesiva" en el clic derecho del escritorio
// -----------------------------------------------------------------

/// Registra (o actualiza la ruta de) la entrada del menú del escritorio.
pub fn register_desktop_menu() {
    let exe = exe_path();
    write_string(DESKTOP_VERB_KEY, "", "Nueva nota adhesiva");
    write_string(DESKTOP_VERB_KEY, "MUIVerb", "Nueva nota adhesiva");
    write_string(DESKTOP_VERB_KEY, "Icon", &format!("\"{exe}\",0"));
    write_string(&format!("{DESKTOP_VERB_KEY}\\command"), "", &format!("\"{exe}\" --new"));
}

pub fn unregister_desktop_menu() {
    unsafe {
        let key = wide(DESKTOP_VERB_KEY);
        RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr());
    }
}
