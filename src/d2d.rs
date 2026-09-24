//! Emojis en color (y títulos con emojis en color), con Direct2D +
//! DirectWrite.
//!
//! El RichEdit de Windows (`msftedit.dll`) dibuja con GDI, y GDI no sabe
//! de fuentes de color: los emojis le salen en blanco y negro, con
//! cualquier opción (el camino con DirectWrite existe solo en el
//! RichEdit de Office). Así que `editor.rs` los vuelve a pintar encima,
//! en color, con esto.
//!
//! `windows-sys` no trae las interfaces COM de Direct2D/DirectWrite, y
//! sumar el crate `windows` entero por cuatro métodos no vale la pena:
//! se llaman por su posición en la tabla virtual (los números salen del
//! orden de declaración en `d2d1.h` y `dwrite.h`, que Microsoft no
//! cambia nunca — es la ABI).

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::HDC;

#[link(name = "d2d1")]
extern "system" {
    fn D2D1CreateFactory(kind: u32, riid: *const GUID, options: *const c_void, factory: *mut *mut c_void) -> i32;
}
#[link(name = "dwrite")]
extern "system" {
    fn DWriteCreateFactory(kind: u32, riid: *const GUID, factory: *mut *mut c_void) -> i32;
}

const IID_ID2D1_FACTORY: GUID = GUID::from_u128(0x06152247_6f50_465a_9245_118bfd3b6007);
const IID_IDWRITE_FACTORY: GUID = GUID::from_u128(0xb859ee5a_d838_4b5b_a2e8_1adc7d93db48);

#[repr(C)]
struct PixelFormat {
    format: u32,
    alpha_mode: u32,
}

#[repr(C)]
struct RenderTargetProperties {
    kind: u32,
    pixel_format: PixelFormat,
    dpi_x: f32,
    dpi_y: f32,
    usage: u32,
    min_level: u32,
}

#[repr(C)]
struct RectF {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

#[repr(C)]
struct ColorF {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

const D2D1_FACTORY_TYPE_SINGLE_THREADED: u32 = 0;
const D2D1_RENDER_TARGET_TYPE_SOFTWARE: u32 = 1; // áreas chiquitas: más rápido que ir y volver de la GPU
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const D2D1_ALPHA_MODE_IGNORE: u32 = 3;
const D2D1_BITMAP_INTERPOLATION_MODE_NEAREST: u32 = 0;

#[repr(C)]
struct SizeU {
    width: u32,
    height: u32,
}

#[repr(C)]
struct BitmapProperties {
    pixel_format: PixelFormat,
    dpi_x: f32,
    dpi_y: f32,
}
const D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT: u32 = 4;
const DWRITE_FACTORY_TYPE_SHARED: u32 = 0;
const DWRITE_FONT_WEIGHT_NORMAL: u32 = 400;
const DWRITE_FONT_STYLE_NORMAL: u32 = 0;
const DWRITE_FONT_STRETCH_NORMAL: u32 = 5;
const DWRITE_TEXT_ALIGNMENT_LEADING: u32 = 0;
const DWRITE_TEXT_ALIGNMENT_CENTER: u32 = 2;
const DWRITE_TRIMMING_GRANULARITY_CHARACTER: u32 = 1;
const DWRITE_FONT_WEIGHT_SEMIBOLD: u32 = 600;

#[repr(C)]
struct Trimming {
    granularity: u32,
    delimiter: u32,
    delimiter_count: u32,
}
const DWRITE_PARAGRAPH_ALIGNMENT_CENTER: u32 = 2;
const DWRITE_WORD_WRAPPING_NO_WRAP: u32 = 1;

/// El método `index` de la tabla virtual de un objeto COM.
unsafe fn method<F: Copy>(obj: *mut c_void, index: usize) -> F {
    let vtbl = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(index))
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = method(obj, 2);
        f(obj);
    }
}

/// Un formato de texto: letra, peso, tamaño (en décimas de píxel) y si
/// va centrado (emojis) o alineado a la izquierda con "…" (títulos).
type Key = (&'static str, u32, u32, bool);

struct Emoji {
    target: *mut c_void, // ID2D1DCRenderTarget
    dwrite: *mut c_void, // IDWriteFactory
    formats: Vec<(Key, *mut c_void)>, // IDWriteTextFormat
}

thread_local! {
    // Todo esto vive en el hilo de las ventanas; `None` si Direct2D no
    // está (no debería pasar en Windows 10/11) y entonces los emojis se
    // quedan como los dibuja el RichEdit.
    static EMOJI: std::cell::RefCell<Option<Emoji>> = const { std::cell::RefCell::new(None) };
}

unsafe fn create() -> Option<Emoji> {
    let mut factory: *mut c_void = null_mut();
    if D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &IID_ID2D1_FACTORY, std::ptr::null(), &mut factory) < 0 {
        return None;
    }
    let props = RenderTargetProperties {
        kind: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixel_format: PixelFormat { format: DXGI_FORMAT_B8G8R8A8_UNORM, alpha_mode: D2D1_ALPHA_MODE_IGNORE },
        dpi_x: 96.0, // 1 DIP = 1 píxel: las coordenadas vienen del RichEdit
        dpi_y: 96.0,
        usage: 0,
        min_level: 0,
    };
    let mut target: *mut c_void = null_mut();
    // ID2D1Factory::CreateDCRenderTarget
    let create_dc: unsafe extern "system" fn(*mut c_void, *const RenderTargetProperties, *mut *mut c_void) -> i32 = method(factory, 16);
    let ok = create_dc(factory, &props, &mut target) >= 0;
    release(factory); // el render target se queda con su propia referencia
    if !ok {
        return None;
    }
    let mut dwrite: *mut c_void = null_mut();
    if DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWRITE_FACTORY, &mut dwrite) < 0 {
        release(target);
        return None;
    }
    Some(Emoji { target, dwrite, formats: Vec::new() })
}

