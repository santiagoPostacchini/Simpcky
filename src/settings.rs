//! "Configuración y sincronización": la otra pestaña de "Todas las
//! notas" (el engranaje al lado del buscador, o el menú de la bandeja).
//! Reúne lo que antes llenaba el menú de la bandeja: la cuenta de Google,
//! el enrollado de las notas nuevas, el atajo del selector de notas, el
//! tema, la transparencia con Windhawk, la escala de las notas, lo de
//! Windows y las actualizaciones.
//!
//! Solo el contenido: la ventana, el desplazamiento y el mouse los pone
//! `allnotes.rs`. Todo en coordenadas del contenido (0 = justo debajo de
//! la barra de arriba, sin desplazar). Dibujado como la bienvenida
//! (`welcome.rs`), con sus mismas piezas.

use std::ptr::null_mut;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;

use crate::app::app;
use crate::persist::RollMode;
use crate::theme;
use crate::welcome::{argb, round_rect, switch, text, ON_COLOR};
use crate::win::wide;

const PAD: i32 = 12;
const SECTION_H: i32 = 34;
const ROW_H: i32 = 44;
const CARD_H: i32 = 150;
const SEGMENTS_H: i32 = 34;
const HINT_H: i32 = 38;
const BUTTON_H: i32 = 30;

/// Las opciones de a una (botones pegados, uno elegido).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Choice {
    Hotkey,
    Transparency,
    Scale,
}

