//! Sincronización de las notas entre compus a través de Google Drive.
//! Totalmente opcional: sin cuenta conectada, nada de esto hace nada.
//!
//! **Modelo.** En Drive hay un único documento con todas las notas y
//! las "lápidas" de las borradas. Cada compu, al sincronizar: baja el
//! documento, lo **fusiona** con lo suyo, sube el resultado si cambió, y
//! aplica en sus ventanas lo que llegó de afuera. La fusión es
//! determinística y da lo mismo en qué orden la hagan las compus, así
//! que todas terminan en el mismo estado.
//!
//! **Reglas de la fusión** (ver `merge`):
//! - Cada nota tiene una identidad global (`uid`), no el número local.
//! - Cada nota tiene cuatro partes (contenido, color, posición/tamaño,
//!   estado), cada una con su hora de modificación: gana la más nueva
//!   **de cada parte**. Mover una nota en la notebook no pisa lo que se
//!   escribió en ella en la de escritorio.
//! - Borrar deja una lápida. Una nota con lápida más nueva que su
//!   último cambio queda borrada; si se la editó *después* de borrarla
//!   en otra compu, la edición gana y la nota vuelve.
//!
//! **Hilos.** La red corre en un hilo aparte (`run_job`) que trabaja
//! sobre una copia; el resultado vuelve al hilo de las ventanas con un
//! mensaje (`WM_APP_SYNC_DONE`) y recién ahí se aplica (`on_done`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::app;
use crate::drive::{self, DriveError};
use crate::json;
use crate::oauth::{self, AuthError};
use crate::persist::{self, NoteData, Tomb, PARTS};
use crate::win::wide;

/// El hilo de red avisa que terminó (`lparam` = `Box<JobResult>`).
pub const WM_APP_SYNC_DONE: u32 = WM_APP + 10;
/// Terminó el inicio de sesión (`lparam` = `Box<Result<String, String>>`).
pub const WM_APP_SIGNIN_DONE: u32 = WM_APP + 11;
/// Timers de la ventana controladora (ver `tray.rs`).
pub const TIMER_SYNC_SOON: usize = 20;
pub const TIMER_SYNC_POLL: usize = 21;
/// Después de un cambio local se espera un poco, por si vienen más
/// (se está escribiendo): una subida cada tanto, no una por tecla.
const SOON_MS: u32 = 4_000;
/// Cada cuánto se mira si otra compu cambió algo. Mirar es barato: una
/// consulta de metadatos; el documento solo se baja si cambió.
const POLL_MS: u32 = 90_000;
/// Las lápidas se olvidan a los 90 días.
const TOMB_TTL_MS: u64 = 90 * 24 * 3600 * 1000;
/// Versión del formato del documento en Drive.
/// La versión más nueva del archivo de Drive que esta app entiende. La
/// 2 suma el formato del texto (`fmt`, ver `richtext.rs`): una versión
/// vieja de Simpcky no lo conoce, y al reescribir el archivo lo
/// perdería, así que ante un 2 se niega a tocarlo y pide actualizar
/// (ver `doc_parse`). Mientras ninguna nota tenga formato se sigue
/// escribiendo 1, y las compus sin actualizar siguen sincronizando.
const FORMAT: u32 = 2;

// -----------------------------------------------------------------
// Fusión (pura: sin ventanas ni red, cubierta por tests)
// -----------------------------------------------------------------

/// Dos versiones de la misma nota: de cada parte, la más nueva. Empate
/// exacto (mismo milisegundo, valores distintos): gana el valor mayor,
/// para que todas las compus elijan lo mismo.
pub fn merge_note(a: &NoteData, b: &NoteData) -> NoteData {
    let mut out = a.clone();
    let (pa, pb) = (a.parts(), b.parts());
    for part in 0..PARTS {
        if b.t[part] > a.t[part] || (b.t[part] == a.t[part] && pb[part] > pa[part]) {
            out.take_part(b, part);
        }
    }
    out
}

