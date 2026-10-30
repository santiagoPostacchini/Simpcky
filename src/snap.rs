//! Acomodar notas en celdas y pilas al arrastrarlas.
//!
//! - **Celdas**: el área de trabajo de cada monitor se divide en celdas
//!   de al menos 220 × 180 (al 100 %), estiradas para llenarla. Llevar la
//!   nota al borde de arriba muestra un panel (1×1, 2×1, 1×2, 2×2 y pila):
//!   al pasar por una opción la nota toma esa forma. Con Ctrl, la nota
//!   encaja con su propio tamaño. Ctrl + borde redimensiona de a una
//!   celda. Si la nota cae sobre otra suelta, la otra se achica a la franja
//!   libre más grande (nunca queda en L) o se corre al lugar libre más
//!   cercano.
//! - **Pilas**: notas enrolladas alineadas por un borde, una debajo de
//!   otra. No se guardan: se reconocen por la posición (con tolerancia,
//!   para las armadas a mano). Soltar una nota sobre una pila la suma
//!   enrollada; sacarla la abre y la pasa a Manual; Ctrl + arrastrar mueve
//!   la pila entera de celda en celda.
//!
//! El arrastre por la barra es propio (no el de Windows) para que la nota
//! pueda cambiar de forma mientras se la lleva. Todo se mueve con las
//! transiciones de `anim.rs`; cuadrícula, vista previa, panel y guías son
//! capas transparentes a los clics que aparecen y se desplazan suave.
//!
//! Arriba, la geometría sin ventanas (con tests). Abajo, el arrastre y las capas.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetCapture, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::anim;
use crate::app::{app, save_all};
use crate::persist::{Layer, RollMode};
use crate::win::wide;

// =================================================================
// Geometría
// =================================================================

/// Celda mínima y separación, a 100 %.
const CELL_W: i32 = 220;
const CELL_H: i32 = 180;
const GAP: i32 = 12;
/// Separación entre las barras de una pila, a 100 %.
const PILE_GAP: i32 = 10;
/// Lo más larga que puede quedar una nota de pila, en celdas.
const PILE_MAX_CELLS: i32 = 2;

/// Un rango de celdas, inclusivo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cells {
    pub c0: i32,
    pub c1: i32,
    pub r0: i32,
    pub r1: i32,
}

impl Cells {
    fn cols(&self) -> i32 {
        self.c1 - self.c0 + 1
    }
    fn rows(&self) -> i32 {
        self.r1 - self.r0 + 1
    }
    fn area(&self) -> i32 {
        self.cols() * self.rows()
    }
    fn meets(&self, o: &Cells) -> bool {
        self.c0 <= o.c1 && o.c0 <= self.c1 && self.r0 <= o.r1 && o.r0 <= self.r1
    }
    fn has(&self, c: i32, r: i32) -> bool {
        (self.c0..=self.c1).contains(&c) && (self.r0..=self.r1).contains(&r)
    }
}

/// La cuadrícula de un área de trabajo, en coordenadas de pantalla.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    left: i32,
    top: i32,
    cols: i32,
    rows: i32,
    cw: f64,
    ch: f64,
    gap: f64,
}

impl Grid {
    /// Tantas celdas de al menos `CELL_W` × `CELL_H` como entren, estiradas
    /// para llenar `work` sin sobrantes. `dpi` ya incluye la escala de las notas.
    #[cfg(test)]
    pub fn new(work: RECT, dpi: i32) -> Grid {
        Grid::fitted(work, dpi, &[], &[])
    }

    /// Como `new`, pero con tantas columnas y filas que los bordes de las
    /// ventanas acomodadas (`xs`, `ys`, en pantalla) caigan entre celdas:
    /// así la cuadrícula sigue el acomodo de Windows (mitades, cuartos, tercios).
    pub fn fitted(work: RECT, dpi: i32, xs: &[i32], ys: &[i32]) -> Grid {
        let gap = GAP * dpi / 96;
        let (mw, mh) = (CELL_W * dpi / 96, CELL_H * dpi / 96);
        let (w, h) = (work.right - work.left, work.bottom - work.top);
        let rel = |v: &[i32], o: i32| v.iter().map(|e| e - o).collect::<Vec<_>>();
        let cols = fit_count(w, mw, gap, &rel(xs, work.left));
        let rows = fit_count(h, mh, gap, &rel(ys, work.top));
        Grid {
            left: work.left,
            top: work.top,
            cols,
            rows,
            cw: (w - gap) as f64 / cols as f64 - gap as f64,
            ch: (h - gap) as f64 / rows as f64 - gap as f64,
            gap: gap as f64,
        }
    }

    fn sx(&self, c: i32) -> i32 {
        self.left + (self.gap + c as f64 * (self.cw + self.gap)).round() as i32
    }
    fn ex(&self, c: i32) -> i32 {
        self.left + (self.gap + c as f64 * (self.cw + self.gap) + self.cw).round() as i32
    }
    fn sy(&self, r: i32) -> i32 {
        self.top + (self.gap + r as f64 * (self.ch + self.gap)).round() as i32
    }
    fn ey(&self, r: i32) -> i32 {
        self.top + (self.gap + r as f64 * (self.ch + self.gap) + self.ch).round() as i32
    }

    pub fn rect(&self, k: Cells) -> RECT {
        RECT { left: self.sx(k.c0), top: self.sy(k.r0), right: self.ex(k.c1), bottom: self.ey(k.r1) }
    }

    fn col_of(&self, x: i32) -> i32 {
        (((x - self.left) as f64 - self.gap / 2.0) / (self.cw + self.gap)).floor().clamp(0.0, (self.cols - 1) as f64) as i32
    }
    fn row_of(&self, y: i32) -> i32 {
        (((y - self.top) as f64 - self.gap / 2.0) / (self.ch + self.gap)).floor().clamp(0.0, (self.rows - 1) as f64) as i32
    }
    /// Columna (o fila) cuyo comienzo está más cerca de `x` (o `y`).
    fn near_col(&self, x: i32, span: i32) -> i32 {
        (((x - self.left) as f64 - self.gap) / (self.cw + self.gap)).round().clamp(0.0, (self.cols - span).max(0) as f64) as i32
    }
    fn near_row(&self, y: i32, span: i32) -> i32 {
        (((y - self.top) as f64 - self.gap) / (self.ch + self.gap)).round().clamp(0.0, (self.rows - span).max(0) as f64) as i32
    }

    /// Las celdas que toca un rectángulo.
    pub fn cells_of(&self, r: &RECT) -> Cells {
        Cells { c0: self.col_of(r.left), c1: self.col_of(r.right - 1), r0: self.row_of(r.top), r1: self.row_of(r.bottom - 1) }
    }

    /// `a` × `b` celdas con la esquina más cerca de (`x`, `y`).
    pub fn snap_shape(&self, x: i32, y: i32, a: i32, b: i32) -> Cells {
        let (a, b) = (a.min(self.cols), b.min(self.rows));
        let (c, r) = (self.near_col(x, a), self.near_row(y, b));
        Cells { c0: c, c1: c + a - 1, r0: r, r1: r + b - 1 }
    }

    /// Cada borde a la línea de celda más cercana, mínimo una celda.
    pub fn snap_rect(&self, r: &RECT) -> Cells {
        let c0 = self.near_col(r.left, 1);
        let c1 = (((r.right - self.left) as f64 - self.gap - self.cw) / (self.cw + self.gap)).round() as i32;
        let r0 = self.near_row(r.top, 1);
        let r1 = (((r.bottom - self.top) as f64 - self.gap - self.ch) / (self.ch + self.gap)).round() as i32;
        Cells { c0, c1: c1.clamp(c0, self.cols - 1), r0, r1: r1.clamp(r0, self.rows - 1) }
    }

    /// Un ancho redondeado a celdas: entre una y `max`.
    fn width_for(&self, w: i32, max: i32) -> i32 {
        let n = ((w as f64 + self.gap) / (self.cw + self.gap)).round().clamp(1.0, max.min(self.cols) as f64);
        (n * self.cw + (n - 1.0) * self.gap).round() as i32
    }

    /// Un tamaño redondeado a celdas.
    pub fn shape_for(&self, w: i32, h: i32) -> (i32, i32) {
        let a = ((w as f64 + self.gap) / (self.cw + self.gap)).round() as i32;
        let b = ((h as f64 + self.gap) / (self.ch + self.gap)).round() as i32;
        (a.clamp(1, self.cols), b.clamp(1, self.rows))
    }

    fn snap_right_edge(&self, x: i32) -> i32 {
        let c = ((x - self.left) as f64 - self.gap - self.cw) / (self.cw + self.gap);
        self.ex(c.round().clamp(0.0, (self.cols - 1) as f64) as i32)
    }
    fn snap_left_edge(&self, x: i32) -> i32 {
        self.sx(self.near_col(x, 1))
    }
    fn snap_top(&self, y: i32) -> i32 {
        self.sy(self.near_row(y, 1))
    }
}

/// Cuántas celdas de al menos `min` (con `gap` entre ellas) en `len`. Sin
/// bordes que respetar, las que entran. Con bordes (relativos al comienzo),
/// la cantidad que deja más bordes justo entre dos celdas, y entre esas la
/// más cercana a lo que entra (se tolera achicar la celda hasta un 20 %).
fn fit_count(len: i32, min: i32, gap: i32, edges: &[i32]) -> i32 {
    let fits = ((len - gap) / (min + gap)).max(1);
    if edges.is_empty() {
        return fits;
    }
    let ideal = (len - gap) as f64 / (min + gap) as f64;
    let aligned = |n: i32| {
        let pitch = (len - gap) as f64 / n as f64;
        edges
            .iter()
            .filter(|&&e| {
                let k = ((e as f64 - gap as f64 / 2.0) / pitch).round();
                (e as f64 - (gap as f64 / 2.0 + k * pitch)).abs() <= gap as f64 / 2.0 + 2.0
            })
            .count()
    };
    (1..=(ideal * 1.25).ceil().max(1.0) as i32)
        .filter(|&n| n == 1 || (len - gap) as f64 / n as f64 - gap as f64 >= min as f64 * 0.8)
        .max_by(|&a, &b| aligned(a).cmp(&aligned(b)).then(((b as f64 - ideal).abs()).total_cmp(&(a as f64 - ideal).abs())))
        .unwrap_or(fits)
}

/// `a` sin las celdas de `o`. Lo que sobra puede ser una L; una nota es
/// un rectángulo, así que queda la franja libre más grande (a igual
/// tamaño: izquierda, arriba, derecha, abajo). `None` si no sobra nada.
pub fn shrink(a: Cells, o: Cells) -> Option<Cells> {
    let strips = [
        (o.c0 > a.c0).then_some(Cells { c1: o.c0 - 1, ..a }),
        (o.r0 > a.r0).then_some(Cells { r1: o.r0 - 1, ..a }),
        (o.c1 < a.c1).then_some(Cells { c0: o.c1 + 1, ..a }),
        (o.r1 < a.r1).then_some(Cells { r0: o.r1 + 1, ..a }),
    ];
    strips.into_iter().flatten().fold(None, |best: Option<Cells>, s| match best {
        Some(b) if b.area() >= s.area() => Some(b),
        _ => Some(s),
    })
}

/// El lugar de `a` × `b` celdas sin obstáculos más cercano a `near`.
fn find_free(g: &Grid, a: i32, b: i32, near: Cells, obst: &[Cells]) -> Option<Cells> {
    let mut best = None;
    let mut best_d = i32::MAX;
    for r in 0..=(g.rows - b) {
        for c in 0..=(g.cols - a) {
            let k = Cells { c0: c, c1: c + a - 1, r0: r, r1: r + b - 1 };
            let d = (c - near.c0).pow(2) + (r - near.r0).pow(2);
            if d < best_d && !obst.iter().any(|o| k.meets(o)) {
                best = Some(k);
                best_d = d;
            }
        }
    }
    best
}

