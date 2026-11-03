//! Un solo dispositivo de GPU para todas las ventanas: D3D11 → Direct2D, y
//! DirectComposition para mostrar cada ventana con alfa por píxel.
//!
//! Cada ventana (nota o capa) tiene una cadena de intercambio "para
//! composición" (premultiplicada, sin superficie de redirección) puesta
//! como contenido de un visual de DirectComposition. Dibujar es: tomar el
//! búfer de atrás como mapa de bits de Direct2D, pintar, `Present`.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

pub struct Gfx {
    pub d3d: ID3D11Device,
    pub dxgi: IDXGIFactory2,
    pub d2d: ID2D1Factory1,
    pub dc: ID2D1DeviceContext,
    pub dcomp: IDCompositionDevice,
    pub dwrite: IDWriteFactory,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let mut d3d = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]),
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let d3d = d3d.unwrap();
            let dxgi_device: IDXGIDevice = d3d.cast()?;
            let dxgi: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let device = d2d.CreateDevice(&dxgi_device)?;
            let dc = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Gfx { d3d, dxgi, d2d, dc, dcomp, dwrite })
        }
    }

    pub fn commit(&self) {
        unsafe {
            let _ = self.dcomp.Commit();
        }
    }
}

/// Lo que muestra una ventana: su cadena de intercambio dentro de un visual.
pub struct Surface {
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    swap: IDXGISwapChain1,
    pub size: (u32, u32),
}

impl Surface {
    pub fn new(g: &Gfx, hwnd: HWND, w: u32, h: u32) -> Result<Surface> {
        unsafe {
            let (w, h) = (w.max(1), h.max(1));
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = g.dxgi.CreateSwapChainForComposition(&g.d3d, &desc, None)?;
            let target = g.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = g.dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            Ok(Surface { _target: target, _visual: visual, swap, size: (w, h) })
        }
    }

    /// Cambia el tamaño de los búferes (lo dibujado se pierde: hay que volver a pintar).
    pub fn resize(&mut self, g: &Gfx, w: u32, h: u32) -> Result<()> {
        let (w, h) = (w.max(1), h.max(1));
        if (w, h) == self.size {
            return Ok(());
        }
        unsafe {
            g.dc.SetTarget(None);
            self.swap.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.size = (w, h);
        Ok(())
    }

    /// Pinta con `f` sobre el búfer de atrás (empieza transparente) y lo muestra.
    pub fn draw(&mut self, g: &Gfx, f: impl FnOnce(&ID2D1DeviceContext)) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bitmap = g.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            g.dc.SetTarget(&bitmap);
            g.dc.BeginDraw();
            g.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            f(&g.dc);
            let r = g.dc.EndDraw(None, None);
            g.dc.SetTarget(None);
            r?;
            self.swap.Present(0, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------
// Atajos para dibujar
// -----------------------------------------------------------------

pub fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    let (r, g, b) = common::theme::rgb(rgb);
    D2D1_COLOR_F { r, g, b, a }
}

/// De 0xAARRGGBB, con la opacidad multiplicada por `k`.
pub fn argb(c: u32, k: f32) -> D2D1_COLOR_F {
    let (r, g, b, a) = common::theme::rgba(c);
    D2D1_COLOR_F { r, g, b, a: a * k }
}

pub fn rectf(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

pub fn rounded(r: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }
}

pub fn brush(dc: &ID2D1DeviceContext, c: D2D1_COLOR_F) -> ID2D1SolidColorBrush {
    unsafe { dc.CreateSolidColorBrush(&c, None).unwrap() }
}

pub fn text_format(g: &Gfx, family: &str, size: f32, weight: DWRITE_FONT_WEIGHT) -> IDWriteTextFormat {
    unsafe {
        let family: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
        g.dwrite
            .CreateTextFormat(PCWSTR(family.as_ptr()), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("es-ar"))
            .unwrap()
    }
}