impl Choice {
    fn options(self) -> &'static [&'static str] {
        match self {
            Choice::Hotkey => &["Ninguno", "Ctrl+Alt+N", "Ctrl+Alt+Espacio"],
            Choice::Transparency => &["Apagada", "Suave", "Media", "Fuerte"],
            Choice::Scale => &["100 %", "125 %", "150 %"],
        }
    }

    fn current(self) -> u8 {
        let s = app().lock().unwrap().settings;
        match self {
            // Opciones: ninguno, Ctrl+Alt+N (2), Ctrl+Alt+Espacio (3).
            Choice::Hotkey => match s.hotkey {
                0 => 0,
                3 => 2,
                _ => 1,
            },
            Choice::Transparency => s.translucency,
            Choice::Scale => s.scale,
        }
    }

    fn set(self, v: u8) {
        match self {
            Choice::Hotkey => crate::tray::set_hotkey([0, 2, 3][v.min(2) as usize]),
            Choice::Transparency => crate::tray::set_translucency(v),
            Choice::Scale => crate::note::set_scale(v),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    Connect,
    CancelSignIn,
    SyncNow,
    Disconnect,
    AutoRoll,
    Dark,
    Pick(Choice, u8),
    Autostart,
    DesktopMenu,
    CheckUpdate,
    InstallUpdate,
    AutoUpdate,
    Privacy,
}

/// Qué muestra la tarjeta de sincronización.
enum Sync {
    Unavailable,
    Offer,
    Waiting,
    Connected(String, String),
}

fn sync_view() -> Sync {
    if !crate::sync::is_configured() {
        Sync::Unavailable
    } else if crate::sync::is_connected() {
        let (email, status) = crate::sync::status();
        Sync::Connected(email, status)
    } else if crate::sync::is_signing_in() {
        Sync::Waiting
    } else {
        Sync::Offer
    }
}

enum Row {
    Section(&'static str),
    Card,
    Switch(Hit, &'static str, bool),
    /// Un título arriba de las opciones de al lado (`Segments`).
    Label(&'static str),
    Segments(Choice),
    Hint(&'static str),
    /// Texto a la izquierda y un botón a la derecha.
    Action(String, Hit, String),
    Link(Hit, &'static str),
}

/// Las filas, con su lugar (coordenadas del contenido), para un ancho
/// de cliente `cw`.
fn rows(cw: i32) -> Vec<(Row, RECT)> {
    let (roll_auto, desktop_menu) = {
        let a = app().lock().unwrap();
        (a.settings.default_roll_mode == RollMode::Auto, a.settings.desktop_menu)
    };
    let version = crate::update::version_text(crate::update::current());
    let update = match crate::update::available() {
        Some(v) => Row::Action(format!("Hay una versión nueva: {}", crate::update::version_text(v)), Hit::InstallUpdate, "Actualizar…".into()),
        None => Row::Action(format!("Simpcky {version}"), Hit::CheckUpdate, "Buscar ahora".into()),
    };
    let hotkey_hint = if crate::tray::hotkey_failed() {
        "Otra aplicación ya usa esa combinación: probá con la otra."
    } else {
        "Muestra tus notas para elegir una y traerla adelante, sin fijarla."
    };
    let mut list = vec![
        (Row::Section("SINCRONIZACIÓN"), SECTION_H),
        (Row::Card, CARD_H),
        (Row::Section("NOTAS"), SECTION_H),
        (Row::Switch(Hit::AutoRoll, "Enrollar las nuevas al quitar el mouse", roll_auto), ROW_H),
        (Row::Label("Atajo para traer una nota al frente"), 28),
        (Row::Segments(Choice::Hotkey), SEGMENTS_H),
        (Row::Hint(hotkey_hint), HINT_H),
        (Row::Section("APARIENCIA"), SECTION_H),
        (Row::Switch(Hit::Dark, "Modo oscuro", theme::is_dark()), ROW_H),
        (Row::Label("Transparencia (requiere Windhawk)"), 28),
        (Row::Segments(Choice::Transparency), SEGMENTS_H),
    ];
    if !crate::glass::mod_loaded() {
        list.push((Row::Hint("Con el mod \"Translucent Windows\" de Windhawk, que no está activo para Simpcky."), HINT_H));
    } else {
        list.push((Row::Hint(""), 10));
    }
    list.extend([
        (Row::Label("Escala de las notas"), 28),
        (Row::Segments(Choice::Scale), SEGMENTS_H),
        (Row::Hint(""), 10),
        (Row::Section("WINDOWS"), SECTION_H),
        (Row::Switch(Hit::Autostart, "Iniciar con Windows", crate::shell::is_autostart_enabled()), ROW_H),
        (Row::Switch(Hit::DesktopMenu, "\"Nueva nota\" en el clic derecho del escritorio", desktop_menu), ROW_H),
        (Row::Section("ACTUALIZACIONES"), SECTION_H),
        (update, ROW_H),
        (Row::Switch(Hit::AutoUpdate, "Buscar automáticamente", crate::update::is_auto()), ROW_H),
        (Row::Link(Hit::Privacy, "Política de privacidad"), 40),
    ]);
    let mut y = 4;
    list.into_iter()
        .map(|(row, h)| {
            let r = RECT { left: PAD, top: y, right: cw - PAD, bottom: y + h };
            y += h;
            (row, r)
        })
        .collect()
}

/// Alto de todo el contenido.
pub fn height(cw: i32) -> i32 {
    rows(cw).last().map(|(_, r)| r.bottom + PAD).unwrap_or(0)
}

// -----------------------------------------------------------------
// Partes que se tocan
// -----------------------------------------------------------------

fn card_buttons(card: &RECT, view: &Sync) -> Vec<(Hit, RECT, &'static str)> {
    let top = card.bottom - 14 - BUTTON_H;
    let button = |left: i32, w: i32| RECT { left, top, right: left + w, bottom: top + BUTTON_H };
    let x = card.left + 14;
    match view {
        Sync::Offer => vec![(Hit::Connect, button(x, 170), "Conectar con Google…")],
        Sync::Waiting => vec![(Hit::CancelSignIn, button(x, 100), "Cancelar")],
        Sync::Connected(..) => vec![(Hit::SyncNow, button(x, 140), "Sincronizar ahora"), (Hit::Disconnect, button(x + 148, 110), "Desconectar…")],
        Sync::Unavailable => vec![],
    }
}

fn action_button(r: &RECT) -> RECT {
    let cy = (r.top + r.bottom) / 2;
    RECT { left: r.right - 118, top: cy - BUTTON_H / 2, right: r.right, bottom: cy + BUTTON_H / 2 }
}

/// `n` opciones repartidas en `r`.
fn segments(r: &RECT, n: usize) -> Vec<RECT> {
    let w = (r.right - r.left - 6) / n as i32;
    (0..n)
        .map(|i| {
            let left = r.left + 3 + i as i32 * w;
            RECT { left, top: r.top + 3, right: if i + 1 == n { r.right - 3 } else { left + w }, bottom: r.bottom - 3 }
        })
        .collect()
}

fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Qué hay en (`x`, `y`) (coordenadas del contenido).
pub fn hit(cw: i32, x: i32, y: i32) -> Hit {
    let view = sync_view();
    for (row, r) in rows(cw) {
        if !inside(&r, x, y) {
            continue;
        }
        return match row {
            Row::Card => card_buttons(&r, &view).into_iter().find(|(_, b, _)| inside(b, x, y)).map(|(h, ..)| h).unwrap_or(Hit::None),
            Row::Switch(h, ..) => h,
            Row::Segments(c) => {
                segments(&r, c.options().len()).iter().position(|s| inside(s, x, y)).map(|i| Hit::Pick(c, i as u8)).unwrap_or(Hit::None)
            }
            Row::Action(_, h, _) if inside(&action_button(&r), x, y) => h,
            Row::Link(h, _) if x < r.left + 170 => h,
            _ => Hit::None,
        };
    }
    Hit::None
}

/// Hacer lo que dice `hit`.
pub fn click(hit: Hit) {
    match hit {
        Hit::Connect => crate::sync::begin_sign_in(),
        Hit::CancelSignIn => crate::sync::cancel_sign_in(),
        Hit::SyncNow => crate::sync::sync_now(),
        Hit::Disconnect => crate::sync::disconnect(),
        Hit::AutoRoll => {
            let auto = crate::app::default_roll_mode() == RollMode::Auto;
            crate::tray::set_default_roll_mode(if auto { RollMode::Manual } else { RollMode::Auto });
        }
        Hit::Dark => theme::set_dark(!theme::is_dark()),
        Hit::Pick(c, v) => c.set(v),
        Hit::Autostart => crate::shell::set_autostart(!crate::shell::is_autostart_enabled()),
        Hit::DesktopMenu => crate::tray::toggle_desktop_menu(),
        Hit::CheckUpdate => crate::update::check_now(),
        Hit::InstallUpdate => crate::update::install(),
        Hit::AutoUpdate => crate::update::toggle_auto(),
        Hit::Privacy => crate::welcome::open_privacy(),
        Hit::None => {}
    }
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

/// Letras: etiqueta, chica, título de sección, título de tarjeta.
static FONTS: Mutex<[isize; 4]> = Mutex::new([0; 4]);

fn fonts() -> [isize; 4] {
    let mut f = FONTS.lock().unwrap();
    if f[0] == 0 {
        let face = wide("Segoe UI");
        let make = |h: i32, weight: u32| unsafe {
            CreateFontW(
                h,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH as u32) | (FF_DONTCARE as u32),
                face.as_ptr(),
            ) as isize
        };
        *f = [make(-14, FW_NORMAL), make(-12, FW_NORMAL), make(-11, FW_SEMIBOLD), make(-14, FW_SEMIBOLD)];
    }
    *f
}

/// Al cerrar la ventana.
pub fn free_fonts() {
    let mut f = FONTS.lock().unwrap();
    for h in f.iter_mut() {
        if *h != 0 {
            unsafe { DeleteObject(*h as HGDIOBJ) };
            *h = 0;
        }
    }
}

/// Texto de una etiqueta, centrado a lo alto en `r` aunque ocupe dos
/// renglones.
unsafe fn label(hdc: HDC, font: isize, color: u32, s: &str, r: RECT) {
    let old = SelectObject(hdc, font as HGDIOBJ);
    let w = wide(s);
    let mut calc = RECT { bottom: r.top, ..r };
    DrawTextW(hdc, w.as_ptr(), -1, &mut calc, DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX);
    let h = calc.bottom - calc.top;
    let mut rc = RECT { top: r.top + ((r.bottom - r.top) - h) / 2, bottom: r.bottom, ..r };
    SetTextColor(hdc, color);
    DrawTextW(hdc, w.as_ptr(), -1, &mut rc, DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS);
    SelectObject(hdc, old);
}

/// Un botón redondeado con su texto.
unsafe fn button(hdc: HDC, g: *mut GpGraphics, r: &RECT, caption: &str, hot: bool, base: u32) {
    let c = theme::chrome();
    round_rect(g, r, 8, argb(0xff, if hot { c.surface_hover } else { base }));
    text(hdc, fonts()[0], c.text, caption, *r, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
}

/// Dibuja el contenido en `hdc`, corrido `dy` (lo que ocupa la barra de
/// arriba menos lo desplazado). `hover`: lo que está bajo el mouse.
pub unsafe fn paint(hdc: HDC, cw: i32, dy: i32, hover: Hit) {
    let [f_label, f_small, f_section, f_title] = fonts();
    let c = theme::chrome();
    let view = sync_view();
    SetBkMode(hdc, TRANSPARENT as i32);

    let mut g: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(hdc, &mut g) != Ok || g.is_null() {
        return;
    }
    GdipSetSmoothingMode(g, SmoothingModeAntiAlias);

    for (row, r) in rows(cw) {
        let r = RECT { top: r.top + dy, bottom: r.bottom + dy, ..r };
        match row {
            Row::Section(title) => {
                let rc = RECT { top: r.bottom - 20, ..r };
                text(hdc, f_section, c.muted, title, rc, DT_SINGLELINE | DT_LEFT | DT_NOPREFIX);
            }
            Row::Card => {
                let card = RECT { bottom: r.bottom - 4, ..r };
                round_rect(g, &card, 10, argb(0xff, c.surface));
                let inner = RECT { left: card.left + 14, top: card.top + 12, right: card.right - 14, bottom: card.bottom };
                let (title, blurb) = match &view {
                    Sync::Unavailable => ("Google Drive", "La sincronización no está disponible en esta compilación de Simpcky.".to_string()),
                    Sync::Offer => (
                        "Tus notas en todas tus compus",
                        "Opcional, con tu cuenta de Google, en una carpeta oculta de tu Drive. Viajan el texto, el nombre y el color; el lugar en el escritorio es de cada compu.".to_string(),
                    ),
                    Sync::Waiting => ("Conectando con Google…", "Terminá de iniciar sesión en el navegador.".to_string()),
                    Sync::Connected(email, status) => {
                        (email.as_str(), format!("{status}.\nViajan el texto, el nombre y el color; el lugar en el escritorio es de cada compu."))
                    }
                };
                text(hdc, f_title, c.text, title, RECT { bottom: inner.top + 20, ..inner }, DT_SINGLELINE | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX);
                text(hdc, f_small, c.muted, &blurb, RECT { top: inner.top + 26, bottom: card.bottom - 14 - BUTTON_H - 6, ..inner }, DT_WORDBREAK | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX);
                for (h, b, caption) in card_buttons(&card, &view) {
                    button(hdc, g, &b, caption, hover == h, c.window);
                }
            }
            Row::Switch(h, title, on) => {
                if hover == h {
                    round_rect(g, &RECT { left: r.left - 6, top: r.top + 2, right: r.right + 6, bottom: r.bottom - 2 }, 8, argb(0xff, c.surface));
                }
                label(hdc, f_label, c.text, title, RECT { right: r.right - 52, ..r });
                switch(g, r.right, (r.top + r.bottom) / 2, on, c.surface_hover);
            }
            Row::Label(title) => {
                text(hdc, f_label, c.text, title, RECT { top: r.top + 4, ..r }, DT_SINGLELINE | DT_LEFT | DT_NOPREFIX);
            }
            Row::Segments(choice) => {
                round_rect(g, &r, 9, argb(0xff, c.surface));
                let current = choice.current();
                let names = choice.options();
                for (i, (s, name)) in segments(&r, names.len()).iter().zip(names).enumerate() {
                    let on = i as u8 == current;
                    if on {
                        round_rect(g, s, 7, argb(0xff, ON_COLOR));
                    } else if hover == Hit::Pick(choice, i as u8) {
                        round_rect(g, s, 7, argb(0xff, c.surface_hover));
                    }
                    let ink = if on { crate::win::rgb(0x1F, 0x29, 0x37) } else { c.text };
                    text(hdc, f_small, ink, name, *s, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
                }
            }
            Row::Hint(s) => {
                text(hdc, f_small, c.muted, s, RECT { top: r.top + 6, ..r }, DT_WORDBREAK | DT_LEFT | DT_NOPREFIX);
            }
            Row::Action(title, h, caption) => {
                let b = action_button(&r);
                label(hdc, f_label, c.text, &title, RECT { right: b.left - 8, ..r });
                button(hdc, g, &b, &caption, hover == h, c.surface);
            }
            Row::Link(h, title) => {
                let rc = RECT { right: r.left + 170, ..r };
                text(hdc, f_small, c.muted, title, rc, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX);
                if hover == h {
                    let old = SelectObject(hdc, f_small as HGDIOBJ);
                    let w = wide(title);
                    let mut size = SIZE { cx: 0, cy: 0 };
                    GetTextExtentPoint32W(hdc, w.as_ptr(), w.len() as i32 - 1, &mut size);
                    SelectObject(hdc, old);
                    let y = (r.top + r.bottom) / 2 + size.cy / 2;
                    let pen = CreatePen(PS_SOLID, 1, c.muted);
                    let old = SelectObject(hdc, pen as HGDIOBJ);
                    MoveToEx(hdc, rc.left, y, null_mut());
                    LineTo(hdc, rc.left + size.cx, y);
                    SelectObject(hdc, old);
                    DeleteObject(pen as HGDIOBJ);
                }
            }
        }
    }
    GdipDeleteGraphics(g);
}