fn overlaps(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// El rectángulo más grande de `work` que no tapa ninguna ventana (de al
/// menos `min_w` × `min_h`). Con ventanas acomodadas por Windows (mitades,
/// cuartos, tercios) es justo la parte libre, y la cuadrícula arranca en
/// el borde de la ventana. `None` si no queda lugar.
pub fn free_area(work: RECT, windows: &[RECT], min_w: i32, min_h: i32) -> Option<RECT> {
    let obs: Vec<RECT> = windows
        .iter()
        .map(|w| RECT { left: w.left.max(work.left), top: w.top.max(work.top), right: w.right.min(work.right), bottom: w.bottom.min(work.bottom) })
        .filter(|w| w.right > w.left && w.bottom > w.top)
        .collect();
    let mut xs = vec![work.left, work.right];
    let mut ys = vec![work.top, work.bottom];
    for o in &obs {
        xs.extend([o.left, o.right]);
        ys.extend([o.top, o.bottom]);
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let mut best: Option<(i64, RECT)> = None;
    for (i, &l) in xs.iter().enumerate() {
        for &r in &xs[i + 1..] {
            if r - l < min_w {
                continue;
            }
            for (j, &t) in ys.iter().enumerate() {
                for &b in &ys[j + 1..] {
                    let area = (r - l) as i64 * (b - t) as i64;
                    if b - t < min_h || best.is_some_and(|(a, _)| a >= area) {
                        continue;
                    }
                    let rc = RECT { left: l, top: t, right: r, bottom: b };
                    if !obs.iter().any(|o| overlaps(o, &rc)) {
                        best = Some((area, rc));
                    }
                }
            }
        }
    }
    best.map(|(_, r)| r)
}

/// Dónde queda la nota que se suelta en `t` y cómo se reacomodan las
/// otras sueltas (`others`: lo que ocupa cada una en pantalla). Solo se
/// tocan las que de verdad pisa: cada una se lee en las celdas más
/// cercanas a su tamaño (nunca crece) y se achica a la franja libre más
/// grande, o se corre al lugar libre más cercano. Las pilas (`zones`) no
/// se achican: si `t` las pisa, se achica `t`. Una nota corrida no
/// empuja a otra.
pub fn plan_around(g: &Grid, mut t: Cells, others: &[(u32, RECT)], zones: &[Cells]) -> (Cells, Vec<(u32, Cells)>) {
    for z in zones {
        if t.meets(z) {
            t = shrink(t, *z).unwrap_or(t);
        }
    }
    // Sigue pisando algo que no se corre (una aplicación, una pila): al lugar libre más cercano.
    if zones.iter().any(|z| t.meets(z)) {
        t = find_free(g, t.cols(), t.rows(), t, zones).or_else(|| find_free(g, 1, 1, t, zones)).unwrap_or(t);
    }
    let tr = g.rect(t);
    let mut ranges: Vec<(u32, Cells)> = others.iter().map(|(id, r)| (*id, g.snap_rect(r))).collect();
    let mut plan = Vec::new();
    for i in 0..ranges.len() {
        let (id, a) = ranges[i];
        if !overlaps(&others[i].1, &tr) {
            continue;
        }
        // Si redondeada ya no choca, alcanza con acomodarla en sus celdas.
        let r = if a.meets(&t) { shrink(a, t) } else { Some(a) }.or_else(|| {
            let mut obst: Vec<Cells> = vec![t];
            obst.extend_from_slice(zones);
            obst.extend(ranges.iter().filter(|(k, _)| *k != id).map(|(_, v)| *v));
            find_free(g, a.cols(), a.rows(), a, &obst).or_else(|| find_free(g, 1, 1, a, &obst))
        });
        if let Some(r) = r {
            ranges[i].1 = r;
            plan.push((id, r));
        }
    }
    (t, plan)
}

/// Una pila: notas enrolladas una debajo de la otra, la primera arriba
/// en `top`, una cada `step`. Ocupan una columna tan ancha como la más
/// ancha, anclada por un borde (`right`: el derecho, en `edge`): si la
/// columna crece, crece hacia el otro lado. Cada nota más angosta va
/// pegada al borde izquierdo o al derecho de la columna (`sides`, `true`
/// = derecho), según dónde se la soltó.
#[derive(Clone, PartialEq, Debug)]
pub struct Pile {
    pub right: bool,
    pub edge: i32,
    pub top: i32,
    pub ids: Vec<u32>,
    pub sides: Vec<bool>,
}

impl Pile {
    fn one(id: u32, right: bool, edge: i32, top: i32) -> Pile {
        Pile { right, edge, top, ids: vec![id], sides: vec![right] }
    }
    /// Los bordes de la columna, si la más ancha mide `max_w`.
    fn column(&self, max_w: i32) -> (i32, i32) {
        if self.right {
            (self.edge - max_w, self.edge)
        } else {
            (self.edge, self.edge + max_w)
        }
    }
    /// Esquina superior izquierda de la nota `i` (de ancho `w`).
    fn place(&self, i: usize, w: i32, max_w: i32, step: i32) -> (i32, i32) {
        let (l, r) = self.column(max_w);
        let right = self.sides.get(i).copied().unwrap_or(self.right);
        (if right { r - w } else { l }, self.top + i as i32 * step)
    }
    /// De qué lado de la columna va una nota de ancho `w` con centro en `cx`.
    fn side_at(&self, cx: i32, w: i32, max_w: i32) -> bool {
        let (l, r) = self.column(max_w.max(w));
        if w >= r - l {
            self.right
        } else {
            cx > (l + r) / 2
        }
    }
    fn insert(&mut self, i: usize, id: u32, right: bool) {
        let i = i.min(self.ids.len());
        self.ids.insert(i, id);
        self.sides.insert(i, right);
    }
    fn remove(&mut self, id: u32) {
        if let Some(i) = self.ids.iter().position(|k| *k == id) {
            self.ids.remove(i);
            self.sides.remove(i);
        }
    }
    /// En qué lugar entra una nota cuya barra está a la altura `cy`.
    fn slot(&self, cy: i32, step: i32, bar: i32) -> usize {
        (0..self.ids.len()).filter(|&i| self.top + i as i32 * step + bar / 2 < cy).count()
    }
    /// Lo que ocupa, con el ancho de la nota más ancha.
    fn bounds(&self, max_w: i32, step: i32, bar: i32) -> RECT {
        let (left, right) = self.column(max_w);
        let n = self.ids.len().max(1) as i32;
        RECT { left, top: self.top, right, bottom: self.top + (n - 1) * step + bar }
    }
}

/// Reconoce las pilas entre las barras (`id`, barra, enrollada) de un
/// monitor: dos o más una debajo de la otra, cada una pegada a un borde de
/// la columna que forman (el izquierdo o el derecho). Con tolerancia, para
/// las armadas a mano: bordes a menos de la separación, y de barra pegada a
/// barra hasta tres separaciones de distancia. Una nota enrollada suelta
/// cuenta como pila de una (se le pueden sumar otras). `right_side(x)`: si
/// una pila con centro en `x` se ancla a la derecha (para crecer hacia el
/// centro de la pantalla).
pub fn find_piles(bars: &[(u32, RECT, bool)], bar: i32, step: i32, right_side: impl Fn(i32) -> bool) -> Vec<Pile> {
    let gap = step - bar;
    let (tol, lo, hi) = (gap + 2, bar - bar / 10, step + 2 * gap);
    struct Chain {
        l: i32,
        r: i32,
        items: Vec<(u32, RECT, bool)>,
    }
    let mut v: Vec<&(u32, RECT, bool)> = bars.iter().collect();
    v.sort_by_key(|b| b.1.top);
    let mut chains: Vec<Chain> = Vec::new();
    for b in v {
        let r = b.1;
        let fit = chains.iter_mut().find(|c| {
            let last = c.items[c.items.len() - 1].1;
            (lo..=hi).contains(&(r.top - last.top)) && ((r.left - c.l).abs() <= tol || (r.right - c.r).abs() <= tol)
        });
        match fit {
            Some(c) => {
                (c.l, c.r) = (c.l.min(r.left), c.r.max(r.right));
                c.items.push(*b);
            }
            None => chains.push(Chain { l: r.left, r: r.right, items: vec![*b] }),
        }
    }
    chains
        .into_iter()
        .filter(|c| c.items.len() >= 2 || c.items[0].2)
        .map(|c| {
            let right = right_side((c.l + c.r) / 2);
            let sides = c
                .items
                .iter()
                .map(|(_, r, _)| {
                    let (at_l, at_r) = ((r.left - c.l).abs() <= tol, (r.right - c.r).abs() <= tol);
                    if at_l && at_r {
                        right
                    } else {
                        at_r
                    }
                })
                .collect();
            Pile { right, edge: if right { c.r } else { c.l }, top: c.items[0].1.top, ids: c.items.iter().map(|b| b.0).collect(), sides }
        })
        .collect()
}

// =================================================================
// Las notas de un monitor
// =================================================================

#[derive(Clone, Copy)]
struct Info {
    id: u32,
    hwnd: HWND,
    x: i32,
    y: i32,
    w: i32,
    /// Alto abierto.
    h: i32,
    rolled: bool,
    bar: i32,
}

impl Info {
    fn bar_rect(&self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.bar }
    }
    /// Lo que ocupa en pantalla: la barra si está enrollada.
    fn visible(&self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + if self.rolled { self.bar } else { self.h } }
    }
}

fn monitor_of(x: i32, y: i32) -> (HMONITOR, RECT) {
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(m, &mut info);
        (m, info.rcWork)
    }
}

/// Las notas del escritorio (no "siempre encima") cuya barra cae en `monitor`.
fn notes_on(monitor: HMONITOR) -> Vec<Info> {
    let list: Vec<(u32, HWND, i32, i32, i32, i32, bool)> = {
        let a = app().lock().unwrap();
        a.notes
            .iter()
            .filter(|(_, nr)| nr.hwnd != 0 && !nr.data.hidden && nr.data.layer != Layer::AlwaysOnTop)
            .map(|(id, nr)| (*id, nr.hwnd as HWND, nr.data.x, nr.data.y, nr.data.w, nr.data.h, nr.data.rolled))
            .collect()
    };
    list.into_iter()
        .filter(|&(_, hwnd, ..)| unsafe { IsWindowVisible(hwnd) } != 0)
        .map(|(id, hwnd, x, y, w, h, rolled)| Info { id, hwnd, x, y, w, h, rolled, bar: crate::note::header_h(hwnd) })
        .filter(|n| monitor_of(n.x + n.w / 2, n.y + n.bar / 2).0 == monitor)
        .collect()
}

/// Lo que ocupan las ventanas de aplicaciones a la vista (no minimizadas,
/// no en otro escritorio virtual, ni la barra de tareas, ni las propias).
fn app_windows() -> Vec<RECT> {
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
    unsafe extern "system" fn each(hwnd: HWND, list: LPARAM) -> BOOL {
        (*(list as *mut Vec<HWND>)).push(hwnd);
        1
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(each), &mut all as *mut Vec<HWND> as LPARAM) };
    let me = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() };
    let shell = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
    all.into_iter()
        .filter_map(|h| unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            let mut pid = 0;
            GetWindowThreadProcessId(h, &mut pid);
            let mut cloaked: u32 = 0;
            DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut _, 4);
            let mut class = [0u16; 64];
            let n = GetClassNameW(h, class.as_mut_ptr(), 64) as usize;
            let class = String::from_utf16_lossy(&class[..n]);
            // Fuera: ocultas, minimizadas, auxiliares, capas que dejan pasar el mouse, las del shell y las propias.
            if IsWindowVisible(h) == 0 || IsIconic(h) != 0 || ex & WS_EX_TOOLWINDOW != 0 || ex & WS_EX_TRANSPARENT != 0 || cloaked != 0 || pid == me || shell.contains(&class.as_str()) {
                return None;
            }
            // Lo que se ve (sin el borde invisible para redimensionar).
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS as u32, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32) != 0 {
                GetWindowRect(h, &mut r);
            }
            (r.right > r.left && r.bottom > r.top).then_some(r)
        })
        .collect()
}

fn dpi_at(x: i32, y: i32) -> i32 {
    crate::flyout::dpi_at(x, y) * crate::note::scale_pct() / 100
}

