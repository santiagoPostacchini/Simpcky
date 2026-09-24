//! Icono y menú de la bandeja del sistema, más la ventana "controladora"
//! oculta que los recibe (sin ella, Shell_NotifyIcon no tiene a quién
//! avisarle de los clics).
//!
//! La controladora es una ventana de nivel superior invisible, no una
//! ventana "solo mensajes" (`HWND_MESSAGE`) como antes: esas no reciben
//! mensajes de difusión, y hay dos que importan mucho:
//! - `TaskbarCreated`: Explorer se reinició. Hay que volver a poner el
//!   icono en la bandeja y recrear las notas (Explorer se llevó puesto
//!   a Progman, y con él a todas las notas ancladas).
//! - `WM_QUERYENDSESSION` / `WM_ENDSESSION`: Windows se apaga o cierra
//!   la sesión. Sin responder, la app quedaba como "no responde" y
//!   Windows la mataba (el evento "Quiesce" del registro de eventos),
//!   sin guardar lo último escrito.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{app, save_all, save_settings};
use crate::note;
use crate::persist::{NoteData, RollMode};
use crate::shell;
use crate::win::wide;

pub const CONTROLLER_CLASS: &str = "SimpckyController";
const WM_TRAYICON: u32 = WM_APP + 1;
/// Otra instancia (lanzada desde el clic derecho del escritorio, con
/// `--new`) pide una nota nueva en (`wparam`, `lparam`), en
/// coordenadas de pantalla.
pub const WM_APP_NEW_AT: u32 = WM_APP + 2;
/// Otra instancia (lanzada sin argumentos: doble clic en el .exe con
/// la app ya abierta) pide mostrar "Todas las notas".
pub const WM_APP_SHOW_ALL: u32 = WM_APP + 3;
/// Otra instancia lanzada con `--quit` (el instalador) pide cerrar.
pub const WM_APP_QUIT: u32 = WM_APP + 5;
/// Alguna nota se quedó sin ventana (ver `note::on_destroy`).
const WM_APP_RECREATE: u32 = WM_APP + 4;
const TRAY_UID: u32 = 1;
const TIMER_RECREATE: usize = 1;
/// Las notas no se pudieron guardar (ver `app::save_all`): wParam 1 si
/// empezó a fallar, 0 si volvió a andar.
pub const WM_APP_SAVE_STATE: u32 = WM_APP + 6;
/// Mirar si el mod de Windhawk se prendió o se apagó (ver `glass.rs`).
const WM_APP_GLASS_CHECK: u32 = WM_APP + 7;

/// Que se mire, en cola y no más de una vez cada unos segundos (lo piden
/// los vigilantes de todas las notas).
pub fn check_glass_soon() {
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap();
        if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)) {
            return;
        }
        *last = Some(std::time::Instant::now());
    }
    let hwnd = app().lock().unwrap().controller_hwnd as HWND;
    if !hwnd.is_null() {
        unsafe { PostMessageW(hwnd, WM_APP_GLASS_CHECK, 0, 0) };
    }
}
/// Mientras no se puede guardar, se reintenta solo.
const TIMER_SAVE_RETRY: usize = 40;
const SAVE_RETRY_MS: u32 = 30_000;

const ID_NEW_NOTE: u32 = 1000;
const ID_EXIT: u32 = 1004;
pub const ID_ALL_NOTES: u32 = 1005;
/// "Configuración y sincronización…" (la pestaña de "Todas las notas").
const ID_SETTINGS: u32 = 1008;
const ID_UPDATE_INSTALL: u32 = 1020;
const ID_SAVE_RETRY: u32 = 1030;

/// El atajo global del selector de notas (ver `picker.rs`).
const HOTKEY_PICKER: i32 = 1;
/// No se pudo registrar el atajo elegido (lo usa otra aplicación).
static HOTKEY_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Registra el atajo elegido en Configuración (o ninguno).
pub fn apply_hotkey() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};
    let (hwnd, choice) = {
        let a = app().lock().unwrap();
        (a.controller_hwnd as HWND, a.settings.hotkey)
    };
    if hwnd.is_null() {
        return;
    }
    unsafe { UnregisterHotKey(hwnd, HOTKEY_PICKER) };
    // Nada con la tecla Windows: Windows y Office ya usan casi todas
    // (Win+Mayús+N abre OneNote).
    let keys = match choice {
        2 => Some((MOD_CONTROL | MOD_ALT, b'N' as u32)),
        3 => Some((MOD_CONTROL | MOD_ALT, 0x20)), // VK_SPACE
        _ => None,
    };
    let ok = keys.is_none_or(|(mods, vk)| unsafe { RegisterHotKey(hwnd, HOTKEY_PICKER, mods | MOD_NOREPEAT, vk) } != 0);
    HOTKEY_FAILED.store(!ok, std::sync::atomic::Ordering::Relaxed);
}

