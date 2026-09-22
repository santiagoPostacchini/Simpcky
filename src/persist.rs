//! Persistencia de las notas en `%APPDATA%\Simpcky\notes.json`.
//!
//! Formato deliberadamente casero (sin serde): el esquema es plano y fijo,
//! y así el binario no carga un framework de serialización entero solo para
//! guardar una lista de notas. Ver `artifact-type/reference/...` (diseño) /
//! la especificación técnica del lienzo para el porqué de cada campo.

use std::fs;
use std::path::PathBuf;

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

/// Capa en la que vive la ventana de la nota.
/// `Desktop` (anclada detrás de los iconos) está en el diseño pero
/// todavía no implementada — ver README. Se conserva el valor si se lee
/// de un archivo futuro, pero hoy se trata como `Normal`.
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

#[derive(Clone, Debug)]
pub struct NoteData {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    /// Alto "desenrollado" — el alto real de la ventana cuando `rolled`
    /// es `false`. Al enrollar, la ventana baja a la altura del
    /// encabezado sin perder este valor, así se restaura tal cual.
    pub h: i32,
    pub color: u8, // índice de paleta 0..=5, ver note::PALETTE
    pub layer: Layer,
    pub roll_mode: RollMode,
    pub rolled: bool,
    pub text: String,
}

impl NoteData {
    pub fn new(id: u32, x: i32, y: i32, color: u8, roll_mode: RollMode) -> Self {
        NoteData {
            id,
            x,
            y,
            w: 280,
            h: 320,
            color,
            layer: Layer::Normal,
            roll_mode,
            rolled: false,
            text: String::new(),
        }
    }
}

fn data_dir() -> PathBuf {
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
    let path = notes_path();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    parse_notes(&text)
}

pub fn save_notes(notes: &[NoteData]) {
    let dir = data_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let json = write_notes(notes);
    let _ = fs::write(notes_path(), json);
}

// ---------------------------------------------------------------------
// Escritura JSON (a mano: el esquema es fijo y plano)
// ---------------------------------------------------------------------

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn write_notes(notes: &[NoteData]) -> String {
    let mut out = String::from("[\n");
    for (i, n) in notes.iter().enumerate() {
        out.push_str("  {");
        out.push_str(&format!("\"id\":{},", n.id));
        out.push_str(&format!("\"x\":{},", n.x));
        out.push_str(&format!("\"y\":{},", n.y));
        out.push_str(&format!("\"w\":{},", n.w));
        out.push_str(&format!("\"h\":{},", n.h));
        out.push_str(&format!("\"color\":{},", n.color));
        out.push_str(&format!("\"layer\":{},", n.layer.as_u8()));
        out.push_str(&format!("\"rollMode\":{},", n.roll_mode.as_u8()));
        out.push_str(&format!("\"rolled\":{},", n.rolled));
        out.push_str(&format!("\"text\":\"{}\"", escape_json(&n.text)));
        out.push('}');
        if i + 1 != notes.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push(']');
    out
}

// ---------------------------------------------------------------------
// Lectura JSON: parser recursivo mínimo, suficiente para un array plano
// de objetos con valores string/number/bool.
// ---------------------------------------------------------------------

enum Json {
    Num(f64),
    Str(String),
    Bool(bool),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    Null,
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn new(s: &'a str) -> Self {
        P { b: s.as_bytes(), i: 0 }
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b' ' | b'\t' | b'\r' | b'\n' => self.i += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn parse_value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.peek()? {
            b'{' => self.parse_obj(),
            b'[' => self.parse_arr(),
            b'"' => self.parse_str().map(Json::Str),
            b't' => {
                self.i += 4;
                Some(Json::Bool(true))
            }
            b'f' => {
                self.i += 5;
                Some(Json::Bool(false))
            }
            b'n' => {
                self.i += 4;
                Some(Json::Null)
            }
            _ => self.parse_num(),
        }
    }

    fn parse_obj(&mut self) -> Option<Json> {
        self.i += 1; // {
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Some(Json::Obj(fields));
        }
        loop {
            self.skip_ws();
            let key = self.parse_str()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return None;
            }
            self.i += 1;
            let val = self.parse_value()?;
            fields.push((key, val));
            self.skip_ws();
            match self.peek()? {
                b',' => {
                    self.i += 1;
                }
                b'}' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Json::Obj(fields))
    }

    fn parse_arr(&mut self) -> Option<Json> {
        self.i += 1; // [
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Some(Json::Arr(items));
        }
        loop {
            let val = self.parse_value()?;
            items.push(val);
            self.skip_ws();
            match self.peek()? {
                b',' => {
                    self.i += 1;
                }
                b']' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Json::Arr(items))
    }

    fn parse_str(&mut self) -> Option<String> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let esc = *self.b.get(self.i)?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.b.get(self.i..self.i + 4)?;
                            let hex = std::str::from_utf8(hex).ok()?;
                            let cp = u32::from_str_radix(hex, 16).ok()?;
                            self.i += 4;
                            if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                            }
                        }
                        other => out.push(other as char),
                    }
                }
                _ => {
                    // Reconstruye UTF-8 multibyte tal cual (el texto ya viene en UTF-8).
                    out.push(c as char);
                }
            }
        }
        Some(out)
    }

    fn parse_num(&mut self) -> Option<Json> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' || c == b'e' || c == b'E' {
                self.i += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        s.parse::<f64>().ok().map(Json::Num)
    }
}

fn obj_get<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn as_i32(v: Option<&Json>, default: i32) -> i32 {
    match v {
        Some(Json::Num(n)) => *n as i32,
        _ => default,
    }
}

fn as_u8(v: Option<&Json>, default: u8) -> u8 {
    match v {
        Some(Json::Num(n)) => *n as u8,
        _ => default,
    }
}

fn as_bool(v: Option<&Json>, default: bool) -> bool {
    match v {
        Some(Json::Bool(b)) => *b,
        _ => default,
    }
}

fn as_str(v: Option<&Json>) -> String {
    match v {
        Some(Json::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn parse_notes(text: &str) -> Vec<NoteData> {
    let mut p = P::new(text);
    let root = match p.parse_value() {
        Some(v) => v,
        None => return Vec::new(),
    };
    let items = match root {
        Json::Arr(items) => items,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in items {
        if let Json::Obj(fields) = item {
            out.push(NoteData {
                id: as_i32(obj_get(&fields, "id"), 0) as u32,
                x: as_i32(obj_get(&fields, "x"), 80),
                y: as_i32(obj_get(&fields, "y"), 80),
                w: as_i32(obj_get(&fields, "w"), 260),
                h: as_i32(obj_get(&fields, "h"), 280),
                color: as_u8(obj_get(&fields, "color"), 0),
                layer: Layer::from_u8(as_u8(obj_get(&fields, "layer"), 0)),
                roll_mode: RollMode::from_u8(as_u8(obj_get(&fields, "rollMode"), 0)),
                rolled: as_bool(obj_get(&fields, "rolled"), false),
                text: as_str(obj_get(&fields, "text")),
            });
        }
    }
    out
}