fn px(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

fn piles_of(infos: &[Info], bar: i32, step: i32, work: RECT) -> Vec<Pile> {
    let bars: Vec<(u32, RECT, bool)> = infos.iter().map(|n| (n.id, n.bar_rect(), n.rolled)).collect();
    let mid = (work.left + work.right) / 2;
    find_piles(&bars, bar, step, |x| x > mid)
}

fn ctrl_down() -> bool {
    FAKE_CTRL.get().unwrap_or_else(|| (unsafe { GetKeyState(VK_CONTROL as i32) }) < 0)
}

fn cursor() -> POINT {
    FAKE.get().unwrap_or_else(|| {
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        pt
    })
}

/// Lo que hace falta para ver las pilas de la nota `hwnd`.
/// (nota, notas del monitor, pilas, alto de barra, paso de la pila)
type Context = (u32, Vec<Info>, Vec<Pile>, i32, i32);

fn context_of(hwnd: HWND) -> Option<Context> {
    let id = crate::note::note_id(hwnd);
    let me = app().lock().unwrap().notes.get(&id).map(|nr| (nr.data.x, nr.data.y, nr.data.w))?;
    let bar = crate::note::header_h(hwnd);
    let (monitor, work) = monitor_of(me.0 + me.2 / 2, me.1 + bar / 2);
    let step = bar + px(dpi_at(me.0, me.1), PILE_GAP);
    let infos = notes_on(monitor);
    let piles = piles_of(&infos, bar, step, work);
    Some((id, infos, piles, bar, step))
}

/// ¿Está la nota en una pila de dos o más? (Con Ctrl, esas no se despliegan.)
pub fn in_pile(hwnd: HWND) -> bool {
    context_of(hwnd).is_some_and(|(id, _, piles, ..)| piles.iter().any(|p| p.ids.len() >= 2 && p.ids.contains(&id)))
}

/// La nota se va (ocultar, borrar): las de abajo en su pila suben.
pub fn close_gap(hwnd: HWND) {
    let Some((id, infos, piles, _, step)) = context_of(hwnd) else { return };
    let Some(mut pile) = piles.into_iter().find(|p| p.ids.len() >= 2 && p.ids.contains(&id)) else { return };
    pile.remove(id);
    let by_id: HashMap<u32, Info> = infos.iter().map(|n| (n.id, *n)).collect();
    settle_pile(&pile, &by_id, step);
    save_all();
}

// =================================================================
// Mover notas y guardar
// =================================================================

fn store(id: u32, x: i32, y: i32, wh: Option<(i32, i32)>) {
    let mut a = app().lock().unwrap();
    if let Some(nr) = a.notes.get_mut(&id) {
        (nr.data.x, nr.data.y) = (x, y);
        if let Some((w, h)) = wh {
            (nr.data.w, nr.data.h) = (w, h);
        }
    }
}

/// Cada nota de la pila a su lugar (con transición) y guardada ahí.
fn settle_pile(pile: &Pile, by_id: &HashMap<u32, Info>, step: i32) {
    let max_w = pile.ids.iter().filter_map(|k| by_id.get(k)).map(|n| n.w).max().unwrap_or(0);
    for (i, id) in pile.ids.iter().enumerate() {
        if let Some(n) = by_id.get(id) {
            let (x, y) = pile.place(i, n.w, max_w, step);
            anim::to_pos(n.hwnd, x, y, anim::MOVE_MS);
            store(*id, x, y, None);
        }
    }
}

/// Lo que ocupa en pantalla una nota acomodada en `r`: si está
/// enrollada, solo la barra (con el ancho de la celda).
fn shown_in(n: &Info, r: RECT) -> RECT {
    RECT { bottom: if n.rolled { r.top + n.bar } else { r.bottom }, ..r }
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

// =================================================================
// El arrastre
// =================================================================

const SHAPES: [(i32, i32); 4] = [(1, 1), (2, 1), (1, 2), (2, 2)];
/// Cada cuánto se mira Ctrl y Esc, y avanzan las capas (Windows redondea a ~16 ms).
const FRAME_MS: u32 = 10;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Choice {
    Shape(i32, i32),
    Pile,
    /// El tacho: soltarla ahí la borra.
    Trash,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    None,
    /// Celdas, y si queda abierta (elegida en el panel).
    Cell(Cells, bool),
    /// Pila, lugar y lado (`true`: pegada al borde derecho de la columna).
    Pile(usize, usize, bool),
    NewPile,
    /// Soltada sobre el tacho del panel.
    Trash,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Move,
    Size,
    PileMove,
}

/// Cómo estaba la nota al empezar (para Esc).
#[derive(Clone, Copy)]
struct Orig {
    rect: RECT,
    w: i32,
    h: i32,
    rolled: bool,
    mode: RollMode,
    /// Bloqueada: el tacho no la toma.
    locked: bool,
}

struct Drag {
    hwnd: HWND,
    id: u32,
    kind: Kind,
    /// La nota es del escritorio: entra en celdas y pilas.
    snaps: bool,
    work: RECT,
    /// La parte libre más grande que dejan las aplicaciones (para el panel y la guía).
    area: RECT,
    grid: Grid,
    /// Celdas tapadas por aplicaciones a la vista: no se dibujan ni se usan.
    blocked: Vec<Cells>,
    dpi: i32,
    step: i32,
    bar: i32,
    by_id: HashMap<u32, Info>,
    /// Las pilas del monitor, sin la nota que se mueve.
    piles: Vec<Pile>,
    /// La pila de la que salió (índice en `piles`).
    from: Option<usize>,
    /// Con Ctrl: la pila que se mueve entera.
    moving: Option<usize>,
    /// Las notas sueltas que se pueden achicar o correr.
    others: Vec<u32>,
    /// Dónde se agarró la nota (respecto de su esquina).
    grab: POINT,
    orig: Orig,
    moved: bool,
    panel: bool,
    choice: Option<Choice>,
    /// La opción bajo el mouse y desde cuándo (para cambiar hay que quedarse un instante).
    hovering: Option<(Choice, std::time::Instant)>,
    mode: Option<Choice>,
    target: Target,
    plan: Vec<(u32, Cells)>,
    /// Pila entera con Ctrl: cómo quedó acomodada la última vez (para no
    /// repetir las transiciones en cada paso).
    last_layout: Option<(i32, i32, Vec<u32>)>,
    fx: Fx,
}

thread_local! {
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    static TIMER: Cell<usize> = const { Cell::new(0) };
    static QUIET: Cell<isize> = const { Cell::new(0) };
}

/// La nota recién soltada no se despliega (ni se enrolla) sola hasta que
/// el mouse salga de ella: si no, una nota Auto que entra en una pila se
/// abría enseguida, con el mouse todavía encima.
pub fn hush(hwnd: HWND, hovered: bool) -> bool {
    if QUIET.get() != hwnd as isize {
        return false;
    }
    if !hovered {
        QUIET.set(0);
    }
    hovered
}

/// ¿Se está arrastrando o redimensionando una nota? (Mientras tanto,
/// ninguna se despliega al pasarle el mouse por encima.)
pub fn busy() -> bool {
    DRAG.with(|d| d.try_borrow().map(|d| d.is_some()).unwrap_or(true))
}

fn new_drag(hwnd: HWND, kind: Kind, grab: POINT) -> Drag {
    let id = crate::note::note_id(hwnd);
    let pt = cursor();
    let (monitor, work) = monitor_of(pt.x, pt.y);
    let dpi = dpi_at(pt.x, pt.y);
    let bar = crate::note::header_h(hwnd);
    let step = bar + px(dpi, PILE_GAP);
    let infos = notes_on(monitor);
    let snaps = infos.iter().any(|n| n.id == id);
    // La cuadrícula, en lo que dejan libre las aplicaciones a la vista (si no
    // queda lugar para una celda, el monitor entero).
    // Las aplicaciones a la vista: la cuadrícula sigue su acomodo y no se usa debajo de ellas.
    let apps: Vec<RECT> = app_windows()
        .into_iter()
        .map(|w| RECT { left: w.left.max(work.left), top: w.top.max(work.top), right: w.right.min(work.right), bottom: w.bottom.min(work.bottom) })
        .filter(|w| w.right > w.left && w.bottom > w.top)
        .collect();
    let g = px(dpi, GAP);
    let inner = |v: i32, lo: i32, hi: i32| v > lo + g && v < hi - g;
    let xs: Vec<i32> = apps.iter().flat_map(|w| [w.left, w.right]).filter(|&x| inner(x, work.left, work.right)).collect();
    let ys: Vec<i32> = apps.iter().flat_map(|w| [w.top, w.bottom]).filter(|&y| inner(y, work.top, work.bottom)).collect();
    let grid = Grid::fitted(work, dpi, &xs, &ys);
    let mut blocked: Vec<Cells> = Vec::new();
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let k = Cells { c0: c, c1: c, r0: r, r1: r };
            let cell = grid.rect(k);
            let covered = apps.iter().any(|a| a.right.min(cell.right) - a.left.max(cell.left) > g && a.bottom.min(cell.bottom) - a.top.max(cell.top) > g);
            if covered {
                blocked.push(k);
            }
        }
    }
    // Todo tapado (una ventana maximizada): se ignoran las aplicaciones.
    if blocked.len() as i32 == grid.cols * grid.rows {
        blocked.clear();
    }
    // Panel y guía: centrados en la parte libre más grande.
    let area = free_area(work, &apps, px(dpi, CELL_W) + 2 * g, px(dpi, CELL_H) + 2 * g).unwrap_or(work);
    let mut piles = if snaps { piles_of(&infos, bar, step, work) } else { Vec::new() };
    let mine = piles.iter().position(|p| p.ids.contains(&id));
    let kind = if kind == Kind::Move && ctrl_down() && mine.is_some() { Kind::PileMove } else { kind };
    let others = if snaps { infos.iter().map(|n| n.id).filter(|k| *k != id && !piles.iter().any(|p| p.ids.contains(k))).collect() } else { Vec::new() };
    let (mut from, mut moving) = (None, None);
    match (kind, mine) {
        (Kind::PileMove, Some(p)) => moving = Some(p),
        (Kind::Move, Some(p)) => {
            piles[p].remove(id);
            from = Some(p);
        }
        _ => {}
    }
    let orig = {
        let a = app().lock().unwrap();
        let d = a.notes.get(&id).map(|nr| (nr.data.w, nr.data.h, nr.data.rolled, nr.data.roll_mode, nr.data.locked)).unwrap_or((0, 0, false, RollMode::Manual, false));
        Orig { rect: window_rect(hwnd), w: d.0, h: d.1, rolled: d.2, mode: d.3, locked: d.4 }
    };
    Drag {
        hwnd,
        id,
        kind,
        snaps,
        work,
        area,
        grid,
        blocked,
        dpi,
        step,
        bar,
        by_id: infos.into_iter().map(|n| (n.id, n)).collect(),
        piles,
        from,
        moving,
        others,
        grab,
        orig,
        moved: false,
        panel: false,
        choice: None,
        hovering: None,
        mode: None,
        target: Target::None,
        plan: Vec::new(),
        last_layout: None,
        fx: Fx::new(work, area, dpi),
    }
}

fn start(d: Drag) {
    DRAG.with(|s| *s.borrow_mut() = Some(d));
    if TIMER.get() == 0 {
        TIMER.set(unsafe { SetTimer(null_mut(), 0, FRAME_MS, Some(tick)) });
    }
}

/// Empieza a arrastrar la nota por la barra; (`x`, `y`): dónde se la
/// agarró, en coordenadas de la nota. Desde acá el mouse es de la nota
/// hasta soltarlo (`drop`), o Esc.
pub fn begin_move(hwnd: HWND, x: i32, y: i32) {
    let d = new_drag(hwnd, Kind::Move, POINT { x, y });
    anim::stop_pos(hwnd);
    crate::note::raise_above_notes(hwnd);
    if d.from.is_some() && !d.orig.rolled {
        // Una nota de pila se lleva como barra.
        crate::note::reshape(hwnd, None, true);
    }
    if d.kind == Kind::PileMove {
        if let Some(p) = d.moving {
            for k in &d.piles[p].ids {
                if let Some(n) = d.by_id.get(k) {
                    crate::note::raise_above_notes(n.hwnd);
                }
            }
        }
    }
    start(d);
    unsafe { SetCapture(hwnd) };
}

/// Empieza a cambiar el tamaño desde un borde (lo lleva Windows, o
/// `note::begin_sizing`; con Ctrl va por celdas, ver `adjust`).
pub fn begin_size(hwnd: HWND) {
    let mut d = new_drag(hwnd, Kind::Size, POINT { x: 0, y: 0 });
    if !d.snaps {
        return;
    }
    d.moved = true;
    start(d);
}

/// Ctrl + Mayús + clic en la barra: la nota pasa a ser una pila (enrollada,
/// en Auto, alineada a la cuadrícula). Si queda sobre otra pila, se suma a esa.
pub fn make_pile(hwnd: HWND) {
    if busy() {
        return;
    }
    let d = new_drag(hwnd, Kind::Move, POINT { x: 0, y: 0 });
    // Ya está en una pila (o no es del escritorio): nada.
    if !d.snaps || d.from.is_some() || d.moving.is_some() {
        return;
    }
    let np = d.new_pile();
    let pile = match d.pile_under(&np, None) {
        Some(Target::Pile(k, idx, side)) => {
            let mut p = d.piles[k].clone();
            p.insert(idx, d.id, side);
            p
        }
        _ => np,
    };
    crate::note::set_roll_mode(hwnd, RollMode::Auto);
    crate::note::reshape(hwnd, None, true);
    settle_pile(&pile, &d.by_id, d.step);
    QUIET.set(hwnd as isize);
    save_all();
}

