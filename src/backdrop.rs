//! Fondo translúcido de las notas del escritorio: el fondo de pantalla,
//! desenfocado, con el color de la nota encima — como el acrílico de
//! Windows 11 o el mod "Translucent Windows" de Windhawk.
//!
//! El efecto de verdad lo hace el compositor de Windows (DWM), y solo en
//! ventanas de primer nivel. Una nota anclada es hija del escritorio (así
//! sobrevive a "Mostrar escritorio", ver `desktop.rs`): ni el mod ni
//! Windows pueden desenfocar lo que tiene detrás. Pero lo que tiene detrás
//! es el fondo de pantalla, que no se mueve: se lo desenfoca una vez, en
//! chico (1/`SHRINK`), y cada nota pinta su pedazo agrandado. La única
//! diferencia con el efecto real: los íconos del escritorio que queden
//! detrás de una nota no se ven a través.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::wide;

const CLSID_DESKTOP_WALLPAPER: GUID = GUID::from_u128(0xC2CF3110_460E_4fc1_B9D0_8A1C0C9CC4BD);
const IID_IDESKTOP_WALLPAPER: GUID = GUID::from_u128(0xB92B56A9_8B55_4E14_9A89_0199BBB6F93B);

/// Cuánto más chico se arma el fondo desenfocado (y cuánto se agranda
/// al pintarlo): el desenfoque sale casi gratis y la memoria es mínima.
const SHRINK: i32 = 6;
/// Tres pasadas de desenfoque de caja de radio 4, en chico: parecido a un
/// desenfoque gaussiano de unos 27 píxeles de pantalla.
const BLUR_RADIUS: i32 = 4;
const BLUR_PASSES: usize = 3;
/// Cuánto de más se lee alrededor al pintar un pedazo (en píxeles del
/// fondo chico): así dos pedazos vecinos pintados por separado empalman
/// sin costura.
const PAD: f32 = 2.0;

const PIXEL_FORMAT_32BPP_RGB: i32 = 0x0002_2009;

// Posiciones de IDesktopWallpaper (DESKTOP_WALLPAPER_POSITION).
const POS_CENTER: i32 = 0;
const POS_TILE: i32 = 1;
const POS_STRETCH: i32 = 2;
const POS_FIT: i32 = 3;
const POS_FILL: i32 = 4;
const POS_SPAN: i32 = 5;

/// El fondo desenfocado de un monitor.
struct Layer {
    /// El monitor, en coordenadas de pantalla.
    rect: RECT,
    w: i32,
    h: i32,
    dib: HBITMAP,
    /// GDI+ mirando los mismos píxeles del DIB (sin copiarlos).
    image: *mut GpBitmap,
}

thread_local! {
    // `None`: todavía no se armó (o hay que rehacerlo). `Some(vacío)`: no
    // hay fondo de pantalla que usar, y las notas se pintan lisas.
    static CACHE: RefCell<Option<Vec<Layer>>> = const { RefCell::new(None) };
    // Armándolo: pedirle el fondo a Windows es una llamada COM a Explorer,
    // que mientras espera atiende mensajes — y una nota puede querer
    // pintarse justo ahí.
    static BUILDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // Alguna nota se pintó lisa mientras se armaba: al terminar, se repintan.
    static REFUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Arma el fondo desenfocado si hace falta. `false` si se está armando
/// (una llamada que entró mientras tanto): esa vez se pinta liso.
pub fn warm() -> bool {
    if CACHE.with(|c| c.borrow().is_some()) {
        return true;
    }
    if BUILDING.with(|b| b.replace(true)) {
        REFUSED.with(|r| r.set(true));
        return false;
    }
    let layers = unsafe { build() };
    BUILDING.with(|b| b.set(false));
    CACHE.with(|c| *c.borrow_mut() = Some(layers));
    if REFUSED.with(|r| r.replace(false)) {
        crate::tray::post_backdrop_ready();
    }
    true
}

/// Cambió el fondo de pantalla o los monitores: se rearma la próxima vez.
pub fn invalidate() {
    CACHE.with(|c| {
        if let Some(layers) = c.borrow_mut().take() {
            for l in layers {
                unsafe {
                    GdipDisposeImage(l.image as *mut GpImage);
                    DeleteObject(l.dib);
                }
            }
        }
    });
}

struct Wall {
    monitor: RECT,
    path: String,
}

unsafe fn method<F: Copy>(obj: *mut c_void, index: usize) -> F {
    let vtbl = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(index))
}

