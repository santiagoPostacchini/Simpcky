//! Un editor de texto chico sobre DirectWrite: cursor, selección, flechas,
//! Inicio/Fin, borrar, Enter, Ctrl+A/C/X/V, clic y arrastre para elegir.
//! El texto se guarda en UTF-16, que es lo que DirectWrite cuenta.

use windows::Win32::Graphics::DirectWrite::*;

#[derive(Default)]
pub struct Editor {
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    /// Columna (en píxeles) que se quiere conservar al subir y bajar.
    want_x: Option<f32>,
    pub layout: Option<IDWriteTextLayout>,
    pub layout_w: f32,
}

impl Editor {
    pub fn new(text: &str) -> Editor {
        let text: Vec<u16> = text.encode_utf16().collect();
        let n = text.len();
        Editor { text, caret: n, anchor: n, ..Default::default() }
    }

    pub fn string(&self) -> String {
        String::from_utf16_lossy(&self.text)
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn changed(&mut self) {
        self.layout = None;
        self.want_x = None;
    }

    pub fn insert(&mut self, s: &[u16]) {
        let (a, b) = self.selection();
        self.text.splice(a..b, s.iter().copied());
        self.caret = a + s.len();
        self.anchor = self.caret;
        self.changed();
    }

    pub fn backspace(&mut self, word: bool) {
        let (a, b) = self.selection();
        if a != b {
            return self.insert(&[]);
        }
        if a == 0 {
            return;
        }
        let from = if word { self.word_left(a) } else { self.prev(a) };
        self.anchor = from;
        self.caret = a;
        self.insert(&[]);
    }

    pub fn delete(&mut self, word: bool) {
        let (a, b) = self.selection();
        if a != b {
            return self.insert(&[]);
        }
        if a >= self.text.len() {
            return;
        }
        let to = if word { self.word_right(a) } else { self.next(a) };
        self.anchor = a;
        self.caret = to;
        self.insert(&[]);
    }

    /// Pares sustitutos: no cortar un emoji por la mitad.
    fn prev(&self, i: usize) -> usize {
        if i >= 2 && (0xDC00..0xE000).contains(&self.text[i - 1]) && (0xD800..0xDC00).contains(&self.text[i - 2]) {
            i - 2
        } else {
            i.saturating_sub(1)
        }
    }
    fn next(&self, i: usize) -> usize {
        if i + 1 < self.text.len() && (0xD800..0xDC00).contains(&self.text[i]) {
            i + 2
        } else {
            (i + 1).min(self.text.len())
        }
    }
    fn is_word(c: u16) -> bool {
        char::from_u32(c as u32).is_some_and(|c| c.is_alphanumeric())
    }
    fn word_left(&self, mut i: usize) -> usize {
        while i > 0 && !Self::is_word(self.text[i - 1]) {
            i -= 1;
        }
        while i > 0 && Self::is_word(self.text[i - 1]) {
            i -= 1;
        }
        i
    }
    fn word_right(&self, mut i: usize) -> usize {
        let n = self.text.len();
        while i < n && !Self::is_word(self.text[i]) {
            i += 1;
        }
        while i < n && Self::is_word(self.text[i]) {
            i += 1;
        }
        i
    }

    fn go(&mut self, to: usize, extend: bool) {
        self.caret = to.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn left(&mut self, extend: bool, word: bool) {
        let (a, b) = self.selection();
        let to = if a != b && !extend { a } else if word { self.word_left(self.caret) } else { self.prev(self.caret) };
        self.want_x = None;
        self.go(to, extend);
    }

    pub fn right(&mut self, extend: bool, word: bool) {
        let (a, b) = self.selection();
        let to = if a != b && !extend { b } else if word { self.word_right(self.caret) } else { self.next(self.caret) };
        self.want_x = None;
        self.go(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        let mut i = self.caret;
        while i > 0 && self.text[i - 1] != b'\n' as u16 {
            i -= 1;
        }
        self.want_x = None;
        self.go(i, extend);
    }

    pub fn end(&mut self, extend: bool) {
        let mut i = self.caret;
        while i < self.text.len() && self.text[i] != b'\n' as u16 {
            i += 1;
        }
        self.want_x = None;
        self.go(i, extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Dónde está el cursor (x, arriba, alto) en el texto.
    pub fn caret_pos(&self) -> Option<(f32, f32, f32)> {
        let l = self.layout.as_ref()?;
        let (mut x, mut y) = (0.0, 0.0);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe { l.HitTestTextPosition(self.caret as u32, false, &mut x, &mut y, &mut m).ok()? };
        Some((x, y, m.height))
    }

    /// Una línea arriba o abajo, conservando la columna.
    pub fn vertical(&mut self, down: bool, extend: bool) {
        let Some((x, y, h)) = self.caret_pos() else { return };
        let want = *self.want_x.get_or_insert(x);
        let ty = if down { y + h * 1.5 } else { y - h * 0.5 };
        if ty < 0.0 {
            return self.go(0, extend);
        }
        let to = self.hit(want, ty);
        let keep = self.want_x;
        self.go(to, extend);
        self.want_x = keep;
    }

    /// El lugar del texto en (x, y), relativo al origen del texto.
    pub fn hit(&self, x: f32, y: f32) -> usize {
        let Some(l) = self.layout.as_ref() else { return self.text.len() };
        let (mut trailing, mut inside) = (Default::default(), Default::default());
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            if l.HitTestPoint(x, y, &mut trailing, &mut inside, &mut m).is_err() {
                return self.text.len();
            }
        }
        let i = m.textPosition as usize + if trailing.as_bool() { m.length as usize } else { 0 };
        i.min(self.text.len())
    }

    pub fn click(&mut self, x: f32, y: f32, extend: bool) {
        let i = self.hit(x, y);
        self.want_x = None;
        self.go(i, extend);
    }

    pub fn selected(&self) -> Vec<u16> {
        let (a, b) = self.selection();
        self.text[a..b].to_vec()
    }
}