/// WM_MOUSEMOVE de la nota: si se la está arrastrando, la nota sigue al mouse.
pub fn drag_move(hwnd: HWND) -> bool {
    let mine = DRAG.with(|s| s.try_borrow().map(|s| s.as_ref().is_some_and(|d| d.hwnd == hwnd && d.kind != Kind::Size)).unwrap_or(false));
    if mine {
        step();
    }
    mine
}

/// WM_LBUTTONUP: se suelta donde esté.
pub fn drop(hwnd: HWND) -> bool {
    let mine = DRAG.with(|s| s.try_borrow().map(|s| s.as_ref().is_some_and(|d| d.hwnd == hwnd && d.kind != Kind::Size)).unwrap_or(false));
    if mine {
        end(false);
    }
    mine
}

/// ¿Sigue apretado el botón con el que se arrastra? (El izquierdo, o el
/// derecho si están invertidos.) Con el cursor falso del guion, siempre.
fn button_down() -> bool {
    if FAKE.get().is_some() {
        return true;
    }
    let vk = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
    (unsafe { GetAsyncKeyState(vk as i32) }) < 0
}

/// WM_EXITSIZEMOVE: terminó de cambiar el tamaño.
pub fn finish_size(hwnd: HWND) {
    let mine = DRAG.with(|s| s.try_borrow().map(|s| s.as_ref().is_some_and(|d| d.hwnd == hwnd && d.kind == Kind::Size)).unwrap_or(false));
    if mine {
        end(unsafe { GetAsyncKeyState(VK_ESCAPE as i32) } < 0);
    }
}

unsafe extern "system" fn tick(_: HWND, _: u32, _: usize, _: u32) {
    let esc = GetAsyncKeyState(VK_ESCAPE as i32) < 0;
    let kind = DRAG.with(|s| s.try_borrow().ok().and_then(|s| s.as_ref().map(|d| d.kind)));
    match kind {
        None => {
            KillTimer(null_mut(), TIMER.replace(0));
        }
        Some(k) if esc && k != Kind::Size => end(true),
        // El arrastre no depende de quedarse con el mouse: una nota anclada es
        // hija del escritorio (de Explorer) y puede perder la captura apenas la
        // toma. Se mira el botón; al soltarlo, se suelta la nota.
        Some(k) if k != Kind::Size && !button_down() => end(false),
        // Ctrl se puede apretar o soltar sin mover el mouse.
        Some(Kind::Size) => {
            DRAG.with(|s| {
                if let std::result::Result::Ok(mut s) = s.try_borrow_mut() {
                    if let Some(d) = s.as_mut() {
                        d.fx.frame();
                    }
                }
            });
        }
        Some(_) => step(),
    }
}

/// Un paso del arrastre: la nota (o la pila) sigue al mouse, se elige
/// el destino y las capas avanzan.
fn step() {
    DRAG.with(|s| {
        let Some(mut s) = s.try_borrow_mut().ok() else { return };
        let Some(d) = s.as_mut() else { return };
        match d.kind {
            Kind::Move => d.update_move(),
            Kind::PileMove => d.update_pile_move(),
            Kind::Size => {}
        }
        d.fx.frame();
    });
}

impl Drag {
    fn max_w(&self, p: &Pile) -> i32 {
        p.ids.iter().filter_map(|k| self.by_id.get(k)).map(|n| n.w).max().unwrap_or(0)
    }

    /// Las celdas de cada pila (salvo la que se mueve entera).
    /// ¿La pila queda entera detrás de aplicaciones? (Entonces no se ve: ni zona ni destino.)
    fn hidden(&self, p: &Pile) -> bool {
        let k = self.grid.cells_of(&p.bounds(self.max_w(p), self.step, self.bar));
        !self.blocked.is_empty() && (k.r0..=k.r1).all(|r| (k.c0..=k.c1).all(|c| self.blocked.iter().any(|b| b.has(c, r))))
    }

    fn zones(&self) -> Vec<(usize, Cells)> {
        self.piles
            .iter()
            .enumerate()
            .filter(|(i, p)| !p.ids.is_empty() && Some(*i) != self.moving && !self.hidden(p))
            .map(|(i, p)| (i, self.grid.cells_of(&p.bounds(self.max_w(p), self.step, self.bar))))
            .collect()
    }

    fn other_rects(&self) -> Vec<(u32, RECT)> {
        self.others.iter().filter_map(|k| self.by_id.get(k)).map(|n| (n.id, n.visible())).collect()
    }

    /// Cambia el plan: las notas que entran o salen de él se deslizan.
    fn replan(&mut self, plan: Vec<(u32, Cells)>) {
        for id in &self.others {
            let (old, new) = (self.plan.iter().find(|p| p.0 == *id), plan.iter().find(|p| p.0 == *id));
            if old.map(|p| p.1) == new.map(|p| p.1) {
                continue;
            }
            let Some(n) = self.by_id.get(id) else { continue };
            let r = match new {
                Some((_, k)) => shown_in(n, self.grid.rect(*k)),
                None => n.visible(),
            };
            anim::to_rect(n.hwnd, r, anim::MOVE_MS);
        }
        self.plan = plan;
    }

    /// Las opciones del panel, en pantalla (las cuatro formas y la pila).
    fn panel_options(&self) -> Vec<RECT> {
        let (opt, gap, pad) = (px(self.dpi, 46), px(self.dpi, 7), px(self.dpi, 7));
        let total = pad * 2 + opt * 6 + gap * 7 + 2;
        let x0 = (self.area.left + self.area.right) / 2 - total / 2 + pad;
        let y0 = self.work.top + px(self.dpi, 8) + pad;
        (0..6)
            .map(|i| {
                // Separadores antes de la pila y antes del tacho.
                let x = x0 + i * (opt + gap) + (i >= 4) as i32 * (gap + 1) + (i >= 5) as i32 * (gap + 1);
                RECT { left: x, top: y0, right: x + opt, bottom: y0 + opt }
            })
            .collect()
    }

    /// La opción elegida en el panel cambia la nota mientras se la lleva.
    fn apply_choice(&mut self, c: Choice) {
        match c {
            // El tacho no cambia la nota: se decide al soltarla.
            Choice::Trash => return,
            Choice::Shape(a, b) => {
                let r = self.grid.rect(Cells { c0: 0, c1: a.min(self.grid.cols) - 1, r0: 0, r1: b.min(self.grid.rows) - 1 });
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                crate::note::reshape(self.hwnd, Some((w, h)), false);
                if let Some(n) = self.by_id.get_mut(&self.id) {
                    (n.w, n.h, n.rolled) = (w, h, false);
                }
                self.grab.x = self.grab.x.min(w / 2);
            }
            Choice::Pile => {
                crate::note::reshape(self.hwnd, None, true);
                if let Some(n) = self.by_id.get_mut(&self.id) {
                    n.rolled = true;
                }
            }
        }
        self.grab.y = self.grab.y.min(self.bar - px(self.dpi, 4));
    }

    fn update_move(&mut self) {
        let pt = cursor();
        let ctrl = ctrl_down();
        let w = self.by_id.get(&self.id).map(|n| n.w).unwrap_or(self.orig.rect.right - self.orig.rect.left);
        let (_, here) = monitor_of(pt.x, pt.y);
        let x = pt.x - self.grab.x;
        let y = (pt.y - self.grab.y).clamp(here.top, (here.bottom - self.bar).max(here.top));
        if !self.moved {
            if (pt.x - self.orig.rect.left - self.grab.x).abs() + (pt.y - self.orig.rect.top - self.grab.y).abs() == 0 {
                return;
            }
            // Salió de su pila: las de abajo suben.
            self.moved = true;
            if let Some(p) = self.from {
                let pile = self.piles[p].clone();
                let max_w = self.max_w(&pile);
                for (i, id) in pile.ids.iter().enumerate() {
                    if let Some(n) = self.by_id.get(id) {
                        let (px_, py_) = pile.place(i, n.w, max_w, self.step);
                        anim::to_pos(n.hwnd, px_, py_, anim::MOVE_MS);
                    }
                }
            }
        }
        crate::desktop::move_to_screen(self.hwnd, x, y);
        if !self.snaps {
            return;
        }
        // Panel: aparece al tocar el borde de arriba; se va al bajar.
        let opts = self.panel_options();
        if pt.y <= self.work.top + px(self.dpi, 4) && (self.area.left..self.area.right).contains(&pt.x) {
            self.panel = true;
        }
        if self.panel {
            let under = opts.iter().position(|r| contains(r, pt)).map(|i| match i {
                4 => Choice::Pile,
                5 => Choice::Trash,
                _ => Choice::Shape(SHAPES[i].0, SHAPES[i].1),
            });
            if under != self.hovering.map(|h| h.0) {
                self.hovering = under.map(|c| (c, std::time::Instant::now()));
            }
            if let Some((c, since)) = self.hovering {
                // La primera se toma enseguida; para cambiar, ~120 ms encima.
                if self.choice != Some(c) && (self.choice.is_none() || since.elapsed().as_millis() >= 120) {
                    self.choice = Some(c);
                    self.apply_choice(c);
                }
            }
            if pt.y > opts[0].bottom + px(self.dpi, 30) {
                self.panel = false;
                // El tacho es solo para soltarla encima: al bajar, nada.
                if self.choice == Some(Choice::Trash) {
                    self.choice = None;
                }
                self.mode = self.choice;
            }
        }
        let me = self.by_id.get(&self.id).copied();
        let grid_on = !self.panel && (matches!(self.mode, Some(Choice::Shape(..))) || ctrl);
        let zones = self.zones();
        let mut plan = Vec::new();
        self.target = if self.panel && self.choice == Some(Choice::Trash) && !self.orig.locked && contains(&opts[5], pt) {
            Target::Trash
        } else if self.panel || !contains(&self.work, pt) {
            Target::None
        } else if grid_on {
            if let Some((p, _)) = zones.iter().find(|(_, z)| contains(&self.grid.rect(*z), pt)) {
                Target::Pile(*p, self.piles[*p].slot(pt.y, self.step, self.bar), self.side_in(*p, x + w / 2))
            } else {
                let (a, b) = match self.mode {
                    Some(Choice::Shape(a, b)) => (a, b),
                    _ => me.map(|n| self.grid.shape_for(n.w, n.h)).unwrap_or((1, 1)),
                };
                let zc: Vec<Cells> = zones.iter().map(|z| z.1).chain(self.blocked.iter().copied()).collect();
                let (t, p) = plan_around(&self.grid, self.grid.snap_shape(x, y, a, b), &self.other_rects(), &zc);
                plan = p;
                Target::Cell(t, matches!(self.mode, Some(Choice::Shape(..))))
            }
        } else {
            let cy = y + self.bar / 2;
            let margin = px(self.dpi, 30);
            let hit = self.piles.iter().enumerate().find(|(_, p)| {
                if p.ids.is_empty() || self.hidden(p) {
                    return false;
                }
                let b = p.bounds(self.max_w(p).max(w), self.step, self.bar);
                pt.x >= b.left - margin && pt.x <= b.right + margin && cy >= b.top - self.bar && cy <= b.bottom + px(self.dpi, 20)
            });
            match hit {
                Some((i, p)) => Target::Pile(i, p.slot(cy, self.step, self.bar), self.side_in(i, x + w / 2)),
                // Una pila nueva no se apila encima de otra: se suma a esa.
                None if self.mode == Some(Choice::Pile) => self.pile_under(&self.new_pile(), None).unwrap_or(Target::NewPile),
                None => Target::None,
            }
        };
        self.replan(plan);
        self.show(grid_on, &zones, Some(opts));
    }

    /// La pila (de las que no se mueven) sobre la que caería `probe`, y en
    /// qué lugar entraría: dos pilas no se apilan una encima de la otra.
    fn pile_under(&self, probe: &Pile, skip: Option<usize>) -> Option<Target> {
        let pb = probe.bounds(self.max_w(probe), self.step, self.bar);
        let (k, o) = self.piles.iter().enumerate().find(|(k, o)| Some(*k) != skip && !o.ids.is_empty() && !self.hidden(o) && overlaps(&pb, &o.bounds(self.max_w(o), self.step, self.bar)))?;
        Some(Target::Pile(k, o.slot(probe.top + self.bar / 2, self.step, self.bar), self.side_in(k, (pb.left + pb.right) / 2)))
    }

    /// De qué lado de la pila `k` iría la nota que se lleva, con centro en `cx`.
    fn side_in(&self, k: usize, cx: i32) -> bool {
        let w = self.by_id.get(&self.id).map(|n| n.w).unwrap_or(0);
        self.piles[k].side_at(cx, w, self.max_w(&self.piles[k]))
    }

