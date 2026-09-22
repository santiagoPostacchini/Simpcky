// Sin consola: Simpcky es una app de bandeja, no una CLI.
#![windows_subsystem = "windows"]

mod app;
mod note;
mod persist;
mod tray;
mod win;

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HINSTANCE;
use windows_sys::Win32::Graphics::GdiPlus::{GdiplusStartup, GdiplusStartupInput};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};

use app::{init_app, save_all};
use persist::{NoteData, RollMode};

fn main() {
    unsafe {
        // Coordenadas correctas en pantallas con distinto DPI: sin esto
        // Windows escala la ventana por nosotros y el layout (header,
        // iconos dibujados a mano) queda borroso o desalineado.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // GDI+ (gdiplus.dll, incluido en Windows desde XP) se usa para
        // dibujar los íconos del encabezado de cada nota con
        // antialiasing real — GDI clásico no lo soporta y se ve
        // dentado. No hace falta apagarlo: el proceso lo libera al salir.
        let mut gdiplus_token: usize = 0;
        let gdiplus_input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        GdiplusStartup(&mut gdiplus_token, &gdiplus_input, null_mut());
    }

    let hinstance = unsafe { GetModuleHandleW(null_mut()) } as HINSTANCE;
    init_app(hinstance as isize);

    note::load_richedit_library();
    note::register_class(hinstance);
    tray::register_class(hinstance);

    load_or_create_notes();
    tray::init(hinstance);

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let ret = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if ret <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn load_or_create_notes() {
    if persist::is_first_run() {
        let id = app::next_note_id();
        let mut data = NoteData::new(id, 140, 120, 0, RollMode::Manual);
        data.text = "¡Bienvenido a Simpcky!\n\n\
Esta nota se enrolla solo cuando tú lo decides: el modo Manual es \
siempre el que viene por defecto, nunca se enrolla sola.\n\n\
Abre el menú ⋯ si alguna vez querés que una nota se enrolle sola al \
quitar el mouse (modo Auto).\n\n\
Clic derecho en el icono de la bandeja para crear más notas."
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
