//! Fantasma de arrastre: una mini nota semitransparente que sigue al
//! cursor mientras se arrastra una tarjeta de "Todas las notas" hacia
//! el escritorio, para que se vea qué se está llevando y dónde va a
//! caer.
//!
//! Es una ventana en capas (`WS_EX_LAYERED`) con alfa por píxel vía
//! `UpdateLayeredWindow` — la única forma de tener esquinas
//! redondeadas suaves y sombra en una ventana flotante — y
//! "transparente" a los clics (`WS_EX_TRANSPARENT`), para no meterse
//! entre el cursor y lo que hay debajo.

use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::wide;

/// Tamaño de la mini nota (sin la sombra).
pub const W: i32 = 200;
pub const H: i32 = 124;
const SHADOW: i32 = 12;
const HEADER: i32 = 30;
const RADIUS: i32 = 10;
const CLASS_NAME: &str = "SimpckyDragGhost";

// Valor fijo de `PixelFormat32bppPARGB` (Gdipluspixelformats.h):
// 32 bits con alfa premultiplicado, lo que espera UpdateLayeredWindow.
const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;

struct Ghost {
    hwnd: isize,
    mem_dc: isize,
    bitmap: isize,
    old_bitmap: isize,
}

static GHOST: Mutex<Option<Ghost>> = Mutex::new(None);