    /// Ctrl + arrastrar una nota de pila: la pila entera salta de celda en
    /// celda. Si cae sobre otra pila, se juntan: las barras se acomodan como
    /// van a quedar, y si se la saca de ahí, vuelven.
    fn update_pile_move(&mut self) {
        let Some(p) = self.moving else { return };
        let pt = cursor();
        self.moved = true;
        let i = self.piles[p].ids.iter().position(|k| *k == self.id).unwrap_or(0) as i32;
        let w = self.by_id.get(&self.id).map(|n| n.w).unwrap_or(0);
        let (x, y) = (pt.x - self.grab.x, pt.y - self.grab.y);
        let mv = &self.piles[p];
        let edge = if mv.right { self.grid.snap_right_edge(x + w) } else { self.grid.snap_left_edge(x) };
        let probe = Pile { edge, top: self.grid.snap_top(y - i * self.step), ..mv.clone() };
        self.target = self.pile_under(&probe, Some(p)).unwrap_or(Target::None);
        let layout = match self.target {
            Target::Pile(k, idx, _) => self.merged(k, idx),
            _ => probe.clone(),
        };
        let key = (layout.edge, layout.top, layout.ids.clone());
        if self.last_layout.as_ref() != Some(&key) {
            self.last_layout = Some(key);
            (self.piles[p].edge, self.piles[p].top) = (probe.edge, probe.top);
            // Cada pila a su lugar: la que se arma (o la que se mueve) y las demás como estaban.
            let others: Vec<Pile> = self.piles.iter().enumerate().filter(|(k, _)| *k != p && Some(*k) != self.target_pile()).map(|(_, o)| o.clone()).collect();
            for pile in others.iter().chain(std::iter::once(&layout)) {
                let max_w = self.max_w(pile);
                for (j, id) in pile.ids.iter().enumerate() {
                    if let Some(n) = self.by_id.get(id) {
                        let (nx, ny) = pile.place(j, n.w, max_w, self.step);
                        anim::to_pos(n.hwnd, nx, ny, 130);
                    }
                }
            }
            // Donde cae la pila, las notas sueltas se corren (como en las celdas).
            let plan = if matches!(self.target, Target::Pile(..)) {
                Vec::new()
            } else {
                let t = self.grid.cells_of(&layout.bounds(self.max_w(&layout), self.step, self.bar));
                plan_around(&self.grid, t, &self.other_rects(), &[]).1
            };
            self.replan(plan);
        }
        let zones = self.zones();
        self.show(true, &zones, None);
    }

    fn target_pile(&self) -> Option<usize> {
        match self.target {
            Target::Pile(k, _, _) => Some(k),
            _ => None,
        }
    }

    /// La pila `k` con la que se mueve entera metida en el lugar `idx`.
    fn merged(&self, k: usize, idx: usize) -> Pile {
        let mut out = self.piles[k].clone();
        if let Some(p) = self.moving {
            let mv = &self.piles[p];
            for (j, id) in mv.ids.iter().enumerate() {
                out.insert(idx + j, *id, mv.sides.get(j).copied().unwrap_or(out.right));
            }
        }
        out
    }

    /// Ctrl al redimensionar: el rectángulo va por celdas.
    fn update_size(&mut self, r: &mut RECT) {
        let zones = self.zones();
        // Una nota de pila (enrollada): solo cambia de largo, de a una celda con
        // Ctrl y hasta dos celdas, y su lado alineado con la pila queda donde estaba.
        if self.by_id.get(&self.id).is_some_and(|n| n.rolled) {
            let raw = (r.right - r.left).min(self.grid.width_for(i32::MAX, PILE_MAX_CELLS));
            let w = if ctrl_down() { self.grid.width_for(raw, PILE_MAX_CELLS) } else { raw };
            let left = match self.piles.iter().find(|p| p.ids.contains(&self.id)) {
                Some(p) => {
                    let i = p.ids.iter().position(|k| *k == self.id).unwrap_or(0);
                    let others = p.ids.iter().filter(|k| **k != self.id).filter_map(|k| self.by_id.get(k)).map(|n| n.w).max().unwrap_or(0);
                    p.place(i, w, others.max(w), self.step).0
                }
                None => r.left,
            };
            *r = RECT { left, top: r.top, right: left + w, bottom: r.top + self.bar };
            self.show(ctrl_down(), &zones, None);
            return;
        }
        if !ctrl_down() {
            self.target = Target::None;
            self.replan(Vec::new());
            self.show(false, &zones, None);
            return;
        }
        let zc: Vec<Cells> = zones.iter().map(|z| z.1).chain(self.blocked.iter().copied()).collect();
        let (t, plan) = plan_around(&self.grid, self.grid.snap_rect(r), &self.other_rects(), &zc);
        *r = self.grid.rect(t);
        self.target = Target::None;
        self.replan(plan);
        self.show(true, &zones, None);
    }

    /// Dónde empezaría una pila nueva con la nota que se lleva: contra el
    /// borde de la celda más cercana, del lado de la pantalla en que está.
    fn new_pile(&self) -> Pile {
        let wr = window_rect(self.hwnd);
        let right = (wr.left + wr.right) / 2 > (self.area.left + self.area.right) / 2;
        let edge = if right { self.grid.snap_right_edge(wr.right) } else { self.grid.snap_left_edge(wr.left) };
        Pile::one(self.id, right, edge, self.grid.snap_top(wr.top))
    }

    /// Lo que tapa la cuadrícula: la nota que se lleva, las sueltas (donde
    /// van a quedar) y las barras de las pilas.
    fn holes(&self) -> Vec<RECT> {
        let mut v = vec![window_rect(self.hwnd)];
        for id in &self.others {
            if let Some(n) = self.by_id.get(id) {
                v.push(match self.plan.iter().find(|p| p.0 == *id) {
                    Some((_, k)) => shown_in(n, self.grid.rect(*k)),
                    None => n.visible(),
                });
            }
        }
        for pile in &self.piles {
            let max_w = self.max_w(pile);
            for (i, id) in pile.ids.iter().enumerate() {
                if let Some(n) = self.by_id.get(id) {
                    let (x, y) = pile.place(i, n.w, max_w, self.step);
                    v.push(RECT { left: x, top: y, right: x + n.w, bottom: y + self.bar });
                }
            }
        }
        v
    }

    /// Lo que las capas tienen que mostrar ahora.
    fn show(&mut self, grid_on: bool, zones: &[(usize, Cells)], opts: Option<Vec<RECT>>) {
        let mut grid = None;
        if grid_on {
            let mut g = GridScene::default();
            for r in 0..self.grid.rows {
                for c in 0..self.grid.cols {
                    if !zones.iter().any(|(_, z)| z.has(c, r)) && !self.blocked.iter().any(|b| b.has(c, r)) {
                        g.cells.push(q(self.grid.rect(Cells { c0: c, c1: c, r0: r, r1: r })));
                    }
                }
            }
            g.zones = zones.iter().map(|(_, z)| q(self.grid.rect(*z))).collect();
            g.holes = self.holes().into_iter().map(q).collect();
            grid = Some(g);
        }
        let mut preview = None;
        let mut line = None;
        match self.target {
            Target::Cell(t, _) => preview = Some(self.grid.rect(t)),
            Target::NewPile => {
                let pile = self.new_pile();
                let w = self.by_id.get(&self.id).map(|n| n.w).unwrap_or(0);
                let (x, y) = pile.place(0, w, w, self.step);
                preview = Some(RECT { left: x, top: y, right: x + w, bottom: y + self.bar });
            }
            Target::Pile(p, idx, side) => {
                // La línea mide lo que la nota y va del lado donde va a quedar.
                let pile = &self.piles[p];
                let me_w = self.by_id.get(&self.id).map(|n| n.w).unwrap_or(0);
                let max_w = self.max_w(pile).max(me_w);
                let (l, r) = pile.column(max_w);
                let (left, right) = match self.kind {
                    Kind::PileMove => (l, r),
                    _ if side => (r - me_w, r),
                    _ => (l, l + me_w),
                };
                let y = pile.top + idx as i32 * self.step - px(self.dpi, PILE_GAP) / 2;
                let t = px(self.dpi, 3).max(2);
                line = Some(RECT { left, top: y - t / 2, right, bottom: y - t / 2 + t });
            }
            _ => {}
        }
        let hot = match self.choice {
            Some(Choice::Shape(a, b)) => SHAPES.iter().position(|s| *s == (a, b)),
            Some(Choice::Pile) => Some(4),
            Some(Choice::Trash) if !self.orig.locked => Some(5),
            Some(Choice::Trash) => None,
            None => None,
        };
        self.fx.want = Want { grid, preview, line, panel: self.panel.then(|| opts.unwrap_or_default()), hot, hint: self.kind == Kind::Move };
    }
}

fn contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

/// WM_WINDOWPOSCHANGING de la nota que se redimensiona: con Ctrl, por celdas.
pub fn adjust(hwnd: HWND, wp: &mut WINDOWPOS) {
    if wp.flags & SWP_NOSIZE != 0 {
        return;
    }
    DRAG.with(|s| {
        let Some(mut s) = s.try_borrow_mut().ok() else { return };
        let Some(d) = s.as_mut().filter(|d| d.hwnd == hwnd && d.kind == Kind::Size) else { return };
        // WINDOWPOS va en coordenadas del padre si la nota está anclada.
        let off = parent_origin(hwnd);
        let mut r = RECT { left: wp.x + off.x, top: wp.y + off.y, right: wp.x + off.x + wp.cx, bottom: wp.y + off.y + wp.cy };
        d.update_size(&mut r);
        (wp.x, wp.y, wp.cx, wp.cy) = (r.left - off.x, r.top - off.y, r.right - r.left, r.bottom - r.top);
        wp.flags &= !SWP_NOMOVE;
        d.fx.frame();
    });
}

fn parent_origin(hwnd: HWND) -> POINT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD != 0 {
            ClientToScreen(GetAncestor(hwnd, GA_PARENT), &mut p);
        }
    }
    p
}

/// Termina: aplica lo elegido, o (Esc) todo vuelve a como estaba.
fn end(cancel: bool) {
    let Some(mut d) = DRAG.with(|s| s.borrow_mut().take()) else { return };
    d.fx.close();
    let hwnd = d.hwnd;
    if d.kind != Kind::Size {
        QUIET.set(hwnd as isize);
    }
    if d.kind != Kind::Size && unsafe { GetCapture() } == hwnd {
        unsafe { ReleaseCapture() };
    }
    // Al tacho: todo vuelve a su lugar y después se borra (o se pregunta).
    let trash = !cancel && d.target == Target::Trash;
    if cancel || !d.moved || trash {
        d.replan(Vec::new());
        for p in 0..d.piles.len() {
            for id in &d.piles[p].ids {
                if let Some(n) = d.by_id.get(id) {
                    anim::to_pos(n.hwnd, n.x, n.y, anim::MOVE_MS);
                }
            }
        }
        if d.kind != Kind::Size {
            let o = d.orig;
            crate::note::reshape(hwnd, Some((o.w, o.h)), o.rolled);
            crate::note::set_roll_mode(hwnd, o.mode);
            anim::to_pos(hwnd, o.rect.left, o.rect.top, anim::MOVE_MS);
        }
        if trash {
            crate::note::trash(hwnd);
        }
        return;
    }
    // Las notas achicadas o corridas quedan así.
    for (id, k) in &d.plan {
        if let Some(n) = d.by_id.get(id) {
            let r = d.grid.rect(*k);
            store(*id, r.left, r.top, Some((r.right - r.left, if n.rolled { n.h } else { r.bottom - r.top })));
        }
    }
    match d.kind {
        Kind::Size => {}
        Kind::PileMove => match (d.target, d.moving) {
            (Target::Pile(k, idx, _), _) => settle_pile(&d.merged(k, idx), &d.by_id, d.step),
            (_, Some(p)) => settle_pile(&d.piles[p], &d.by_id, d.step),
            _ => {}
        },
        Kind::Move => {
            if let Some(p) = d.from {
                settle_pile(&d.piles[p], &d.by_id, d.step);
            }
            let from_pile = d.from.is_some();
            if from_pile {
                // Sacada de una pila: se abre y pasa a Manual (salvo que vuelva a una).
                crate::note::set_roll_mode(hwnd, RollMode::Manual);
            }
            match d.target {
                Target::Cell(t, open) => {
                    let r = d.grid.rect(t);
                    let (w, h) = (r.right - r.left, r.bottom - r.top);
                    let rolled = !(open || from_pile) && app().lock().unwrap().notes.get(&d.id).is_some_and(|nr| nr.data.rolled);
                    crate::note::reshape(hwnd, Some((w, h)), rolled);
                    store(d.id, r.left, r.top, Some((w, h)));
                    anim::to_pos(hwnd, r.left, r.top, anim::MOVE_MS);
                }
                Target::Pile(p, i, side) => {
                    let mut pile = d.piles[p].clone();
                    pile.insert(i, d.id, side);
                    // En una pila, la nota se despliega sola al pasarle el mouse.
                    crate::note::set_roll_mode(hwnd, RollMode::Auto);
                    crate::note::reshape(hwnd, None, true);
                    settle_pile(&pile, &d.by_id, d.step);
                }
                Target::NewPile => {
                    let pile = d.new_pile();
                    crate::note::set_roll_mode(hwnd, RollMode::Auto);
                    crate::note::reshape(hwnd, None, true);
                    settle_pile(&pile, &d.by_id, d.step);
                }
                // (El tacho se atendió arriba.)
                Target::None | Target::Trash => {
                    if from_pile {
                        crate::note::reshape(hwnd, None, false);
                    }
                    // La posición, no el tamaño: puede estar a mitad de abrirse.
                    let wr = window_rect(hwnd);
                    store(d.id, wr.left, wr.top, None);
                }
            }
        }
    }
    save_all();
}

