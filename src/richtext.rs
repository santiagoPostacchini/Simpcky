//! Formato del texto de una nota, sin ventanas: lo que se guarda y se
//! sincroniza, y las reglas para reconocer listas y emojis.
//!
//! El texto sigue siendo texto plano (`NoteData::text`, con `\n`), y
//! las listas viven en él mismo como un prefijo al comienzo del
//! párrafo — `☐ ` / `☑ ` (tarea) o `• ` (viñeta) —, así una versión
//! vieja de Simpcky, la vista previa de "Todas las notas" o un copiar y
//! pegar las siguen mostrando bien. Lo único que va aparte es el estilo
//! (negrita, cursiva, subrayado, tachado), en `NoteData::fmt`: tramos en
//! unidades UTF-16 del texto, que es como cuenta posiciones el RichEdit
//! (y un `\n` ocupa una, igual que su `\r`).
//!
//! La cadena de estilos es canónica (mismo formato → misma cadena), así
//! dos compus con Windows distintos no se pasan la misma nota de una a
//! otra "arreglándola" para siempre, como pasaría guardando RTF.

pub const BOX_OPEN: char = '\u{2610}'; // ☐
pub const BOX_DONE: char = '\u{2611}'; // ☑
pub const BULLET: char = '\u{2022}'; // •

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum List {
    None,
    Bullet,
    Todo { done: bool },
}

impl List {
    pub fn prefix(self) -> &'static str {
        match self {
            List::None => "",
            List::Bullet => "\u{2022} ",
            List::Todo { done: false } => "\u{2610} ",
            List::Todo { done: true } => "\u{2611} ",
        }
    }
}

/// Qué es un párrafo (sus unidades UTF-16, sin el salto final) y cuánto
/// mide su prefijo. El espacio después de la marca es opcional: si el
/// usuario lo borró, la marca sola sigue contando.
pub fn list_of(para: &[u16]) -> (List, usize) {
    let kind = match para.first().copied() {
        Some(c) if c == BOX_OPEN as u16 => List::Todo { done: false },
        Some(c) if c == BOX_DONE as u16 => List::Todo { done: true },
        Some(c) if c == BULLET as u16 => List::Bullet,
        _ => return (List::None, 0),
    };
    let len = if para.get(1) == Some(&(' ' as u16)) { 2 } else { 1 };
    (kind, len)
}

/// Lo que se escribió al comienzo de un párrafo que, seguido de un
/// espacio, lo convierte en lista (como en Keep o Notion): `[]` o `[ ]`
/// → tarea, `-` o `*` → viñeta.
pub fn shortcut_list(typed: &[u16]) -> Option<List> {
    let s = String::from_utf16_lossy(typed);
    match s.as_str() {
        "[]" | "[ ]" => Some(List::Todo { done: false }),
        "[x]" | "[X]" => Some(List::Todo { done: true }),
        "-" | "*" => Some(List::Bullet),
        _ => None,
    }
}

/// Los párrafos de un texto de RichEdit (separados por `\r`), como
/// (inicio, fin) en unidades UTF-16, sin el separador.
pub fn paragraphs(text: &[u16]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &c) in text.iter().enumerate() {
        if c == '\r' as u16 || c == '\n' as u16 {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, text.len()));
    out
}

// -----------------------------------------------------------------
// Estilos
// -----------------------------------------------------------------

/// Negrita, cursiva, subrayado, tachado — en ese orden en todos lados.
pub const STYLES: usize = 4;
const LETTERS: [char; STYLES] = ['b', 'i', 'u', 's'];
pub const BOLD: usize = 0;
pub const ITALIC: usize = 1;
pub const UNDERLINE: usize = 2;
pub const STRIKE: usize = 3;

/// Tramos (inicio, largo) de cada estilo, ordenados y sin solaparse.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Styles {
    pub spans: [Vec<(u32, u32)>; STYLES],
}

