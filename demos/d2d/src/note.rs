//! Una nota: su ventana, su superficie y cómo se dibuja.
//!
//! La superficie se hace del tamaño de la nota **abierta**: al enrollarse,
//! la ventana se achica y el compositor recorta el contenido, que queda
//! quieto (como en Simpcky: "la nota lo tapa"). Solo se vuelve a pintar si
//! cambia el ancho, el texto, el color o lo que hay bajo el mouse.

use std::time::Instant;

use common::store::NoteData;
use common::theme;
use windows::Win32::Foundation::HWND as WHWND;

use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_sys::Win32::Foundation::*;

use crate::editor::Editor;
use crate::gfx::{self, Gfx, Surface};

pub const BAR: i32 = 40;
const PAD: f32 = 12.0;

pub struct Fonts {
    pub title: IDWriteTextFormat,
    pub body: IDWriteTextFormat,
    pub icons: IDWriteTextFormat,
    pub label: IDWriteTextFormat,
}

impl Fonts {
    pub fn new(g: &Gfx) -> Fonts {
        let title = gfx::text_format(g, "Segoe UI", 14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD);
        let icons = gfx::text_format(g, "Segoe Fluent Icons", 14.0, DWRITE_FONT_WEIGHT_NORMAL);
        let label = gfx::text_format(g, "Segoe UI", 13.0, DWRITE_FONT_WEIGHT_NORMAL);
        unsafe {
            for f in [&title, &icons] {
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            }
            let _ = icons.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = label.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let _ = label.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_FAR);
        }
        Fonts { title, body: gfx::text_format(g, "Segoe UI", 15.0, DWRITE_FONT_WEIGHT_NORMAL), icons, label }
    }
}

/// Botones del encabezado (de derecha a izquierda): pin, nota nueva.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Button {
    Pin,
    New,
}

pub struct NoteWin {
    pub data: NoteData,
    pub hwnd: HWND,
    pub surf: Surface,
    pub ed: Editor,
    pub hovered: bool,
    pub focused: bool,
    pub hot: Option<Button>,
    /// Botón apretado (se activa al soltar encima).
    pub pressed: Option<Button>,
    /// Eligiendo texto con el mouse.
    pub selecting: bool,
    /// Auto: se enrolla en este momento si el mouse no volvió.
    pub roll_at: Option<Instant>,
}

impl NoteWin {
    pub fn new(g: &Gfx, hwnd: HWND, data: NoteData) -> NoteWin {
        let surf = Surface::new(g, WHWND(hwnd as _), data.w.max(1) as u32, data.h.max(BAR) as u32).expect("superficie");
        let ed = Editor::new(&data.text);
        NoteWin { data, hwnd, surf, ed, hovered: false, focused: false, hot: None, pressed: None, selecting: false, roll_at: None }
    }

    pub fn buttons(&self, w: i32) -> [(Button, RECT); 2] {
        let r = |i: i32| RECT { left: w - 6 - 32 * (i + 1), top: 5, right: w - 6 - 32 * i, bottom: 35 };
        [(Button::Pin, r(0)), (Button::New, r(1))]
    }

