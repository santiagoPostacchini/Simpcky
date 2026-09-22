//! Estado global de la aplicación.
//!
//! Todo Simpcky corre en un solo hilo (el de la ventana de mensajes de
//! Win32), así que un `Mutex` aquí no es por concurrencia real sino para
//! poder guardar handles (`*mut c_void`, que no son `Send`/`Sync`) detrás
//! de una `static` seguro. Todos los handles se guardan como `isize` y se
//! recastean en el punto de uso.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::persist::{self, NoteData, RollMode};

pub struct NoteRuntime {
    pub data: NoteData,
    /// HWND de la ventana de la nota (0 hasta que WM_CREATE la registra).
    pub hwnd: isize,
    /// HWND del control RichEdit hijo.
    pub edit: isize,
}

pub struct AppState {
    pub hinstance: isize,
    pub notes: HashMap<u32, NoteRuntime>,
    pub next_id: u32,
    /// Modo de enrollado para las notas nuevas. Manual salvo que el
    /// usuario cambie el valor por defecto desde el menú de la bandeja.
    pub default_roll_mode: RollMode,
    pub controller_hwnd: isize,
    /// HFONT compartido por todas las notas (Segoe UI), creado la
    /// primera vez que hace falta.
    pub font: isize,
}

static APP: OnceLock<Mutex<AppState>> = OnceLock::new();

pub fn init_app(hinstance: isize) {
    let _ = APP.set(Mutex::new(AppState {
        hinstance,
        notes: HashMap::new(),
        next_id: 1,
        default_roll_mode: RollMode::Manual,
        controller_hwnd: 0,
        font: 0,
    }));
}

pub fn app() -> &'static Mutex<AppState> {
    APP.get().expect("app::init_app no fue llamado todavía")
}

/// Vuelca el estado actual de todas las notas a `notes.json`.
/// Se llama tras cualquier cambio que valga la pena recordar
/// (mover, redimensionar, cambiar color/capa/modo, editar texto…).
pub fn save_all() {
    let a = app().lock().unwrap();
    let list: Vec<NoteData> = a.notes.values().map(|nr| nr.data.clone()).collect();
    persist::save_notes(&list);
}

/// Siguiente id libre para una nota nueva.
pub fn next_note_id() -> u32 {
    let mut a = app().lock().unwrap();
    let id = a.next_id;
    a.next_id += 1;
    id
}
