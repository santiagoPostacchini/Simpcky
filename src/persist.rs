//! Persistencia en `%APPDATA%\Simpcky\`:
//! - `notes.json`: las notas;
//! - `settings.json`: preferencias de la app (tema, etc.);
//! - `sync.json`: estado de la sincronización (id del archivo en Drive,
//!   cuenta conectada) y las "lápidas" de las notas borradas.
//!
//! JSON escrito a mano (ver `json.rs`): el esquema es chico y fijo, y así
//! el binario no carga un framework de serialización entero.

use std::fs;
use std::path::PathBuf;

use crate::json::{self, Json};

/// Modo de enrollado de una nota. `Manual` es siempre el valor por
/// defecto de una nota nueva: nunca se enrolla sola salvo que el usuario
/// (por nota, o como valor por defecto global) elija `Auto`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RollMode {
    Manual,
    Auto,
}

impl RollMode {
    pub fn as_u8(self) -> u8 {
        match self {
            RollMode::Manual => 0,
            RollMode::Auto => 1,
        }
    }
    pub fn from_u8(v: u8) -> Self {
        if v == 1 {
            RollMode::Auto
        } else {
            RollMode::Manual
        }
    }
}

/// Capa en la que vive la ventana de la nota. En la práctica es
/// binario: `AlwaysOnTop` (ventana normal, con botón en la barra de
/// tareas) o "widget de escritorio" — y `Normal` y `Desktop` son
/// ambos ese segundo caso (se conserva la distinción solo por
/// compatibilidad con archivos viejos; ver `note::set_layer`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Normal,
    Desktop,
    AlwaysOnTop,
}

impl Layer {
    pub fn as_u8(self) -> u8 {
        match self {
            Layer::Normal => 0,
            Layer::Desktop => 1,
            Layer::AlwaysOnTop => 2,
        }
    }
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Layer::Desktop,
            2 => Layer::AlwaysOnTop,
            _ => Layer::Normal,
        }
    }
}

/// Las cuatro partes de una nota que se sincronizan por separado, cada
/// una con su propia hora de última modificación (`NoteData::t`). Así,
/// mover una nota en una compu nunca pisa lo que se escribió en ella en
/// otra: gana el cambio más nuevo **de cada parte**, no de la nota
/// entera.
pub const CONTENT: usize = 0; // nombre y texto
pub const COLOR: usize = 1;
pub const GEOM: usize = 2; // posición y tamaño
pub const STATE: usize = 3; // capa, modo de enrollado, enrollada
pub const PARTS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct NoteData {
    /// Número local de esta compu (lo usa la ventana). No se sincroniza:
    /// cada compu numera sus notas a su manera.
    pub id: u32,
    /// Identidad de la nota en todas las compus: 128 bits al azar.
    pub uid: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    /// Alto "desenrollado" — el alto real de la ventana cuando `rolled`
    /// es `false`. Al enrollar, la ventana baja a la altura del
    /// encabezado sin perder este valor, así se restaura tal cual.
    pub h: i32,
    pub color: u8, // índice de paleta 0..=5, ver theme.rs
    pub layer: Layer,
    pub roll_mode: RollMode,
    pub rolled: bool,
    /// Nombre elegido por el usuario. Vacío = se muestra la primera
    /// línea del texto (y si tampoco hay, "Nota").
    pub title: String,
    pub text: String,
    /// Negrita, cursiva, subrayado y tachado del texto (ver
    /// `richtext.rs`). Vacío en una nota sin formato — y entonces ni se
    /// escribe en el archivo, que queda igual que antes de que existiera.
    pub fmt: String,
    /// Última modificación de cada parte (ms desde 1970; ver `CONTENT`,
    /// `COLOR`, `GEOM`, `STATE`). 0 = nunca se guardó (nota recién
    /// creada: `app::save_all` le pone la hora).
    pub t: [u64; PARTS],
}

impl NoteData {
    pub fn new(id: u32, x: i32, y: i32, color: u8, roll_mode: RollMode) -> Self {
        NoteData {
            id,
            uid: new_uid(),
            x,
            y,
            w: 280,
            h: 320,
            color,
            // Por defecto, "widget de escritorio": anclada al escritorio,
            // sobrevive a "Mostrar escritorio". Solo "Siempre encima" la
            // saca de ahí y la convierte en una ventana normal con botón
            // en la barra de tareas.
            layer: Layer::Desktop,
            roll_mode,
            rolled: false,
            title: String::new(),
            text: String::new(),
            fmt: String::new(),
            t: [0; PARTS],
        }
    }