// =================================================================
// Guion de prueba
// =================================================================
//
// Solo en una instancia de prueba (`SIMPCKY_INSTANCIA`) con `SIMPCKY_GUION`
// apuntando a un archivo de pasos: un cursor falso hace los arrastres, para
// probar todo esto sin mouse real (lo de afuera saca capturas cuando el
// registro de `SIMPCKY_PAINT_LOG` dice FOTO). Un paso por línea (en los
// títulos, `_` es un espacio):
//   tomar <título> <dx> <dy>          agarra la barra de esa nota en (dx, dy)
//   ir <x> <y> <ms>                   lleva el cursor ahí en ese tiempo
//   ctrl <0|1>                        suelta o aprieta Ctrl
//   esperar <ms>
//   foto <nombre>                     avisa en el registro y espera 400 ms
//   soltar | esc
//   hacer_pila <título>               como Ctrl + Mayús + clic en su barra
//   opcion <i>                        el cursor a una opción del panel (0-3 formas, 4 pila, 5 tacho)
//   estirar <título> <dw> <dh> <ms>   cambia el tamaño desde abajo a la derecha

thread_local! {
    static FAKE: Cell<Option<POINT>> = const { Cell::new(None) };
    static FAKE_CTRL: Cell<Option<bool>> = const { Cell::new(None) };
    static GUION: RefCell<Option<Guion>> = const { RefCell::new(None) };
}

struct Guion {
    steps: Vec<Vec<String>>,
    i: usize,
    since: std::time::Instant,
    from: POINT,
    /// `estirar`: la nota y su rectángulo al empezar.
    sizing: Option<(HWND, RECT)>,
    said: bool,
}

pub fn start_guion() {
    if !crate::app::is_test_instance() {
        return;
    }
    let Some(text) = std::env::var_os("SIMPCKY_GUION").and_then(|p| std::fs::read_to_string(p).ok()) else { return };
    let steps = text.lines().map(|l| l.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).filter(|v| !v.is_empty()).collect();
    let from = cursor();
    let g = Guion { steps, i: 0, since: std::time::Instant::now(), from, sizing: None, said: false };
    GUION.with(|s| *s.borrow_mut() = Some(g));
    unsafe { SetTimer(null_mut(), 0, 16, Some(guion_tick)) };
}

fn note_by_title(title: &str) -> Option<HWND> {
    let title = title.replace('_', " ");
    let a = app().lock().unwrap();
    a.notes.values().find(|nr| nr.data.title == title && nr.hwnd != 0).map(|nr| nr.hwnd as HWND)
}

fn guion_mark(text: &str) {
    crate::trace::rect(text, null_mut(), Some(&RECT { left: 0, top: 0, right: 0, bottom: 0 }));
}

fn guion_next() {
    let from = cursor();
    GUION.with(|g| {
        if let Some(g) = g.borrow_mut().as_mut() {
            g.i += 1;
            g.since = std::time::Instant::now();
            g.from = from;
            g.said = false;
        }
    });
}

unsafe extern "system" fn guion_tick(_: HWND, _: u32, id: usize, _: u32) {
    loop {
        let cur = GUION.with(|g| {
            let g = g.borrow();
            let g = g.as_ref()?;
            let step = g.steps.get(g.i)?;
            Some((step[0].clone(), step[1..].to_vec(), g.since.elapsed().as_millis() as i32, g.from, g.sizing, g.said))
        });
        let Some((cmd, args, elapsed, from, sizing, said)) = cur else {
            guion_mark("GUION FIN");
            KillTimer(null_mut(), id);
            return;
        };
        let num = |i: usize| args.get(i).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
        match cmd.as_str() {
            "tomar" => {
                if let Some(h) = args.first().and_then(|t| note_by_title(t)) {
                    let r = window_rect(h);
                    FAKE.set(Some(POINT { x: r.left + num(1), y: r.top + num(2) }));
                    begin_move(h, num(1), num(2));
                }
            }
            "ir" => {
                let t = (elapsed as f32 / num(2).max(1) as f32).min(1.0);
                let p = POINT { x: from.x + ((num(0) - from.x) as f32 * t) as i32, y: from.y + ((num(1) - from.y) as f32 * t) as i32 };
                FAKE.set(Some(p));
                step();
                if t < 1.0 {
                    return;
                }
            }
            "ctrl" => FAKE_CTRL.set(Some(num(0) == 1)),
            "esperar" => {
                if elapsed < num(0) {
                    step();
                    return;
                }
            }
            "foto" => {
                if !said {
                    guion_mark(&format!("FOTO {}", args.first().map(String::as_str).unwrap_or("")));
                    GUION.with(|g| {
                        if let Some(g) = g.borrow_mut().as_mut() {
                            g.said = true;
                        }
                    });
                }
                if elapsed < 400 {
                    step();
                    return;
                }
            }
            "soltar" | "esc" => end(cmd == "esc"),
            "opcion" => {
                // El cursor al centro de una opción del panel (0-3 formas, 4 pila, 5 tacho).
                let at = DRAG.with(|s| s.try_borrow().ok().and_then(|s| s.as_ref().and_then(|d| d.panel_options().get(num(0) as usize).copied())));
                if let Some(r) = at {
                    FAKE.set(Some(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }));
                    step();
                }
            }
            "hacer_pila" => {
                if let Some(h) = args.first().and_then(|t| note_by_title(t)) {
                    let r = window_rect(h);
                    FAKE.set(Some(POINT { x: r.left + 20, y: r.top + 10 }));
                    make_pile(h);
                }
            }
            "estirar" => {
                let (h, r0) = match sizing {
                    Some(s) => s,
                    None => {
                        let Some(h) = args.first().and_then(|t| note_by_title(t)) else {
                            guion_next();
                            continue;
                        };
                        let r0 = window_rect(h);
                        FAKE.set(Some(POINT { x: r0.right - 2, y: r0.bottom - 2 }));
                        begin_size(h);
                        GUION.with(|g| {
                            if let Some(g) = g.borrow_mut().as_mut() {
                                g.sizing = Some((h, r0));
                            }
                        });
                        (h, r0)
                    }
                };
                let t = (elapsed as f32 / num(3).max(1) as f32).min(1.0);
                let (w, hh) = (r0.right - r0.left + (num(1) as f32 * t) as i32, r0.bottom - r0.top + (num(2) as f32 * t) as i32);
                let off = parent_origin(h);
                SetWindowPos(h, null_mut(), r0.left - off.x, r0.top - off.y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE);
                if t < 1.0 {
                    return;
                }
                SendMessageW(h, WM_EXITSIZEMOVE, 1, 0);
                GUION.with(|g| {
                    if let Some(g) = g.borrow_mut().as_mut() {
                        g.sizing = None;
                    }
                });
            }
            _ => {}
        }
        guion_next();
    }
}

// =================================================================
// Las capas: cuadrícula, vista previa, línea de la pila, panel y guía
// =================================================================

#[derive(Default, PartialEq, Clone)]
struct GridScene {
    cells: Vec<[i32; 4]>,
    zones: Vec<[i32; 4]>,
    /// Donde hay notas: ahí no se dibuja (la cuadrícula queda "debajo").
    holes: Vec<[i32; 4]>,
}

/// Lo que las capas tienen que mostrar.
#[derive(Default)]
struct Want {
    grid: Option<GridScene>,
    preview: Option<RECT>,
    line: Option<RECT>,
    /// Las opciones del panel, si está a la vista.
    panel: Option<Vec<RECT>>,
    hot: Option<usize>,
    /// La guía arriba al centro: "llevala hasta acá".
    hint: bool,
}

fn q(r: RECT) -> [i32; 4] {
    [r.left, r.top, r.right, r.bottom]
}

fn rq(v: &[i32; 4]) -> RECT {
    RECT { left: v[0], top: v[1], right: v[2], bottom: v[3] }
}

/// Acerca `v` a `to` un poco por cuadro (así todo llega suave).
fn ease(v: &mut f32, to: f32, k: f32) {
    *v += (to - *v) * k;
    if (to - *v).abs() < 0.01 {
        *v = to;
    }
}

struct Colors {
    accent: u32,
    zone_fill: u32,
    grid_line: u32,
    ink: u32,
    panel: u32,
    panel_line: u32,
    option: u32,
    danger: u32,
}

/// Los colores del boceto, claros u oscuros como el resto de la app.
fn colors() -> Colors {
    if crate::theme::is_dark() {
        Colors {
            accent: 0xff60_a5fa,
            zone_fill: 0x4760_a5fa,
            grid_line: 0x66e2_e8f0,
            ink: 0xffe5_e7eb,
            panel: 0xf528_2a2e,
            panel_line: 0xff30_343a,
            option: 0xff4b_5058,
            danger: 0xfff8_7171,
        }
    } else {
        Colors {
            accent: 0xff3b_82f6,
            zone_fill: 0x423b_82f6,
            grid_line: 0x661e_293b,
            ink: 0xff1f_2937,
            panel: 0xf2fa_fafa,
            panel_line: 0xffdf_e3dc,
            option: 0xffd4_d7dc,
            danger: 0xffdc_2626,
        }
    }
}

fn with_alpha(c: u32, a: f32) -> u32 {
    ((((c >> 24) as f32 * a) as u32) << 24) | (c & 0x00ff_ffff)
}

struct Fx {
    work: RECT,
    area: RECT,
    dpi: i32,
    want: Want,
    grid: Lay,
    preview: Lay,
    line: Lay,
    hint: Lay,
    panel: Lay,
    grid_scene: Option<GridScene>,
    grid_a: f32,
    prev: Option<[f32; 4]>,
    prev_a: f32,
    line_at: Option<RECT>,
    line_a: f32,
    panel_opts: Vec<RECT>,
    panel_hot: Option<Option<usize>>,
    panel_a: f32,
    hint_a: f32,
}

impl Fx {
    fn new(work: RECT, area: RECT, dpi: i32) -> Fx {
        // Creadas en este orden quedan apiladas así: la cuadrícula abajo, el panel arriba.
        let mut fx = Fx {
            work,
            area,
            dpi,
            want: Want::default(),
            grid: Lay::default(),
            preview: Lay::default(),
            line: Lay::default(),
            hint: Lay::default(),
            panel: Lay::default(),
            grid_scene: None,
            grid_a: 0.0,
            prev: None,
            prev_a: 0.0,
            line_at: None,
            line_a: 0.0,
            panel_opts: Vec::new(),
            panel_hot: None,
            panel_a: 0.0,
            hint_a: 0.0,
        };
        for l in [&mut fx.grid, &mut fx.preview, &mut fx.line, &mut fx.hint, &mut fx.panel] {
            l.create();
        }
        fx
    }

    fn close(&mut self) {
        for l in [&mut self.grid, &mut self.preview, &mut self.line, &mut self.hint, &mut self.panel] {
            l.close();
        }
    }