impl Styles {
    /// Ordena, junta tramos pegados o superpuestos y recorta lo que
    /// pase del largo del texto.
    pub fn normalize(&mut self, text_len: u32) {
        for spans in self.spans.iter_mut() {
            let mut v: Vec<(u32, u32)> = spans
                .iter()
                .filter_map(|&(s, l)| {
                    let end = s.saturating_add(l).min(text_len);
                    (end > s).then_some((s, end))
                })
                .collect();
            v.sort();
            let mut merged: Vec<(u32, u32)> = Vec::with_capacity(v.len());
            for (s, e) in v {
                match merged.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => merged.push((s, e)),
                }
            }
            *spans = merged.into_iter().map(|(s, e)| (s, e - s)).collect();
        }
    }

    /// Quita el estilo `style` de [start, end).
    pub fn clear_range(&mut self, style: usize, start: u32, end: u32) {
        let mut out = Vec::new();
        for &(s, l) in &self.spans[style] {
            let e = s + l;
            if e <= start || s >= end {
                out.push((s, l));
                continue;
            }
            if s < start {
                out.push((s, start - s));
            }
            if e > end {
                out.push((end, e - end));
            }
        }
        self.spans[style] = out;
    }
}

/// `"b0+5,12+8 i3+1"`: cada estilo con su letra y sus tramos. Vacío si
/// no hay ninguno.
pub fn styles_to_string(st: &Styles) -> String {
    let mut parts = Vec::new();
    for (k, spans) in st.spans.iter().enumerate() {
        if spans.is_empty() {
            continue;
        }
        let list: Vec<String> = spans.iter().map(|(s, l)| format!("{s}+{l}")).collect();
        parts.push(format!("{}{}", LETTERS[k], list.join(",")));
    }
    parts.join(" ")
}

/// Lo inverso de [`styles_to_string`]. Lo que no se entiende se ignora
/// (un archivo tocado a mano no rompe la nota, a lo sumo pierde algún
/// estilo).
pub fn styles_from_str(s: &str, text_len: u32) -> Styles {
    let mut st = Styles::default();
    for part in s.split_whitespace() {
        let mut chars = part.chars();
        let Some(letter) = chars.next() else { continue };
        let Some(k) = LETTERS.iter().position(|&c| c == letter) else { continue };
        for span in chars.as_str().split(',') {
            let Some((a, b)) = span.split_once('+') else { continue };
            if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                st.spans[k].push((a, b));
            }
        }
    }
    st.normalize(text_len);
    st
}

pub fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

// -----------------------------------------------------------------
// Emojis
// -----------------------------------------------------------------

/// Los emojis de un texto UTF-16, como (inicio, fin): secuencias
/// completas (piel, género, familias con ZWJ, banderas, teclas), para
/// dibujarlas en color de una sola vez.
///
/// El RichEdit de Windows dibuja con GDI, que no sabe de fuentes de
/// color: sin esto los emojis salen en blanco y negro (ver `editor.rs`).
pub fn emoji_clusters(text: &[u16]) -> Vec<(usize, usize)> {
    let cps = decode(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < cps.len() {
        let (pos, cp) = cps[i];
        let next = cps.get(i + 1).map(|&(_, c)| c);
        let keycap = matches!(cp, 0x30..=0x39 | 0x23 | 0x2A) && next == Some(0xFE0F) && cps.get(i + 2).map(|c| c.1) == Some(0x20E3);
        if !(is_emoji(cp, next) || keycap) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if is_regional(cp) && next.is_some_and(is_regional) {
            j += 1; // bandera: dos letras regionales
        }
        loop {
            match cps.get(j).map(|c| c.1) {
                Some(0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F) => j += 1,
                Some(0x200D) if cps.get(j + 1).is_some() => j += 2,
                _ => break,
            }
        }
        let end = cps.get(j).map(|&(p, _)| p).unwrap_or(text.len());
        out.push((pos, end));
        i = j;
    }
    out
}

fn decode(text: &[u16]) -> Vec<(usize, u32)> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let u = text[i] as u32;
        if (0xD800..0xDC00).contains(&u) && i + 1 < text.len() && (0xDC00..0xE000).contains(&(text[i + 1] as u32)) {
            let cp = 0x10000 + ((u - 0xD800) << 10) + (text[i + 1] as u32 - 0xDC00);
            out.push((i, cp));
            i += 2;
        } else {
            out.push((i, u));
            i += 1;
        }
    }
    out
}

fn is_regional(cp: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&cp)
}

