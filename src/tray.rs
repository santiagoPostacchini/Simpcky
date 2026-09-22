//! Icono y menú de la bandeja del sistema, más la ventana "controladora"
//! oculta que los recibe (sin ella, Shell_NotifyIcon no tiene a quién
//! avisarle de los clics).

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{app, save_all};
use crate::note;
use crate::persist::{NoteData, RollMode};
use crate::win::wide;

const CONTROLLER_CLASS: &str = "SimpckyController";
const WM_TRAYICON: u32 = WM_APP + 1;
const TRAY_UID: u32 = 1;

const ID_NEW_NOTE: u32 = 1000;
const ID_ROLL_DEFAULT_MANUAL: u32 = 1001;
const ID_ROLL_DEFAULT_AUTO: u32 = 1002;
const ID_AUTOSTART: u32 = 1003;
const ID_EXIT: u32 = 1004;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Simpcky";

/// Registra la clase de la ventana controladora oculta.
pub fn register_class(hinstance: HINSTANCE) {
    unsafe {
        let class_name = wide(CONTROLLER_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(controller_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

/// Crea la ventana controladora (invisible) y el icono de la bandeja.
/// Devuelve su HWND.
pub fn init(hinstance: HINSTANCE) -> HWND {
    let class_name = wide(CONTROLLER_CLASS);
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            hinstance,
            null(),
        )
    };
    if !hwnd.is_null() {
        add_tray_icon(hwnd);
        app().lock().unwrap().controller_hwnd = hwnd as isize;
    }
    hwnd
}

fn set_wide_buf(dst: &mut [u16], s: &str) {
    let w = wide(s);
    let n = w.len().min(dst.len());
    dst[..n].copy_from_slice(&w[..n]);
    if n < dst.len() {
        dst[n] = 0;
    } else if n > 0 {
        dst[n - 1] = 0;
    }
}

fn add_tray_icon(hwnd: HWND) {
    unsafe {
        let icon = LoadIconW(null_mut(), IDI_APPLICATION);
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = icon;
        set_wide_buf(&mut nid.szTip, "Simpcky — notas adhesivas");
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn remove_tray_icon(hwnd: HWND) {
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Crea una nota nueva en una posición en cascada y la persiste.
pub fn spawn_new_note() {
    let id = crate::app::next_note_id();
    let (x, y, roll_mode) = {
        let a = app().lock().unwrap();
        let step = (a.notes.len() as i32) % 8;
        (120 + step * 28, 120 + step * 28, a.default_roll_mode)
    };
    let color = ((id.saturating_sub(1)) % note::PALETTE.len() as u32) as u8;
    let data = NoteData::new(id, x, y, color, roll_mode);
    let hwnd = note::create_note(data);
    if !hwnd.is_null() {
        save_all();
    }
}

fn show_tray_menu(hwnd: HWND) {
    let manual = { app().lock().unwrap().default_roll_mode == RollMode::Manual };
    let autostart_on = is_autostart_enabled();

    unsafe {
        let menu = CreatePopupMenu();

        let new_note = wide("Nueva nota");
        AppendMenuW(menu, MF_STRING, ID_NEW_NOTE as usize, new_note.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        let roll_menu = CreatePopupMenu();
        let manual_label = wide("Manual (predeterminado)");
        let auto_label = wide("Auto");
        AppendMenuW(
            roll_menu,
            MF_STRING | if manual { MF_CHECKED } else { MF_UNCHECKED },
            ID_ROLL_DEFAULT_MANUAL as usize,
            manual_label.as_ptr(),
        );
        AppendMenuW(
            roll_menu,
            MF_STRING | if !manual { MF_CHECKED } else { MF_UNCHECKED },
            ID_ROLL_DEFAULT_AUTO as usize,
            auto_label.as_ptr(),
        );
        let roll_label = wide("Enrollar notas nuevas");
        AppendMenuW(menu, MF_POPUP, roll_menu as usize, roll_label.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        let autostart_label = wide("Iniciar con Windows");
        AppendMenuW(
            menu,
            MF_STRING | if autostart_on { MF_CHECKED } else { MF_UNCHECKED },
            ID_AUTOSTART as usize,
            autostart_label.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        let exit_label = wide("Salir");
        AppendMenuW(menu, MF_STRING, ID_EXIT as usize, exit_label.as_ptr());

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_LEFTALIGN, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}

// -----------------------------------------------------------------
// Iniciar con Windows (HKCU\...\Run)
// -----------------------------------------------------------------

fn exe_path_wide() -> Vec<u16> {
    let mut buf = vec![0u16; 512];
    let len = unsafe { GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    buf.truncate(len.max(0) as usize);
    buf.push(0);
    buf
}

fn is_autostart_enabled() -> bool {
    unsafe {
        let subkey = wide(RUN_KEY);
        let mut hkey: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != ERROR_SUCCESS {
            return false;
        }
        let value = wide(RUN_VALUE);
        let mut kind: u32 = 0;
        let mut size: u32 = 0;
        let ok = RegQueryValueExW(hkey, value.as_ptr(), null(), &mut kind, null_mut(), &mut size) == ERROR_SUCCESS;
        RegCloseKey(hkey);
        ok
    }
}

fn toggle_autostart() {
    let enable = !is_autostart_enabled();
    unsafe {
        let subkey = wide(RUN_KEY);
        let mut hkey: HKEY = null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_READ,
            null(),
            &mut hkey,
            null_mut(),
        ) != ERROR_SUCCESS
        {
            return;
        }
        let value = wide(RUN_VALUE);
        if enable {
            let path = exe_path_wide();
            let bytes = std::slice::from_raw_parts(path.as_ptr() as *const u8, path.len() * 2);
            RegSetValueExW(hkey, value.as_ptr(), 0, REG_SZ, bytes.as_ptr(), bytes.len() as u32);
        } else {
            RegDeleteValueW(hkey, value.as_ptr());
        }
        RegCloseKey(hkey);
    }
}

fn handle_command(hwnd: HWND, id: u32) {
    match id {
        ID_NEW_NOTE => spawn_new_note(),
        ID_ROLL_DEFAULT_MANUAL => {
            app().lock().unwrap().default_roll_mode = RollMode::Manual;
        }
        ID_ROLL_DEFAULT_AUTO => {
            app().lock().unwrap().default_roll_mode = RollMode::Auto;
        }
        ID_AUTOSTART => toggle_autostart(),
        ID_EXIT => {
            note::flush_all();
            unsafe { DestroyWindow(hwnd) };
        }
        _ => {}
    }
}

unsafe extern "system" fn controller_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            match lparam as u32 {
                WM_LBUTTONUP => spawn_new_note(),
                WM_RBUTTONUP => show_tray_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            handle_command(hwnd, (wparam & 0xffff) as u32);
            0
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
