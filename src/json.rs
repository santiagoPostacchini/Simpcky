//! JSON mínimo, escrito a mano (sin serde, para que el binario siga
//! chico): un parser recursivo que alcanza para `notes.json`, los
//! ajustes, el documento de sincronización y las respuestas de Google,
//! más el escape de cadenas para escribir.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Num(f64),
    Str(String),
    Bool(bool),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    Null,
}

impl Json {
    /// Campo de un objeto (`None` si no es un objeto o no está).
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    /// Atajos con valor por defecto, para leer campos opcionales.
    pub fn str_or(&self, key: &str, default: &str) -> String {
        self.get(key).and_then(Json::as_str).unwrap_or(default).to_string()
    }

    pub fn i32_or(&self, key: &str, default: i32) -> i32 {
        self.get(key).and_then(Json::as_f64).map(|n| n as i32).unwrap_or(default)
    }

    pub fn u8_or(&self, key: &str, default: u8) -> u8 {
        self.get(key).and_then(Json::as_f64).map(|n| n as u8).unwrap_or(default)
    }

    /// Enteros grandes (marcas de tiempo en milisegundos): un `f64`
    /// representa exacto cualquier entero hasta 2^53, y una marca de
    /// tiempo actual anda por 2^41.
    pub fn u64_or(&self, key: &str, default: u64) -> u64 {
        self.get(key).and_then(Json::as_f64).map(|n| n.max(0.0) as u64).unwrap_or(default)
    }

    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Json::as_bool).unwrap_or(default)
    }
}

pub fn parse(text: &str) -> Option<Json> {
    let mut p = P { b: text.as_bytes(), i: 0 };
    p.parse_value()
}

/// Escapa una cadena para ir entre comillas en un JSON.
pub fn escape(s: &str) -> String {
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

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\r' | b'\n') = self.b.get(self.i) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, word: &[u8]) -> bool {
        if self.b.get(self.i..self.i + word.len()) == Some(word) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn parse_value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.peek()? {
            b'{' => self.parse_obj(),
            b'[' => self.parse_arr(),
            b'"' => self.parse_str().map(Json::Str),
            b't' => self.eat(b"true").then_some(Json::Bool(true)),
            b'f' => self.eat(b"false").then_some(Json::Bool(false)),
            b'n' => self.eat(b"null").then_some(Json::Null),
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
                b',' => self.i += 1,
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
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Json::Arr(items))
    }

    fn hex4(&mut self) -> Option<u32> {
        let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        let v = u32::from_str_radix(hex, 16).ok()?;
        self.i += 4;
        Some(v)
    }

    /// Lee un string JSON. Los bytes se juntan tal cual y se decodifican
    /// como UTF-8 al final: convertir cada byte por separado a `char`
    /// rompía toda tilde o eñe ("Ã±"), y empeoraba con cada guardado.
    fn parse_str(&mut self) -> Option<String> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let esc = *self.b.get(self.i)?;
                    self.i += 1;
                    match esc {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            // Par sustituto UTF-16 (caracteres fuera del
                            // plano básico, como los emoji).
                            if (0xD800..0xDC00).contains(&cp) && self.eat(b"\\u") {
                                let low = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other), // \" \\ \/
                    }
                }
                _ => out.push(c),
            }
        }
        Some(String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
    }

    fn parse_num(&mut self) -> Option<Json> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        s.parse::<f64>().ok().map(Json::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_escape() {
        let s = "línea 1\n\"comillas\" \\ ñ 😀 \t fin";
        let json = format!("{{\"t\":\"{}\"}}", escape(s));
        assert_eq!(parse(&json).unwrap().str_or("t", ""), s);
    }

    #[test]
    fn surrogate_pairs_and_numbers() {
        let v = parse(r#"{"e":"😀","n":1790079957123,"b":true,"x":null,"a":[1,2]}"#).unwrap();
        assert_eq!(v.str_or("e", ""), "😀");
        assert_eq!(v.u64_or("n", 0), 1_790_079_957_123);
        assert!(v.bool_or("b", false));
        assert_eq!(v.get("x"), Some(&Json::Null));
        assert_eq!(v.get("a").and_then(Json::as_array).map(|a| a.len()), Some(2));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("{\"a\":").is_none());
        assert!(parse("tru").is_none());
    }
}
