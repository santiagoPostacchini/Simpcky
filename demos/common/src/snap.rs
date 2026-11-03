//! Celdas y pilas al arrastrar, portado de `src/snap.rs` de Simpcky.
//!
//! Misma lógica (panel arriba con 1×1, 2×1, 1×2, 2×2, pila y tacho;
//! cuadrícula con Ctrl o con una forma elegida; las notas pisadas se
//! achican o se corren; pilas que se reconocen por la posición; Ctrl +
//! arrastrar una nota de pila mueve la pila entera; Esc vuelve todo), pero
//! sin ventanas: le pide a la demo (`Host`) que mueva las notas, y deja en
//! `Layers` lo que las capas tienen que mostrar en cada cuadro, ya suavizado.
//!
//! Diferencias con la app: un solo monitor por arrastre (el de donde
//! empezó), sin redimensionar con Ctrl + borde, y las capas se desvanecen
//! al soltar en vez de cerrarse de golpe.

use std::collections::HashMap;
use std::time::Instant;

use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::HMONITOR;

use crate::anim::{Motion, MOVE_MS, ROLL_MS};
use crate::geom::*;
use crate::win::{contains, cursor, monitor_of};

/// Una nota del escritorio, como la ve el arrastre.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    /// Alto abierto.
    pub h: i32,
    pub rolled: bool,
    pub bar: i32,
}

impl Info {
    fn bar_rect(&self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.bar }
    }
    fn visible(&self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + if self.rolled { self.bar } else { self.h } }
    }
}

/// Lo que la demo pone: sus notas y sus ventanas.
pub trait Host {
    /// Notas del escritorio a la vista (no "siempre encima") en `monitor`.
    fn notes_on(&self, monitor: HMONITOR) -> Vec<Info>;
    /// Dónde está la ventana de la nota ahora, en pantalla.
    fn window_rect(&self, id: u32) -> RECT;
    /// Mueve o cambia de tamaño la ventana ya (sin transición).
    fn place(&mut self, id: u32, pos: Option<(i32, i32)>, size: Option<(i32, i32)>);
    /// (ancho, alto abierto, enrollada, Auto, bloqueada).
    fn state(&self, id: u32) -> (i32, i32, bool, bool, bool);
    /// Guarda la forma (tamaño abierto, si viene) y si está enrollada.
    fn set_shape(&mut self, id: u32, size: Option<(i32, i32)>, rolled: bool);
    fn set_auto(&mut self, id: u32, auto: bool);
    fn store(&mut self, id: u32, x: i32, y: i32, size: Option<(i32, i32)>);
    /// Arriba de las otras notas.
    fn raise(&mut self, id: u32);
    /// Soltada en el tacho.
    fn trash(&mut self, id: u32);
    fn save(&mut self);
    /// Alto de la barra, en píxeles.
    fn bar(&self) -> i32;
    fn dpi(&self) -> i32 {
        96
    }
    fn dark(&self) -> bool;
}

