//! Geometría de celdas y pilas, copiada tal cual de `src/snap.rs` de Simpcky
//! (con sus tests) por `demos/tools/extract_geom.py`, para que las demos se
//! comporten igual que la app. Solo cambia la visibilidad (`pub`).

#![allow(clippy::all)]

pub use windows_sys::Win32::Foundation::RECT;

/// Celda mínima y separación, a 100 %.
pub const CELL_W: i32 = 220;
pub const CELL_H: i32 = 180;
pub const GAP: i32 = 12;
/// Separación entre las barras de una pila, a 100 %.
pub const PILE_GAP: i32 = 10;
/// Lo más larga que puede quedar una nota de pila, en celdas.
pub const PILE_MAX_CELLS: i32 = 2;

/// Un rango de celdas, inclusivo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cells {
    pub c0: i32,
    pub c1: i32,
    pub r0: i32,
    pub r1: i32,
}

impl Cells {
    pub fn cols(&self) -> i32 {
        self.c1 - self.c0 + 1
    }
    pub fn rows(&self) -> i32 {
        self.r1 - self.r0 + 1
    }
    pub fn area(&self) -> i32 {
        self.cols() * self.rows()
    }
    pub fn meets(&self, o: &Cells) -> bool {
        self.c0 <= o.c1 && o.c0 <= self.c1 && self.r0 <= o.r1 && o.r0 <= self.r1
    }
    pub fn has(&self, c: i32, r: i32) -> bool {
        (self.c0..=self.c1).contains(&c) && (self.r0..=self.r1).contains(&r)
    }
}

/// La cuadrícula de un área de trabajo, en coordenadas de pantalla.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub left: i32,
    pub top: i32,
    pub cols: i32,
    pub rows: i32,
    pub cw: f64,
    pub ch: f64,
    pub gap: f64,
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

    pub fn sx(&self, c: i32) -> i32 {
        self.left + (self.gap + c as f64 * (self.cw + self.gap)).round() as i32
    }
    pub fn ex(&self, c: i32) -> i32 {
        self.left + (self.gap + c as f64 * (self.cw + self.gap) + self.cw).round() as i32
    }
    pub fn sy(&self, r: i32) -> i32 {
        self.top + (self.gap + r as f64 * (self.ch + self.gap)).round() as i32
    }
    pub fn ey(&self, r: i32) -> i32 {
        self.top + (self.gap + r as f64 * (self.ch + self.gap) + self.ch).round() as i32
    }

    pub fn rect(&self, k: Cells) -> RECT {
        RECT { left: self.sx(k.c0), top: self.sy(k.r0), right: self.ex(k.c1), bottom: self.ey(k.r1) }
    }

    pub fn col_of(&self, x: i32) -> i32 {
        (((x - self.left) as f64 - self.gap / 2.0) / (self.cw + self.gap)).floor().clamp(0.0, (self.cols - 1) as f64) as i32
    }
    pub fn row_of(&self, y: i32) -> i32 {
        (((y - self.top) as f64 - self.gap / 2.0) / (self.ch + self.gap)).floor().clamp(0.0, (self.rows - 1) as f64) as i32
    }
    /// Columna (o fila) cuyo comienzo está más cerca de `x` (o `y`).
    pub fn near_col(&self, x: i32, span: i32) -> i32 {
        (((x - self.left) as f64 - self.gap) / (self.cw + self.gap)).round().clamp(0.0, (self.cols - span).max(0) as f64) as i32
    }
    pub fn near_row(&self, y: i32, span: i32) -> i32 {
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
    pub fn width_for(&self, w: i32, max: i32) -> i32 {
        let n = ((w as f64 + self.gap) / (self.cw + self.gap)).round().clamp(1.0, max.min(self.cols) as f64);
        (n * self.cw + (n - 1.0) * self.gap).round() as i32
    }

    /// Un tamaño redondeado a celdas.
    pub fn shape_for(&self, w: i32, h: i32) -> (i32, i32) {
        let a = ((w as f64 + self.gap) / (self.cw + self.gap)).round() as i32;
        let b = ((h as f64 + self.gap) / (self.ch + self.gap)).round() as i32;
        (a.clamp(1, self.cols), b.clamp(1, self.rows))
    }

    pub fn snap_right_edge(&self, x: i32) -> i32 {
        let c = ((x - self.left) as f64 - self.gap - self.cw) / (self.cw + self.gap);
        self.ex(c.round().clamp(0.0, (self.cols - 1) as f64) as i32)
    }
    pub fn snap_left_edge(&self, x: i32) -> i32 {
        self.sx(self.near_col(x, 1))
    }
    pub fn snap_top(&self, y: i32) -> i32 {
        self.sy(self.near_row(y, 1))
    }
}

/// Cuántas celdas de al menos `min` (con `gap` entre ellas) en `len`. Sin
/// bordes que respetar, las que entran. Con bordes (relativos al comienzo),
/// la cantidad que deja más bordes justo entre dos celdas, y entre esas la
/// más cercana a lo que entra (se tolera achicar la celda hasta un 20 %).
pub fn fit_count(len: i32, min: i32, gap: i32, edges: &[i32]) -> i32 {
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
pub fn find_free(g: &Grid, a: i32, b: i32, near: Cells, obst: &[Cells]) -> Option<Cells> {
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

pub fn overlaps(a: &RECT, b: &RECT) -> bool {
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
    pub fn one(id: u32, right: bool, edge: i32, top: i32) -> Pile {
        Pile { right, edge, top, ids: vec![id], sides: vec![right] }
    }
    /// Los bordes de la columna, si la más ancha mide `max_w`.
    pub fn column(&self, max_w: i32) -> (i32, i32) {
        if self.right {
            (self.edge - max_w, self.edge)
        } else {
            (self.edge, self.edge + max_w)
        }
    }
    /// Esquina superior izquierda de la nota `i` (de ancho `w`).
    pub fn place(&self, i: usize, w: i32, max_w: i32, step: i32) -> (i32, i32) {
        let (l, r) = self.column(max_w);
        let right = self.sides.get(i).copied().unwrap_or(self.right);
        (if right { r - w } else { l }, self.top + i as i32 * step)
    }
    /// De qué lado de la columna va una nota de ancho `w` con centro en `cx`.
    pub fn side_at(&self, cx: i32, w: i32, max_w: i32) -> bool {
        let (l, r) = self.column(max_w.max(w));
        if w >= r - l {
            self.right
        } else {
            cx > (l + r) / 2
        }
    }
    pub fn insert(&mut self, i: usize, id: u32, right: bool) {
        let i = i.min(self.ids.len());
        self.ids.insert(i, id);
        self.sides.insert(i, right);
    }
    pub fn remove(&mut self, id: u32) {
        if let Some(i) = self.ids.iter().position(|k| *k == id) {
            self.ids.remove(i);
            self.sides.remove(i);
        }
    }
    /// En qué lugar entra una nota cuya barra está a la altura `cy`.
    pub fn slot(&self, cy: i32, step: i32, bar: i32) -> usize {
        (0..self.ids.len()).filter(|&i| self.top + i as i32 * step + bar / 2 < cy).count()
    }
    /// Lo que ocupa, con el ancho de la nota más ancha.
    pub fn bounds(&self, max_w: i32, step: i32, bar: i32) -> RECT {
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


/// Un rectángulo como arreglo (izquierda, arriba, derecha, abajo).
pub fn q(r: RECT) -> [i32; 4] {
    [r.left, r.top, r.right, r.bottom]
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