    /// Un cuadro: cada capa se acerca a lo que tiene que mostrar.
    fn frame(&mut self) {
        let c = colors();
        let dpi = self.dpi;
        let f = |v: i32| (v * dpi / 96) as f32;

        // Cuadrícula y zonas de las pilas: aparecen con un fundido.
        if let Some(g) = &self.want.grid {
            if self.grid_scene.as_ref() != Some(g) {
                let (w, h) = (self.work.right - self.work.left, self.work.bottom - self.work.top);
                let o = (self.work.left, self.work.top);
                let g2 = g.clone();
                self.grid.paint(w, h, |gr| unsafe {
                    for r in g2.holes.iter().map(rq) {
                        let path = round_rect_path(&r, o, f(8));
                        GdipSetClipPath(gr, path, CombineModeExclude);
                        GdipDeletePath(path);
                    }
                    for r in g2.cells.iter().map(rq) {
                        shape(gr, &r, o, f(6), None, Some((c.grid_line, f(1).max(1.5), true)));
                    }
                    for r in g2.zones.iter().map(rq) {
                        shape(gr, &r, o, f(8), Some(c.zone_fill), Some((with_alpha(c.accent, 0.55), f(1).max(1.5), true)));
                        let text = RECT { left: r.left + f(8) as i32, top: r.top, right: r.right - f(8) as i32, bottom: r.bottom - f(8) as i32 };
                        label(gr, "Soltá acá para sumar a la pila", f(13), c.ink, &text, o);
                    }
                });
                self.grid_scene = Some(g.clone());
            }
        }
        ease(&mut self.grid_a, if self.want.grid.is_some() { 1.0 } else { 0.0 }, 0.35);
        self.grid.present(self.work.left, self.work.top, self.grid_a);

        // Vista previa: se desliza de celda en celda.
        if let Some(t) = self.want.preview {
            let to = [t.left as f32, t.top as f32, t.right as f32, t.bottom as f32];
            match &mut self.prev {
                Some(cur) if self.prev_a > 0.0 => {
                    for i in 0..4 {
                        ease(&mut cur[i], to[i], 0.4);
                    }
                }
                _ => self.prev = Some(to),
            }
        }
        ease(&mut self.prev_a, if self.want.preview.is_some() { 1.0 } else { 0.0 }, 0.35);
        if let Some(cur) = self.prev {
            let r = RECT { left: cur[0].round() as i32, top: cur[1].round() as i32, right: cur[2].round() as i32, bottom: cur[3].round() as i32 };
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if self.preview.size != (w, h) {
                self.preview.paint(w, h, |gr| unsafe {
                    let box_ = RECT { left: 0, top: 0, right: w, bottom: h };
                    shape(gr, &box_, (0, 0), f(8), Some(c.zone_fill), Some((c.accent, f(2), false)));
                });
            }
            self.preview.present(r.left, r.top, self.prev_a);
        }

        // Línea de la pila: dónde entra la nota.
        if let Some(t) = self.want.line {
            let ring = f(2).max(2.0) as i32;
            let r = RECT { left: t.left - ring, top: t.top - ring, right: t.right + ring, bottom: t.bottom + ring };
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if self.line.size != (w, h) {
                self.line.paint(w, h, |gr| unsafe {
                    let outer = RECT { left: 0, top: 0, right: w, bottom: h };
                    shape(gr, &outer, (0, 0), f(4), Some(c.zone_fill), None);
                    let inner = RECT { left: ring, top: ring, right: w - ring, bottom: h - ring };
                    shape(gr, &inner, (0, 0), f(2), Some(c.accent), None);
                });
            }
            self.line_at = Some(r);
        }
        ease(&mut self.line_a, if self.want.line.is_some() { 1.0 } else { 0.0 }, 0.5);
        if let Some(r) = self.line_at {
            self.line.present(r.left, r.top, self.line_a);
        }

        // Panel: baja un poco mientras aparece.
        if let Some(opts) = &self.want.panel {
            let same = self.panel_opts.iter().map(|r| q(*r)).eq(opts.iter().map(|r| q(*r)));
            if !opts.is_empty() && (!same || self.panel_hot != Some(self.want.hot)) {
                self.panel_opts = opts.clone();
                self.panel_hot = Some(self.want.hot);
                self.paint_panel(&c);
            }
        }
        ease(&mut self.panel_a, if self.want.panel.is_some() { 1.0 } else { 0.0 }, 0.35);
        if !self.panel_opts.is_empty() {
            let m = self.panel_margin();
            let first = self.panel_opts[0];
            let pad = px(self.dpi, 7);
            let slide = ((1.0 - self.panel_a) * f(-12)) as i32;
            self.panel.present(first.left - pad - m, first.top - pad - m + slide, self.panel_a);
        }

        // Guía arriba al centro, mientras se arrastra.
        if self.hint.size == (0, 0) {
            let (w, h) = ((self.area.right - self.area.left) * 22 / 100, px(self.dpi, 4).max(3));
            self.hint.paint(w, h, |gr| unsafe {
                let r = RECT { left: 0, top: -h, right: w, bottom: h };
                shape(gr, &r, (0, 0), f(4), Some(c.accent), None);
            });
        }
        ease(&mut self.hint_a, if self.want.hint && self.want.panel.is_none() { 0.55 } else { 0.0 }, 0.25);
        let hx = (self.area.left + self.area.right) / 2 - self.hint.size.0 / 2;
        self.hint.present(hx, self.work.top, self.hint_a);
    }

    fn panel_margin(&self) -> i32 {
        px(self.dpi, 22)
    }

    fn paint_panel(&mut self, c: &Colors) {
        let dpi = self.dpi;
        let f = |v: i32| (v * dpi / 96) as f32;
        let (m, pad) = (self.panel_margin(), px(dpi, 7));
        let opts = self.panel_opts.clone();
        let hot = self.panel_hot.flatten();
        let (first, last) = (opts[0], opts[opts.len() - 1]);
        let bg = RECT { left: first.left - pad, top: first.top - pad, right: last.right + pad, bottom: last.bottom + pad };
        let o = (bg.left - m, bg.top - m);
        let (w, h) = (bg.right - bg.left + 2 * m, bg.bottom - bg.top + 2 * m);
        let (accent, option, panel, panel_line, zone_fill, danger) = (c.accent, c.option, c.panel, c.panel_line, c.zone_fill, c.danger);
        self.panel.paint(w, h, |g| unsafe {
            // Sombra: capas redondeadas cada vez más grandes y más tenues, corridas hacia abajo.
            let spread = m - px(dpi, 6);
            for i in 0..spread {
                let s = i;
                let r = RECT { left: bg.left - s, top: bg.top - s + px(dpi, 8), right: bg.right + s, bottom: bg.bottom + s + px(dpi, 8) };
                let a = (16.0 * (1.0 - i as f32 / spread as f32)) as u32;
                shape(g, &r, o, f(10) + s as f32, Some(a << 24), None);
            }
            shape(g, &bg, o, f(10), Some(panel), Some((panel_line, 1.0, false)));
            for k in [3, 4] {
                let sx = (opts[k].right + opts[k + 1].left) / 2;
                let sep = RECT { left: sx, top: first.top, right: sx + 1, bottom: first.bottom };
                shape(g, &sep, o, 0.0, Some(panel_line), None);
            }
            for (i, r) in opts.iter().enumerate() {
                let is_hot = hot == Some(i);
                // El tacho, en rojo.
                let (hot_ink, hot_fill) = if i == 5 { (danger, with_alpha(danger, 0.25)) } else { (accent, zone_fill) };
                if is_hot {
                    shape(g, r, o, f(6), Some(hot_fill), None);
                }
                let ink = if is_hot { hot_ink } else { option };
                let (bw, bh) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
                if i < 4 {
                    let (a, b) = SHAPES[i];
                    let (u, gp) = (100.0 / 2.3, 100.0 * 0.15 / 2.3);
                    let sw = (a as f32 * u + (a - 1) as f32 * gp) * 0.8 / 100.0;
                    let sh = (b as f32 * u + (b - 1) as f32 * gp) * 0.8 / 100.0;
                    let piece = RECT {
                        left: r.left + (bw * 0.12) as i32,
                        top: r.top + (bh * 0.12) as i32,
                        right: r.left + (bw * (0.12 + sw)) as i32,
                        bottom: r.top + (bh * (0.12 + sh)) as i32,
                    };
                    shape(g, &piece, o, f(2), Some(ink), None);
                } else if i == 5 {
                    // Tacho: la tapa con su manija y el cuerpo, un poco más angosto abajo.
                    let (cx, w) = (r.left + (bw * 0.5) as i32, bw * 0.46);
                    let lid = RECT { left: cx - (w * 0.62) as i32, top: r.top + (bh * 0.26) as i32, right: cx + (w * 0.62) as i32, bottom: r.top + (bh * 0.32) as i32 };
                    let handle = RECT { left: cx - (w * 0.2) as i32, top: r.top + (bh * 0.19) as i32, right: cx + (w * 0.2) as i32, bottom: r.top + (bh * 0.27) as i32 };
                    shape(g, &handle, o, f(1), Some(ink), None);
                    shape(g, &lid, o, f(1), Some(ink), None);
                    let mut path: *mut GpPath = null_mut();
                    GdipCreatePath(FillModeAlternate, &mut path);
                    let (top, bottom) = ((r.top + (bh * 0.36) as i32 - o.1) as f32, (r.top + (bh * 0.8) as i32 - o.1) as f32);
                    let (x0, x1) = ((cx - o.0) as f32 - w * 0.5, (cx - o.0) as f32 + w * 0.5);
                    let pts = [
                        PointF { X: x0, Y: top },
                        PointF { X: x1, Y: top },
                        PointF { X: x1 - w * 0.08, Y: bottom },
                        PointF { X: x0 + w * 0.08, Y: bottom },
                    ];
                    GdipAddPathPolygon(path, pts.as_ptr(), 4);
                    let mut brush: *mut GpSolidFill = null_mut();
                    GdipCreateSolidFill(ink, &mut brush);
                    GdipFillPath(g, brush as *mut GpBrush, path);
                    GdipDeleteBrush(brush as *mut GpBrush);
                    GdipDeletePath(path);
                } else {
                    for (k, frac) in [0.64f32, 0.5, 0.4].iter().enumerate() {
                        let top = r.top + (bh * (0.16 + k as f32 * 0.25)) as i32;
                        let right = r.right - (bw * 0.12) as i32;
                        let bar = RECT { left: right - (bw * frac) as i32, top, right, bottom: top + (bh * 0.17) as i32 };
                        shape(g, &bar, o, f(2), Some(ink), None);
                    }
                }
            }
        });
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.close();
    }
}

/// Una capa: ventana en capas, transparente a los clics, con su lienzo.
#[derive(Default)]
struct Lay {
    hwnd: isize,
    mem_dc: isize,
    bitmap: isize,
    old_bitmap: isize,
    bits: usize,
    size: (i32, i32),
    /// Lo último que se mostró (posición y opacidad), para no repetirlo.
    shown: Option<(i32, i32, u8)>,
    dirty: bool,
}

const CLASS_NAME: &str = "SimpckySnapLayer";
// `PixelFormat32bppPARGB` (Gdipluspixelformats.h), como en `ghost.rs`.
const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;

fn register_class() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| unsafe {
        let class_name = wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(layer_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: app().lock().unwrap().hinstance as HINSTANCE,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    });
}

/// Las capas nunca se quedan con el mouse.
unsafe extern "system" fn layer_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        return HTTRANSPARENT as LRESULT;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

impl Lay {
    fn create(&mut self) {
        register_class();
        unsafe {
            let class_name = wide(CLASS_NAME);
            let hinstance = { app().lock().unwrap().hinstance } as HINSTANCE;
            self.hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                hinstance,
                null(),
            ) as isize;
        }
    }

    fn free_canvas(&mut self) {
        if self.mem_dc != 0 {
            unsafe {
                SelectObject(self.mem_dc as HDC, self.old_bitmap as HGDIOBJ);
                DeleteObject(self.bitmap as HGDIOBJ);
                DeleteDC(self.mem_dc as HDC);
            }
        }
        (self.mem_dc, self.bitmap, self.old_bitmap, self.bits, self.size) = (0, 0, 0, 0, (0, 0));
    }

    /// Redibuja la capa en un lienzo de `w` × `h` con `draw`.
    fn paint(&mut self, w: i32, h: i32, draw: impl FnOnce(*mut GpGraphics)) {
        if self.hwnd == 0 || w <= 0 || h <= 0 {
            return;
        }
        unsafe {
            if self.size != (w, h) {
                self.free_canvas();
                let screen = GetDC(null_mut());
                let mem_dc = CreateCompatibleDC(screen);
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w;
                bmi.bmiHeader.biHeight = -h;
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB;
                let mut bits: *mut core::ffi::c_void = null_mut();
                let bitmap = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                ReleaseDC(null_mut(), screen);
                if bitmap.is_null() || bits.is_null() {
                    DeleteDC(mem_dc);
                    return;
                }
                self.old_bitmap = SelectObject(mem_dc, bitmap as HGDIOBJ) as isize;
                (self.mem_dc, self.bitmap, self.bits, self.size) = (mem_dc as isize, bitmap as isize, bits as usize, (w, h));
            }
            let mut bmp: *mut GpBitmap = null_mut();
            if GdipCreateBitmapFromScan0(w, h, w * 4, PIXEL_FORMAT_32BPP_PARGB, self.bits as *mut u8, &mut bmp) != Ok {
                return;
            }
            let mut g: *mut GpGraphics = null_mut();
            GdipGetImageGraphicsContext(bmp as *mut GpImage, &mut g);
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
            GdipGraphicsClear(g, 0);
            draw(g);
            GdipDeleteGraphics(g);
            GdipDisposeImage(bmp as *mut GpImage);
        }
        self.dirty = true;
    }

    /// La muestra en (`x`, `y`) con opacidad `a` (0 a 1); en 0, la esconde.
    fn present(&mut self, x: i32, y: i32, a: f32) {
        if self.hwnd == 0 || self.mem_dc == 0 {
            return;
        }
        let alpha = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        if !self.dirty && self.shown == Some((x, y, alpha)) {
            return;
        }
        unsafe {
            if alpha == 0 {
                ShowWindow(self.hwnd as HWND, SW_HIDE);
            } else {
                let screen = GetDC(null_mut());
                let dst = POINT { x, y };
                let size = SIZE { cx: self.size.0, cy: self.size.1 };
                let src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
                UpdateLayeredWindow(self.hwnd as HWND, screen, &dst, &size, self.mem_dc as HDC, &src, 0, &blend, ULW_ALPHA);
                ReleaseDC(null_mut(), screen);
                if !matches!(self.shown, Some((_, _, v)) if v > 0) {
                    ShowWindow(self.hwnd as HWND, SW_SHOWNOACTIVATE);
                }
            }
        }
        self.shown = Some((x, y, alpha));
        self.dirty = false;
    }

    fn close(&mut self) {
        if self.hwnd != 0 {
            unsafe { DestroyWindow(self.hwnd as HWND) };
        }
        self.free_canvas();
        *self = Lay::default();
    }
}