unsafe fn take_co_string(p: *mut u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
    CoTaskMemFree(p as *const c_void);
    s
}

/// Qué fondo tiene cada monitor, cómo está ubicado y el color de fondo
/// (lo que se ve alrededor de una imagen "ajustada" o "centrada").
fn wallpapers() -> (Vec<Wall>, i32, u32) {
    let mut walls = Vec::new();
    let mut position = POS_FILL;
    let mut background = unsafe { GetSysColor(COLOR_DESKTOP) };
    unsafe {
        CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let mut dw: *mut c_void = null_mut();
        if CoCreateInstance(&CLSID_DESKTOP_WALLPAPER, null_mut(), CLSCTX_ALL, &IID_IDESKTOP_WALLPAPER, &mut dw) >= 0 && !dw.is_null() {
            // IDesktopWallpaper (shobjidl_core.h), por su lugar en la tabla.
            let get_wallpaper: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut u16) -> i32 = method(dw, 4);
            let path_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32 = method(dw, 5);
            let count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = method(dw, 6);
            let monitor_rect: unsafe extern "system" fn(*mut c_void, *const u16, *mut RECT) -> i32 = method(dw, 7);
            let get_background: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = method(dw, 9);
            let get_position: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = method(dw, 11);
            let mut n = 0u32;
            count(dw, &mut n);
            for i in 0..n {
                let mut id: *mut u16 = null_mut();
                if path_at(dw, i, &mut id) < 0 || id.is_null() {
                    continue;
                }
                let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                let mut path: *mut u16 = null_mut();
                let ok = monitor_rect(dw, id, &mut rc) >= 0;
                get_wallpaper(dw, id, &mut path);
                CoTaskMemFree(id as *const c_void);
                let path = take_co_string(path);
                if ok && rc.right > rc.left && rc.bottom > rc.top {
                    walls.push(Wall { monitor: rc, path });
                }
            }
            get_position(dw, &mut position);
            get_background(dw, &mut background);
            let release: unsafe extern "system" fn(*mut c_void) -> u32 = method(dw, 2);
            release(dw);
        }
    }
    if walls.is_empty() {
        // Sin IDesktopWallpaper: el fondo de siempre, en el monitor
        // principal, ubicado según el registro.
        let mut buf = [0u16; 260];
        unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr() as *mut c_void, 0) };
        let path = crate::win::from_wide(&buf);
        let rc = unsafe {
            RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
        };
        walls.push(Wall { monitor: rc, path });
        position = match (registry_desktop("WallpaperStyle").as_str(), registry_desktop("TileWallpaper").as_str()) {
            (_, "1") => POS_TILE,
            ("2", _) => POS_STRETCH,
            ("6", _) => POS_FIT,
            ("10", _) => POS_FILL,
            ("22", _) => POS_SPAN,
            _ => POS_CENTER,
        };
    }
    // Presentación de diapositivas o Spotlight: la imagen de ahora está
    // en la copia que arma Windows.
    let transcoded = std::env::var("APPDATA").map(|a| format!("{a}\\Microsoft\\Windows\\Themes\\TranscodedWallpaper")).unwrap_or_default();
    for w in walls.iter_mut() {
        if w.path.is_empty() || !std::path::Path::new(&w.path).exists() {
            w.path = transcoded.clone();
        }
    }
    (walls, position, background)
}

fn registry_desktop(name: &str) -> String {
    let key = wide("Control Panel\\Desktop");
    let value = wide(name);
    let mut buf = [0u16; 64];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr() as *mut c_void, &mut size)
    };
    if ok == 0 {
        crate::win::from_wide(&buf)
    } else {
        String::new()
    }
}

