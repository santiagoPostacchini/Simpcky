//! Imágenes dentro de las notas (capturas pegadas con Ctrl+V).
//!
//! En el texto, cada imagen es un carácter de uso privado (`richtext::
//! IMAGE_BASE`…) en su propio párrafo; el PNG va en el formato de la nota
//! (`p<carácter>:<base64>`, ver `richtext::image_tokens`), así viaja con el
//! texto a `notes.json` (cifrado) y a Drive sin cambiar nada más. El
//! RichEdit le da a ese párrafo el alto de la imagen y `editor.rs` la pinta
//! encima, como los emojis.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::core::GUID;
use windows_sys::Win32::Graphics::Gdi::{BITMAPINFO, BITMAPINFOHEADER};
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::UI::Shell::SHCreateMemStream;

use crate::win::wide;

const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;
/// El codificador PNG de GDI+ (`image/png`).
const PNG_ENCODER: GUID = GUID::from_u128(0x557CF406_1A04_11D3_9A73_0000F81EF32E);
/// Lo más ancho que se guarda una imagen: una captura de un monitor 4K se
/// achica a esto (pesa la cuarta parte y en una nota no se ve más grande).
pub const MAX_W: i32 = 1600;
const CF_DIB: u32 = 8;

/// Una imagen ya decodificada (PARGB, lista para pintar), con su PNG para
/// guardarla y una copia al tamaño en que se muestra.
pub struct Image {
    pub w: i32,
    pub h: i32,
    pub b64: String,
    full: *mut GpBitmap,
    shown: Option<(i32, i32, *mut GpBitmap)>,
}

impl Image {
    pub fn from_b64(b64: &str) -> Option<Image> {
        let png = crate::crypto::b64url_decode(b64)?;
        let (full, w, h) = unsafe { decode(&png)? };
        Some(Image { w, h, b64: b64.to_string(), full, shown: None })
    }

    pub fn from_png(png: &[u8]) -> Option<Image> {
        let (full, w, h) = unsafe { decode(png)? };
        Some(Image { w, h, b64: crate::crypto::b64url(png), full, shown: None })
    }

    /// Ancho y alto con que se muestra en un renglón de `avail` píxeles.
    pub fn fit(&self, avail: i32) -> (i32, i32) {
        let w = self.w.min(avail.max(16)).max(1);
        (w, (self.h as i64 * w as i64 / self.w.max(1) as i64).max(1) as i32)
    }

    /// La imagen a ese tamaño (se rehace solo si cambió).
    pub fn at(&mut self, w: i32, h: i32) -> *mut GpBitmap {
        match self.shown {
            Some((sw, sh, b)) if (sw, sh) == (w, h) => b,
            _ => {
                if let Some((_, _, b)) = self.shown.take() {
                    unsafe { GdipDisposeImage(b as *mut GpImage) };
                }
                let b = unsafe { scaled(self.full as *mut GpImage, w, h) };
                if b.is_null() {
                    return self.full;
                }
                self.shown = Some((w, h, b));
                b
            }
        }
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            if let Some((_, _, b)) = self.shown.take() {
                GdipDisposeImage(b as *mut GpImage);
            }
            GdipDisposeImage(self.full as *mut GpImage);
        }
    }
}

// IStream por su lugar en la tabla virtual: IUnknown 0-2, Read 3, Write 4, Seek 5.
unsafe fn vcall<F: Copy>(obj: *mut c_void, i: usize) -> F {
    let vtbl = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(i))
}

unsafe fn release(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = vcall(obj, 2);
    f(obj);
}

/// Todo lo que tiene el stream.
unsafe fn stream_bytes(s: *mut c_void) -> Option<Vec<u8>> {
    type Seek = unsafe extern "system" fn(*mut c_void, i64, u32, *mut u64) -> i32;
    type Read = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32;
    let seek: Seek = vcall(s, 5);
    let read: Read = vcall(s, 3);
    let mut len = 0u64;
    if seek(s, 0, 2, &mut len) < 0 || seek(s, 0, 0, null_mut()) < 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let mut got = 0u32;
    (read(s, buf.as_mut_ptr() as *mut c_void, len as u32, &mut got) >= 0 && got as u64 == len).then_some(buf)
}