    /// El valor de cada parte, como texto comparable: si cambia, esa
    /// parte cambió. Deja afuera lo que no es un cambio de verdad:
    /// `Normal` y `Desktop` son la misma capa, y en modo Auto la nota se
    /// enrolla y desenrolla sola con el mouse (eso no se sincroniza).
    pub fn parts(&self) -> [String; PARTS] {
        let top = self.layer == Layer::AlwaysOnTop;
        let rolled = self.roll_mode == RollMode::Manual && self.rolled;
        [
            // El formato va pegado solo si hay: una nota sin formato da
            // la misma parte que antes, y no parece "cambiada" al
            // actualizar la app.
            if self.fmt.is_empty() {
                format!("{}\u{1}{}", self.title, self.text)
            } else {
                format!("{}\u{1}{}\u{1}{}", self.title, self.text, self.fmt)
            },
            self.color.to_string(),
            format!("{},{},{},{}", self.x, self.y, self.w, self.h),
            format!("{top},{},{rolled}", self.roll_mode.as_u8()),
        ]
    }

    /// Copia la parte `part` (valores y hora) de `other`.
    pub fn take_part(&mut self, other: &NoteData, part: usize) {
        match part {
            CONTENT => {
                self.title = other.title.clone();
                self.text = other.text.clone();
                self.fmt = other.fmt.clone();
            }
            COLOR => self.color = other.color,
            GEOM => {
                self.x = other.x;
                self.y = other.y;
                self.w = other.w;
                self.h = other.h;
            }
            STATE => {
                self.layer = other.layer;
                self.roll_mode = other.roll_mode;
                self.rolled = other.rolled;
            }
            _ => unreachable!("parte de nota inexistente: {part}"),
        }
        self.t[part] = other.t[part];
    }

    /// Cuándo se tocó la nota por última vez (cualquier parte).
    pub fn last_change(&self) -> u64 {
        self.t.iter().copied().max().unwrap_or(0)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn new_uid() -> String {
    crate::crypto::hex(&crate::crypto::random_bytes(16))
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("Simpcky")
}

fn notes_path() -> PathBuf {
    data_dir().join("notes.json")
}

/// `true` la primera vez que se ejecuta la app en esta cuenta (todavía
/// no existe `notes.json`). Se usa para decidir si se crea la nota de
/// bienvenida.
pub fn is_first_run() -> bool {
    !notes_path().exists()
}

pub fn load_notes() -> Vec<NoteData> {
    let Ok(text) = fs::read_to_string(notes_path()) else { return Vec::new() };
    parse_notes(&text, now_ms())
}

fn parse_notes(text: &str, now: u64) -> Vec<NoteData> {
    let Some(root) = json::parse(text) else { return Vec::new() };
    root.as_array()
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| {
            let mut n = note_from_json(item)?;
            // Notas de antes de la sincronización: se les da identidad
            // (y se guarda enseguida, ver main.rs) y una hora de
            // modificación.
            if n.uid.is_empty() {
                n.uid = new_uid();
            }
            if n.t == [0; PARTS] {
                n.t = [now; PARTS];
            }
            Some(n)
        })
        .collect()
}

/// `false` si no se pudo escribir (ver `app::save_all`: se avisa).
pub fn save_notes(notes: &[NoteData]) -> bool {
    if fs::create_dir_all(data_dir()).is_err() {
        return false;
    }
    let body: Vec<String> = notes.iter().map(|n| format!("  {}", note_json(n, true))).collect();
    write_atomic(&notes_path(), &format!("[\n{}\n]", body.join(",\n")))
}

/// Una nota en JSON. `local`: incluir el número local (`notes.json`) o
/// no (el documento que se sube a Drive).
pub fn note_json(n: &NoteData, local: bool) -> String {
    let id = if local { format!("\"id\":{},", n.id) } else { String::new() };
    let fmt = if n.fmt.is_empty() { String::new() } else { format!("\"fmt\":\"{}\",", json::escape(&n.fmt)) };
    format!(
        "{{{id}\"uid\":\"{}\",\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"color\":{},\"layer\":{},\"rollMode\":{},\"rolled\":{},\"title\":\"{}\",\"text\":\"{}\",{fmt}\"t\":[{},{},{},{}]}}",
        json::escape(&n.uid),
        n.x,
        n.y,
        n.w,
        n.h,
        n.color,
        n.layer.as_u8(),
        n.roll_mode.as_u8(),
        n.rolled,
        json::escape(&n.title),
        json::escape(&n.text),
        n.t[0],
        n.t[1],
        n.t[2],
        n.t[3],
    )
}

pub fn note_from_json(j: &Json) -> Option<NoteData> {
    j.get("text")?; // no es una nota
    let mut t = [0u64; PARTS];
    if let Some(arr) = j.get("t").and_then(Json::as_array) {
        for (i, v) in arr.iter().take(PARTS).enumerate() {
            t[i] = v.as_f64().unwrap_or(0.0).max(0.0) as u64;
        }
    }
    Some(NoteData {
        id: j.i32_or("id", 0) as u32,
        uid: j.str_or("uid", ""),
        x: j.i32_or("x", 80),
        y: j.i32_or("y", 80),
        w: j.i32_or("w", 280),
        h: j.i32_or("h", 320),
        color: j.u8_or("color", 0),
        layer: Layer::from_u8(j.u8_or("layer", 1)),
        roll_mode: RollMode::from_u8(j.u8_or("rollMode", 0)),
        rolled: j.bool_or("rolled", false),
        title: j.str_or("title", ""),
        text: j.str_or("text", ""),
        fmt: j.str_or("fmt", ""),
        t,
    })
}

/// Escribe a un archivo temporal y lo renombra encima del real: si la
/// app muere a mitad de la escritura (un apagado, un cuelgue), queda
/// el archivo anterior entero en vez de uno cortado por la mitad.
///
/// `true` si quedó escrito. Antes los errores se ignoraban, y un antivirus
/// que no dejaba escribir hacía que la app "guardara" durante horas sin
/// guardar nada, sin que nadie se enterara.
fn write_atomic(path: &std::path::Path, contents: &str) -> bool {
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, contents).is_ok() {
        if fs::rename(&tmp, path).is_ok() {
            return true;
        }
        let _ = fs::remove_file(&tmp);
    }
    fs::write(path, contents).is_ok()
}