fn is_emoji(cp: u32, next: Option<u32>) -> bool {
    if (0x1F000..=0x1FAFF).contains(&cp) && !(0x1F3FB..=0x1F3FF).contains(&cp) {
        return true;
    }
    // Símbolos del plano básico que son emoji por defecto (Unicode,
    // Emoji_Presentation=Yes). El resto de esa zona (☐ ☑ ✔ ❤ …) es
    // texto salvo que venga seguido del selector de emoji U+FE0F — y
    // así las casillas de las listas de tareas nunca se pintan como
    // emoji.
    const DEFAULT: &[(u32, u32)] = &[
        (0x231A, 0x231B),
        (0x23E9, 0x23EC),
        (0x23F0, 0x23F0),
        (0x23F3, 0x23F3),
        (0x25FD, 0x25FE),
        (0x2614, 0x2615),
        (0x2648, 0x2653),
        (0x267F, 0x267F),
        (0x2693, 0x2693),
        (0x26A1, 0x26A1),
        (0x26AA, 0x26AB),
        (0x26BD, 0x26BE),
        (0x26C4, 0x26C5),
        (0x26CE, 0x26CE),
        (0x26D4, 0x26D4),
        (0x26EA, 0x26EA),
        (0x26F2, 0x26F3),
        (0x26F5, 0x26F5),
        (0x26FA, 0x26FA),
        (0x26FD, 0x26FD),
        (0x2705, 0x2705),
        (0x270A, 0x270B),
        (0x2728, 0x2728),
        (0x274C, 0x274C),
        (0x274E, 0x274E),
        (0x2753, 0x2755),
        (0x2757, 0x2757),
        (0x2795, 0x2797),
        (0x27B0, 0x27B0),
        (0x27BF, 0x27BF),
        (0x2B1B, 0x2B1C),
        (0x2B50, 0x2B50),
        (0x2B55, 0x2B55),
    ];
    if DEFAULT.iter().any(|&(a, b)| (a..=b).contains(&cp)) {
        return true;
    }
    let symbol = matches!(cp, 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x2300..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299);
    symbol && next == Some(0xFE0F)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn styles_roundtrip_and_normalize() {
        let st = styles_from_str("b12+8,0+5,4+3 i3+1 x9+9 s2+100 u", 30);
        assert_eq!(st.spans[BOLD], vec![(0, 7), (12, 8)]);
        assert_eq!(st.spans[ITALIC], vec![(3, 1)]);
        assert_eq!(st.spans[STRIKE], vec![(2, 28)]);
        assert!(st.spans[UNDERLINE].is_empty());
        assert_eq!(styles_to_string(&st), "b0+7,12+8 i3+1 s2+28");
        assert_eq!(styles_from_str(&styles_to_string(&st), 30), st);
        assert_eq!(styles_to_string(&Styles::default()), "");
    }

    #[test]
    fn clear_range_splits_spans() {
        let mut st = styles_from_str("s0+10", 10);
        st.clear_range(STRIKE, 3, 6);
        assert_eq!(st.spans[STRIKE], vec![(0, 3), (6, 4)]);
    }

    #[test]
    fn recognizes_lists() {
        assert_eq!(list_of(&w("☐ pan")), (List::Todo { done: false }, 2));
        assert_eq!(list_of(&w("☑ pan")), (List::Todo { done: true }, 2));
        assert_eq!(list_of(&w("• pan")), (List::Bullet, 2));
        assert_eq!(list_of(&w("☐pan")), (List::Todo { done: false }, 1));
        assert_eq!(list_of(&w("pan ☐")), (List::None, 0));
        assert_eq!(shortcut_list(&w("[ ]")), Some(List::Todo { done: false }));
        assert_eq!(shortcut_list(&w("-")), Some(List::Bullet));
        assert_eq!(shortcut_list(&w("--")), None);
        assert_eq!(paragraphs(&w("a\rbc\r")), vec![(0, 1), (2, 4), (5, 5)]);
    }

    #[test]
    fn finds_whole_emoji_sequences() {
        let t = w("hola 😀 👍🏽 👨‍👩‍👧 🇦🇷 ❤️ ✅ ☐ ☑ ✔ 1️⃣.");
        let found: Vec<String> = emoji_clusters(&t).iter().map(|&(a, b)| String::from_utf16_lossy(&t[a..b])).collect();
        assert_eq!(found, vec!["😀", "👍🏽", "👨‍👩‍👧", "🇦🇷", "❤️", "✅", "1️⃣"]);
    }
}