/// Fusiona dos conjuntos de notas y lápidas. El resultado no depende del
/// orden de los argumentos (salvo los números locales, que se toman de
/// `local`). Sale ordenado por `uid`.
pub fn merge(local: &[NoteData], local_tombs: &[Tomb], remote: &[NoteData], remote_tombs: &[Tomb], now: u64) -> (Vec<NoteData>, Vec<Tomb>) {
    let mut tombs: HashMap<String, u64> = HashMap::new();
    for t in local_tombs.iter().chain(remote_tombs) {
        let at = tombs.entry(t.uid.clone()).or_insert(0);
        *at = (*at).max(t.at);
    }

    let mut notes: HashMap<String, NoteData> = HashMap::new();
    for n in remote {
        notes.insert(n.uid.clone(), NoteData { id: 0, ..n.clone() });
    }
    for n in local {
        let merged = match notes.get(&n.uid) {
            Some(r) => NoteData { id: n.id, ..merge_note(n, r) },
            None => n.clone(),
        };
        notes.insert(n.uid.clone(), merged);
    }

    let mut out_notes = Vec::new();
    for (uid, n) in notes {
        match tombs.get(&uid) {
            Some(&at) if at >= n.last_change() => {} // borrada
            Some(_) => {
                // Editada después de que la borraran en otra compu:
                // vuelve, y su lápida se descarta.
                tombs.remove(&uid);
                out_notes.push(n);
            }
            None => out_notes.push(n),
        }
    }
    out_notes.sort_by(|a, b| a.uid.cmp(&b.uid));

    let mut out_tombs: Vec<Tomb> = tombs
        .into_iter()
        .filter(|(_, at)| at + TOMB_TTL_MS > now)
        .map(|(uid, at)| Tomb { uid, at })
        .collect();
    out_tombs.sort_by(|a, b| a.uid.cmp(&b.uid));
    (out_notes, out_tombs)
}

/// El documento de Drive (siempre en el mismo orden: dos documentos con
/// el mismo contenido dan exactamente el mismo texto).
fn doc_json(notes: &[NoteData], tombs: &[Tomb]) -> String {
    let items: Vec<String> = notes.iter().map(|n| persist::note_json(n, false)).collect();
    let format = if notes.iter().any(|n| !n.fmt.is_empty()) { FORMAT } else { 1 };
    format!("{{\"format\":{format},\"notes\":[\n{}\n],\"deleted\":{}}}", items.join(",\n"), persist::tombs_json(tombs))
}

