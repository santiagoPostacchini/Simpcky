//! Transiciones de las notas, como `src/anim.rs` de Simpcky: posición y
//! tamaño por separado, ~200 ms con salida suave (cúbica). Sin ventanas:
//! dice dónde va cada nota en cada cuadro y el que la dibuja la mueve.

use std::collections::HashMap;
use std::time::Instant;

use windows_sys::Win32::Foundation::RECT;

pub const MOVE_MS: u32 = 200;
pub const ROLL_MS: u32 = 170;

#[derive(Clone, Copy, Debug)]
struct Part {
    from: (i32, i32),
    to: (i32, i32),
    start: Instant,
    ms: u32,
}

impl Part {
    fn at(&self, now: Instant) -> ((i32, i32), bool) {
        let t = (now.duration_since(self.start).as_secs_f32() * 1000.0 / self.ms as f32).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - t).powi(3);
        let lerp = |a: i32, b: i32| a + ((b - a) as f32 * e).round() as i32;
        ((lerp(self.from.0, self.to.0), lerp(self.from.1, self.to.1)), t >= 1.0)
    }
}

#[derive(Default, Clone, Copy, Debug)]
struct Tween {
    pos: Option<Part>,
    size: Option<Part>,
}

/// Un paso: (nota, posición, tamaño, terminó).
pub type Step = (u32, Option<(i32, i32)>, Option<(i32, i32)>, bool);

#[derive(Default)]
pub struct Motion {
    tweens: HashMap<u32, Tween>,
    /// Con los efectos de animación de Windows apagados, todo es inmediato.
    pub instant: bool,
}

impl Motion {
    pub fn new() -> Motion {
        Motion { tweens: HashMap::new(), instant: !crate::win::animations_enabled() }
    }

    /// Lleva la nota de `cur` a `pos`/`size`. Devuelve lo que hay que
    /// aplicar ya (lo que no se anima).
    pub fn set(&mut self, id: u32, cur: RECT, pos: Option<(i32, i32)>, size: Option<(i32, i32)>, ms: u32) -> Step {
        let (cur_pos, cur_size) = ((cur.left, cur.top), (cur.right - cur.left, cur.bottom - cur.top));
        let now = Instant::now();
        let instant = ms == 0 || self.instant;
        let tw = self.tweens.entry(id).or_default();
        let mut now_pos = None;
        let mut now_size = None;
        if let Some(p) = pos {
            if p != cur_pos && !instant {
                // Si ya iba hacia ahí, que siga (no se reinicia el tiempo).
                if tw.pos.map(|q| q.to) != Some(p) {
                    tw.pos = Some(Part { from: cur_pos, to: p, start: now, ms });
                }
            } else {
                tw.pos = None;
                now_pos = (p != cur_pos).then_some(p);
            }
        }
        if let Some(s) = size {
            if s != cur_size && !instant {
                if tw.size.map(|q| q.to) != Some(s) {
                    tw.size = Some(Part { from: cur_size, to: s, start: now, ms });
                }
            } else {
                tw.size = None;
                now_size = (s != cur_size).then_some(s);
            }
        }
        if tw.pos.is_none() && tw.size.is_none() {
            self.tweens.remove(&id);
        }
        (id, now_pos, now_size, true)
    }

    /// El arrastre toma la nota: deja de ir hacia donde iba (el tamaño sigue).
    pub fn stop_pos(&mut self, id: u32) {
        if let Some(tw) = self.tweens.get_mut(&id) {
            tw.pos = None;
        }
    }

    pub fn sizing(&self, id: u32) -> bool {
        self.tweens.get(&id).is_some_and(|t| t.size.is_some())
    }

    pub fn busy(&self) -> bool {
        !self.tweens.is_empty()
    }

    /// Dónde va cada nota que se mueve, ahora.
    pub fn tick(&mut self, now: Instant) -> Vec<Step> {
        let mut out = Vec::new();
        self.tweens.retain(|&id, tw| {
            let mut pos = None;
            if let Some(p) = tw.pos {
                let (v, end) = p.at(now);
                pos = Some(v);
                if end {
                    tw.pos = None;
                }
            }
            let mut size = None;
            if let Some(s) = tw.size {
                let (v, end) = s.at(now);
                size = Some(v);
                if end {
                    tw.size = None;
                }
            }
            let over = tw.pos.is_none() && tw.size.is_none();
            out.push((id, pos, size, over));
            !over
        });
        out
    }
}