fn px(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

fn shown_in(n: &Info, r: RECT) -> RECT {
    RECT { bottom: if n.rolled { r.top + n.bar } else { r.bottom }, ..r }
}

const SHAPES: [(i32, i32); 4] = [(1, 1), (2, 1), (1, 2), (2, 2)];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Choice {
    Shape(i32, i32),
    Pile,
    Trash,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    None,
    Cell(Cells, bool),
    Pile(usize, usize, bool),
    NewPile,
    Trash,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Move,
    PileMove,
}

#[derive(Clone, Copy)]
struct Orig {
    rect: RECT,
    w: i32,
    h: i32,
    rolled: bool,
    auto: bool,
    locked: bool,
}

struct Drag {
    id: u32,
    kind: Kind,
    snaps: bool,
    work: RECT,
    area: RECT,
    grid: Grid,
    blocked: Vec<Cells>,
    dpi: i32,
    step: i32,
    bar: i32,
    by_id: HashMap<u32, Info>,
    piles: Vec<Pile>,
    from: Option<usize>,
    moving: Option<usize>,
    others: Vec<u32>,
    grab: POINT,
    orig: Orig,
    moved: bool,
    panel: bool,
    choice: Option<Choice>,
    hovering: Option<(Choice, Instant)>,
    mode: Option<Choice>,
    target: Target,
    plan: Vec<(u32, Cells)>,
    last_layout: Option<(i32, i32, Vec<u32>)>,
}

// -----------------------------------------------------------------
// Lo que muestran las capas
// -----------------------------------------------------------------

#[derive(Clone, Default, PartialEq, Debug)]
pub struct GridScene {
    pub cells: Vec<[i32; 4]>,
    pub zones: Vec<[i32; 4]>,
    /// Donde hay notas: ahí no se dibuja (la cuadrícula queda "debajo").
    pub holes: Vec<[i32; 4]>,
}

#[derive(Default, Clone)]
struct Want {
    grid: Option<GridScene>,
    preview: Option<RECT>,
    line: Option<RECT>,
    panel: Option<Vec<RECT>>,
    hot: Option<usize>,
    hint: bool,
}

/// El estado de las capas en este cuadro, ya suavizado. En coordenadas de
/// pantalla; la demo las dibuja relativas a `work`.
#[derive(Clone)]
pub struct Layers {
    pub work: RECT,
    pub area: RECT,
    pub dpi: i32,
    pub dark: bool,
    pub grid: Option<GridScene>,
    pub grid_a: f32,
    /// Vista previa: (izquierda, arriba, derecha, abajo) mientras se desliza.
    pub preview: Option<[f32; 4]>,
    pub preview_a: f32,
    pub line: Option<[i32; 4]>,
    pub line_a: f32,
    /// Las seis opciones del panel (formas, pila, tacho) y cuál está elegida.
    pub panel: Vec<[i32; 4]>,
    pub hot: Option<usize>,
    pub panel_a: f32,
    /// La guía arriba al centro.
    pub hint: [i32; 4],
    pub hint_a: f32,
    /// Cambia cada vez que algo de esto cambia (para no redibujar de más).
    pub serial: u64,
}

impl Layers {
    /// ¿Se ve algo?
    pub fn visible(&self) -> bool {
        self.grid_a > 0.0 || self.preview_a > 0.0 || self.line_a > 0.0 || self.panel_a > 0.0 || self.hint_a > 0.0
    }
    /// Margen del panel (para su sombra).
    pub fn panel_margin(&self) -> i32 {
        px(self.dpi, 22)
    }
}

struct Fx {
    want: Want,
    layers: Layers,
    last: Instant,
}

/// Acerca `v` a `to` un poco por cuadro de 60 Hz (como `snap::ease`), sea
/// cual sea la frecuencia de la pantalla.
fn ease(v: &mut f32, to: f32, k: f32, dt: f32) -> bool {
    let before = *v;
    let k = 1.0 - (1.0 - k).powf(dt * 60.0);
    *v += (to - *v) * k;
    if (to - *v).abs() < 0.01 {
        *v = to;
    }
    *v != before
}

impl Fx {
    fn new() -> Fx {
        let z = [0; 4];
        Fx {
            want: Want::default(),
            layers: Layers {
                work: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                area: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                dpi: 96,
                dark: false,
                grid: None,
                grid_a: 0.0,
                preview: None,
                preview_a: 0.0,
                line: None,
                line_a: 0.0,
                panel: Vec::new(),
                hot: None,
                panel_a: 0.0,
                hint: z,
                hint_a: 0.0,
                serial: 0,
            },
            last: Instant::now(),
        }
    }

    fn reset(&mut self, work: RECT, area: RECT, dpi: i32, dark: bool) {
        let l = &mut self.layers;
        (l.work, l.area, l.dpi, l.dark) = (work, area, dpi, dark);
        let (w, h) = ((area.right - area.left) * 22 / 100, px(dpi, 4).max(3));
        let hx = (area.left + area.right) / 2 - w / 2;
        l.hint = [hx, work.top, hx + w, work.top + h];
        self.last = Instant::now();
        l.serial += 1;
    }

    /// Un cuadro: cada capa se acerca a lo que tiene que mostrar.
    fn frame(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let w = &self.want;
        let l = &mut self.layers;
        let mut changed = false;
        if let Some(g) = &w.grid {
            if l.grid.as_ref() != Some(g) {
                l.grid = Some(g.clone());
                changed = true;
            }
        }
        changed |= ease(&mut l.grid_a, if w.grid.is_some() { 1.0 } else { 0.0 }, 0.35, dt);
        if let Some(t) = w.preview {
            let to = [t.left as f32, t.top as f32, t.right as f32, t.bottom as f32];
            match &mut l.preview {
                Some(cur) if l.preview_a > 0.0 => {
                    for i in 0..4 {
                        changed |= ease(&mut cur[i], to[i], 0.4, dt);
                    }
                }
                _ => {
                    l.preview = Some(to);
                    changed = true;
                }
            }
        }
        changed |= ease(&mut l.preview_a, if w.preview.is_some() { 1.0 } else { 0.0 }, 0.35, dt);
        if let Some(t) = w.line {
            let ring = px(l.dpi, 2).max(2);
            let r = [t.left - ring, t.top - ring, t.right + ring, t.bottom + ring];
            if l.line != Some(r) {
                l.line = Some(r);
                changed = true;
            }
        }
        changed |= ease(&mut l.line_a, if w.line.is_some() { 1.0 } else { 0.0 }, 0.5, dt);
        if let Some(opts) = &w.panel {
            let v: Vec<[i32; 4]> = opts.iter().map(|r| q(*r)).collect();
            if !v.is_empty() && (l.panel != v || l.hot != w.hot) {
                l.panel = v;
                l.hot = w.hot;
                changed = true;
            }
        }
        changed |= ease(&mut l.panel_a, if w.panel.is_some() { 1.0 } else { 0.0 }, 0.35, dt);
        changed |= ease(&mut l.hint_a, if w.hint && w.panel.is_none() { 0.55 } else { 0.0 }, 0.25, dt);
        if changed {
            l.serial += 1;
        }
        l.visible()
    }
}

// -----------------------------------------------------------------
// El arrastre
// -----------------------------------------------------------------

pub struct Snap {
    drag: Option<Drag>,
    pub motion: Motion,
    fx: Fx,
    /// La nota recién soltada no se despliega sola hasta que el mouse salga.
    quiet: Option<u32>,
}

impl Default for Snap {
    fn default() -> Self {
        Snap::new()
    }
}

impl Snap {
    pub fn new() -> Snap {
        Snap { drag: None, motion: Motion::new(), fx: Fx::new(), quiet: None }
    }

    /// ¿Se está arrastrando?
    pub fn busy(&self) -> bool {
        self.drag.is_some()
    }

    pub fn dragging(&self) -> Option<u32> {
        self.drag.as_ref().map(|d| d.id)
    }

    /// ¿Se movió desde que se la agarró? (Si no, al soltar fue un clic.)
    pub fn moved(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.moved)
    }

    pub fn layers(&self) -> &Layers {
        &self.fx.layers
    }

    /// Como `snap::hush`: `true` si la nota no tiene que desplegarse sola todavía.
    pub fn hush(&mut self, id: u32, hovered: bool) -> bool {
        if self.quiet != Some(id) {
            return false;
        }
        if !hovered {
            self.quiet = None;
        }
        hovered
    }

    /// Mueve con transición (posición y/o tamaño).
    pub fn animate(&mut self, host: &mut dyn Host, id: u32, pos: Option<(i32, i32)>, size: Option<(i32, i32)>, ms: u32) {
        let cur = host.window_rect(id);
        let (_, p, s, _) = self.motion.set(id, cur, pos, size, ms);
        if p.is_some() || s.is_some() {
            host.place(id, p, s);
        }
    }

    /// Cambia la forma de la nota (como `note::reshape`): tamaño abierto,
    /// y enrollada o no; la ventana va con transición.
    pub fn reshape(&mut self, host: &mut dyn Host, id: u32, size: Option<(i32, i32)>, rolled: bool) {
        host.set_shape(id, size, rolled);
        let (w, h, rolled, ..) = host.state(id);
        let bar = host.bar();
        self.animate(host, id, None, Some((w, if rolled { bar } else { h })), ROLL_MS);
    }

    /// Un cuadro: sigue el arrastre (Esc, botón suelto), avanza las
    /// transiciones y las capas. `true` mientras haya algo moviéndose.
    pub fn frame(&mut self, host: &mut dyn Host) -> bool {
        if self.drag.is_some() {
            if crate::win::esc_down() {
                self.end(host, true);
            } else if !crate::win::button_down() {
                self.end(host, false);
            } else {
                self.step(host);
            }
        }
        for (id, pos, size, _) in self.motion.tick(Instant::now()) {
            host.place(id, pos, size);
        }
        let fx = self.fx.frame();
        self.drag.is_some() || self.motion.busy() || fx
    }

    fn build(&self, host: &dyn Host, id: u32, grab: POINT, pt: POINT) -> Drag {
        let (monitor, work) = monitor_of(pt.x, pt.y);
        let dpi = host.dpi();
        let bar = host.bar();
        let step = bar + px(dpi, PILE_GAP);
        let infos = host.notes_on(monitor);
        let snaps = infos.iter().any(|n| n.id == id);
        let apps: Vec<RECT> = crate::win::app_windows()
            .into_iter()
            .map(|w| RECT { left: w.left.max(work.left), top: w.top.max(work.top), right: w.right.min(work.right), bottom: w.bottom.min(work.bottom) })
            .filter(|w| w.right > w.left && w.bottom > w.top)
            .collect();
        let g = px(dpi, GAP);
        let inner = |v: i32, lo: i32, hi: i32| v > lo + g && v < hi - g;
        let xs: Vec<i32> = apps.iter().flat_map(|w| [w.left, w.right]).filter(|&x| inner(x, work.left, work.right)).collect();
        let ys: Vec<i32> = apps.iter().flat_map(|w| [w.top, w.bottom]).filter(|&y| inner(y, work.top, work.bottom)).collect();
        let grid = Grid::fitted(work, dpi, &xs, &ys);
        let mut blocked = Vec::new();
        for r in 0..grid.rows {
            for c in 0..grid.cols {
                let k = Cells { c0: c, c1: c, r0: r, r1: r };
                let cell = grid.rect(k);
                if apps.iter().any(|a| a.right.min(cell.right) - a.left.max(cell.left) > g && a.bottom.min(cell.bottom) - a.top.max(cell.top) > g) {
                    blocked.push(k);
                }
            }
        }
        if blocked.len() as i32 == grid.cols * grid.rows {
            blocked.clear();
        }
        let area = free_area(work, &apps, px(dpi, CELL_W) + 2 * g, px(dpi, CELL_H) + 2 * g).unwrap_or(work);
        let mut piles = if snaps {
            let bars: Vec<(u32, RECT, bool)> = infos.iter().map(|n| (n.id, n.bar_rect(), n.rolled)).collect();
            let mid = (work.left + work.right) / 2;
            find_piles(&bars, bar, step, |x| x > mid)
        } else {
            Vec::new()
        };
        let mine = piles.iter().position(|p| p.ids.contains(&id));
        let kind = if crate::win::ctrl_down() && mine.is_some() { Kind::PileMove } else { Kind::Move };
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
        let (w, h, rolled, auto, locked) = host.state(id);
        Drag {
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
            orig: Orig { rect: host.window_rect(id), w, h, rolled, auto, locked },
            moved: false,
            panel: false,
            choice: None,
            hovering: None,
            mode: None,
            target: Target::None,
            plan: Vec::new(),
            last_layout: None,
        }
    }

    /// Empieza a arrastrar la nota por la barra; `grab`: dónde se la agarró
    /// (respecto de su esquina). La demo captura el mouse.
    pub fn begin_move(&mut self, host: &mut dyn Host, id: u32, grab: POINT) {
        let d = self.build(host, id, grab, cursor());
        self.motion.stop_pos(id);
        host.raise(id);
        if d.from.is_some() && !d.orig.rolled {
            self.reshape(host, id, None, true);
        }
        if d.kind == Kind::PileMove {
            if let Some(p) = d.moving {
                for k in d.piles[p].ids.clone() {
                    host.raise(k);
                }
            }
        }
        self.fx.reset(d.work, d.area, d.dpi, host.dark());
        self.drag = Some(d);
    }

    /// Ctrl + Mayús + clic en la barra: la nota pasa a ser una pila.
    pub fn make_pile(&mut self, host: &mut dyn Host, id: u32) {
        if self.busy() {
            return;
        }
        let d = self.build(host, id, POINT { x: 0, y: 0 }, cursor());
        if !d.snaps || d.from.is_some() || d.moving.is_some() {
            return;
        }
        let np = d.new_pile(host);
        let pile = match d.pile_under(&np, None) {
            Some(Target::Pile(k, idx, side)) => {
                let mut p = d.piles[k].clone();
                p.insert(idx, d.id, side);
                p
            }
            _ => np,
        };
        host.set_auto(id, true);
        self.reshape(host, id, None, true);
        self.settle_pile(host, &pile, &d.by_id, d.step);
        self.quiet = Some(id);
        host.save();
    }

    /// El mouse se movió (WM_MOUSEMOVE de la nota que se arrastra).
    pub fn step(&mut self, host: &mut dyn Host) {
        let Some(mut d) = self.drag.take() else { return };
        match d.kind {
            Kind::Move => self.update_move(host, &mut d),
            Kind::PileMove => self.update_pile_move(host, &mut d),
        }
        self.drag = Some(d);
    }

    fn settle_pile(&mut self, host: &mut dyn Host, pile: &Pile, by_id: &HashMap<u32, Info>, step: i32) {
        let max_w = pile.ids.iter().filter_map(|k| by_id.get(k)).map(|n| n.w).max().unwrap_or(0);
        for (i, id) in pile.ids.iter().enumerate() {
            if let Some(n) = by_id.get(id) {
                let (x, y) = pile.place(i, n.w, max_w, step);
                self.animate(host, *id, Some((x, y)), None, MOVE_MS);
                host.store(*id, x, y, None);
            }
        }
    }

    fn replan(&mut self, host: &mut dyn Host, d: &mut Drag, plan: Vec<(u32, Cells)>) {
        for id in d.others.clone() {
            let (old, new) = (d.plan.iter().find(|p| p.0 == id), plan.iter().find(|p| p.0 == id));
            if old.map(|p| p.1) == new.map(|p| p.1) {
                continue;
            }
            let Some(n) = d.by_id.get(&id).copied() else { continue };
            let r = match new {
                Some((_, k)) => shown_in(&n, d.grid.rect(*k)),
                None => n.visible(),
            };
            self.animate(host, id, Some((r.left, r.top)), Some((r.right - r.left, r.bottom - r.top)), MOVE_MS);
        }
        d.plan = plan;
    }

    fn apply_choice(&mut self, host: &mut dyn Host, d: &mut Drag, c: Choice) {
        match c {
            Choice::Trash => return,
            Choice::Shape(a, b) => {
                let r = d.grid.rect(Cells { c0: 0, c1: a.min(d.grid.cols) - 1, r0: 0, r1: b.min(d.grid.rows) - 1 });
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                self.reshape(host, d.id, Some((w, h)), false);
                if let Some(n) = d.by_id.get_mut(&d.id) {
                    (n.w, n.h, n.rolled) = (w, h, false);
                }
                d.grab.x = d.grab.x.min(w / 2);
            }
            Choice::Pile => {
                self.reshape(host, d.id, None, true);
                if let Some(n) = d.by_id.get_mut(&d.id) {
                    n.rolled = true;
                }
            }
        }
        d.grab.y = d.grab.y.min(d.bar - px(d.dpi, 4));
    }

    fn update_move(&mut self, host: &mut dyn Host, d: &mut Drag) {
        let pt = cursor();
        let ctrl = crate::win::ctrl_down();
        let w = d.by_id.get(&d.id).map(|n| n.w).unwrap_or(d.orig.rect.right - d.orig.rect.left);
        let (_, here) = monitor_of(pt.x, pt.y);
        let x = pt.x - d.grab.x;
        let y = (pt.y - d.grab.y).clamp(here.top, (here.bottom - d.bar).max(here.top));
        if !d.moved {
            if (pt.x - d.orig.rect.left - d.grab.x).abs() + (pt.y - d.orig.rect.top - d.grab.y).abs() == 0 {
                return;
            }
            d.moved = true;
            if let Some(p) = d.from {
                let pile = d.piles[p].clone();
                let max_w = d.max_w(&pile);
                for (i, id) in pile.ids.iter().enumerate() {
                    if let Some(n) = d.by_id.get(id).copied() {
                        let (px_, py_) = pile.place(i, n.w, max_w, d.step);
                        self.animate(host, n.id, Some((px_, py_)), None, MOVE_MS);
                    }
                }
            }
        }
        host.place(d.id, Some((x, y)), None);
        if !d.snaps {
            return;
        }
        let opts = d.panel_options();
        if pt.y <= d.work.top + px(d.dpi, 4) && (d.area.left..d.area.right).contains(&pt.x) {
            d.panel = true;
        }
        if d.panel {
            let under = opts.iter().position(|r| contains(r, pt)).map(|i| match i {
                4 => Choice::Pile,
                5 => Choice::Trash,
                _ => Choice::Shape(SHAPES[i].0, SHAPES[i].1),
            });
            if under != d.hovering.map(|h| h.0) {
                d.hovering = under.map(|c| (c, Instant::now()));
            }
            if let Some((c, since)) = d.hovering {
                if d.choice != Some(c) && (d.choice.is_none() || since.elapsed().as_millis() >= 120) {
                    d.choice = Some(c);
                    self.apply_choice(host, d, c);
                }
            }
            if pt.y > opts[0].bottom + px(d.dpi, 30) {
                d.panel = false;
                if d.choice == Some(Choice::Trash) {
                    d.choice = None;
                }
                d.mode = d.choice;
            }
        }
        let me = d.by_id.get(&d.id).copied();
        let grid_on = !d.panel && (matches!(d.mode, Some(Choice::Shape(..))) || ctrl);
        let zones = d.zones();
        let mut plan = Vec::new();
        d.target = if d.panel && d.choice == Some(Choice::Trash) && !d.orig.locked && contains(&opts[5], pt) {
            Target::Trash
        } else if d.panel || !contains(&d.work, pt) {
            Target::None
        } else if grid_on {
            if let Some((p, _)) = zones.iter().find(|(_, z)| contains(&d.grid.rect(*z), pt)) {
                Target::Pile(*p, d.piles[*p].slot(pt.y, d.step, d.bar), d.side_in(*p, x + w / 2))
            } else {
                let (a, b) = match d.mode {
                    Some(Choice::Shape(a, b)) => (a, b),
                    _ => me.map(|n| d.grid.shape_for(n.w, n.h)).unwrap_or((1, 1)),
                };
                let zc: Vec<Cells> = zones.iter().map(|z| z.1).chain(d.blocked.iter().copied()).collect();
                let (t, p) = plan_around(&d.grid, d.grid.snap_shape(x, y, a, b), &d.other_rects(), &zc);
                plan = p;
                Target::Cell(t, matches!(d.mode, Some(Choice::Shape(..))))
            }
        } else {
            let cy = y + d.bar / 2;
            let margin = px(d.dpi, 30);
            let hit = d.piles.iter().enumerate().find(|(_, p)| {
                if p.ids.is_empty() || d.hidden(p) {
                    return false;
                }
                let b = p.bounds(d.max_w(p).max(w), d.step, d.bar);
                pt.x >= b.left - margin && pt.x <= b.right + margin && cy >= b.top - d.bar && cy <= b.bottom + px(d.dpi, 20)
            });
            match hit {
                Some((i, p)) => Target::Pile(i, p.slot(cy, d.step, d.bar), d.side_in(i, x + w / 2)),
                None if d.mode == Some(Choice::Pile) => d.pile_under(&d.new_pile(host), None).unwrap_or(Target::NewPile),
                None => Target::None,
            }
        };
        self.replan(host, d, plan);
        self.show(host, d, grid_on, &zones, Some(opts));
    }

    fn update_pile_move(&mut self, host: &mut dyn Host, d: &mut Drag) {
        let Some(p) = d.moving else { return };
        let pt = cursor();
        d.moved = true;
        let i = d.piles[p].ids.iter().position(|k| *k == d.id).unwrap_or(0) as i32;
        let w = d.by_id.get(&d.id).map(|n| n.w).unwrap_or(0);
        let (x, y) = (pt.x - d.grab.x, pt.y - d.grab.y);
        let mv = &d.piles[p];
        let edge = if mv.right { d.grid.snap_right_edge(x + w) } else { d.grid.snap_left_edge(x) };
        let probe = Pile { edge, top: d.grid.snap_top(y - i * d.step), ..mv.clone() };
        d.target = d.pile_under(&probe, Some(p)).unwrap_or(Target::None);
        let layout = match d.target {
            Target::Pile(k, idx, _) => d.merged(k, idx),
            _ => probe.clone(),
        };
        let key = (layout.edge, layout.top, layout.ids.clone());
        if d.last_layout.as_ref() != Some(&key) {
            d.last_layout = Some(key);
            (d.piles[p].edge, d.piles[p].top) = (probe.edge, probe.top);
            let tp = d.target_pile();
            let others: Vec<Pile> = d.piles.iter().enumerate().filter(|(k, _)| *k != p && Some(*k) != tp).map(|(_, o)| o.clone()).collect();
            for pile in others.iter().chain(std::iter::once(&layout)) {
                let max_w = d.max_w(pile);
                for (j, id) in pile.ids.iter().enumerate() {
                    if let Some(n) = d.by_id.get(id).copied() {
                        let (nx, ny) = pile.place(j, n.w, max_w, d.step);
                        self.animate(host, n.id, Some((nx, ny)), None, 130);
                    }
                }
            }
            let plan = if matches!(d.target, Target::Pile(..)) {
                Vec::new()
            } else {
                let t = d.grid.cells_of(&layout.bounds(d.max_w(&layout), d.step, d.bar));
                plan_around(&d.grid, t, &d.other_rects(), &[]).1
            };
            self.replan(host, d, plan);
        }
        let zones = d.zones();
        self.show(host, d, true, &zones, None);
    }

    fn show(&mut self, host: &dyn Host, d: &Drag, grid_on: bool, zones: &[(usize, Cells)], opts: Option<Vec<RECT>>) {
        let mut grid = None;
        if grid_on {
            let mut g = GridScene::default();
            for r in 0..d.grid.rows {
                for c in 0..d.grid.cols {
                    if !zones.iter().any(|(_, z)| z.has(c, r)) && !d.blocked.iter().any(|b| b.has(c, r)) {
                        g.cells.push(q(d.grid.rect(Cells { c0: c, c1: c, r0: r, r1: r })));
                    }
                }
            }
            g.zones = zones.iter().map(|(_, z)| q(d.grid.rect(*z))).collect();
            g.holes = d.holes(host).into_iter().map(q).collect();
            grid = Some(g);
        }
        let mut preview = None;
        let mut line = None;
        match d.target {
            Target::Cell(t, _) => preview = Some(d.grid.rect(t)),
            Target::NewPile => {
                let pile = d.new_pile(host);
                let w = d.by_id.get(&d.id).map(|n| n.w).unwrap_or(0);
                let (x, y) = pile.place(0, w, w, d.step);
                preview = Some(RECT { left: x, top: y, right: x + w, bottom: y + d.bar });
            }
            Target::Pile(p, idx, side) => {
                let pile = &d.piles[p];
                let me_w = d.by_id.get(&d.id).map(|n| n.w).unwrap_or(0);
                let max_w = d.max_w(pile).max(me_w);
                let (l, r) = pile.column(max_w);
                let (left, right) = match d.kind {
                    Kind::PileMove => (l, r),
                    _ if side => (r - me_w, r),
                    _ => (l, l + me_w),
                };
                let y = pile.top + idx as i32 * d.step - px(d.dpi, PILE_GAP) / 2;
                let t = px(d.dpi, 3).max(2);
                line = Some(RECT { left, top: y - t / 2, right, bottom: y - t / 2 + t });
            }
            _ => {}
        }
        let hot = match d.choice {
            Some(Choice::Shape(a, b)) => SHAPES.iter().position(|s| *s == (a, b)),
            Some(Choice::Pile) => Some(4),
            Some(Choice::Trash) if !d.orig.locked => Some(5),
            _ => None,
        };
        self.fx.want = Want { grid, preview, line, panel: d.panel.then(|| opts.unwrap_or_default()), hot, hint: d.kind == Kind::Move };
    }

    /// Se suelta (o con Esc, `cancel`: todo vuelve a su lugar).
    pub fn end(&mut self, host: &mut dyn Host, cancel: bool) {
        let Some(mut d) = self.drag.take() else { return };
        // Las capas se desvanecen (en Simpcky se cierran de golpe).
        self.fx.want = Want::default();
        let id = d.id;
        self.quiet = Some(id);
        let trash = !cancel && d.target == Target::Trash;
        if cancel || !d.moved || trash {
            self.replan(host, &mut d, Vec::new());
            for p in 0..d.piles.len() {
                for k in d.piles[p].ids.clone() {
                    if let Some(n) = d.by_id.get(&k).copied() {
                        self.animate(host, k, Some((n.x, n.y)), None, MOVE_MS);
                    }
                }
            }
            let o = d.orig;
            self.reshape(host, id, Some((o.w, o.h)), o.rolled);
            host.set_auto(id, o.auto);
            self.animate(host, id, Some((o.rect.left, o.rect.top)), None, MOVE_MS);
            if trash {
                host.trash(id);
            }
            return;
        }
        for (k, c) in d.plan.clone() {
            if let Some(n) = d.by_id.get(&k).copied() {
                let r = d.grid.rect(c);
                host.store(k, r.left, r.top, Some((r.right - r.left, if n.rolled { n.h } else { r.bottom - r.top })));
            }
        }
        match d.kind {
            Kind::PileMove => match (d.target, d.moving) {
                (Target::Pile(k, idx, _), _) => {
                    let m = d.merged(k, idx);
                    self.settle_pile(host, &m, &d.by_id, d.step);
                }
                (_, Some(p)) => {
                    let pile = d.piles[p].clone();
                    self.settle_pile(host, &pile, &d.by_id, d.step);
                }
                _ => {}
            },
            Kind::Move => {
                if let Some(p) = d.from {
                    let pile = d.piles[p].clone();
                    self.settle_pile(host, &pile, &d.by_id, d.step);
                }
                let from_pile = d.from.is_some();
                if from_pile {
                    host.set_auto(id, false);
                }
                match d.target {
                    Target::Cell(t, open) => {
                        let r = d.grid.rect(t);
                        let (w, h) = (r.right - r.left, r.bottom - r.top);
                        let rolled = !(open || from_pile) && host.state(id).2;
                        self.reshape(host, id, Some((w, h)), rolled);
                        host.store(id, r.left, r.top, Some((w, h)));
                        self.animate(host, id, Some((r.left, r.top)), None, MOVE_MS);
                    }
                    Target::Pile(p, i, side) => {
                        let mut pile = d.piles[p].clone();
                        pile.insert(i, id, side);
                        host.set_auto(id, true);
                        self.reshape(host, id, None, true);
                        self.settle_pile(host, &pile, &d.by_id, d.step);
                    }
                    Target::NewPile => {
                        let pile = d.new_pile(host);
                        host.set_auto(id, true);
                        self.reshape(host, id, None, true);
                        self.settle_pile(host, &pile, &d.by_id, d.step);
                    }
                    Target::None | Target::Trash => {
                        if from_pile {
                            self.reshape(host, id, None, false);
                        }
                        let wr = host.window_rect(id);
                        host.store(id, wr.left, wr.top, None);
                    }
                }
            }
        }
        host.save();
    }
}

