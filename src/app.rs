//! Estado global de la aplicación.
//!
//! Las ventanas corren en un solo hilo, así que el `Mutex` no es tanto
//! por concurrencia (solo la sincronización usa otro hilo, y trabaja
//! sobre copias) sino para poder guardar handles (`*mut c_void`, que no
//! son `Send`/`Sync`) detrás de una `static` segura. Todos los handles
//! se guardan como `isize` y se recastean en el punto de uso.
//!
//! Regla de oro: nunca llamar a una API que pueda despachar mensajes
//! (SendMessage, SetWindowPos, DestroyWindow, MessageBox…) con el lock
//! tomado. El mensaje puede volver a entrar a un wndproc nuestro, que
//! intenta tomar el mismo lock, y el hilo se cuelga para siempre.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::persist::{self, NoteData, RollMode, Settings, SyncState, Tomb, PARTS};

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
    /// El usuario confirmó "Eliminar" (o la borraron en otra compu): el
    /// WM_DESTROY que sigue sí borra los datos. Cualquier otra
    /// destrucción (Explorer reiniciándose, cierre de sesión) los
    /// conserva.
    pub deleting: bool,
}

pub struct AppState {
    pub hinstance: isize,
    pub notes: HashMap<u32, NoteRuntime>,
    pub next_id: u32,
    pub settings: Settings,
    pub controller_hwnd: isize,
    /// Estado de la sincronización (y lápidas de las notas borradas).
    pub sync: SyncState,
    /// Hubo cambios locales que todavía no se subieron.
    pub sync_dirty: bool,
    /// Cómo estaba cada parte de cada nota la última vez que se guardó
    /// (número local → uid y partes): así `save_all` sabe qué cambió y
    /// le pone la hora a esa parte, y qué notas se borraron.
    pub saved_parts: HashMap<u32, (String, [String; PARTS])>,
}

static APP: OnceLock<Mutex<AppState>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub fn init_app(hinstance: isize, settings: Settings, sync: SyncState) {
    let _ = APP.set(Mutex::new(AppState {
        hinstance,
        notes: HashMap::new(),
        next_id: 1,
        settings,
        controller_hwnd: 0,
        sync,
        sync_dirty: false,
        saved_parts: HashMap::new(),
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

/// Punto de partida de la detección de cambios, después de cargar las
/// notas del disco (lo cargado no es un cambio).
pub fn init_saved_parts() {
    let mut a = app().lock().unwrap();
    let parts: HashMap<u32, (String, [String; PARTS])> =
        a.notes.values().map(|nr| (nr.data.id, (nr.data.uid.clone(), nr.data.parts()))).collect();
    a.saved_parts = parts;
}

/// Lo que llega de otra compu no es un cambio local: se registra como
/// ya guardado, así `save_all` no le pone la hora de ahora.
pub fn mark_saved(data: &NoteData) {
    app().lock().unwrap().saved_parts.insert(data.id, (data.uid.clone(), data.parts()));
}

/// Una nota que se borra porque la borraron en otra compu: su lápida ya
/// existe, no hace falta otra.
pub fn forget_saved(id: u32) {
    app().lock().unwrap().saved_parts.remove(&id);
}

/// Vuelca el estado actual de todas las notas a `notes.json`.
/// Se llama tras cualquier cambio que valga la pena recordar
/// (mover, redimensionar, cambiar color/capa/modo, editar texto…).
///
/// De paso, es donde se detectan los cambios para la sincronización:
/// cada parte de cada nota que difiera de lo último guardado recibe la
/// hora actual, y cada nota que ya no está deja una lápida.
pub fn save_all() {
    let (changed, controller) = {
        let mut guard = app().lock().unwrap();
        let a = &mut *guard;
        let now = persist::now_ms();
        let mut changed = false;

        for nr in a.notes.values_mut() {
            let parts = nr.data.parts();
            match a.saved_parts.get(&nr.data.id) {
                Some((_, prev)) => {
                    for (i, (old, new)) in prev.iter().zip(parts.iter()).enumerate() {
                        if old != new {
                            nr.data.t[i] = now;
                            changed = true;
                        }
                    }
                }
                None => {
                    // Nota nueva de esta compu.
                    for t in nr.data.t.iter_mut().filter(|t| **t == 0) {
                        *t = now;
                    }
                    changed = true;
                }
            }
            a.saved_parts.insert(nr.data.id, (nr.data.uid.clone(), parts));
        }

        let gone: Vec<u32> = a.saved_parts.keys().filter(|id| !a.notes.contains_key(id)).copied().collect();
        let mut new_tombs = false;
        for id in gone {
            if let Some((uid, _)) = a.saved_parts.remove(&id) {
                a.sync.tombs.retain(|t| t.uid != uid);
                a.sync.tombs.push(Tomb { uid, at: now });
                new_tombs = true;
                changed = true;
            }
        }

        let mut list: Vec<NoteData> = a.notes.values().map(|nr| nr.data.clone()).collect();
        list.sort_by_key(|n| n.id);
        persist::save_notes(&list);
        if new_tombs {
            persist::save_sync_state(&a.sync);
        }
        if changed {
            a.sync_dirty = true;
        }
        (changed, a.controller_hwnd)
    };
    // "Todas las notas" se entera de cualquier cambio (se refresca con
    // un mensaje en cola, nunca en el acto: ver la regla de oro arriba).
    crate::allnotes::notify_changed();
    if changed {
        crate::sync::request_soon(controller as windows_sys::Win32::Foundation::HWND);
    }
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
