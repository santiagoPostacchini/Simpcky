// Sin consola: Simpcky es una app de bandeja, no una CLI. (Salvo al
// correr los tests, que necesitan la consola para mostrar resultados.)
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod allnotes;
mod app;
mod backdrop;
mod crypto;
mod d2d;
mod desktop;
mod drive;
mod editor;
mod flyout;
mod ghost;
mod http;
mod icon;
mod json;
mod note;
mod oauth;
mod persist;
mod richtext;
mod rename;
mod shell;
mod sync;
mod theme;
mod tray;
mod update;
mod welcome;
mod win;

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, POINT};
use windows_sys::Win32::Graphics::GdiPlus::{GdiplusStartup, GdiplusStartupInput};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{CreateMutexW, OpenProcess, Sleep, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, GetWindowThreadProcessId,
    PostMessageW, TranslateMessage, MSG, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use app::{init_app, save_all};
use persist::{NoteData, RollMode, Settings};
use win::wide;

fn main() {
    // Antes que nada: con esto las coordenadas son píxeles reales en
    // cualquier monitor (también las del cursor que se le pasa a la
    // otra instancia). Sin esto Windows escala las ventanas por
    // nosotros y el dibujo a mano queda borroso o desalineado.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    // `--new`: lo manda "Nueva nota adhesiva" del clic derecho del
    // escritorio (ver `shell.rs`).
    let want_new = std::env::args().skip(1).any(|a| a == "--new");
    // `--quit`: lo usa el instalador (y el desinstalador) para cerrar la
    // app abierta de forma ordenada — guardando lo que se esté
    // escribiendo — antes de reemplazar o borrar el .exe.
    let want_quit = std::env::args().skip(1).any(|a| a == "--quit");
    // `--welcome`: abrir la bienvenida aunque no sea la primera vez.
    let want_welcome = std::env::args().skip(1).any(|a| a == "--welcome");

    // Una sola instancia. Una segunda copia cargaría las mismas notas
    // otra vez (todas duplicadas en pantalla) y las dos se pisarían al
    // guardar. Si ya hay una corriendo, se le pasa el pedido y listo.
    if !claim_single_instance() {
        if want_quit {
            quit_running_instance();
        } else {
            forward_to_running_instance(want_new);
        }
        return;
    }
    if want_quit {
        return; // no había ninguna abierta: nada que cerrar
    }

    unsafe {
        // GDI+ (gdiplus.dll, incluido en Windows desde XP) se usa para
        // todo lo que necesita antialiasing: íconos del encabezado,
        // tarjetas de "Todas las notas", el fantasma de arrastre. No
        // hace falta apagarlo: el proceso lo libera al salir.
        let mut gdiplus_token: usize = 0;
        let gdiplus_input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        GdiplusStartup(&mut gdiplus_token, &gdiplus_input, null_mut());
    }

    // Ajustes: los guardados, o (la primera vez) el tema que ya usa
    // Windows para las apps.
    let saved = persist::load_settings();
    let first_settings = saved.is_none();
    let settings = saved.unwrap_or(Settings {
        dark: theme::system_prefers_dark(),
        default_roll_mode: RollMode::Manual,
        desktop_menu: true,
        auto_update: true,
        translucency: 2,
    });
    theme::init(settings.dark);

    let hinstance = unsafe { GetModuleHandleW(null_mut()) } as HINSTANCE;
    init_app(hinstance as isize, settings, persist::load_sync_state());
    if first_settings {
        app::save_settings();
    }

    // Integración con el shell. Se reescribe en cada arranque a
    // propósito: si el .exe cambió de carpeta, las rutas se corrigen
    // solas.
    if settings.desktop_menu {
        shell::register_desktop_menu();
    }
    shell::refresh_autostart_path();

    note::load_richedit_library();
    note::register_class(hinstance);
    flyout::register_class(hinstance);
    tray::register_class(hinstance);
    allnotes::register_class(hinstance);
    welcome::register_class(hinstance);

    // El fondo desenfocado de las notas translúcidas, antes de que haya
    // notas que pintar (armarlo le pide el fondo de pantalla a Explorer).
    if app::app().lock().unwrap().settings.translucency > 0 {
        backdrop::warm();
    }

    load_or_create_notes();
    // Lo que se acaba de cargar es el punto de partida para detectar
    // cambios. Y se guarda enseguida: las notas de antes de la
    // sincronización recibieron su identidad al cargarse, y tiene que
    // quedar fija (si no, en cada arranque serían "otras" notas).
    app::init_saved_parts();
    save_all();
    tray::init(hinstance);
    sync::on_startup();
    update::on_startup();

    // Primera vez en esta compu (no había ajustes guardados): la
    // bienvenida, con lo que conviene decidir de entrada.
    if first_settings || want_welcome {
        welcome::show();
    }

    if want_new {
        // La app no estaba abierta y la arrancó el clic derecho del
        // escritorio: la nota va donde está el cursor.
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        tray::spawn_note_near_cursor(pt.x, pt.y);
    }

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let ret = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if ret <= 0 {
            break;
        }
        // Los atajos se miran acá, antes de repartir el mensaje: el
        // foco de una nota vive en su RichEdit, que si no se quedaría
        // con la tecla y nunca llegaría a la ventana de la nota.
        // El bit 30 de lParam marca la repetición automática de una
        // tecla mantenida apretada: sin mirarlo, dejar Ctrl+N apretado
        // medio segundo creaba una docena de notas.
        let repeat = (msg.lParam >> 30) & 1 != 0;
        // Con el menú de una nota abierto, las flechas, Enter y Esc son
        // del menú (que no tiene el foco: ver `flyout.rs`).
        if (msg.message == WM_KEYDOWN || msg.message == WM_SYSKEYDOWN) && flyout::handle_key(msg.wParam as u32) {
            continue;
        }
        if msg.message == WM_KEYDOWN && note::handle_shortcut(msg.hwnd, msg.wParam as u32, repeat) {
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// `true` si esta es la única instancia (y se queda con el mutex hasta
/// que el proceso termine).
fn claim_single_instance() -> bool {
    let name = wide("Local\\Simpcky.SingleInstance");
    unsafe {
        let handle = CreateMutexW(null(), 0, name.as_ptr());
        // El handle no se cierra nunca a propósito: el mutex tiene que
        // vivir exactamente lo que vive el proceso.
        !handle.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Le pasa el pedido a la instancia que ya está corriendo: una nota
/// nueva donde está el cursor (`--new`) o, si se abrió el .exe a
/// secas, la ventana "Todas las notas".
fn forward_to_running_instance(want_new: bool) {
    let controller = find_controller();
    unsafe {
        if controller.is_null() {
            return;
        }
        // Este proceso lo lanzó el usuario (tiene permiso para pasar
        // ventanas al frente); se lo cede a la otra instancia, que es
        // la que va a mostrar la nota. Sin esto Windows la deja atrás,
        // titilando en la barra de tareas.
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(controller, &mut pid);
        AllowSetForegroundWindow(pid);
        if want_new {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            PostMessageW(controller, tray::WM_APP_NEW_AT, pt.x as isize as usize, pt.y as isize);
        } else {
            PostMessageW(controller, tray::WM_APP_SHOW_ALL, 0, 0);
        }
    }
}

/// La ventana controladora de la instancia que ya está corriendo (con
/// un poco de espera, por si justo está arrancando).
fn find_controller() -> HWND {
    let class = wide(tray::CONTROLLER_CLASS);
    unsafe {
        for _ in 0..30 {
            let controller = FindWindowW(class.as_ptr(), null());
            if !controller.is_null() {
                return controller;
            }
            Sleep(100);
        }
    }
    null_mut()
}

/// `--quit`: le pide a la instancia abierta que guarde y se cierre, y
/// espera (hasta 10 s) a que su proceso termine de verdad, para que el
/// instalador encuentre el .exe libre.
fn quit_running_instance() {
    let controller = find_controller();
    if controller.is_null() {
        return;
    }
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(controller, &mut pid);
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        PostMessageW(controller, tray::WM_APP_QUIT, 0, 0);
        if !process.is_null() {
            WaitForSingleObject(process, 10_000);
            CloseHandle(process);
        }
    }
}

fn load_or_create_notes() {
    if persist::is_first_run() {
        let id = app::next_note_id();
        let mut data = NoteData::new(id, 140, 120, 0, RollMode::Manual);
        data.title = "¡Bienvenido a Simpcky!".to_string();
        data.text = "Esta nota se enrolla solo cuando tú lo decides: el modo Manual es \
siempre el que viene por defecto, nunca se enrolla sola.\n\n\
Doble clic en el título (o F2) para cambiarle el nombre.\n\n\
Clic en el icono de la bandeja, o clic derecho en el escritorio → \
\"Nueva nota adhesiva\", para crear más."
            .to_string();
        note::create_note(data);
        save_all();
        return;
    }

    let notes = persist::load_notes();
    let mut max_id = 0u32;
    for data in notes {
        max_id = max_id.max(data.id);
        note::create_note(data);
    }
    // Deja next_id por delante del mayor id cargado para no reciclar ids.
    {
        let mut a = app::app().lock().unwrap();
        if a.next_id <= max_id {
            a.next_id = max_id + 1;
        }
    }
}
