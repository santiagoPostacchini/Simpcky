//! La capa de celdas: una ventana del tamaño del área de trabajo, siempre
//! encima y transparente a los clics, que dibuja `snap.layers()` (como
//! `snap::Fx` en Simpcky, que usa cinco ventanas en capas con GDI+).
//! Acá es una sola superficie de GPU que se repinta entera en cada cuadro
//! en que algo cambia.

use std::time::Instant;

use common::snap::Layers;
use common::theme::{snap_colors, with_alpha};
use common::win::wide;
use windows::Win32::Foundation::HWND as WHWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Gfx, Surface};
use crate::note::Fonts;

pub struct Overlay {
    pub hwnd: HWND,
    surf: Surface,
    work: RECT,
    serial: u64,
    shown: bool,
    dashes: ID2D1StrokeStyle,
}

const CLASS: &str = "SimpckyD2DCapa";

unsafe extern "system" fn proc_(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_NCHITTEST {
        return HTTRANSPARENT as LRESULT;
    }
    DefWindowProcW(h, m, w, l)
}

impl Drop for Overlay {
    /// La capa se rehace al cambiar de monitor: que no quede la ventana vieja.
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

impl Overlay {
    pub fn new(g: &Gfx, work: RECT) -> Overlay {
        unsafe {
            let class = wide(CLASS);
            let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: GetModuleHandleW(std::ptr::null()), lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
            let (w, h) = (work.right - work.left, work.bottom - work.top);
            let hwnd = CreateWindowExW(ex, class.as_ptr(), class.as_ptr(), WS_POPUP, work.left, work.top, w, h, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
            let surf = Surface::new(g, WHWND(hwnd as _), w as u32, h as u32).expect("capa");
            g.commit();
            let props = D2D1_STROKE_STYLE_PROPERTIES { dashStyle: D2D1_DASH_STYLE_DASH, ..Default::default() };
            let dashes = ID2D1Factory::CreateStrokeStyle(&g.d2d, &props, None).unwrap();
            Overlay { hwnd, surf, work, serial: u64::MAX, shown: false, dashes }
        }
    }

    pub fn work(&self) -> RECT {
        self.work
    }

    pub fn hide(&mut self) {
        if self.shown {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.shown = false;
        }
    }

    /// Dibuja si algo cambió y se asegura de que esté a la vista.
    pub fn update(&mut self, g: &Gfx, fonts: &Fonts, l: &Layers) {
        if l.serial != self.serial {
            self.serial = l.serial;
            let start = Instant::now();
            let o = (self.work.left as f32, self.work.top as f32);
            let dashes = self.dashes.clone();
            let _ = self.surf.draw(g, |dc| paint(dc, fonts, &dashes, l, o));
            common::framelog::mark("capa", 0, start.elapsed());
        }
        if !self.shown {
            unsafe { SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW) };
            self.shown = true;
        }
    }
}

fn rf(v: &[i32; 4], o: (f32, f32)) -> D2D_RECT_F {
    gfx::rectf(v[0] as f32 - o.0, v[1] as f32 - o.1, v[2] as f32 - o.0, v[3] as f32 - o.1)
}

fn paint(dc: &ID2D1DeviceContext, fonts: &Fonts, dashes: &ID2D1StrokeStyle, l: &Layers, o: (f32, f32)) {
    let c = snap_colors(l.dark);
    let f = |v: i32| (v * l.dpi / 96) as f32;
    unsafe {
        // Cuadrícula y zonas de las pilas, con un fundido; nada donde hay notas.
        if let Some(g) = l.grid.as_ref().filter(|_| l.grid_a > 0.0) {
            let a = l.grid_a;
            let line = gfx::brush(dc, gfx::argb(c.grid_line, a));
            for r in &g.cells {
                dc.DrawRoundedRectangle(&gfx::rounded(rf(r, o), f(6)), &line, f(1).max(1.5), dashes);
            }
            let fill = gfx::brush(dc, gfx::argb(c.zone_fill, a));
            let edge = gfx::brush(dc, gfx::argb(with_alpha(c.accent, 0.55), a));
            let ink = gfx::brush(dc, gfx::argb(c.ink, a));
            let text: Vec<u16> = "Soltá acá para sumar a la pila".encode_utf16().collect();
            for r in &g.zones {
                let rr = rf(r, o);
                dc.FillRoundedRectangle(&gfx::rounded(rr, f(8)), &fill);
                dc.DrawRoundedRectangle(&gfx::rounded(rr, f(8)), &edge, f(1).max(1.5), dashes);
                let t = gfx::rectf(rr.left + f(8), rr.top, rr.right - f(8), rr.bottom - f(8));
                dc.DrawText(&text, &fonts.label, &t, &ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }
            // Los huecos: se escribe "transparente" encima (copia, no mezcla).
            dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
            let clear = gfx::brush(dc, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 });
            for r in &g.holes {
                dc.FillRoundedRectangle(&gfx::rounded(rf(r, o), f(8)), &clear);
            }
            dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
        }

        // Vista previa: se desliza de celda en celda.
        if let Some(p) = l.preview.filter(|_| l.preview_a > 0.0) {
            let r = gfx::rectf(p[0] - o.0, p[1] - o.1, p[2] - o.0, p[3] - o.1);
            dc.FillRoundedRectangle(&gfx::rounded(r, f(8)), &gfx::brush(dc, gfx::argb(c.zone_fill, l.preview_a)));
            dc.DrawRoundedRectangle(&gfx::rounded(r, f(8)), &gfx::brush(dc, gfx::argb(c.accent, l.preview_a)), f(2), None);
        }

        // Línea de la pila: dónde entra la nota.
        if let Some(v) = l.line.filter(|_| l.line_a > 0.0) {
            let r = rf(&v, o);
            let ring = f(2).max(2.0);
            dc.FillRoundedRectangle(&gfx::rounded(r, f(4)), &gfx::brush(dc, gfx::argb(c.zone_fill, l.line_a)));
            let inner = gfx::rectf(r.left + ring, r.top + ring, r.right - ring, r.bottom - ring);
            dc.FillRoundedRectangle(&gfx::rounded(inner, f(2)), &gfx::brush(dc, gfx::argb(c.accent, l.line_a)));
        }

        // Panel: baja un poco mientras aparece.
        if !l.panel.is_empty() && l.panel_a > 0.0 {
            let a = l.panel_a;
            let slide = (1.0 - a) * f(-12);
            let pad = f(7);
            let (first, last) = (l.panel[0], l.panel[l.panel.len() - 1]);
            let bg = gfx::rectf(first[0] as f32 - pad - o.0, first[1] as f32 - pad - o.1 + slide, last[2] as f32 + pad - o.0, last[3] as f32 + pad - o.1 + slide);
            // Sombra: capas redondeadas cada vez más grandes y más tenues.
            let spread = (l.panel_margin() - 6 * l.dpi / 96) as f32;
            let mut s = 0.0;
            while s < spread {
                let k = 16.0 / 255.0 * (1.0 - s / spread) * a;
                let r = gfx::rectf(bg.left - s, bg.top - s + f(8), bg.right + s, bg.bottom + s + f(8));
                dc.FillRoundedRectangle(&gfx::rounded(r, f(10) + s), &gfx::brush(dc, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: k }));
                s += 1.0;
            }
            dc.FillRoundedRectangle(&gfx::rounded(bg, f(10)), &gfx::brush(dc, gfx::argb(c.panel, a)));
            dc.DrawRoundedRectangle(&gfx::rounded(bg, f(10)), &gfx::brush(dc, gfx::argb(c.panel_line, a)), 1.0, None);
            let opt = |i: usize| {
                let v = l.panel[i];
                gfx::rectf(v[0] as f32 - o.0, v[1] as f32 - o.1 + slide, v[2] as f32 - o.0, v[3] as f32 - o.1 + slide)
            };
            for k in [3, 4] {
                let sx = ((opt(k).right + opt(k + 1).left) / 2.0).round();
                dc.FillRectangle(&gfx::rectf(sx, opt(0).top, sx + 1.0, opt(0).bottom), &gfx::brush(dc, gfx::argb(c.panel_line, a)));
            }
            for i in 0..l.panel.len().min(6) {
                let r = opt(i);
                let is_hot = l.hot == Some(i);
                let (hot_ink, hot_fill) = if i == 5 { (c.danger, with_alpha(c.danger, 0.25)) } else { (c.accent, c.zone_fill) };
                if is_hot {
                    dc.FillRoundedRectangle(&gfx::rounded(r, f(6)), &gfx::brush(dc, gfx::argb(hot_fill, a)));
                }
                let ink = gfx::brush(dc, gfx::argb(if is_hot { hot_ink } else { c.option }, a));
                let (bw, bh) = (r.right - r.left, r.bottom - r.top);
                if i < 4 {
                    let (sa, sb) = [(1, 1), (2, 1), (1, 2), (2, 2)][i];
                    let (u, gp) = (100.0 / 2.3, 100.0 * 0.15 / 2.3);
                    let sw = (sa as f32 * u + (sa - 1) as f32 * gp) * 0.8 / 100.0;
                    let sh = (sb as f32 * u + (sb - 1) as f32 * gp) * 0.8 / 100.0;
                    let piece = gfx::rectf(r.left + bw * 0.12, r.top + bh * 0.12, r.left + bw * (0.12 + sw), r.top + bh * (0.12 + sh));
                    dc.FillRoundedRectangle(&gfx::rounded(piece, f(2)), &ink);
                } else if i == 5 {
                    // Tacho: manija, tapa y cuerpo un poco más angosto abajo.
                    let (cx, w) = (r.left + bw * 0.5, bw * 0.46);
                    dc.FillRoundedRectangle(&gfx::rounded(gfx::rectf(cx - w * 0.2, r.top + bh * 0.19, cx + w * 0.2, r.top + bh * 0.27), f(1)), &ink);
                    dc.FillRoundedRectangle(&gfx::rounded(gfx::rectf(cx - w * 0.62, r.top + bh * 0.26, cx + w * 0.62, r.top + bh * 0.32), f(1)), &ink);
                    let (top, bottom) = (r.top + bh * 0.36, r.top + bh * 0.8);
                    if let Ok(path) = dc.GetFactory().and_then(|fac| fac.CreatePathGeometry()) {
                        if let Ok(sink) = path.Open() {
                            sink.BeginFigure(windows_numerics::Vector2 { X: cx - w * 0.5, Y: top }, D2D1_FIGURE_BEGIN_FILLED);
                            sink.AddLine(windows_numerics::Vector2 { X: cx + w * 0.5, Y: top });
                            sink.AddLine(windows_numerics::Vector2 { X: cx + w * 0.5 - w * 0.08, Y: bottom });
                            sink.AddLine(windows_numerics::Vector2 { X: cx - w * 0.5 + w * 0.08, Y: bottom });
                            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                            let _ = sink.Close();
                            dc.FillGeometry(&path, &ink, None);
                        }
                    }
                } else {
                    for (k, frac) in [0.64f32, 0.5, 0.4].iter().enumerate() {
                        let top = r.top + bh * (0.16 + k as f32 * 0.25);
                        let right = r.right - bw * 0.12;
                        dc.FillRoundedRectangle(&gfx::rounded(gfx::rectf(right - bw * frac, top, right, top + bh * 0.17), f(2)), &ink);
                    }
                }
            }
        }

        // Guía arriba al centro, mientras se arrastra.
        if l.hint_a > 0.0 {
            let h = l.hint;
            let r = gfx::rectf(h[0] as f32 - o.0, h[1] as f32 - o.1 - (h[3] - h[1]) as f32, h[2] as f32 - o.0, h[3] as f32 - o.1);
            dc.FillRoundedRectangle(&gfx::rounded(r, f(4)), &gfx::brush(dc, gfx::argb(c.accent, l.hint_a)));
        }
    }
}