unsafe fn text_format(e: &mut Emoji, key: Key) -> *mut c_void {
    if let Some(&(_, f)) = e.formats.iter().find(|(k, _)| *k == key) {
        return f;
    }
    let (family, weight, size10, centered) = key;
    let family = crate::win::wide(family);
    let locale = crate::win::wide("");
    let mut format: *mut c_void = null_mut();
    // IDWriteFactory::CreateTextFormat
    let create: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void, u32, u32, u32, f32, *const u16, *mut *mut c_void) -> i32 =
        method(e.dwrite, 15);
    if create(
        e.dwrite,
        family.as_ptr(),
        null_mut(),
        weight,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        size10 as f32 / 10.0,
        locale.as_ptr(),
        &mut format,
    ) < 0
    {
        return null_mut();
    }
    // IDWriteTextFormat::SetTextAlignment / SetParagraphAlignment / SetWordWrapping
    let set: unsafe extern "system" fn(*mut c_void, u32) -> i32 = method(format, 3);
    set(format, if centered { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING });
    let set: unsafe extern "system" fn(*mut c_void, u32) -> i32 = method(format, 4);
    set(format, DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    let set: unsafe extern "system" fn(*mut c_void, u32) -> i32 = method(format, 5);
    set(format, DWRITE_WORD_WRAPPING_NO_WRAP);
    if !centered {
        // "…" al final si no entra (IDWriteFactory::CreateEllipsisTrimmingSign
        // + IDWriteTextFormat::SetTrimming).
        let mut sign: *mut c_void = null_mut();
        let create_sign: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32 = method(e.dwrite, 20);
        if create_sign(e.dwrite, format, &mut sign) >= 0 {
            let trimming = Trimming { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiter_count: 0 };
            let set_trimming: unsafe extern "system" fn(*mut c_void, *const Trimming, *mut c_void) -> i32 = method(format, 9);
            set_trimming(format, &trimming, sign);
            release(sign);
        }
    }
    e.formats.push((key, format));
    format
}