// ---------------------------------------------------------------------
// Ajustes de la app (`settings.json`, al lado de `notes.json`)
// ---------------------------------------------------------------------

/// Preferencias globales. Van en un archivo aparte para que un
/// `notes.json` viejo (que es solo un array) siga leyéndose tal cual.
/// No se sincronizan: cada compu tiene las suyas.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub dark: bool,
    /// Modo de enrollado para las notas nuevas (Manual salvo que el
    /// usuario elija otra cosa en el menú de la bandeja).
    pub default_roll_mode: RollMode,
    /// "Nueva nota adhesiva" en el menú del clic derecho del escritorio.
    pub desktop_menu: bool,
    /// Buscar versiones nuevas en GitHub una vez por día (ver
    /// `update.rs`). Se puede apagar: la app no se conecta a nada que
    /// el usuario no haya elegido.
    pub auto_update: bool,
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// `None` si todavía no hay ajustes guardados (primera vez con esta
/// versión): el que llama decide los valores iniciales.
pub fn load_settings() -> Option<Settings> {
    let j = json::parse(&fs::read_to_string(settings_path()).ok()?)?;
    Some(Settings {
        dark: j.bool_or("dark", false),
        default_roll_mode: RollMode::from_u8(j.u8_or("defaultRollMode", 0)),
        desktop_menu: j.bool_or("desktopMenu", true),
        auto_update: j.bool_or("autoUpdate", true),
    })
}

pub fn save_settings(s: &Settings) -> bool {
    if fs::create_dir_all(data_dir()).is_err() {
        return false;
    }
    let json = format!(
        "{{\"dark\":{},\"defaultRollMode\":{},\"desktopMenu\":{},\"autoUpdate\":{}}}\n",
        s.dark,
        s.default_roll_mode.as_u8(),
        s.desktop_menu,
        s.auto_update
    );
    write_atomic(&settings_path(), &json)
}

// ---------------------------------------------------------------------
// Estado de la sincronización (`sync.json`)
// ---------------------------------------------------------------------