    pub fn button_at(&self, x: i32, y: i32) -> Option<Button> {
        if !self.hovered {
            return None;
        }
        let w = self.surf.size.0 as i32;
        self.buttons(w).iter().find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(b, _)| *b)
    }

    fn text_width(&self) -> f32 {
        (self.surf.size.0 as f32 - 2.0 * PAD).max(10.0)
    }

    /// El texto armado para el ancho actual (se rehace si cambió algo).
    pub fn ensure_layout(&mut self, g: &Gfx, fonts: &Fonts) {
        let w = self.text_width();
        if self.ed.layout.is_some() && self.ed.layout_w == w {
            return;
        }
        let h = (self.surf.size.1 as f32 - BAR as f32 - 16.0).max(10.0);
        self.ed.layout = unsafe { g.dwrite.CreateTextLayout(&self.ed.text, &fonts.body, w, h).ok() };
        self.ed.layout_w = w;
    }

    /// Origen del texto dentro de la ventana.
    pub fn text_origin() -> (f32, f32) {
        (PAD, BAR as f32 + 8.0)
    }

    pub fn paint(&mut self, g: &Gfx, fonts: &Fonts, dark: bool, glass: u8) {
        let start = Instant::now();
        self.ensure_layout(g, fonts);
        let (w, h) = (self.surf.size.0 as f32, self.surf.size.1 as f32);
        let (head, body, ink) = theme::note_colors(self.data.color, dark);
        let (ha, ba) = theme::glass_alpha(glass, dark);
        let (ha, ba) = (ha as f32 / 255.0, ba as f32 / 255.0);
        let title_a = theme::title_alpha(glass, dark) as f32 / 255.0;
        let title: Vec<u16> = self.data.display_title().encode_utf16().collect();
        let pinned = self.data.layer == 2;
        let (hovered, hot, focused) = (self.hovered, self.hot, self.focused);
        let buttons = self.buttons(w as i32);
        let layout = self.ed.layout.clone();
        let (sel_a, sel_b) = self.ed.selection();
        let caret = if focused { self.ed.caret_pos() } else { None };
        let _ = self.surf.draw(g, |dc| unsafe {
            dc.FillRectangle(&gfx::rectf(0.0, BAR as f32, w, h), &gfx::brush(dc, gfx::color(body, ba)));
            dc.FillRectangle(&gfx::rectf(0.0, 0.0, w, BAR as f32), &gfx::brush(dc, gfx::color(head, ha)));
            let ink_b = gfx::brush(dc, gfx::color(ink, 1.0));
            let right = if hovered { buttons[1].1.left as f32 - 4.0 } else { w - PAD };
            dc.DrawText(&title, &fonts.title, &gfx::rectf(PAD, 0.0, right, BAR as f32), &gfx::brush(dc, gfx::color(ink, title_a)), D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, DWRITE_MEASURING_MODE_NATURAL);
            if hovered {
                for (b, r) in buttons {
                    let rf = gfx::rectf(r.left as f32, r.top as f32, r.right as f32, r.bottom as f32);
                    if hot == Some(b) {
                        dc.FillRoundedRectangle(&gfx::rounded(rf, 5.0), &gfx::brush(dc, gfx::color(ink, 0.12)));
                    }
                    let glyph: &[u16] = match b {
                        Button::New => &[0xE710],
                        Button::Pin if pinned => &[0xE840],
                        Button::Pin => &[0xE718],
                    };
                    dc.DrawText(glyph, &fonts.icons, &rf, &ink_b, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                }
            }
            let (ox, oy) = Self::text_origin();
            if let Some(l) = &layout {
                if focused && sel_a != sel_b {
                    let mut n = 0u32;
                    let _ = l.HitTestTextRange(sel_a as u32, (sel_b - sel_a) as u32, ox, oy, None, &mut n);
                    let mut v = vec![DWRITE_HIT_TEST_METRICS::default(); n as usize];
                    if l.HitTestTextRange(sel_a as u32, (sel_b - sel_a) as u32, ox, oy, Some(&mut v), &mut n).is_ok() {
                        let sb = gfx::brush(dc, gfx::color(0x3b82f6, 0.30));
                        for m in &v[..n as usize] {
                            dc.FillRectangle(&gfx::rectf(m.left, m.top, m.left + m.width.max(4.0), m.top + m.height), &sb);
                        }
                    }
                }
                dc.DrawTextLayout(windows_numerics::Vector2 { X: ox, Y: oy }, l, &ink_b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
            }
            if let Some((cx, cy, ch)) = caret {
                dc.FillRectangle(&gfx::rectf(ox + cx, oy + cy, ox + cx + 1.5, oy + cy + ch), &ink_b);
            }
        });
        common::framelog::mark("nota", self.data.id, start.elapsed());
    }
}