pub fn hotkey_failed() -> bool {
    HOTKEY_FAILED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_hotkey(choice: u8) {
    app().lock().unwrap().settings.hotkey = choice;
    save_settings();
    apply_hotkey();
}

fn taskbar_created_msg() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static MSG: AtomicU32 = AtomicU32::new(0);
    let cached = MSG.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let name = wide("TaskbarCreated");
    let id = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    MSG.store(id, Ordering::Relaxed);
    id
}

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
            hIcon: crate::icon::app_icon_large(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: crate::icon::app_icon(),
        };
        RegisterClassExW(&wc);
    }
}

/// Crea la ventana controladora (invisible) y el icono de la bandeja.
/// Devuelve su HWND.
pub fn init(hinstance: HINSTANCE) -> HWND {
    let class_name = wide(CONTROLLER_CLASS);
    let title = wide("Simpcky");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP, // sin WS_VISIBLE: nunca se muestra
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        )
    };
    if !hwnd.is_null() {
        // Sin esto, un proceso con menos privilegios (o el propio
        // Explorer en algunas configuraciones) no puede hacernos llegar
        // TaskbarCreated.
        unsafe { ChangeWindowMessageFilterEx(hwnd, taskbar_created_msg(), MSGFLT_ALLOW, null_mut()) };
        add_tray_icon(hwnd);
        app().lock().unwrap().controller_hwnd = hwnd as isize;
        apply_hotkey();
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
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = crate::icon::app_icon();
        set_wide_buf(&mut nid.szTip, "Simpcky — notas adhesivas");
        // Si ya estaba (NIM_ADD falla), se actualiza.
        if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

/// Qué hacer si el usuario hace clic en la notificación que se está
/// mostrando (solo algunas tienen acción: "hay una versión nueva").
static BALLOON_ACTION: std::sync::Mutex<Option<fn()>> = std::sync::Mutex::new(None);

/// Aviso tipo globo desde el ícono de la bandeja (Windows lo muestra
/// como notificación del sistema).
pub fn notify(title: &str, text: &str) {
    *BALLOON_ACTION.lock().unwrap() = None;
    show_balloon(title, text);
}

/// Como `notify`, pero con algo que hacer si le hacen clic.
pub fn notify_action(title: &str, text: &str, action: fn()) {
    *BALLOON_ACTION.lock().unwrap() = Some(action);
    show_balloon(title, text);
}

fn show_balloon(title: &str, text: &str) {
    let hwnd = { app().lock().unwrap().controller_hwnd } as HWND;
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO;
        set_wide_buf(&mut nid.szInfoTitle, title);
        set_wide_buf(&mut nid.szInfo, text);
        Shell_NotifyIconW(NIM_MODIFY, &nid);
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

/// Empezó a fallar el guardado de las notas, o volvió a andar. Que se
/// note: un aviso de Windows, un ícono en la barra de cada nota y un
/// ítem en el menú de la bandeja; y mientras tanto se reintenta solo.
fn on_save_state(hwnd: HWND, failing: bool) {
    unsafe {
        if failing {
            SetTimer(hwnd, TIMER_SAVE_RETRY, SAVE_RETRY_MS, None);
        } else {
            KillTimer(hwnd, TIMER_SAVE_RETRY);
        }
    }
    if failing {
        let text = if crate::sync::is_connected() {
            "Algo (seguramente el antivirus) no deja escribir en el disco. Lo que escribís se sigue sincronizando con Google Drive, y Simpcky reintenta guardar solo."
        } else {
            "Algo (seguramente el antivirus) no deja escribir en el disco: no cierres Simpcky hasta que vuelva a guardar. Reintenta solo cada 30 segundos."
        };
        notify("Simpcky no puede guardar tus notas", text);
    } else {
        notify("Simpcky volvió a guardar tus notas", "Ya está todo guardado en el disco.");
    }
    note::repaint_headers();
}

/// Pide (en cola) que se recreen las notas que se quedaron sin ventana.
pub fn request_recreate() {
    let hwnd = { app().lock().unwrap().controller_hwnd } as HWND;
    if !hwnd.is_null() {
        unsafe { PostMessageW(hwnd, WM_APP_RECREATE, 0, 0) };
    }
}

// -----------------------------------------------------------------
// Notas nuevas
// -----------------------------------------------------------------

/// Nota nueva desde la bandeja (o Ctrl+N): en la primera posición de
/// la cascada que no esté ocupada por otra nota. Antes la posición
/// salía de "cuántas notas hay", y la nota nueva caía justo encima (o
/// debajo) de una vieja.
pub fn spawn_new_note() {
    let taken: Vec<(i32, i32)> = {
        let a = app().lock().unwrap();
        a.notes.values().map(|nr| (nr.data.x, nr.data.y)).collect()
    };
    let (base_x, base_y) = unsafe {
        let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut core::ffi::c_void, 0);
        (work.left + 120, work.top + 100)
    };
    let mut pos = (base_x, base_y);
    for step in 0..24 {
        let candidate = (base_x + step * 32, base_y + step * 32);
        let busy = taken.iter().any(|&(x, y)| (x - candidate.0).abs() < 16 && (y - candidate.1).abs() < 16);
        pos = candidate;
        if !busy {
            break;
        }
    }
    spawn_note_at(pos.0, pos.1);
}

/// Crea una nota con su esquina superior izquierda cerca de (`x`, `y`)
/// (coordenadas de pantalla), ajustada para que entre entera en el
/// monitor, y la muestra al frente con el cursor listo para escribir.
///
/// Mostrarla al frente es lo que faltaba: una nota nueva es un widget
/// de escritorio, y el escritorio está DETRÁS de todas las ventanas.
/// Se creaba bien (quedaba en `notes.json`) pero nacía tapada por lo
/// que hubiera abierto — "Nueva nota" parecía no hacer nada. Ahora se
/// asoma adelante (`note::open_note`) y vuelve sola a su lugar en el
/// escritorio apenas se hace clic en otra cosa.
pub fn spawn_note_at(x: i32, y: i32) {
    let id = crate::app::next_note_id();
    let roll_mode = crate::app::default_roll_mode();
    let color = ((id.saturating_sub(1)) % crate::theme::PALETTE_LEN as u32) as u8;
    let mut data = NoteData::new(id, x, y, color, roll_mode);
    (data.w, data.h) = note::default_size(x, y);
    let (cx, cy) = note::clamp_to_work_area(x, y, data.w, data.h);
    data.x = cx;
    data.y = cy;
    let hwnd = note::create_note(data);
    if !hwnd.is_null() {
        save_all();
        note::open_note(hwnd);
    }
}

/// Nota nueva desde el clic derecho del escritorio: con el encabezado
/// justo bajo el cursor, como si se la hubiera "sacado" de ahí.
pub fn spawn_note_near_cursor(x: i32, y: i32) {
    spawn_note_at(x - 24, y - 16);
}

// -----------------------------------------------------------------
// Menú
// -----------------------------------------------------------------

/// El menú de la bandeja: lo de todos los días. Las preferencias y la
/// cuenta de Google están en "Configuración y sincronización" (ver
/// `settings.rs`).
fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();

        // Si no se puede guardar, es lo primero que tiene que verse.
        if crate::app::save_failing() {
            let label = wide("\u{26A0} No se pueden guardar las notas: reintentar");
            AppendMenuW(menu, MF_STRING, ID_SAVE_RETRY as usize, label.as_ptr());
            SetMenuDefaultItem(menu, ID_SAVE_RETRY, 0);
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
        }

        // Una actualización pendiente va primero y en negrita: es lo
        // único del menú que el usuario no sabe que existe.
        if let Some(v) = crate::update::available() {
            let label = wide(&format!("Actualizar a la versión {}…", crate::update::version_text(v)));
            AppendMenuW(menu, MF_STRING, ID_UPDATE_INSTALL as usize, label.as_ptr());
            SetMenuDefaultItem(menu, ID_UPDATE_INSTALL, 0);
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
        }

        let new_note = wide("Nueva nota\tCtrl+N");
        AppendMenuW(menu, MF_STRING, ID_NEW_NOTE as usize, new_note.as_ptr());
        let all_notes = wide("Todas las notas");
        AppendMenuW(menu, MF_STRING, ID_ALL_NOTES as usize, all_notes.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        let settings = wide("Configuración y sincronización…");
        AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, settings.as_ptr());
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

pub fn set_default_roll_mode(mode: RollMode) {
    app().lock().unwrap().settings.default_roll_mode = mode;
    save_settings();
}

/// Transparencia con Windhawk: 0 (apagada) … 3 (fuerte).
pub fn set_translucency(level: u8) {
    app().lock().unwrap().settings.translucency = level.min(3);
    crate::app::save_settings();
    if crate::glass::update() {
        note::recreate_all();
    } else {
        note::refresh_glass();
    }
}

pub fn toggle_desktop_menu() {
    let enable = {
        let mut a = app().lock().unwrap();
        a.settings.desktop_menu = !a.settings.desktop_menu;
        a.settings.desktop_menu
    };
    if enable {
        shell::register_desktop_menu();
    } else {
        shell::unregister_desktop_menu();
    }
    save_settings();
}

/// Salir de verdad: guardar todo, sacar el icono, terminar el bucle de
/// mensajes. Las ventanas de las notas se van con el proceso (sin
/// WM_DESTROY), así que sus datos quedan intactos en disco.
/// Salir, desde otro módulo (el actualizador, antes de instalar).
pub fn quit_app() {
    let hwnd = { app().lock().unwrap().controller_hwnd } as HWND;
    if !hwnd.is_null() {
        exit_app(hwnd);
    }
}

fn exit_app(hwnd: HWND) {
    crate::app::set_shutting_down();
    note::flush_all();
    unsafe { DestroyWindow(hwnd) };
}

fn handle_command(hwnd: HWND, id: u32) {
    match id {
        ID_NEW_NOTE => spawn_new_note(),
        ID_ALL_NOTES => crate::allnotes::show(),
        ID_SETTINGS => crate::allnotes::show_settings(),
        ID_EXIT => exit_app(hwnd),
        ID_UPDATE_INSTALL => crate::update::install(),
        ID_SAVE_RETRY => crate::app::save_all(),
        _ => {}
    }
}

unsafe extern "system" fn controller_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            match lparam as u32 {
                WM_LBUTTONUP => {
                    // Igual que antes de abrir el menú: pasar al frente
                    // mientras el clic en la bandeja nos da permiso, así
                    // la nota nueva puede tomar el foco después.
                    SetForegroundWindow(hwnd);
                    spawn_new_note();
                }
                WM_RBUTTONUP => show_tray_menu(hwnd),
                NIN_BALLOONUSERCLICK => {
                    let action = BALLOON_ACTION.lock().unwrap().take();
                    if let Some(action) = action {
                        action();
                    }
                }
                NIN_BALLOONTIMEOUT | NIN_BALLOONHIDE => {
                    *BALLOON_ACTION.lock().unwrap() = None;
                }
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            handle_command(hwnd, (wparam & 0xffff) as u32);
            0
        }
        WM_APP_NEW_AT => {
            spawn_note_near_cursor(wparam as isize as i32, lparam as i32);
            0
        }
        WM_APP_SHOW_ALL => {
            crate::allnotes::show();
            0
        }
        WM_APP_RECREATE => {
            // Con un respiro: si fue Explorer reiniciándose, Progman
            // tarda un momento en volver a existir.
            SetTimer(hwnd, TIMER_RECREATE, 1500, None);
            0
        }
        crate::sync::WM_APP_SYNC_DONE => {
            crate::sync::on_done(lparam);
            0
        }
        crate::sync::WM_APP_SIGNIN_DONE => {
            crate::sync::on_sign_in_done(lparam);
            0
        }
        crate::update::WM_APP_UPDATE_CHECKED => {
            crate::update::on_checked(wparam, lparam);
            0
        }
        crate::update::WM_APP_UPDATE_DOWNLOADED => {
            crate::update::on_downloaded(lparam);
            0
        }
        WM_TIMER if wparam == crate::update::TIMER_UPDATE_CHECK => {
            crate::update::on_timer();
            0
        }
        WM_TIMER if wparam == crate::sync::TIMER_SYNC_SOON || wparam == crate::sync::TIMER_SYNC_POLL => {
            crate::sync::on_timer(wparam);
            0
        }
        WM_HOTKEY => {
            if wparam as i32 == HOTKEY_PICKER {
                crate::picker::toggle();
            }
            0
        }
        WM_APP_GLASS_CHECK => {
            if crate::glass::update() {
                note::recreate_all();
            }
            0
        }
        WM_APP_SAVE_STATE => {
            on_save_state(hwnd, wparam != 0);
            0
        }
        WM_TIMER if wparam == TIMER_SAVE_RETRY => {
            crate::app::save_all();
            0
        }
        WM_TIMER if wparam == TIMER_RECREATE => {
            let progman = wide("Progman");
            if !FindWindowW(progman.as_ptr(), null()).is_null() {
                KillTimer(hwnd, TIMER_RECREATE);
                note::recreate_lost();
            }
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wparam != 0 {
                crate::app::set_shutting_down();
                note::flush_all();
                // ENDSESSION_CLOSEAPP: no es Windows apagándose sino un
                // instalador que necesita el .exe libre (Restart
                // Manager). Ahí hay que cerrarse de verdad.
                if lparam as u32 & ENDSESSION_CLOSEAPP != 0 {
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_APP_QUIT => {
            exit_app(hwnd);
            0
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ if msg == taskbar_created_msg() => {
            // Explorer se reinició: icono de vuelta a la bandeja, y las
            // notas que se quedaron sin ventana, de vuelta al escritorio.
            add_tray_icon(hwnd);
            SetTimer(hwnd, TIMER_RECREATE, 500, None);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
