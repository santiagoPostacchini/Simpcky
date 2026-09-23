//! Estado global de la aplicación.
//!
//! Todo Simpcky corre en un solo hilo (el de la ventana de mensajes de
//! Win32), así que un `Mutex` aquí no es por concurrencia real sino para
//! poder guardar handles (`*mut c_void`, que no son `Send`/`Sync`) detrás
//! de una `static` seguro. Todos los handles se guardan como `isize` y se
//! recastean en el punto de uso.
//!
//! Regla de oro: nunca llamar a una API que pueda despachar mensajes
//! (SendMessage, SetWindowPos, DestroyWindow, MessageBox…) con el lock
//! tomado. El mensaje puede volver a entrar a un wndproc nuestro, que
//! intenta tomar el mismo lock, y el hilo se cuelga para siempre.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::persist::{self, NoteData, RollMode, Settings};

pub struct NoteRuntime {
    pub data: NoteData,
    /// HWND de la ventana de la nota (0 hasta que WM_CREATE la registra,
    /// y otra vez 0 si la ventana se destruyó sin que el usuario borrara
    /// la nota — ver `note::on_destroy`).
    pub hwnd: isize,
    /// HWND del control RichEdit hijo.
    pub edit: isize,
    /// Widget de escritorio "asomado" al frente (abierto desde "Todas
    /// las notas"): vuelve a anclarse solo al perder el foco.
    pub peeking: bool,
    /// El usuario confirmó "Eliminar": el WM_DESTROY que sigue sí borra
    /// los datos. Cualquier otra destrucción (Explorer reiniciándose,
    /// cierre de sesión) los conserva.
    pub deleting: bool,
}

pub struct AppState {
    pub hinstance: isize,
    pub notes: HashMap<u32, NoteRuntime>,
    pub next_id: u32,
    pub settings: Settings,
    pub controller_hwnd: isize,
    /// HFONT compartido por todas las notas (Segoe UI), creado la
    /// primera vez que hace falta.
    pub font: isize,
    /// HFONT semibold, más chico, para el título del encabezado.
    pub header_font: isize,
}

static APP: OnceLock<Mutex<AppState>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub fn init_app(hinstance: isize, settings: Settings) {
    let _ = APP.set(Mutex::new(AppState {
        hinstance,
        notes: HashMap::new(),
        next_id: 1,
        settings,
        controller_hwnd: 0,
        font: 0,
        header_font: 0,
    }));
}

pub fn app() -> &'static Mutex<AppState> {
    APP.get().expect("app::init_app no fue llamado todavía")
}

/// La app se está cerrando (Salir, o fin de sesión de Windows): las
/// ventanas que se destruyan a partir de acá no se recrean.
pub fn set_shutting_down() {
    SHUTTING_DOWN.store(true, Ordering::Relaxed);
}

pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::Relaxed)
}

pub fn default_roll_mode() -> RollMode {
    app().lock().unwrap().settings.default_roll_mode
}

/// Vuelca el estado actual de todas las notas a `notes.json`.
/// Se llama tras cualquier cambio que valga la pena recordar
/// (mover, redimensionar, cambiar color/capa/modo, editar texto…).
pub fn save_all() {
    {
        let a = app().lock().unwrap();
        let mut list: Vec<NoteData> = a.notes.values().map(|nr| nr.data.clone()).collect();
        list.sort_by_key(|n| n.id);
        persist::save_notes(&list);
    }
    // "Todas las notas" se entera de cualquier cambio (se refresca con
    // un mensaje en cola, nunca en el acto: ver la regla de oro arriba).
    crate::allnotes::notify_changed();
}

pub fn save_settings() {
    let s = { app().lock().unwrap().settings };
    let dark = crate::theme::is_dark();
    persist::save_settings(&Settings { dark, ..s });
}

/// Siguiente id libre para una nota nueva.
pub fn next_note_id() -> u32 {
    let mut a = app().lock().unwrap();
    let id = a.next_id;
    a.next_id += 1;
    id
}