/// Dónde queda la imagen (ancho `iw`, alto `ih`) dentro de `base`, según
/// cómo la ubica Windows. `None` para "en mosaico".
fn placement(base: RECT, iw: f32, ih: f32, position: i32) -> Option<(f32, f32, f32, f32)> {
    let (bx, by) = (base.left as f32, base.top as f32);
    let (bw, bh) = ((base.right - base.left) as f32, (base.bottom - base.top) as f32);
    let centered = |w: f32, h: f32| Some((bx + (bw - w) / 2.0, by + (bh - h) / 2.0, w, h));
    match position {
        POS_TILE => None,
        POS_STRETCH => Some((bx, by, bw, bh)),
        POS_CENTER => centered(iw, ih),
        POS_FIT => {
            let s = (bw / iw).min(bh / ih);
            centered(iw * s, ih * s)
        }
        _ => {
            // Rellenar (y "extender", sobre todos los monitores juntos).
            let s = (bw / iw).max(bh / ih);
            centered(iw * s, ih * s)
        }
    }
}

unsafe fn build() -> Vec<Layer> {
    let (walls, position, background) = wallpapers();
    // "Extender": una sola imagen sobre el rectángulo de todos los monitores.
    let span = walls.iter().fold(RECT { left: i32::MAX, top: i32::MAX, right: i32::MIN, bottom: i32::MIN }, |a, w| RECT {
        left: a.left.min(w.monitor.left),
        top: a.top.min(w.monitor.top),
        right: a.right.max(w.monitor.right),
        bottom: a.bottom.max(w.monitor.bottom),
    });
    let mut layers = Vec::new();
    for wall in &walls {
        let m = wall.monitor;
        let w = ((m.right - m.left) + SHRINK - 1) / SHRINK;
        let h = ((m.bottom - m.top) + SHRINK - 1) / SHRINK;
        let screen = GetDC(null_mut());
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // de arriba hacia abajo
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut c_void = null_mut();
        let dib = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        ReleaseDC(null_mut(), screen);
        if dib.is_null() || bits.is_null() {
            continue;
        }
        let mem = CreateCompatibleDC(null_mut());
        let old = SelectObject(mem, dib);
        let brush = CreateSolidBrush(background);
        FillRect(mem, &RECT { left: 0, top: 0, right: w, bottom: h }, brush);
        DeleteObject(brush);

        let path = wide(&wall.path);
        let mut img: *mut GpBitmap = null_mut();
        if !wall.path.is_empty() && GdipCreateBitmapFromFile(path.as_ptr(), &mut img) == Ok && !img.is_null() {
            let (mut iw, mut ih) = (0u32, 0u32);
            GdipGetImageWidth(img as *mut GpImage, &mut iw);
            GdipGetImageHeight(img as *mut GpImage, &mut ih);
            let mut g: *mut GpGraphics = null_mut();
            if iw > 0 && ih > 0 && GdipCreateFromHDC(mem, &mut g) == Ok && !g.is_null() {
                GdipSetInterpolationMode(g, InterpolationModeHighQualityBicubic);
                GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
                let base = if position == POS_SPAN { span } else { m };
                let k = 1.0 / SHRINK as f32;
                let (ox, oy) = (m.left as f32, m.top as f32);
                match placement(base, iw as f32, ih as f32, position) {
                    Some((x, y, dw, dh)) => {
                        GdipDrawImageRectRect(
                            g,
                            img as *mut GpImage,
                            (x - ox) * k,
                            (y - oy) * k,
                            dw * k,
                            dh * k,
                            0.0,
                            0.0,
                            iw as f32,
                            ih as f32,
                            UnitPixel,
                            null(),
                            0,
                            null_mut(),
                        );
                    }
                    None => {
                        let mut ty = base.top as f32;
                        while ty < m.bottom as f32 {
                            let mut tx = base.left as f32;
                            while tx < m.right as f32 {
                                GdipDrawImageRectRect(
                                    g,
                                    img as *mut GpImage,
                                    (tx - ox) * k,
                                    (ty - oy) * k,
                                    iw as f32 * k,
                                    ih as f32 * k,
                                    0.0,
                                    0.0,
                                    iw as f32,
                                    ih as f32,
                                    UnitPixel,
                                    null(),
                                    0,
                                    null_mut(),
                                );
                                tx += iw as f32;
                            }
                            ty += ih as f32;
                        }
                    }
                }
                GdipDeleteGraphics(g);
            }
            // La imagen entera (un 4K son 33 MB) no se guarda.
            GdipDisposeImage(img as *mut GpImage);
        }
        SelectObject(mem, old);
        DeleteDC(mem);
        GdiFlush();

        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
        blur(px, w as usize, h as usize);

        let mut image: *mut GpBitmap = null_mut();
        GdipCreateBitmapFromScan0(w, h, w * 4, PIXEL_FORMAT_32BPP_RGB, bits as *const u8, &mut image);
        if image.is_null() {
            DeleteObject(dib);
            continue;
        }
        layers.push(Layer { rect: m, w, h, dib, image });
    }
    layers
}