/// Una copia PARGB de `src` a `w`×`h` (con buen suavizado al achicar).
unsafe fn scaled(src: *mut GpImage, w: i32, h: i32) -> *mut GpBitmap {
    let mut dst: *mut GpBitmap = null_mut();
    if GdipCreateBitmapFromScan0(w, h, 0, PIXEL_FORMAT_32BPP_PARGB, std::ptr::null(), &mut dst) != Ok || dst.is_null() {
        return null_mut();
    }
    let mut g: *mut GpGraphics = null_mut();
    if GdipGetImageGraphicsContext(dst as *mut GpImage, &mut g) == Ok && !g.is_null() {
        GdipSetInterpolationMode(g, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        GdipDrawImageRectI(g, src, 0, 0, w, h);
        GdipDeleteGraphics(g);
    }
    dst
}

unsafe fn size_of(img: *mut GpImage) -> (i32, i32) {
    let (mut w, mut h) = (0u32, 0u32);
    GdipGetImageWidth(img, &mut w);
    GdipGetImageHeight(img, &mut h);
    (w as i32, h as i32)
}

/// PNG (o lo que lea GDI+) → bitmap PARGB propio (no depende del stream).
unsafe fn decode(png: &[u8]) -> Option<(*mut GpBitmap, i32, i32)> {
    let s = SHCreateMemStream(png.as_ptr(), png.len() as u32);
    if s.is_null() {
        return None;
    }
    let mut src: *mut GpBitmap = null_mut();
    let ok = GdipCreateBitmapFromStream(s, &mut src) == Ok && !src.is_null();
    let out = if ok {
        let (w, h) = size_of(src as *mut GpImage);
        let b = if w > 0 && h > 0 { scaled(src as *mut GpImage, w, h) } else { null_mut() };
        GdipDisposeImage(src as *mut GpImage);
        (!b.is_null()).then_some((b, w, h))
    } else {
        None
    };
    release(s);
    out
}

unsafe fn encode(img: *mut GpImage) -> Option<Vec<u8>> {
    let s = SHCreateMemStream(std::ptr::null(), 0);
    if s.is_null() {
        return None;
    }
    let out = if GdipSaveImageToStream(img, s, &PNG_ENCODER, std::ptr::null()) == Ok { stream_bytes(s) } else { None };
    release(s);
    out
}

/// Si es más ancha que `MAX_W`, la achica (y la vuelve a codificar).
unsafe fn shrink(img: *mut GpImage, png: Option<Vec<u8>>) -> Option<Vec<u8>> {
    let (w, h) = size_of(img);
    if w <= 0 || h <= 0 {
        return None;
    }
    if w <= MAX_W {
        return match png {
            Some(p) => Some(p),
            None => encode(img),
        };
    }
    let nh = (h as i64 * MAX_W as i64 / w as i64).max(1) as i32;
    let small = scaled(img, MAX_W, nh);
    if small.is_null() {
        return None;
    }
    let out = encode(small as *mut GpImage);
    GdipDisposeImage(small as *mut GpImage);
    out
}

/// ¿Hay una imagen (y no texto) para pegar?
pub fn clipboard_has_image() -> bool {
    unsafe {
        let png = RegisterClipboardFormatW(wide("PNG").as_ptr());
        IsClipboardFormatAvailable(13) == 0 && (IsClipboardFormatAvailable(png) != 0 || IsClipboardFormatAvailable(CF_DIB) != 0)
    }
}

/// La imagen del portapapeles, como PNG (achicada si hace falta). Primero el
/// formato "PNG" (el de las capturas, con su transparencia), si no CF_DIB.
pub fn clipboard_png() -> Option<Vec<u8>> {
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return None;
        }
        let png_fmt = RegisterClipboardFormatW(wide("PNG").as_ptr());
        let out = if IsClipboardFormatAvailable(png_fmt) != 0 {
            global_bytes(GetClipboardData(png_fmt)).and_then(|png| {
                let (b, _, _) = decode(&png)?;
                let r = shrink(b as *mut GpImage, Some(png));
                GdipDisposeImage(b as *mut GpImage);
                r
            })
        } else if IsClipboardFormatAvailable(CF_DIB) != 0 {
            from_dib(GetClipboardData(CF_DIB))
        } else {
            None
        };
        CloseClipboard();
        out
    }
}