fn register_class() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| unsafe {
        let class_name = wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: crate::app::app().lock().unwrap().hinstance as HINSTANCE,
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

fn argb(a: u8, c: u32) -> u32 {
    (crate::note::colorref_to_argb(c) & 0x00ff_ffff) | ((a as u32) << 24)
}

unsafe fn round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> *mut GpPath {
    let d = r * 2.0;
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    path
}

unsafe fn fill_path(g: *mut GpGraphics, path: *mut GpPath, color: u32) {
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    GdipFillPath(g, brush as *mut GpBrush, path);
    GdipDeleteBrush(brush as *mut GpBrush);
}

unsafe fn draw_text(g: *mut GpGraphics, text: &str, size: f32, bold: bool, color: u32, rect: RectF) {
    let face = wide("Segoe UI");
    let mut family: *mut GpFontFamily = null_mut();
    if GdipCreateFontFamilyFromName(face.as_ptr(), null_mut(), &mut family) != Ok {
        return;
    }
    let mut font: *mut GpFont = null_mut();
    GdipCreateFont(family, size, if bold { FontStyleBold } else { FontStyleRegular }, UnitPixel, &mut font);
    let mut format: *mut GpStringFormat = null_mut();
    GdipCreateStringFormat(0, 0, &mut format);
    GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
    GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    let w = wide(text);
    GdipDrawString(g, w.as_ptr(), -1, font, &rect, format, brush as *mut GpBrush);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeleteStringFormat(format);
    GdipDeleteFont(font);
    GdipDeleteFontFamily(family);
}

/// Dibuja la mini nota en un DIB de 32 bits con alfa premultiplicado.
unsafe fn render(mem_dc: HDC, bits: *mut u8, color: u8, title: &str) {
    let (header, body, ink) = crate::theme::note_colors(color);
    let (tw, th) = (W + SHADOW * 2, H + SHADOW * 2);
    let mut bmp: *mut GpBitmap = null_mut();
    if GdipCreateBitmapFromScan0(tw, th, tw * 4, PIXEL_FORMAT_32BPP_PARGB, bits, &mut bmp) != Ok {
        return;
    }
    let mut g: *mut GpGraphics = null_mut();
    GdipGetImageGraphicsContext(bmp as *mut GpImage, &mut g);
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
    GdipGraphicsClear(g, 0);

    // Sombra: capas redondeadas cada vez más grandes y más tenues.
    for i in 0..SHADOW {
        let spread = i as f32;
        let path = round_rect_path(
            SHADOW as f32 - spread,
            SHADOW as f32 - spread + 3.0,
            W as f32 + spread * 2.0,
            H as f32 + spread * 2.0,
            RADIUS as f32 + spread,
        );
        let alpha = (14.0 * (1.0 - spread / SHADOW as f32)) as u8;
        fill_path(g, path, (alpha as u32) << 24);
        GdipDeletePath(path);
    }

    let (x, y) = (SHADOW as f32, SHADOW as f32);
    let card = round_rect_path(x, y, W as f32, H as f32, RADIUS as f32);
    fill_path(g, card, argb(0xff, body));
    // Encabezado: la misma forma, recortada a la franja de arriba.
    GdipSetClipRectI(g, SHADOW, SHADOW, W, HEADER, CombineModeReplace);
    fill_path(g, card, argb(0xff, header));
    GdipResetClip(g);
    GdipDeletePath(card);

    draw_text(
        g,
        title,
        13.0,
        true,
        argb(0xff, ink),
        RectF { X: x + 12.0, Y: y, Width: W as f32 - 24.0, Height: HEADER as f32 },
    );
    draw_text(
        g,
        "Soltala en el escritorio",
        12.0,
        false,
        argb(0xb0, ink),
        RectF { X: x + 12.0, Y: y + HEADER as f32 + 10.0, Width: W as f32 - 24.0, Height: 22.0 },
    );

    GdipDeleteGraphics(g);
    GdipDisposeImage(bmp as *mut GpImage);
    let _ = mem_dc;
}

/// Muestra el fantasma de la nota (`color`, `title`) con su esquina
/// superior izquierda en (`x`, `y`), coordenadas de pantalla.
pub fn show(color: u8, title: &str, x: i32, y: i32) {
    hide();
    register_class();
    unsafe {
        let class_name = wide(CLASS_NAME);
        let hinstance = { crate::app::app().lock().unwrap().hinstance } as HINSTANCE;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            null(),
            WS_POPUP,
            x - SHADOW,
            y - SHADOW,
            W + SHADOW * 2,
            H + SHADOW * 2,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }

        let screen = GetDC(null_mut());
        let mem_dc = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = W + SHADOW * 2;
        bmi.bmiHeader.biHeight = -(H + SHADOW * 2); // de arriba hacia abajo
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut core::ffi::c_void = null_mut();
        let bitmap = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        ReleaseDC(null_mut(), screen);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(mem_dc);
            DestroyWindow(hwnd);
            return;
        }
        let old_bitmap = SelectObject(mem_dc, bitmap as HGDIOBJ);
        render(mem_dc, bits as *mut u8, color, title);

        *GHOST.lock().unwrap() = Some(Ghost {
            hwnd: hwnd as isize,
            mem_dc: mem_dc as isize,
            bitmap: bitmap as isize,
            old_bitmap: old_bitmap as isize,
        });
    }
    move_to(x, y, false);
    let hwnd = GHOST.lock().unwrap().as_ref().map(|g| g.hwnd).unwrap_or(0);
    if hwnd != 0 {
        unsafe { ShowWindow(hwnd as HWND, SW_SHOWNOACTIVATE) };
    }
}

/// Mueve el fantasma. `droppable`: si soltar ahí dejaría la nota en el
/// escritorio (más opaco) o no (más transparente, "acá no").
pub fn move_to(x: i32, y: i32, droppable: bool) {
    let guard = GHOST.lock().unwrap();
    let Some(g) = guard.as_ref() else { return };
    unsafe {
        let screen = GetDC(null_mut());
        let dst = POINT { x: x - SHADOW, y: y - SHADOW };
        let size = SIZE { cx: W + SHADOW * 2, cy: H + SHADOW * 2 };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: if droppable { 235 } else { 130 },
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(g.hwnd as HWND, screen, &dst, &size, g.mem_dc as HDC, &src, 0, &blend, ULW_ALPHA);
        ReleaseDC(null_mut(), screen);
    }
}

pub fn hide() {
    let ghost = GHOST.lock().unwrap().take();
    if let Some(g) = ghost {
        unsafe {
            DestroyWindow(g.hwnd as HWND);
            SelectObject(g.mem_dc as HDC, g.old_bitmap as HGDIOBJ);
            DeleteObject(g.bitmap as HGDIOBJ);
            DeleteDC(g.mem_dc as HDC);
        }
    }
}