/// Desenfoque de caja, separable, `BLUR_PASSES` veces: con tres pasadas
/// ya se parece a uno gaussiano.
fn blur(px: &mut [u32], w: usize, h: usize) {
    let mut tmp = vec![0u32; px.len()];
    for _ in 0..BLUR_PASSES {
        box_pass(px, &mut tmp, w, h, 1, w); // horizontal
        box_pass(&tmp, px, h, w, w, 1); // vertical (se intercambian los ejes)
    }
}

/// Una pasada en una dirección: `len` píxeles por línea (separados por
/// `step`), `lines` líneas (separadas por `stride`), con los bordes
/// estirados.
fn box_pass(src: &[u32], dst: &mut [u32], len: usize, lines: usize, step: usize, stride: usize) {
    let r = BLUR_RADIUS;
    let n = (2 * r + 1) as u32;
    for line in 0..lines {
        let base = line * stride;
        let at = |i: i32| src[base + (i.clamp(0, len as i32 - 1) as usize) * step];
        let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
        for i in -r..=r {
            let c = at(i);
            sr += (c >> 16) & 0xff;
            sg += (c >> 8) & 0xff;
            sb += c & 0xff;
        }
        for i in 0..len as i32 {
            dst[base + i as usize * step] = ((sr / n) << 16) | ((sg / n) << 8) | (sb / n);
            let (add, sub) = (at(i + r + 1), at(i - r));
            sr = sr + ((add >> 16) & 0xff) - ((sub >> 16) & 0xff);
            sg = sg + ((add >> 8) & 0xff) - ((sub >> 8) & 0xff);
            sb = sb + (add & 0xff) - (sub & 0xff);
        }
    }
}