impl Drag {
    fn max_w(&self, p: &Pile) -> i32 {
        p.ids.iter().filter_map(|k| self.by_id.get(k)).map(|n| n.w).max().unwrap_or(0)
    }

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

    fn panel_options(&self) -> Vec<RECT> {
        let (opt, gap, pad) = (px(self.dpi, 46), px(self.dpi, 7), px(self.dpi, 7));
        let total = pad * 2 + opt * 6 + gap * 7 + 2;
        let x0 = (self.area.left + self.area.right) / 2 - total / 2 + pad;
        let y0 = self.work.top + px(self.dpi, 8) + pad;
        (0..6)
            .map(|i| {
                let x = x0 + i * (opt + gap) + (i >= 4) as i32 * (gap + 1) + (i >= 5) as i32 * (gap + 1);
                RECT { left: x, top: y0, right: x + opt, bottom: y0 + opt }
            })
            .collect()
    }

    fn pile_under(&self, probe: &Pile, skip: Option<usize>) -> Option<Target> {
        let pb = probe.bounds(self.max_w(probe), self.step, self.bar);
        let (k, o) = self.piles.iter().enumerate().find(|(k, o)| Some(*k) != skip && !o.ids.is_empty() && !self.hidden(o) && overlaps(&pb, &o.bounds(self.max_w(o), self.step, self.bar)))?;
        Some(Target::Pile(k, o.slot(probe.top + self.bar / 2, self.step, self.bar), self.side_in(k, (pb.left + pb.right) / 2)))
    }

    fn side_in(&self, k: usize, cx: i32) -> bool {
        let w = self.by_id.get(&self.id).map(|n| n.w).unwrap_or(0);
        self.piles[k].side_at(cx, w, self.max_w(&self.piles[k]))
    }

    fn target_pile(&self) -> Option<usize> {
        match self.target {
            Target::Pile(k, _, _) => Some(k),
            _ => None,
        }
    }

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

    fn new_pile(&self, host: &dyn Host) -> Pile {
        let wr = host.window_rect(self.id);
        let right = (wr.left + wr.right) / 2 > (self.area.left + self.area.right) / 2;
        let edge = if right { self.grid.snap_right_edge(wr.right) } else { self.grid.snap_left_edge(wr.left) };
        Pile { right, edge, top: self.grid.snap_top(wr.top), ids: vec![self.id], sides: vec![right] }
    }

    fn holes(&self, host: &dyn Host) -> Vec<RECT> {
        let mut v = vec![host.window_rect(self.id)];
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
}
