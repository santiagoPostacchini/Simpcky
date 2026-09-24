//! Notas translúcidas con Windhawk: el mod "Translucent Windows" (de
//! Undisputed00x) pone el desenfoque de Windows detrás de las notas, igual
//! que en el resto de las aplicaciones.
//!
//! El mod solo toca ventanas de primer nivel con estilo de ventana "de
//! verdad" (`WS_POPUPWINDOW`, entre otros), y cuando se crean. Una nota
//! anclada es hija del escritorio: nunca le llegaría. Por eso, con esto
//! prendido y el mod activo, las notas del escritorio son ventanas de
//! primer nivel **poseídas** por el escritorio (`desktop::own`): Windows las
//! mantiene justo encima de él —detrás de las aplicaciones, visibles con
//! "Mostrar escritorio"— y el mod las ve como a cualquier otra.
//!
//! Y se pintan con transparencia de verdad: el color de la nota con alfa,
//! premultiplicado, en un DIB de 32 bits que se copia tal cual (`Canvas`).
//! GDI a secas escribe alfa 0 y el compositor muestra ese color aclarado,
//! como si fuera luz; el negro sí queda transparente. El texto GDI lo
//! recompone el propio mod, con su alfa.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::System::ProcessStatus::{EnumProcessModules, GetModuleBaseNameW};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;

/// Notas en modo vidrio ahora mismo (elegido, y el mod está cargado).
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// ¿Está el mod "Translucent Windows" cargado en este proceso? Windhawk
/// inyecta cada mod como una DLL que se llama como él.
pub fn mod_loaded() -> bool {
    unsafe {
        let process = GetCurrentProcess();
        let mut modules: Vec<HMODULE> = vec![null_mut(); 1024];
        let mut needed = 0u32;
        let size = (modules.len() * std::mem::size_of::<HMODULE>()) as u32;
        if EnumProcessModules(process, modules.as_mut_ptr(), size, &mut needed) == 0 {
            return false;
        }
        let n = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
        modules[..n].iter().any(|&m| {
            let mut buf = [0u16; 260];
            let len = GetModuleBaseNameW(process, m, buf.as_mut_ptr(), buf.len() as u32) as usize;
            String::from_utf16_lossy(&buf[..len.min(buf.len())]).to_ascii_lowercase().starts_with("translucent-windows")
        })
    }
}