/// Pinta en `dest` (coordenadas de `hdc`) el fondo desenfocado que queda
/// detrás de `screen` (el mismo rectángulo, en la pantalla), con el color
/// `tint` encima a opacidad `alpha` (0–255). `false` si no hay fondo de
/// pantalla con qué hacerlo (y entonces hay que pintar liso).
pub fn paint(hdc: HDC, dest: RECT, screen: RECT, tint: u32, alpha: u8) -> bool {
    if dest.right <= dest.left || dest.bottom <= dest.top {
        return true;
    }
    if !warm() {
        return false;
    }
    CACHE.with(|c| unsafe {
        let c = c.borrow();
        let Some(layers) = c.as_ref() else { return false };
        if layers.is_empty() {
            return false;
        }
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(hdc, &mut g) != Ok || g.is_null() {
            return false;
        }
        // Lo que no caiga sobre ningún monitor (no debería), liso.
        let mut solid: *mut GpSolidFill = null_mut();
        GdipCreateSolidFill(crate::note::colorref_to_argb(tint), &mut solid);
        GdipFillRectangleI(g, solid as *mut GpBrush, dest.left, dest.top, dest.right - dest.left, dest.bottom - dest.top);
        GdipDeleteBrush(solid as *mut GpBrush);

        GdipSetInterpolationMode(g, InterpolationModeHighQualityBilinear);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        GdipSetClipRectI(g, dest.left, dest.top, dest.right - dest.left, dest.bottom - dest.top, CombineModeIntersect);
        let mut attrs: *mut GpImageAttributes = null_mut();
        GdipCreateImageAttributes(&mut attrs);
        GdipSetImageAttributesWrapMode(attrs, WrapModeTileFlipXY, 0, 0);
        for l in layers {
            let inter = RECT {
                left: screen.left.max(l.rect.left),
                top: screen.top.max(l.rect.top),
                right: screen.right.min(l.rect.right),
                bottom: screen.bottom.min(l.rect.bottom),
            };
            if inter.right <= inter.left || inter.bottom <= inter.top {
                continue;
            }
            // Escala real de este monitor (el fondo chico se redondea).
            let kx = l.w as f32 / (l.rect.right - l.rect.left) as f32;
            let ky = l.h as f32 / (l.rect.bottom - l.rect.top) as f32;
            let sx = (inter.left - l.rect.left) as f32 * kx - PAD;
            let sy = (inter.top - l.rect.top) as f32 * ky - PAD;
            let sw = (inter.right - inter.left) as f32 * kx + PAD * 2.0;
            let sh = (inter.bottom - inter.top) as f32 * ky + PAD * 2.0;
            let dx = (dest.left + inter.left - screen.left) as f32 - PAD / kx;
            let dy = (dest.top + inter.top - screen.top) as f32 - PAD / ky;
            GdipDrawImageRectRect(g, l.image as *mut GpImage, dx, dy, sw / kx, sh / ky, sx, sy, sw, sh, UnitPixel, attrs, 0, null_mut());
        }
        GdipDisposeImageAttributes(attrs);
        let mut veil: *mut GpSolidFill = null_mut();
        let argb = (crate::note::colorref_to_argb(tint) & 0x00ff_ffff) | ((alpha as u32) << 24);
        GdipCreateSolidFill(argb, &mut veil);
        GdipFillRectangleI(g, veil as *mut GpBrush, dest.left, dest.top, dest.right - dest.left, dest.bottom - dest.top);
        GdipDeleteBrush(veil as *mut GpBrush);
        GdipDeleteGraphics(g);
        true
    })
}

/// Opacidad del color de la nota sobre el fondo (encabezado, cuerpo),
/// según el nivel elegido (0 = sin transparencia).
pub fn opacity(level: u8) -> Option<(u8, u8)> {
    match level {
        1 => Some((0xB0, 0x90)), // suave
        2 => Some((0xCC, 0xB2)), // media
        3 => Some((0xE6, 0xD6)), // fuerte
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_flat_color_and_softens_edges() {
        let (w, h) = (20usize, 10usize);
        let mut px = vec![0x00_40_80_c0u32; w * h];
        blur(&mut px, w, h);
        assert!(px.iter().all(|&c| c == 0x00_40_80_c0));
        let mut px: Vec<u32> = (0..w * h).map(|i| if i % w < w / 2 { 0 } else { 0x00ff_ffff }).collect();
        blur(&mut px, w, h);
        let mid = px[5 * w + w / 2 - 1] & 0xff;
        assert!(mid > 0x20 && mid < 0xe0, "{mid:x}");
    }

    #[test]
    fn places_like_windows() {
        let screen = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        assert_eq!(placement(screen, 3840.0, 2160.0, POS_FILL), Some((0.0, 0.0, 1920.0, 1080.0)));
        let (x, y, w, h) = placement(screen, 1000.0, 1000.0, POS_FIT).unwrap();
        assert_eq!((x, y, w, h), (420.0, 0.0, 1080.0, 1080.0));
        let (x, y, w, h) = placement(screen, 1000.0, 1000.0, POS_FILL).unwrap();
        assert_eq!((x, w), (0.0, 1920.0));
        assert!(y < 0.0 && h == 1920.0);
        assert_eq!(placement(screen, 100.0, 100.0, POS_TILE), None);
    }
}
