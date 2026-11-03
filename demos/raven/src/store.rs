//! Datos de las notas: el mismo `notes.json` que usa Simpcky.
//!
//! La demo **lee** las notas de Simpcky (`%APPDATA%\Simpcky\notes.json`)
//! la primera vez, pero escribe en su propia carpeta
//! (`%APPDATA%\SimpckyRaven`): probarla nunca pisa las notas reales.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn is_false(b: &bool) -> bool { !*b }
fn d80() -> i32 { 80 }
fn d280() -> i32 { 280 }
fn d320() -> i32 { 320 }
fn d1() -> u8 { 1 }

/// Una nota, con los mismos campos y nombres que `persist::NoteData`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NoteData {
    #[serde(default)]
    pub id: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    #[serde(default)]
    pub uid: String,
    #[serde(default = "d80")]
    pub x: i32,
    #[serde(default = "d80")]
    pub y: i32,
    #[serde(default = "d280")]
    pub w: i32,
    #[serde(default = "d320")]
    pub h: i32,
    #[serde(default)]
    pub color: u8,
    /// 0/1 = en el escritorio, 2 = siempre encima.
    #[serde(default = "d1")]
    pub layer: u8,
    /// 0 = Manual, 1 = Auto.
    #[serde(default, rename = "rollMode")]
    pub roll_mode: u8,
    #[serde(default)]
    pub rolled: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// Formato del RichEdit de Simpcky. La demo no lo interpreta (usa
    /// Markdown), pero lo conserva tal cual al guardar.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fmt: String,
    #[serde(default)]
    pub t: [u64; 4],
}

impl NoteData {
    pub fn new(id: u32, x: i32, y: i32, color: u8) -> Self {
        let now = now_ms();
        NoteData {
            id,
            hidden: false,
            locked: false,
            uid: new_uid(),
            x,
            y,
            w: 280,
            h: 320,
            color,
            layer: 1,
            roll_mode: 0,
            rolled: false,
            title: String::new(),
            text: String::new(),
            fmt: String::new(),
            t: [now; 4],
        }
    }

    /// El nombre que se ve: el título, o la primera línea del texto.
    pub fn display_title(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        let first = self.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let first = first.trim_start_matches(['#', '-', '*', ' ', '[', ']', 'x']).trim();
        if first.is_empty() {
            "Nota".to_string()
        } else {
            first.chars().take(40).collect()
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Settings {
    #[serde(default)]
    pub dark: bool,
    /// Opacidad de las notas (1.0 = opacas).
    #[serde(default = "one")]
    pub opacity: f32,
    /// Índice en `ROLL_DELAYS_MS`.
    #[serde(default)]
    pub roll_delay: u8,
}

fn one() -> f32 { 1.0 }

impl Default for Settings {
    fn default() -> Self { Settings { dark: false, opacity: 1.0, roll_delay: 0 } }
}

pub const ROLL_DELAYS_MS: [u32; 5] = [0, 500, 1000, 2000, 3000];

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 32 cifras hexadecimales, como las de Simpcky (sin criptografía: es
/// solo un identificador para una demo).
pub fn new_uid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(now_ms());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn appdata() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()))
}

pub fn data_dir() -> PathBuf { appdata().join("SimpckyRaven") }

/// Notas de la demo; si todavía no hay, las de Simpcky (solo lectura).
pub fn load_notes() -> Vec<NoteData> {
    for path in [data_dir().join("notes.json"), appdata().join("Simpcky").join("notes.json")] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(notes) = serde_json::from_str::<Vec<NoteData>>(&text) {
                return notes;
            }
        }
    }
    Vec::new()
}

pub fn save_notes(notes: &[NoteData]) {
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(text) = serde_json::to_string(notes) {
        write_atomic(&data_dir().join("notes.json"), &text);
    }
}

pub fn load_settings() -> Settings {
    std::fs::read_to_string(data_dir().join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(text) = serde_json::to_string(s) {
        write_atomic(&data_dir().join("settings.json"), &text);
    }
}

/// Como en Simpcky: se escribe aparte y se renombra encima.
fn write_atomic(path: &std::path::Path, contents: &str) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, contents).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}