unsafe fn global_bytes(h: windows_sys::Win32::Foundation::HANDLE) -> Option<Vec<u8>> {
    if h.is_null() {
        return None;
    }
    let p = GlobalLock(h) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
    GlobalUnlock(h);
    Some(v)
}

/// Un CF_DIB (encabezado, colores y píxeles seguidos) → PNG.
unsafe fn from_dib(h: windows_sys::Win32::Foundation::HANDLE) -> Option<Vec<u8>> {
    if h.is_null() {
        return None;
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        return None;
    }
    let size = GlobalSize(h);
    let hdr = &*(p as *const BITMAPINFOHEADER);
    // Los píxeles van después del encabezado, las máscaras (BI_BITFIELDS
    // con encabezado de 40 bytes) y la paleta.
    let masks = if hdr.biCompression == 3 && hdr.biSize == 40 { 12 } else { 0 };
    let colors = if hdr.biClrUsed > 0 { hdr.biClrUsed } else if hdr.biBitCount <= 8 { 1 << hdr.biBitCount } else { 0 };
    let off = hdr.biSize as usize + masks + colors as usize * 4;
    let mut out = None;
    if off < size && hdr.biWidth > 0 && hdr.biHeight != 0 {
        let mut b: *mut GpBitmap = null_mut();
        if GdipCreateBitmapFromGdiDib(p as *const BITMAPINFO, p.add(off) as *mut c_void, &mut b) == Ok && !b.is_null() {
            out = shrink(b as *mut GpImage, None);
            GdipDisposeImage(b as *mut GpImage);
        }
    }
    GlobalUnlock(h);
    out
}

/// Un PNG de un solo color, para las pruebas.
#[cfg(test)]
pub fn test_png(w: i32, h: i32, argb: u32) -> Vec<u8> {
    unsafe {
        let input = GdiplusStartupInput { GdiplusVersion: 1, DebugEventCallback: 0, SuppressBackgroundThread: 0, SuppressExternalCodecs: 0 };
        let mut token = 0usize;
        GdiplusStartup(&mut token, &input, null_mut());
        let px = vec![argb; (w * h) as usize];
        let mut b: *mut GpBitmap = null_mut();
        GdipCreateBitmapFromScan0(w, h, w * 4, PIXEL_FORMAT_32BPP_PARGB, px.as_ptr() as *const u8, &mut b);
        let png = encode(b as *mut GpImage).unwrap();
        GdipDisposeImage(b as *mut GpImage);
        png
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una imagen hecha acá pasa a PNG y vuelve igual de tamaño (GDI+ de verdad).
    #[test]
    fn png_ida_y_vuelta() {
        unsafe {
            let input = GdiplusStartupInput { GdiplusVersion: 1, DebugEventCallback: 0, SuppressBackgroundThread: 0, SuppressExternalCodecs: 0 };
            let mut token = 0usize;
            GdiplusStartup(&mut token, &input, null_mut());
            let mut b: *mut GpBitmap = null_mut();
            assert_eq!(GdipCreateBitmapFromScan0(2000, 500, 0, PIXEL_FORMAT_32BPP_PARGB, std::ptr::null(), &mut b), Ok);
            let png = shrink(b as *mut GpImage, None).expect("png");
            GdipDisposeImage(b as *mut GpImage);
            let img = Image::from_png(&png).expect("decodifica");
            assert_eq!((img.w, img.h), (MAX_W, 400));
            assert_eq!(img.fit(800), (800, 200));
            let again = Image::from_b64(&img.b64).expect("base64");
            assert_eq!((again.w, again.h), (MAX_W, 400));
        }
    }
}
