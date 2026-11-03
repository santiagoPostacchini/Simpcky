//! Lo poco que Raven no ofrece y la demo toma directo de Win32.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() }

/// Nota "en el escritorio": poseída por la ventana del escritorio
/// (`Progman`), como las notas privadas de Simpcky (`desktop::own`). Queda
/// detrás de las aplicaciones y fuera de la barra de tareas. Raven dibuja
/// con Vulkan en una ventana de primer nivel, así que el anclaje como hija
/// del escritorio (`WS_CHILD` de Simpcky) no está disponible.
pub fn own_by_desktop(hwnd: usize, on_desktop: bool) {
    unsafe {
        let hwnd = hwnd as HWND;
        let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
        let owner = if on_desktop { progman as isize } else { 0 };
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner);
        // Fuera de la barra de tareas mientras está en el escritorio.
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let ex = if on_desktop { (ex | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW } else { (ex & !WS_EX_TOOLWINDOW) | WS_EX_APPWINDOW };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
}

/// Esquinas redondeadas de Windows 11, como las notas de Simpcky.
pub fn round_corners(hwnd: usize) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    unsafe {
        let pref = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd as HWND, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const _ as *const _, 4);
    }
}

/// (x, y, ancho) de la ventana en pantalla.
pub fn window_rect(hwnd: usize) -> Option<(i32, i32, i32)> {
    unsafe {
        let mut r: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
        (GetWindowRect(hwnd as HWND, &mut r) != 0).then_some((r.left, r.top, r.right - r.left))
    }
}

/// ¿Está el cursor sobre la ventana?
pub fn cursor_inside(hwnd: usize) -> bool {
    unsafe {
        let mut p: windows_sys::Win32::Foundation::POINT = std::mem::zeroed();
        let mut r: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
        GetCursorPos(&mut p) != 0
            && GetWindowRect(hwnd as HWND, &mut r) != 0
            && p.x >= r.left
            && p.x < r.right
            && p.y >= r.top
            && p.y < r.bottom
    }
}

/// (memoria en uso, memoria privada) del proceso, en bytes.
pub fn process_memory() -> (usize, usize) {
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb) != 0 {
            (pmc.WorkingSetSize, pmc.PrivateUsage)
        } else {
            (0, 0)
        }
    }
}