fn color(c: u32) -> ColorF {
    ColorF {
        r: (c & 0xff) as f32 / 255.0,
        g: ((c >> 8) & 0xff) as f32 / 255.0,
        b: ((c >> 16) & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// Pinta `text` (un emoji) en color, centrado en `cell`, sobre un fondo
/// liso `bg` que tapa lo que había (el emoji en blanco y negro del
/// RichEdit). Solo se dibuja la parte de `cell` que cae dentro de
/// `clip`. `false` si no se pudo (y entonces queda lo que había).
/// Con `bg` en `None`, sobre lo que ya hay pintado en `hdc`.
pub fn draw_emoji(hdc: HDC, cell: RECT, clip: RECT, text: &[u16], size_px: u32, bg: Option<u32>) -> bool {
    draw(hdc, cell, clip, text, ("Segoe UI Emoji", DWRITE_FONT_WEIGHT_NORMAL, size_px * 10, true), 0, bg)
}

/// El título de una nota, en semibold, con "…" si no entra y los emojis
/// en color (con GDI salían en gris, al lado de los del texto en color).
pub fn draw_title(hdc: HDC, rect: RECT, text: &str, ink: u32, bg: Option<u32>, size_px: i32) -> bool {
    let w: Vec<u16> = text.encode_utf16().collect();
    draw(hdc, rect, rect, &w, ("Segoe UI", DWRITE_FONT_WEIGHT_SEMIBOLD, size_px.max(1) as u32 * 10, false), ink, bg)
}

/// Lo que hay pintado en `bound` de `hdc`, como bitmap de Direct2D (para
/// el render target `rt`, ya atado al DC). Nulo si no se pudo.
unsafe fn copy_under(rt: *mut c_void, hdc: HDC, bound: &RECT) -> *mut c_void {
    use windows_sys::Win32::Graphics::Gdi::*;
    let (w, h) = (bound.right - bound.left, bound.bottom - bound.top);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = null_mut();
    let dib = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if dib.is_null() || bits.is_null() {
        return null_mut();
    }
    let mem = CreateCompatibleDC(hdc);
    let old = SelectObject(mem, dib);
    BitBlt(mem, 0, 0, w, h, hdc, bound.left, bound.top, SRCCOPY);
    GdiFlush();
    let props = BitmapProperties {
        pixel_format: PixelFormat { format: DXGI_FORMAT_B8G8R8A8_UNORM, alpha_mode: D2D1_ALPHA_MODE_IGNORE },
        dpi_x: 96.0,
        dpi_y: 96.0,
    };
    let mut bitmap: *mut c_void = null_mut();
    // ID2D1RenderTarget::CreateBitmap (copia los píxeles)
    let create: unsafe extern "system" fn(*mut c_void, SizeU, *const c_void, u32, *const BitmapProperties, *mut *mut c_void) -> i32 =
        method(rt, 4);
    create(rt, SizeU { width: w as u32, height: h as u32 }, bits, (w * 4) as u32, &props, &mut bitmap);
    SelectObject(mem, old);
    DeleteDC(mem);
    DeleteObject(dib);
    bitmap
}

fn draw(hdc: HDC, cell: RECT, clip: RECT, text: &[u16], key: Key, fg: u32, bg: Option<u32>) -> bool {
    let bound = RECT {
        left: cell.left.max(clip.left),
        top: cell.top.max(clip.top),
        right: cell.right.min(clip.right),
        bottom: cell.bottom.min(clip.bottom),
    };
    if bound.right <= bound.left || bound.bottom <= bound.top || text.is_empty() {
        return true;
    }
    EMOJI.with(|cell_state| unsafe {
        let mut state = cell_state.borrow_mut();
        if state.is_none() {
            *state = create();
        }
        let Some(e) = state.as_mut() else { return false };
        let format = text_format(e, key);
        if format.is_null() {
            return false;
        }
        let rt = e.target;
        // Sin fondo propio: el DC render target no mezcla con lo que había
        // (lo pisa, negro donde no dibuja). Se copia lo que hay debajo y se
        // lo usa de fondo; así el texto va además con ClearType.
        let under = if bg.is_none() { copy_under(rt, hdc, &bound) } else { null_mut() };
        // ID2D1DCRenderTarget::BindDC — al rectángulo que se ve, así lo
        // que quede afuera no se toca.
        let bind: unsafe extern "system" fn(*mut c_void, HDC, *const RECT) -> i32 = method(rt, 57);
        if bind(rt, hdc, &bound) < 0 {
            release(under);
            return false;
        }
        let mut bg_brush: *mut c_void = null_mut();
        let mut fg_brush: *mut c_void = null_mut();
        // ID2D1RenderTarget::CreateSolidColorBrush
        let brush: unsafe extern "system" fn(*mut c_void, *const ColorF, *const c_void, *mut *mut c_void) -> i32 = method(rt, 8);
        if let Some(bg) = bg {
            brush(rt, &color(bg), std::ptr::null(), &mut bg_brush);
        }
        brush(rt, &color(fg), std::ptr::null(), &mut fg_brush);
        if (bg.is_some() && bg_brush.is_null()) || fg_brush.is_null() {
            release(bg_brush);
            release(fg_brush);
            release(under);
            return false;
        }
        // Coordenadas relativas al rectángulo atado.
        let (ox, oy) = (bound.left as f32, bound.top as f32);
        let whole = RectF { left: 0.0, top: 0.0, right: (bound.right - bound.left) as f32, bottom: (bound.bottom - bound.top) as f32 };
        let layout = RectF {
            left: cell.left as f32 - ox,
            top: cell.top as f32 - oy,
            right: cell.right as f32 - ox,
            bottom: cell.bottom as f32 - oy,
        };
        let begin: unsafe extern "system" fn(*mut c_void) = method(rt, 48);
        let fill: unsafe extern "system" fn(*mut c_void, *const RectF, *mut c_void) = method(rt, 17);
        let draw_text: unsafe extern "system" fn(*mut c_void, *const u16, u32, *mut c_void, *const RectF, *mut c_void, u32, u32) =
            method(rt, 27);
        let end: unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64) -> i32 = method(rt, 49);
        begin(rt);
        if !under.is_null() {
            // ID2D1RenderTarget::DrawBitmap
            let draw_bitmap: unsafe extern "system" fn(*mut c_void, *mut c_void, *const RectF, f32, u32, *const RectF) = method(rt, 26);
            draw_bitmap(rt, under, &whole, 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST, std::ptr::null());
        } else if !bg_brush.is_null() {
            fill(rt, &whole, bg_brush);
        }
        draw_text(
            rt,
            text.as_ptr(),
            text.len() as u32,
            format,
            &layout,
            fg_brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            0,
        );
        let hr = end(rt, null_mut(), null_mut());
        release(bg_brush);
        release(fg_brush);
        release(under);
        if hr < 0 {
            // D2DERR_RECREATE_TARGET u otra falla: se arma todo de nuevo
            // la próxima vez.
            for (_, f) in e.formats.drain(..) {
                release(f);
            }
            release(e.target);
            release(e.dwrite);
            *state = None;
            return false;
        }
        true
    })
}