unsafe fn round_rect_path(r: &RECT, o: (i32, i32), radius: f32) -> *mut GpPath {
    let (x, y) = ((r.left - o.0) as f32, (r.top - o.1) as f32);
    let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
    let d = (radius * 2.0).min(w).min(h).max(1.0);
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    path
}

/// Rectángulo redondeado relleno y/o con borde (color, ancho, punteado).
unsafe fn shape(g: *mut GpGraphics, r: &RECT, o: (i32, i32), radius: f32, fill: Option<u32>, line: Option<(u32, f32, bool)>) {
    let path = round_rect_path(r, o, radius);
    if let Some(c) = fill {
        let mut brush: *mut GpSolidFill = null_mut();
        GdipCreateSolidFill(c, &mut brush);
        GdipFillPath(g, brush as *mut GpBrush, path);
        GdipDeleteBrush(brush as *mut GpBrush);
    }
    if let Some((c, width, dashed)) = line {
        let mut pen: *mut GpPen = null_mut();
        GdipCreatePen1(c, width, UnitPixel, &mut pen);
        if dashed {
            GdipSetPenDashStyle(pen, DashStyleDash);
        }
        GdipDrawPath(g, pen, path);
        GdipDeletePen(pen);
    }
    GdipDeletePath(path);
}

unsafe fn label(g: *mut GpGraphics, text: &str, size: f32, color: u32, r: &RECT, o: (i32, i32)) {
    let face = wide("Segoe UI Semibold");
    let mut family: *mut GpFontFamily = null_mut();
    if GdipCreateFontFamilyFromName(face.as_ptr(), null_mut(), &mut family) != Ok {
        return;
    }
    let mut font: *mut GpFont = null_mut();
    GdipCreateFont(family, size, FontStyleRegular, UnitPixel, &mut font);
    let mut format: *mut GpStringFormat = null_mut();
    GdipCreateStringFormat(0, 0, &mut format);
    GdipSetStringFormatAlign(format, StringAlignmentCenter);
    GdipSetStringFormatLineAlign(format, StringAlignmentFar);
    GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    let w = wide(text);
    let rect = RectF { X: (r.left - o.0) as f32, Y: (r.top - o.1) as f32, Width: (r.right - r.left) as f32, Height: (r.bottom - r.top) as f32 };
    GdipDrawString(g, w.as_ptr(), -1, font, &rect, format, brush as *mut GpBrush);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeleteStringFormat(format);
    GdipDeleteFont(font);
    GdipDeleteFontFamily(family);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT { left: l, top: t, right: r, bottom: b }
    }
    fn k(c0: i32, c1: i32, r0: i32, r1: i32) -> Cells {
        Cells { c0, c1, r0, r1 }
    }

    #[test]
    fn la_cuadricula_llena_la_pantalla() {
        // 1920 × 1080 sin la barra de tareas, a 100 %: 8 × 5.
        let g = Grid::new(rect(0, 0, 1920, 1032), 96);
        assert_eq!((g.cols, g.rows), (8, 5));
        let all = g.rect(k(0, 7, 0, 4));
        assert_eq!((all.left, all.top, all.right, all.bottom), (12, 12, 1908, 1020));
        // 1366 × 768: 5 × 3; 2560 × 1440: 10 × 7.
        let g = Grid::new(rect(0, 0, 1366, 720), 96);
        assert_eq!((g.cols, g.rows), (5, 3));
        let g = Grid::new(rect(0, 0, 2560, 1392), 96);
        assert_eq!((g.cols, g.rows), (10, 7));
    }

    #[test]
    fn encajar_en_celdas() {
        let g = Grid::new(rect(0, 0, 1920, 1032), 96);
        let one = g.rect(k(1, 1, 1, 1));
        assert_eq!(g.cells_of(&one), k(1, 1, 1, 1));
        assert_eq!(g.snap_shape(one.left + 20, one.top - 30, 2, 1), k(1, 2, 1, 1));
        // Un poco más grande que dos celdas sigue siendo dos.
        let two = g.rect(k(2, 3, 0, 0));
        assert_eq!(g.snap_rect(&rect(two.left + 10, two.top, two.right + 40, two.bottom)), k(2, 3, 0, 0));
        assert_eq!(g.shape_for(280, 320), (1, 2));
    }

    #[test]
    fn la_tapada_se_achica_sin_quedar_en_l() {
        // La captura: 2 × 1, cae una nota en su celda derecha → queda la izquierda.
        assert_eq!(shrink(k(3, 4, 1, 1), k(4, 4, 1, 1)), Some(k(3, 3, 1, 1)));
        // 2 × 2 menos una esquina: una franja de dos celdas, nunca la L de tres.
        assert_eq!(shrink(k(0, 1, 0, 1), k(1, 1, 1, 1)).map(|c| c.area()), Some(2));
        // Tapada entera.
        assert_eq!(shrink(k(2, 2, 2, 2), k(1, 3, 1, 3)), None);
    }

    #[test]
    fn la_tapada_entera_se_corre() {
        let g = Grid::new(rect(0, 0, 1920, 1032), 96);
        let (t, plan) = plan_around(&g, k(2, 3, 1, 2), &[(7, g.rect(k(2, 2, 1, 1)))], &[]);
        assert_eq!(t, k(2, 3, 1, 2));
        assert_eq!(plan.len(), 1);
        assert!(!plan[0].1.meets(&t));
        // Una nota suelta sin alinear que la nueva pisa en parte: no crece.
        let loose = rect(700, 640, 1120, 1120);
        let (t, plan) = plan_around(&g, k(2, 3, 2, 3), &[(8, loose)], &[]);
        let r = g.rect(plan[0].1);
        assert!(!plan[0].1.meets(&t));
        assert!(r.right - r.left <= 420 && r.bottom - r.top <= 480);
        // Una que no toca, ni se mueve.
        assert!(plan_around(&g, k(0, 0, 0, 0), &[(9, loose)], &[]).1.is_empty());
        // Y si pisa una pila, se achica la que se suelta.
        let (t, _) = plan_around(&g, k(5, 6, 0, 0), &[], &[k(6, 7, 0, 1)]);
        assert_eq!(t, k(5, 5, 0, 0));
    }

    #[test]
    fn reconocer_pilas() {
        let right = |_x: i32| true;
        // Tu pila: alineadas a la derecha, anchos distintos, una cada 50.
        let bars = [(1, rect(1478, 12, 1908, 52), true), (2, rect(1578, 62, 1908, 102), true), (3, rect(1638, 112, 1908, 152), true)];
        let piles = find_piles(&bars, 40, 50, right);
        assert_eq!(piles, vec![Pile { right: true, edge: 1908, top: 12, ids: vec![1, 2, 3], sides: vec![true; 3] }]);
        // Lados mezclados: una ancha arriba, una chica a la izquierda y otra a la derecha.
        let bars = [(1, rect(1000, 0, 1500, 40), true), (2, rect(1000, 50, 1200, 90), true), (3, rect(1300, 100, 1500, 140), true)];
        let p = &find_piles(&bars, 40, 50, right)[0];
        assert_eq!((p.ids.len(), p.sides.clone()), (3, vec![true, false, true]));
        // Armada a mano: bordes corridos unos píxeles, separaciones desparejas.
        let bars = [(1, rect(1470, 10, 1905, 50), true), (2, rect(1580, 55, 1912, 95), true), (3, rect(1640, 112, 1900, 152), true)];
        let p = &find_piles(&bars, 40, 50, right)[0];
        assert_eq!((p.ids.len(), p.edge), (3, 1912));
        // Con un hueco grande, son dos pilas.
        let bars = [(1, rect(0, 0, 300, 40), true), (2, rect(0, 50, 300, 90), true), (3, rect(0, 200, 300, 240), true), (4, rect(0, 250, 300, 290), true)];
        assert_eq!(find_piles(&bars, 40, 50, |_| false).len(), 2);
        // Igual ancho a la izquierda de la pantalla: pila izquierda.
        let p = &find_piles(&bars[..2], 40, 50, |_| false)[0];
        assert!(!p.right && p.edge == 0);
        // Una enrollada sola es una pila de una; una abierta sola, no.
        assert_eq!(find_piles(&[(1, rect(0, 0, 300, 40), true)], 40, 50, right).len(), 1);
        assert!(find_piles(&[(1, rect(0, 0, 300, 40), false)], 40, 50, right).is_empty());
    }

    #[test]
    fn la_cuadricula_sigue_el_acomodo_de_windows() {
        // 1920 a 150 %: entran 5 columnas de 330 + 18.
        assert_eq!(fit_count(1920, 330, 18, &[]), 5);
        // Una ventana en la mitad: columnas pares, el borde cae entre dos.
        assert_eq!(fit_count(1920, 330, 18, &[960]) % 2, 0);
        // Tercios: múltiplo de 3.
        assert_eq!(fit_count(1920, 330, 18, &[640, 1280]) % 3, 0);
        // Alto de 1131 con una ventana en la mitad de abajo: filas pares.
        assert_eq!(fit_count(1131, 270, 18, &[565]) % 2, 0);
    }

    #[test]
    fn la_cuadricula_va_donde_no_hay_aplicaciones() {
        let work = rect(0, 0, 1920, 1032);
        // Sin ventanas: todo el monitor.
        assert_eq!(free_area(work, &[], 244, 204).map(q), Some([0, 0, 1920, 1032]));
        // Navegador acomodado a la izquierda: la mitad derecha, desde su borde.
        assert_eq!(free_area(work, &[rect(0, 0, 960, 1032)], 244, 204).map(q), Some([960, 0, 1920, 1032]));
        // Dos cuartos a la derecha: la mitad izquierda.
        let quarters = [rect(960, 0, 1920, 516), rect(960, 516, 1920, 1032)];
        assert_eq!(free_area(work, &quarters, 244, 204).map(q), Some([0, 0, 960, 1032]));
        // Una ventana de otro monitor no cuenta.
        assert_eq!(free_area(work, &[rect(1920, 0, 3840, 1200)], 244, 204).map(q), Some([0, 0, 1920, 1032]));
        // Maximizada: no queda lugar.
        assert!(free_area(work, &[rect(-8, -8, 1928, 1040)], 244, 204).is_none());
    }

    #[test]
    fn lugar_en_la_pila() {
        let p = Pile { right: true, edge: 1000, top: 100, ids: vec![1, 2, 3], sides: vec![true, false, true] };
        assert_eq!(p.slot(90, 50, 40), 0);
        assert_eq!(p.slot(130, 50, 40), 1);
        assert_eq!(p.slot(400, 50, 40), 3);
        // Columna de 500 anclada a la derecha en 1000: la 3 a la derecha, la 2 a la izquierda.
        assert_eq!(p.place(2, 300, 500, 50), (700, 200));
        assert_eq!(p.place(1, 300, 500, 50), (500, 150));
        // Una nota chica soltada del lado izquierdo de la columna va a la izquierda.
        assert!(!p.side_at(560, 200, 500));
        assert!(p.side_at(900, 200, 500));
    }
}