fn doc_parse(text: &str) -> Result<(Vec<NoteData>, Vec<Tomb>), String> {
    let j = json::parse(text).ok_or("el archivo de Drive está dañado")?;
    if j.u64_or("format", 1) > FORMAT as u64 {
        return Err("las notas de Drive son de una versión más nueva de Simpcky: actualizá la app en esta compu".into());
    }
    let mut notes: Vec<NoteData> = j
        .get("notes")
        .and_then(json::Json::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(persist::note_from_json)
        .filter(|n| !n.uid.is_empty())
        .collect();
    notes.sort_by(|a, b| a.uid.cmp(&b.uid));
    let mut tombs = persist::tombs_from_json(j.get("deleted"));
    tombs.sort_by(|a, b| a.uid.cmp(&b.uid));
    Ok((notes, tombs))
}

// -----------------------------------------------------------------
// El trabajo de red (en un hilo aparte)
// -----------------------------------------------------------------

struct Job {
    notes: Vec<NoteData>,
    tombs: Vec<Tomb>,
    /// MD5 del documento de Drive la última vez que se sincronizó.
    md5: String,
    /// Hubo cambios locales desde entonces (si no, y Drive tampoco
    /// cambió, no hay nada que hacer).
    dirty: bool,
}

enum Outcome {
    Unchanged { file_id: String, md5: String },
    Merged { notes: Vec<NoteData>, tombs: Vec<Tomb>, file_id: String, md5: String },
}

enum SyncError {
    /// Hay que volver a conectar la cuenta.
    Revoked,
    Other(String),
}

type JobResult = Result<Outcome, SyncError>;

impl From<DriveError> for SyncError {
    fn from(e: DriveError) -> Self {
        match e {
            DriveError::Auth(AuthError::Revoked) | DriveError::Auth(AuthError::NotConnected) => SyncError::Revoked,
            DriveError::Auth(AuthError::Network(m)) | DriveError::Http(m) => SyncError::Other(m),
        }
    }
}

fn run_job(job: Job) -> JobResult {
    let found = drive::find()?;
    if let Some(r) = &found {
        if r.md5 == job.md5 && !job.dirty {
            return Ok(Outcome::Unchanged { file_id: r.id.clone(), md5: r.md5.clone() });
        }
    }
    let (remote_notes, remote_tombs, remote_doc) = match &found {
        Some(r) => {
            let text = drive::download(&r.id)?;
            let (n, t) = doc_parse(&text).map_err(SyncError::Other)?;
            let canonical = doc_json(&n, &t);
            (n, t, Some(canonical))
        }
        None => (Vec::new(), Vec::new(), None),
    };

    let (notes, tombs) = merge(&job.notes, &job.tombs, &remote_notes, &remote_tombs, persist::now_ms());
    let doc = doc_json(&notes, &tombs);

    let remote = match (&found, remote_doc) {
        (Some(r), Some(prev)) if prev == doc => r.clone(), // Drive ya tiene esto
        (Some(r), _) => match drive::update(&r.id, &doc)? {
            Some(updated) => updated,
            None => drive::create(&doc)?, // lo borraron entre medio
        },
        (None, _) => drive::create(&doc)?,
    };
    Ok(Outcome::Merged { notes, tombs, file_id: remote.id, md5: remote.md5 })
}

// -----------------------------------------------------------------
// Estado en memoria (solo del hilo de las ventanas)
// -----------------------------------------------------------------

struct Runtime {
    running: bool,
    /// Se pidió otra sincronización mientras había una en curso.
    again: bool,
    signing_in: Option<Arc<AtomicBool>>,
    last_error: String,
}

static RT: Mutex<Runtime> = Mutex::new(Runtime { running: false, again: false, signing_in: None, last_error: String::new() });

fn controller() -> HWND {
    app().lock().unwrap().controller_hwnd as HWND
}

pub fn is_configured() -> bool {
    oauth::is_configured()
}

pub fn is_connected() -> bool {
    !app().lock().unwrap().sync.email.is_empty() && oauth::has_account()
}

/// Para el menú de la bandeja: (cuenta, estado).
pub fn status() -> (String, String) {
    let (email, last) = {
        let a = app().lock().unwrap();
        (a.sync.email.clone(), a.sync.last_sync)
    };
    let rt = RT.lock().unwrap();
    let state = if rt.running {
        "Sincronizando…".to_string()
    } else if !rt.last_error.is_empty() {
        format!("No se pudo sincronizar: {}", rt.last_error)
    } else if last == 0 {
        "Todavía no se sincronizó".to_string()
    } else {
        format!("Sincronizado {}", ago(last))
    };
    (email, state)
}

pub fn is_signing_in() -> bool {
    RT.lock().unwrap().signing_in.is_some()
}

fn ago(ms: u64) -> String {
    let secs = persist::now_ms().saturating_sub(ms) / 1000;
    match secs {
        0..=59 => "hace un momento".into(),
        60..=3599 => format!("hace {} min", secs / 60),
        3600..=86_399 => format!("hace {} h", secs / 3600),
        _ => format!("hace {} días", secs / 86_400),
    }
}

// -----------------------------------------------------------------
// Cuándo sincronizar
// -----------------------------------------------------------------

/// Al arrancar la app: si hay cuenta conectada, sincronizar enseguida y
/// después cada tanto.
pub fn on_startup() {
    if is_connected() {
        let hwnd = controller();
        unsafe {
            SetTimer(hwnd, TIMER_SYNC_POLL, POLL_MS, None);
            SetTimer(hwnd, TIMER_SYNC_SOON, 1_500, None);
        }
    }
}

/// Hubo un cambio local (lo llama `app::save_all`): sincronizar dentro
/// de unos segundos, contando desde el último cambio.
pub fn request_soon(controller: HWND) {
    if !controller.is_null() && is_connected() {
        unsafe { SetTimer(controller, TIMER_SYNC_SOON, SOON_MS, None) };
    }
}

/// Timers de la controladora.
pub fn on_timer(id: usize) {
    if id == TIMER_SYNC_SOON {
        unsafe { KillTimer(controller(), TIMER_SYNC_SOON) };
    }
    start();
}

/// "Sincronizar ahora" del menú.
pub fn sync_now() {
    app().lock().unwrap().sync_dirty = true; // bajar y comparar sí o sí
    start();
}

fn start() {
    if !is_connected() {
        return;
    }
    {
        let mut rt = RT.lock().unwrap();
        if rt.running {
            rt.again = true;
            return;
        }
        rt.running = true;
        rt.again = false;
    }
    // Lo que se esté escribiendo en este momento también cuenta.
    crate::note::commit_all_text();
    let job = {
        let mut a = app().lock().unwrap();
        let job = Job {
            notes: a.notes.values().map(|nr| nr.data.clone()).collect(),
            tombs: a.sync.tombs.clone(),
            md5: a.sync.md5.clone(),
            dirty: a.sync_dirty,
        };
        a.sync_dirty = false;
        job
    };
    let hwnd = controller() as isize;
    std::thread::spawn(move || {
        let result: Box<JobResult> = Box::new(run_job(job));
        let ptr = Box::into_raw(result);
        if unsafe { PostMessageW(hwnd as HWND, WM_APP_SYNC_DONE, 0, ptr as isize) } == 0 {
            drop(unsafe { Box::from_raw(ptr) });
        }
    });
}

/// Volvió el hilo de red (`WM_APP_SYNC_DONE`).
pub fn on_done(lparam: isize) {
    let result = *unsafe { Box::from_raw(lparam as *mut JobResult) };
    let again = {
        let mut rt = RT.lock().unwrap();
        rt.running = false;
        std::mem::take(&mut rt.again)
    };
    match result {
        Ok(Outcome::Unchanged { file_id, md5 }) => finish_ok(file_id, md5),
        Ok(Outcome::Merged { notes, tombs, file_id, md5 }) => {
            apply(notes, tombs);
            finish_ok(file_id, md5);
        }
        Err(SyncError::Revoked) => {
            // La credencial ya no sirve: se borra (no hay nada que revocar).
            oauth::forget();
            disconnect_local();
            crate::tray::notify(
                "Simpcky se desconectó de Google Drive",
                "Google ya no acepta la conexión guardada. Volvé a conectarla desde el ícono de la bandeja.",
            );
        }
        Err(SyncError::Other(msg)) => {
            RT.lock().unwrap().last_error = msg;
            // Lo que no se pudo subir se reintenta en la próxima vuelta.
            app().lock().unwrap().sync_dirty = true;
        }
    }
    if again {
        request_soon(controller());
    }
}

fn finish_ok(file_id: String, md5: String) {
    RT.lock().unwrap().last_error.clear();
    let state = {
        let mut a = app().lock().unwrap();
        a.sync.file_id = file_id;
        a.sync.md5 = md5;
        a.sync.last_sync = persist::now_ms();
        a.sync.clone()
    };
    persist::save_sync_state(&state);
}

/// Aplica el resultado de la fusión a las notas de esta compu.
///
/// Mientras la red trabajaba, el usuario pudo seguir escribiendo: por
/// eso cada nota se vuelve a fusionar contra su versión *actual* (no
/// la copia que se mandó), y gana lo más nuevo de cada lado. Lo local
/// que haya quedado sin subir se sube en la vuelta siguiente.
fn apply(merged: Vec<NoteData>, merged_tombs: Vec<Tomb>) {
    crate::note::commit_all_text();

    let mut updates: Vec<NoteData> = Vec::new();
    let mut creates: Vec<NoteData> = Vec::new();
    let mut deletes: Vec<u32> = Vec::new();
    {
        let mut a = app().lock().unwrap();
        let live: HashMap<String, NoteData> = a.notes.values().map(|nr| (nr.data.uid.clone(), nr.data.clone())).collect();
        let local_tombs: HashMap<String, u64> = a.sync.tombs.iter().map(|t| (t.uid.clone(), t.at)).collect();
        let merged_tomb_at: HashMap<&str, u64> = merged_tombs.iter().map(|t| (t.uid.as_str(), t.at)).collect();

        for m in &merged {
            match live.get(&m.uid) {
                Some(l) => {
                    let f = NoteData { id: l.id, ..merge_note(l, m) };
                    if &f != l {
                        updates.push(f);
                    }
                }
                None => {
                    // ¿La borraron acá mientras tanto? Entonces no se trae
                    // de vuelta (la lápida sube en la próxima vuelta).
                    let deleted_here = local_tombs.get(&m.uid).is_some_and(|&at| at >= m.last_change());
                    if !deleted_here {
                        creates.push(m.clone());
                    }
                }
            }
        }
        let merged_uids: std::collections::HashSet<&str> = merged.iter().map(|m| m.uid.as_str()).collect();
        for l in live.values() {
            if !merged_uids.contains(l.uid.as_str()) && merged_tomb_at.get(l.uid.as_str()).is_some_and(|&at| at >= l.last_change()) {
                deletes.push(l.id);
            }
        }

        // Lápidas: las de la fusión más las que se hayan sumado acá
        // mientras tanto.
        let mut tombs = merged_tombs.clone();
        for t in &a.sync.tombs {
            if !tombs.iter().any(|x| x.uid == t.uid) && !merged.iter().any(|m| m.uid == t.uid) {
                tombs.push(t.clone());
            }
        }
        a.sync.tombs = tombs;

        // Números locales para las notas nuevas.
        for c in &mut creates {
            c.id = a.next_id;
            a.next_id += 1;
        }
    }

    for n in updates {
        crate::note::apply_remote(n);
    }
    for n in creates {
        crate::app::mark_saved(&n);
        crate::note::create_note(n);
    }
    for id in deletes {
        crate::note::delete_silently(id);
    }
    crate::app::save_all();
    let state = app().lock().unwrap().sync.clone();
    persist::save_sync_state(&state);
}

// -----------------------------------------------------------------
// Conectar y desconectar la cuenta
// -----------------------------------------------------------------

/// "Sincronizar con Google Drive…": abre el navegador para iniciar
/// sesión. Todo pasa en otro hilo; el resultado vuelve con
/// `WM_APP_SIGNIN_DONE`.
pub fn begin_sign_in() {
    if !oauth::is_configured() {
        message(
            "Esta copia de Simpcky se compiló sin la conexión con Google configurada.\n\n\
Ver docs/sincronizacion-google.md para configurarla.",
        );
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut rt = RT.lock().unwrap();
        if rt.signing_in.is_some() {
            drop(rt);
            crate::tray::notify("Simpcky", "Ya hay un inicio de sesión abierto en el navegador.");
            return;
        }
        rt.signing_in = Some(cancel.clone());
    }
    let hwnd = controller() as isize;
    std::thread::spawn(move || {
        let result: Box<Result<String, String>> = Box::new(oauth::sign_in(&cancel));
        let ptr = Box::into_raw(result);
        if unsafe { PostMessageW(hwnd as HWND, WM_APP_SIGNIN_DONE, 0, ptr as isize) } == 0 {
            drop(unsafe { Box::from_raw(ptr) });
        }
    });
    crate::tray::notify("Conectar con Google Drive", "Se abrió el navegador: iniciá sesión con tu cuenta de Google ahí.");
    crate::welcome::refresh();
}

pub fn cancel_sign_in() {
    if let Some(flag) = RT.lock().unwrap().signing_in.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

pub fn on_sign_in_done(lparam: isize) {
    let result = *unsafe { Box::from_raw(lparam as *mut Result<String, String>) };
    RT.lock().unwrap().signing_in = None;
    crate::welcome::refresh();
    match result {
        Ok(email) => {
            let state = {
                let mut a = app().lock().unwrap();
                a.sync.email = email.clone();
                // Otra cuenta (o la misma, de nuevo): empezar de cero la
                // relación con Drive, y fusionar todo lo que haya acá.
                a.sync.file_id.clear();
                a.sync.md5.clear();
                a.sync_dirty = true;
                a.sync.clone()
            };
            persist::save_sync_state(&state);
            RT.lock().unwrap().last_error.clear();
            crate::tray::notify("Conectado con Google Drive", &format!("{email}. Tus notas se van a mantener iguales en todas tus compus."));
            unsafe { SetTimer(controller(), TIMER_SYNC_POLL, POLL_MS, None) };
            start();
        }
        Err(msg) if msg.contains("cancelado") => {}
        Err(msg) => crate::tray::notify("No se pudo conectar con Google Drive", &msg),
    }
}

/// "Desconectar…": las notas se quedan en esta compu, solo dejan de
/// sincronizarse. La credencial se revoca en Google (en otro hilo, por
/// si no hay red) y se borra de acá.
pub fn disconnect() {
    let text = wide(
        "¿Dejar de sincronizar esta compu con Google Drive?\n\n\
Tus notas se quedan como están acá, y también en Drive y en tus otras compus.",
    );
    let title = wide("Simpcky");
    let res = unsafe { MessageBoxW(controller(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION) };
    if res != IDYES {
        return;
    }
    // El hilo le pide a Google que invalide la credencial y después la
    // borra de acá (si se borrara ya, no habría con qué revocarla).
    std::thread::spawn(oauth::sign_out);
    disconnect_local();
}

fn disconnect_local() {
    oauth::invalidate_access();
    let state = {
        let mut a = app().lock().unwrap();
        a.sync.email.clear();
        a.sync.file_id.clear();
        a.sync.md5.clear();
        a.sync.clone()
    };
    persist::save_sync_state(&state);
    RT.lock().unwrap().last_error.clear();
    let hwnd = controller();
    unsafe {
        KillTimer(hwnd, TIMER_SYNC_POLL);
        KillTimer(hwnd, TIMER_SYNC_SOON);
    }
}

fn message(text: &str) {
    let t = wide(text);
    let title = wide("Simpcky");
    unsafe { MessageBoxW(controller(), t.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::{Layer, RollMode, CONTENT, GEOM};

    fn note(uid: &str, text: &str, t: u64) -> NoteData {
        let mut n = NoteData::new(1, 100, 100, 0, RollMode::Manual);
        n.uid = uid.into();
        n.text = text.into();
        n.t = [t; PARTS];
        n
    }

    const NOW: u64 = 1_800_000_000_000;

    #[test]
    fn union_of_different_notes() {
        let (n, t) = merge(&[note("a", "A", 10)], &[], &[note("b", "B", 10)], &[], NOW);
        assert_eq!(n.iter().map(|x| x.uid.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(t.is_empty());
    }

    #[test]
    fn newer_part_wins_independently() {
        // Notebook: movió la nota (t=20). Escritorio: editó el texto (t=30).
        let mut laptop = note("a", "viejo", 10);
        laptop.x = 999;
        laptop.t[GEOM] = 20;
        let mut desktop = note("a", "nuevo", 10);
        desktop.t[CONTENT] = 30;
        let (n, _) = merge(&[laptop.clone()], &[], &[desktop.clone()], &[], NOW);
        assert_eq!(n[0].text, "nuevo");
        assert_eq!(n[0].x, 999);
        // Y da lo mismo quién fusiona.
        let (m, _) = merge(&[desktop], &[], &[laptop], &[], NOW);
        assert_eq!((m[0].text.as_str(), m[0].x), ("nuevo", 999));
    }

    #[test]
    fn exact_tie_is_deterministic() {
        let a = note("a", "aaa", 10);
        let b = note("a", "bbb", 10);
        assert_eq!(merge_note(&a, &b).text, merge_note(&b, &a).text);
    }

    #[test]
    fn deletion_wins_over_older_note() {
        // Horas realistas: una lápida de hace más de 90 días se descarta
        // (ver old_tombs_are_pruned).
        let deleted = Tomb { uid: "a".into(), at: NOW - 1_000 };
        let (n, t) = merge(&[note("a", "A", NOW - 5_000)], &[], &[], &[deleted.clone()], NOW);
        assert!(n.is_empty());
        assert_eq!(t, vec![deleted]);
    }

    #[test]
    fn edit_after_deletion_resurrects() {
        let mut edited = note("a", "A", 10);
        edited.t[CONTENT] = 60;
        let (n, t) = merge(&[edited], &[], &[], &[Tomb { uid: "a".into(), at: 50 }], NOW);
        assert_eq!(n.len(), 1);
        assert!(t.is_empty());
    }

    #[test]
    fn old_tombs_are_pruned() {
        let old = Tomb { uid: "x".into(), at: NOW - TOMB_TTL_MS - 1 };
        let (_, t) = merge(&[], &[old], &[], &[], NOW);
        assert!(t.is_empty());
    }

    #[test]
    fn local_ids_are_kept_and_remote_ones_cleared() {
        let mut l = note("a", "A", 10);
        l.id = 42;
        let mut r = note("b", "B", 10);
        r.id = 7;
        let (n, _) = merge(&[l], &[], &[r], &[], NOW);
        assert_eq!(n.iter().map(|x| x.id).collect::<Vec<_>>(), [42, 0]);
    }

    #[test]
    fn document_roundtrip_is_canonical() {
        let mut a = note("b", "con \"comillas\"\ny ñ", 5);
        a.layer = Layer::AlwaysOnTop;
        let notes = vec![note("a", "A", 1), a];
        let tombs = vec![Tomb { uid: "z".into(), at: 9 }];
        let doc = doc_json(&notes, &tombs);
        let (n2, t2) = doc_parse(&doc).unwrap();
        assert_eq!(doc_json(&n2, &t2), doc);
    }

    #[test]
    fn refuses_newer_format() {
        assert!(doc_parse(r#"{"format":99,"notes":[],"deleted":[]}"#).is_err());
    }
}
