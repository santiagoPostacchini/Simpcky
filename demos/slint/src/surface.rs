use common::{framelog, win};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor};
use std::{
    ptr::{null, null_mut},
    rc::Rc,
    time::Instant,
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub struct Surface {
    pub hwnd: HWND,
    pub adapter: Rc<MinimalSoftwareWindow>,
    pub size: (i32, i32),
    pixels: Vec<PremultipliedRgbaColor>,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}
impl Surface {
    pub fn new(
        adapter: Rc<MinimalSoftwareWindow>,
        overlay: bool,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Self {
        let extra = if overlay {
            WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOPMOST
        } else {
            0
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | extra,
                win::wide("SimpckySlint").as_ptr(),
                win::wide("Simpcky Slint").as_ptr(),
                WS_POPUP,
                x,
                y,
                w,
                h,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        assert!(!hwnd.is_null(), "No se pudo crear la ventana");
        let mut s = Self {
            hwnd,
            adapter,
            size: (0, 0),
            pixels: Vec::new(),
            dc: null_mut(),
            bitmap: null_mut(),
            old: null_mut(),
            bits: null_mut(),
        };
        s.resize(w, h);
        s
    }
    fn free(&mut self) {
        unsafe {
            if !self.dc.is_null() {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
            self.dc = null_mut();
        }
    }
    pub fn resize(&mut self, w: i32, h: i32) {
        let (w, h) = (w.max(1), h.max(1));
        if self.size == (w, h) {
            return;
        }
        self.free();
        unsafe {
            self.dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            self.bitmap = CreateDIBSection(
                self.dc,
                &info,
                DIB_RGB_COLORS,
                &mut self.bits as *mut _ as *mut _,
                null_mut(),
                0,
            );
            assert!(!self.bitmap.is_null(), "No se pudo crear el DIB");
            self.old = SelectObject(self.dc, self.bitmap);
        }
        self.size = (w, h);
        self.pixels = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        self.adapter
            .set_size(slint::PhysicalSize::new(w as u32, h as u32));
        self.adapter.request_redraw();
    }
    pub fn draw(&mut self, kind: &str, id: u32, holes: &[[i32; 4]]) {
        let start = Instant::now();
        let stride = self.size.0 as usize;
        let mut changed = false;
        self.adapter.draw_if_needed(|renderer| {
            let region = renderer.render(&mut self.pixels, stride);
            // El DIB conserva los pÃ­xeles fuera de la regiÃ³n sucia.
            let origin = region.bounding_box_origin();
            let size = region.bounding_box_size();
            for y in origin.y.max(0) as usize
                ..(origin.y.max(0) as usize + size.height as usize).min(self.size.1 as usize)
            {
                for x in origin.x.max(0) as usize
                    ..(origin.x.max(0) as usize + size.width as usize).min(stride)
                {
                    let p = self.pixels[y * stride + x];
                    unsafe {
                        *self.bits.add(y * stride + x) =
                            u32::from_le_bytes([p.blue, p.green, p.red, p.alpha]);
                    }
                }
            }
            changed = true;
        });
        if !changed {
            return;
        }
        // Los huecos se aplican al DIB, sin alterar el bÃºfer reutilizado de Slint.
        if kind == "capa" {
            for (i, p) in self.pixels.iter().enumerate() {
                unsafe {
                    *self.bits.add(i) = u32::from_le_bytes([p.blue, p.green, p.red, p.alpha]);
                }
            }
            for r in holes {
                for y in r[1].max(0)..r[3].min(self.size.1) {
                    for x in r[0].max(0)..r[2].min(self.size.0) {
                        let cx = (x as f32 + 0.5)
                            .clamp((r[0] + 8) as f32, (r[2] - 8).max(r[0] + 8) as f32);
                        let cy = (y as f32 + 0.5)
                            .clamp((r[1] + 8) as f32, (r[3] - 8).max(r[1] + 8) as f32);
                        if (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2) <= 64.0 {
                            unsafe {
                                *self.bits.add(y as usize * stride + x as usize) = 0;
                            }
                        }
                    }
                }
            }
        }
        unsafe {
            let r = win::window_rect(self.hwnd);
            let dst = POINT {
                x: r.left,
                y: r.top,
            };
            let src = POINT { x: 0, y: 0 };
            let size = SIZE {
                cx: self.size.0,
                cy: self.size.1,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                &dst,
                &size,
                self.dc,
                &src,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
        framelog::mark(kind, id, start.elapsed());
        // Volcado opcional para revisar el render sin acceso a la pantalla.
        if let Some(dir) = std::env::var_os("SIMPCKY_DUMP_DIR") {
            let path = std::path::PathBuf::from(dir)
                .join(format!("{kind}-{id}-{}-{}.bgra", self.size.0, self.size.1));
            let bytes = unsafe {
                std::slice::from_raw_parts(self.bits as *const u8, self.pixels.len() * 4)
            };
            let _ = std::fs::write(path, bytes);
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        self.free();
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
