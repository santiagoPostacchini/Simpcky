//! Lo de Win32 que usan las dos demos.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

pub fn contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

pub fn cursor() -> POINT {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    pt
}

pub fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL as i32) < 0 }
}

pub fn esc_down() -> bool {
    unsafe { GetAsyncKeyState(VK_ESCAPE as i32) < 0 }
}

/// ¿Sigue apretado el botón con el que se arrastra? (El izquierdo, o el
/// derecho si están invertidos.)
pub fn button_down() -> bool {
    let vk = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// Monitor y área de trabajo (sin la barra de tareas) en (`x`, `y`).
pub fn monitor_of(x: i32, y: i32) -> (HMONITOR, RECT) {
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(m, &mut info);
        (m, info.rcWork)
    }
}

pub fn window_rect(hwnd: HWND) -> RECT {
    let mut r = rect(0, 0, 0, 0);
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// ¿Están prendidos los efectos de animación de Windows?
pub fn animations_enabled() -> bool {
    let mut on: BOOL = 1;
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on as *mut BOOL as *mut _, 0) };
    on != 0
}

/// Lo que ocupan las ventanas de aplicaciones a la vista (como `snap::app_windows`).
pub fn app_windows() -> Vec<RECT> {
    unsafe extern "system" fn each(hwnd: HWND, list: LPARAM) -> BOOL {
        (*(list as *mut Vec<HWND>)).push(hwnd);
        1
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(each), &mut all as *mut Vec<HWND> as LPARAM) };
    let me = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() };
    let shell = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
    all.into_iter()
        .filter_map(|h| unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            let mut pid = 0;
            GetWindowThreadProcessId(h, &mut pid);
            let mut cloaked: u32 = 0;
            DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut _, 4);
            let mut class = [0u16; 64];
            let n = GetClassNameW(h, class.as_mut_ptr(), 64) as usize;
            let class = String::from_utf16_lossy(&class[..n]);
            if IsWindowVisible(h) == 0 || IsIconic(h) != 0 || ex & WS_EX_TOOLWINDOW != 0 || ex & WS_EX_TRANSPARENT != 0 || cloaked != 0 || pid == me || shell.contains(&class.as_str()) {
                return None;
            }
            let mut r = rect(0, 0, 0, 0);
            if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32) != 0 {
                GetWindowRect(h, &mut r);
            }
            (r.right > r.left && r.bottom > r.top).then_some(r)
        })
        .collect()
}

// -----------------------------------------------------------------
// Ventanas de las notas
// -----------------------------------------------------------------

/// Nota "en el escritorio": poseída por `Progman` (como el modo vidrio de
/// Simpcky): queda detrás de las aplicaciones, fuera de la barra de tareas.
/// Con `false`, ventana "siempre encima".
pub fn set_layer(hwnd: HWND, on_desktop: bool) {
    unsafe {
        let progman = FindWindowW(wide("Progman").as_ptr(), null_mut());
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, if on_desktop { progman as isize } else { 0 });
        let after = if on_desktop { HWND_NOTOPMOST } else { HWND_TOPMOST };
        SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Esquinas redondeadas de Windows 11.
pub fn round_corners(hwnd: HWND, round: bool) {
    let pref: i32 = if round { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
    unsafe { DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const i32 as *const _, 4) };
}

#[repr(C)]
struct AccentPolicy {
    state: i32,
    flags: i32,
    color: u32,
    animation: i32,
}

#[repr(C)]
struct CompositionData {
    attribute: i32,
    data: *mut core::ffi::c_void,
    size: usize,
}

type SetWca = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

/// Desenfoque de Windows detrás de la ventana, sin Windhawk.
///
/// `SetWindowCompositionAttribute(WCA_ACCENT_POLICY)` con
/// `ACCENT_ENABLE_BLURBEHIND` y **flags 0**: con flags 2 (usar el color de
/// tinte, lo que suelen copiar los ejemplos) Windows 11 25H2 pinta casi
/// negro. Anda con ventanas en capas (`UpdateLayeredWindow`) y con
/// ventanas normales de marco extendido; no con ventanas hijas. El color
/// de la nota (con su alfa) se pinta encima, premultiplicado.
pub fn set_blur(hwnd: HWND, on: bool) -> bool {
    unsafe {
        let user32 = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide("user32.dll").as_ptr());
        let Some(f) = windows_sys::Win32::System::LibraryLoader::GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr() as *const u8) else {
            return false;
        };
        let f: SetWca = std::mem::transmute(f);
        let mut policy = AccentPolicy { state: if on { 3 } else { 0 }, flags: 0, color: 0, animation: 0 };
        let mut data = CompositionData { attribute: 19, data: &mut policy as *mut AccentPolicy as *mut _, size: std::mem::size_of::<AccentPolicy>() };
        f(hwnd, &mut data) != 0
    }
}

/// Para ventanas que no son en capas: el contenido con alfa se compone
/// sobre el desenfoque (marco extendido a toda la ventana).
pub fn extend_frame(hwnd: HWND) {
    let m = windows_sys::Win32::UI::Controls::MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &m) };
}

/// (memoria en uso, memoria privada) del proceso.
pub fn process_memory() -> (usize, usize) {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if K32GetProcessMemoryInfo(windows_sys::Win32::System::Threading::GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb) != 0 {
            (pmc.WorkingSetSize, pmc.PrivateUsage)
        } else {
            (0, 0)
        }
    }
}
