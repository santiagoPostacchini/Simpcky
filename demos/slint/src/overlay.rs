use crate::{next_adapter, note::color, surface::Surface, Overlay, Shape};
use common::{snap::Layers, theme};
use slint::{ComponentHandle, ModelRc, VecModel};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
pub struct CellOverlay {
    ui: Overlay,
    surface: Surface,
    serial: u64,
}
fn shape(r: [f32; 4], radius: f32, fill: u32, stroke: u32, thickness: f32) -> Shape {
    Shape {
        x: r[0],
        y: r[1],
        w: (r[2] - r[0]).max(0.0),
        h: (r[3] - r[1]).max(0.0),
        radius,
        fill: color(fill, (fill >> 24) as u8),
        stroke: color(stroke, (stroke >> 24) as u8),
        thickness,
        ..Default::default()
    }
}
impl CellOverlay {
    pub fn new(l: &Layers) -> Self {
        let adapter = next_adapter();
        let ui = Overlay::new().unwrap();
        let w = l.work.right - l.work.left;
        let h = l.work.bottom - l.work.top;
        let surface = Surface::new(adapter, true, l.work.left, l.work.top, w, h);
        ui.show().unwrap();
        surface
            .adapter
            .set_size(slint::PhysicalSize::new(w as u32, h as u32));
        Self {
            ui,
            surface,
            serial: u64::MAX,
        }
    }
    pub fn update(&mut self, l: &Layers) {
        if !l.visible() {
            unsafe {
                ShowWindow(self.surface.hwnd, SW_HIDE);
            }
            return;
        }
        if self.serial == l.serial {
            return;
        }
        self.serial = l.serial;
        self.surface
            .resize(l.work.right - l.work.left, l.work.bottom - l.work.top);
        unsafe {
            SetWindowPos(
                self.surface.hwnd,
                HWND_TOPMOST,
                l.work.left,
                l.work.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        let c = theme::snap_colors(l.dark);
        let mut v = Vec::new();
        let local = |r: [i32; 4]| {
            [
                (r[0] - l.work.left) as f32,
                (r[1] - l.work.top) as f32,
                (r[2] - l.work.left) as f32,
                (r[3] - l.work.top) as f32,
            ]
        };
        let alpha = theme::with_alpha;
        if let Some(g) = &l.grid {
            for r in &g.cells {
                v.push(shape(local(*r), 6.0, 0, alpha(c.grid_line, l.grid_a), 1.5));
            }
            for r in &g.zones {
                let mut s = shape(
                    local(*r),
                    6.0,
                    alpha(c.zone_fill, l.grid_a),
                    alpha(c.accent, l.grid_a * 0.55),
                    1.5,
                );
                s.label = "SoltÃ¡ acÃ¡ para sumar a la pila".into();
                s.ink = color(c.ink, (255.0 * l.grid_a) as u8);
                v.push(s);
            }
        }
        if let Some(r) = l.preview {
            v.push(shape(
                [
                    r[0] - l.work.left as f32,
                    r[1] - l.work.top as f32,
                    r[2] - l.work.left as f32,
                    r[3] - l.work.top as f32,
                ],
                8.0,
                alpha(c.zone_fill, l.preview_a),
                alpha(c.accent, l.preview_a),
                2.0,
            ));
        }
        if let Some(r) = l.line {
            let r = local(r);
            v.push(shape(r, 4., alpha(c.zone_fill, l.line_a), 0, 0.));
            v.push(shape(
                [r[0] + 2., r[1] + 2., r[2] - 2., r[3] - 2.],
                2.,
                alpha(c.accent, l.line_a),
                0,
                0.,
            ));
        }
        if let (Some(first), Some(last)) = (l.panel.first(), l.panel.last()) {
            let slide = (1. - l.panel_a) * -12.;
            let panel = |r: [i32; 4]| {
                let mut r = local(r);
                r[1] += slide;
                r[3] += slide;
                r
            };
            let a = panel(*first);
            let b = panel(*last);
            let bg = [a[0] - 7., a[1] - 7., b[2] + 7., b[3] + 7.];
            for i in (0..16).rev() {
                let f = i as f32;
                v.push(shape(
                    [bg[0] - f, bg[1] - f + 8., bg[2] + f, bg[3] + f + 8.],
                    10. + f,
                    alpha(0x08000000, l.panel_a),
                    0,
                    0.,
                ));
            }
            v.push(shape(
                bg,
                10.,
                alpha(c.panel, l.panel_a),
                alpha(c.panel_line, l.panel_a),
                1.,
            ));
            for (i, r) in l.panel.iter().enumerate() {
                let r = panel(*r);
                let hot = l.hot == Some(i);
                let ink = if hot {
                    if i == 5 {
                        c.danger
                    } else {
                        c.accent
                    }
                } else {
                    c.option
                };
                let ink = alpha(ink, l.panel_a);
                if hot {
                    v.push(shape(
                        r,
                        6.,
                        alpha(
                            if i == 5 {
                                alpha(c.danger, 0.25)
                            } else {
                                c.zone_fill
                            },
                            l.panel_a,
                        ),
                        0,
                        0.,
                    ));
                }
                if i == 4 || i == 5 {
                    v.push(shape(
                        [r[0] - 4., r[1], r[0] - 3., r[3]],
                        0.,
                        alpha(c.panel_line, l.panel_a),
                        0,
                        0.,
                    ));
                }
                let (x, y) = (r[0] + 6., r[1] + 6.);
                if i < 4 {
                    let (w, h) = match i {
                        0 => (16., 16.),
                        1 => (34., 16.),
                        2 => (16., 34.),
                        _ => (34., 34.),
                    };
                    v.push(shape([x, y, x + w, y + h], 2., ink, 0, 0.));
                } else if i == 4 {
                    for (k, w) in [29., 23., 18.].into_iter().enumerate() {
                        let top = y + k as f32 * 11.;
                        v.push(shape(
                            [r[2] - 6. - w, top, r[2] - 6., top + 7.],
                            2.,
                            ink,
                            0,
                            0.,
                        ));
                    }
                } else {
                    v.push(shape([x + 7., y + 11., x + 27., y + 31.], 2., ink, 0, 0.));
                    v.push(shape([x + 4., y + 6., x + 30., y + 9.], 1., ink, 0, 0.));
                    v.push(shape([x + 13., y + 2., x + 21., y + 7.], 1., ink, 0, 0.));
                }
            }
        }
        v.push(shape(local(l.hint), 4., alpha(c.accent, l.hint_a), 0, 0.));
        self.ui.set_shapes(ModelRc::new(VecModel::from(v)));
        let holes = if l.grid_a > 0. {
            l.grid
                .as_ref()
                .map(|g| {
                    g.holes
                        .iter()
                        .map(|r| {
                            [
                                r[0] - l.work.left,
                                r[1] - l.work.top,
                                r[2] - l.work.left,
                                r[3] - l.work.top,
                            ]
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.surface.draw("capa", 0, &holes);
        unsafe {
            ShowWindow(self.surface.hwnd, SW_SHOWNOACTIVATE);
        }
    }
}