/// "Lápida" de una nota borrada: sin esto, la otra compu (que todavía
/// la tiene) la volvería a subir y la nota resucitaría.
#[derive(Clone, Debug, PartialEq)]
pub struct Tomb {
    pub uid: String,
    pub at: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SyncState {
    /// Cuenta conectada (vacío = sincronización apagada).
    pub email: String,
    /// Id del archivo en Drive y MD5 de la última versión que se vio.
    pub file_id: String,
    pub md5: String,
    /// Última sincronización exitosa (ms desde 1970).
    pub last_sync: u64,
    /// Notas borradas en esta compu o en otras. Se registran aunque la
    /// sincronización esté apagada: si se prende más tarde, lo borrado
    /// mientras tanto no tiene que volver.
    pub tombs: Vec<Tomb>,
}

fn sync_path() -> PathBuf {
    data_dir().join("sync.json")
}

pub fn load_sync_state() -> SyncState {
    let Some(j) = fs::read_to_string(sync_path()).ok().and_then(|t| json::parse(&t)) else {
        return SyncState::default();
    };
    SyncState {
        email: j.str_or("email", ""),
        file_id: j.str_or("fileId", ""),
        md5: j.str_or("md5", ""),
        last_sync: j.u64_or("lastSync", 0),
        tombs: tombs_from_json(j.get("deleted")),
    }
}

pub fn save_sync_state(s: &SyncState) -> bool {
    if fs::create_dir_all(data_dir()).is_err() {
        return false;
    }
    let json = format!(
        "{{\"email\":\"{}\",\"fileId\":\"{}\",\"md5\":\"{}\",\"lastSync\":{},\"deleted\":{}}}\n",
        json::escape(&s.email),
        json::escape(&s.file_id),
        json::escape(&s.md5),
        s.last_sync,
        tombs_json(&s.tombs)
    );
    write_atomic(&sync_path(), &json)
}

pub fn tombs_json(tombs: &[Tomb]) -> String {
    let items: Vec<String> =
        tombs.iter().map(|t| format!("{{\"uid\":\"{}\",\"at\":{}}}", json::escape(&t.uid), t.at)).collect();
    format!("[{}]", items.join(","))
}

pub fn tombs_from_json(j: Option<&Json>) -> Vec<Tomb> {
    j.and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|t| Tomb { uid: t.str_or("uid", ""), at: t.u64_or("at", 0) })
        .filter(|t| !t.uid.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_roundtrip_keeps_everything() {
        let mut n = NoteData::new(7, -40, 1200, 3, RollMode::Auto);
        n.title = "Súper \"lista\"".into();
        n.text = "leche\npan\t😀".into();
        n.layer = Layer::AlwaysOnTop;
        n.rolled = true;
        n.t = [1, 2, 3, 1_790_079_957_123];
        let back = note_from_json(&json::parse(&note_json(&n, true)).unwrap()).unwrap();
        assert_eq!(back, n);
        // Sin el número local (el documento de Drive), todo lo demás igual.
        let remote = note_from_json(&json::parse(&note_json(&n, false)).unwrap()).unwrap();
        assert_eq!(remote.id, 0);
        assert_eq!(remote.uid, n.uid);
    }

    /// El formato de antes de la sincronización (copiado de un
    /// `notes.json` real): tiene que cargar entero, y recibir identidad y
    /// hora.
    #[test]
    fn migrates_pre_sync_format() {
        let old = r#"[
  {"id":1,"x":200,"y":200,"w":280,"h":320,"color":2,"layer":1,"rollMode":0,"rolled":false,"title":"","text":"Prueba anclaje al escritorio"},
  {"id":33,"x":1560,"y":49,"w":280,"h":320,"color":2,"layer":1,"rollMode":0,"rolled":false,"title":"123124","text":"adasdasd"}
]"#;
        let notes = parse_notes(old, 42);
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0].id, notes[0].text.as_str(), notes[0].x), (1, "Prueba anclaje al escritorio", 200));
        assert_eq!((notes[1].id, notes[1].title.as_str()), (33, "123124"));
        assert!(notes.iter().all(|n| n.uid.len() == 32 && n.t == [42; PARTS]));
        assert_ne!(notes[0].uid, notes[1].uid);
        // Y lo que se guarda después se vuelve a leer igual.
        let saved: Vec<String> = notes.iter().map(|n| note_json(n, true)).collect();
        let again = parse_notes(&format!("[{}]", saved.join(",")), 99);
        assert_eq!(again, notes);
    }

    #[test]
    fn fmt_is_saved_only_when_present() {
        let mut n = NoteData::new(1, 0, 0, 0, RollMode::Manual);
        n.text = "hola".into();
        let plain = note_json(&n, true);
        assert!(!plain.contains("fmt"));
        let before = n.parts();
        n.fmt = "b0+4".into();
        let rich = note_json(&n, true);
        assert!(rich.contains("\"fmt\":\"b0+4\""));
        assert_ne!(before[CONTENT], n.parts()[CONTENT]);
        let back = note_from_json(&crate::json::parse(&rich).unwrap()).unwrap();
        assert_eq!(back.fmt, "b0+4");
    }

    #[test]
    fn parts_ignore_non_changes() {
        let mut a = NoteData::new(1, 0, 0, 0, RollMode::Auto);
        let mut b = a.clone();
        a.layer = Layer::Normal;
        b.layer = Layer::Desktop;
        b.rolled = true; // en modo Auto, enrollarse con el mouse no es un cambio
        assert_eq!(a.parts(), b.parts());
    }
}