/// Vuelve a mirar si corresponde el modo vidrio (el nivel elegido y si el
/// mod está). `true` si cambió: hay que rehacer las ventanas de las notas.
pub fn update() -> bool {
    let wanted = crate::app::app().lock().unwrap().settings.translucency > 0;
    let now = wanted && mod_loaded();
    ACTIVE.swap(now, Ordering::Relaxed) != now
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Opacidad del color de la nota (encabezado, cuerpo) sobre el
/// desenfoque, o `None` si las notas van lisas.
pub fn opacity() -> Option<(u8, u8)> {
    if !active() {
        return None;
    }
    match crate::app::app().lock().unwrap().settings.translucency {
        1 => Some((0x70, 0x30)), // suave
        3 => Some((0xD0, 0xA0)), // fuerte
        _ => Some((0xA0, 0x60)), // media
    }
}

/// Opacidad del título de la nota en modo vidrio: como el resto, deja
/// ver un poco el desenfoque (más cuanto más suave el nivel).
pub fn title_alpha() -> u8 {
    match crate::app::app().lock().unwrap().settings.translucency {
        1 => 0x99, // suave
        3 => 0xDD, // fuerte
        _ => 0xBB, // media
    }
}

/// Un píxel BGRA premultiplicado a partir de un color GDI (0x00BBGGRR).
fn premultiplied(color: u32, alpha: u8) -> u32 {
    let a = alpha as u32;
    let r = (color & 0xff) * a / 255;
    let g = ((color >> 8) & 0xff) * a / 255;
    let b = ((color >> 16) & 0xff) * a / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Un lienzo de 32 bits con alfa premultiplicado: se pinta acá (a mano,
/// con GDI+ o con texto GDI, que el mod recompone) y se copia a la ventana
/// con alfa y todo.
pub struct Canvas {
    pub dc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Option<Canvas> {
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h; // de arriba hacia abajo
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            let mut bits: *mut std::ffi::c_void = null_mut();
            let dib = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if dib.is_null() || bits.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(null_mut());
            let old = SelectObject(dc, dib);
            Some(Canvas { dc, dib, old, bits: bits as *mut u32, w, h })
        }
    }

    /// `r` (coordenadas del lienzo) con `color` a opacidad `alpha`.
    pub fn fill(&self, r: RECT, color: u32, alpha: u8) {
        let px = premultiplied(color, alpha);
        let (l, t) = (r.left.max(0), r.top.max(0));
        let (rr, b) = (r.right.min(self.w), r.bottom.min(self.h));
        unsafe {
            GdiFlush();
            for y in t..b {
                let row = std::slice::from_raw_parts_mut(self.bits.add((y * self.w) as usize), self.w as usize);
                row[l as usize..rr.max(l) as usize].fill(px);
            }
        }
    }

    /// Dibujar con GDI+ sobre el lienzo, con el alfa bien (sobre un
    /// bitmap PARGB que mira los mismos píxeles).
    pub fn with_graphics(&self, f: impl FnOnce(*mut GpGraphics)) {
        unsafe {
            GdiFlush();
            let mut bmp: *mut GpBitmap = null_mut();
            if GdipCreateBitmapFromScan0(self.w, self.h, self.w * 4, PIXEL_FORMAT_32BPP_PARGB, self.bits as *const u8, &mut bmp) != Ok
                || bmp.is_null()
            {
                return;
            }
            let mut g: *mut GpGraphics = null_mut();
            if GdipGetImageGraphicsContext(bmp as *mut GpImage, &mut g) == Ok && !g.is_null() {
                GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
                f(g);
                GdipDeleteGraphics(g);
            }
            GdipDisposeImage(bmp as *mut GpImage);
        }
    }

    /// Texto GDI (`DrawTextW` con `format`) en `r`, en `color`, con su
    /// alfa: se dibuja blanco sobre negro en un lienzo aparte y el brillo
    /// de cada píxel es la opacidad con la que se compone el color. No
    /// depende de que el mod recomponga el texto.
    pub fn text(&self, r: RECT, text: &str, font: HFONT, color: u32, format: u32) {
        self.text_alpha(r, text, font, color, 255, format);
    }

    /// `text`, con el texto a opacidad `alpha`.
    pub fn text_alpha(&self, r: RECT, text: &str, font: HFONT, color: u32, alpha: u8, format: u32) {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let Some(mask) = Canvas::new(w, h) else { return };
        let full = RECT { left: 0, top: 0, right: w, bottom: h };
        mask.fill(full, 0, 255);
        let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let old = SelectObject(mask.dc, font);
            SetBkMode(mask.dc, TRANSPARENT as i32);
            SetTextColor(mask.dc, 0x00ff_ffff);
            let mut rc = full;
            DrawTextW(mask.dc, t.as_ptr(), -1, &mut rc, format);
            SelectObject(mask.dc, old);
            GdiFlush();
            let (cr, cg, cb) = (color & 0xff, (color >> 8) & 0xff, (color >> 16) & 0xff);
            for y in 0..h {
                let dy = r.top + y;
                if dy < 0 || dy >= self.h {
                    continue;
                }
                for x in 0..w {
                    let dx = r.left + x;
                    if dx < 0 || dx >= self.w {
                        continue;
                    }
                    let m = *mask.bits.add((y * w + x) as usize);
                    let cov = ((m >> 16) & 0xff).max((m >> 8) & 0xff).max(m & 0xff) * alpha as u32 / 255;
                    if cov == 0 {
                        continue;
                    }
                    let d = self.bits.add((dy * self.w + dx) as usize);
                    let keep = 255 - cov;
                    let mix = |src: u32, sh: u32| src * cov / 255 + ((*d >> sh) & 0xff) * keep / 255;
                    *d = (mix(255, 24) << 24) | (mix(cr, 16) << 16) | (mix(cg, 8) << 8) | mix(cb, 0);
                }
            }
        }
    }

    /// Copia el lienzo a `hdc` en (`x`, `y`), alfa incluido.
    pub fn blit(&self, hdc: HDC, x: i32, y: i32) {
        unsafe {
            GdiFlush();
            BitBlt(hdc, x, y, self.w, self.h, self.dc, 0, 0, SRCCOPY);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteDC(self.dc);
            DeleteObject(self.dib);
        }
    }
}

/// `r` de `hdc` con `color` a opacidad `alpha`, de verdad translúcido.
pub fn fill(hdc: HDC, r: RECT, color: u32, alpha: u8) {
    if let Some(c) = Canvas::new(r.right - r.left, r.bottom - r.top) {
        c.fill(RECT { left: 0, top: 0, right: r.right - r.left, bottom: r.bottom - r.top }, color, alpha);
        c.blit(hdc, r.left, r.top);
    }
}

/// ARGB (lo que espera GDI+) de un color GDI con opacidad.
pub fn argb(color: u32, alpha: u8) -> u32 {
    (crate::note::colorref_to_argb(color) & 0x00ff_ffff) | ((alpha as u32) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplies() {
        // 0x00BBGGRR amarillo oscuro, a 60 %
        assert_eq!(premultiplied(0x0014_556B, 0x99), 0x9940_330C);
        assert_eq!(premultiplied(0x00ff_ffff, 0), 0);
        assert_eq!(premultiplied(0x0000_00ff, 255), 0xffff_0000);
    }
}
