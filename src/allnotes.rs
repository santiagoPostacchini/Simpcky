//! Ventana "Todas las notas": buscador + grilla de tarjetas con todas
//! las notas, para encontrar cualquiera y abrirla — útil sobre todo
//! para las que viven ancladas al escritorio, donde no hay Alt+Tab que
//! valga.
//!
//! El layout sigue la pantalla "Menús y bandeja" del lienzo de diseño:
//! buscador en píldora con lupa y una grilla de tres columnas de
//! tarjetas del color de cada nota (las enrolladas, más bajitas; las de
//! "siempre encima", con el pin en la esquina). Al lado del buscador,
//! el interruptor de tema (luna = pasar a oscuro, sol = pasar a claro).
//!
//! Con las tarjetas:
//! - doble clic / Enter: abrir la nota (se asoma al frente, ver
//!   `note::open_note`), también si estaba guardada acá (oculta);
//! - arrastrarla afuera de la ventana y soltarla sobre el escritorio:
//!   la nota queda ahí como widget (ver `ghost.rs`);
//! - clic derecho: abrir, ocultar, cambiar nombre, eliminar.
//!
//! Las notas ocultas (`NoteData::hidden`) viven solo acá: tarjeta más
//! apagada y con el ojo tachado.
//!
//! El engranaje al lado del buscador pasa a la otra pestaña,
//! "Configuración y sincronización" (ver `settings.rs`).
//!
//! Nada de esto existe como control nativo de Windows, así que se pinta
//! a mano: GDI+ para las formas redondeadas con antialiasing y GDI para
//! el texto, con doble buffer para que no parpadee.

use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Controls::{SetScrollInfo, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::app;
use crate::flyout;
use crate::note;
use crate::settings;
use crate::persist::{Layer, NoteData};
use crate::theme;
use crate::win::{from_wide, wide};

const CLASS_NAME: &str = "SimpckyAllNotes";
const ID_SEARCH: usize = 1;
/// Refrescar la grilla (en cola; ver `notify_changed`).
const WM_APP_REFRESH: u32 = WM_APP + 30;

/// Medidas del diseño (pantalla "Menús y bandeja", ventana de 360 px).
const PAD: i32 = 12;
const GAP: i32 = 10;
const COLS: i32 = 3;
const CARD_H: i32 = 82;
/// Una nota enrollada se muestra como una tarjeta baja, solo el
/// título — igual que en el diseño.
const CARD_H_ROLLED: i32 = 34;
const CARD_R: i32 = 8;
const SEARCH_H: i32 = 34;
const SEARCH_R: i32 = 8;

const MENU_OPEN: usize = 1;
const MENU_RENAME: usize = 2;
const MENU_DELETE: usize = 3;
const MENU_HIDE: usize = 4;

/// Qué muestra la ventana.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Notes,
    Settings,
}

struct Item {
    id: u32,
    hidden: bool,
    locked: bool,
    color: u8,
    pinned: bool,
    rolled: bool,
    title: String,
    preview: String,
}

struct State {
    hwnd: isize,
    edit: isize,
    search_brush: isize,
    ui_font: isize,
    title_font: isize,
    items: Vec<Item>,
    sel: i32,
    hover: i32,
    /// El mouse sobre el botón de arriba (engranaje, o volver).
    hover_top_btn: bool,
    scroll: i32,
    view: View,
    /// Lo que está bajo el mouse en la configuración.
    settings_hover: settings::Hit,
    /// Dónde estaba la lista al pasar a la configuración.
    notes_scroll: i32,
    /// Tarjeta apretada con el botón izquierdo (-1 = ninguna), dónde, y
    /// si ya se convirtió en arrastre.
    press: i32,
    press_pt: POINT,
    grab: POINT,
    dragging: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            hwnd: 0,
            edit: 0,
            search_brush: 0,
            ui_font: 0,
            title_font: 0,
            items: Vec::new(),
            sel: -1,
            hover: -1,
            hover_top_btn: false,
            scroll: 0,
            view: View::Notes,
            settings_hover: settings::Hit::None,
            notes_scroll: 0,
            press: -1,
            press_pt: POINT { x: 0, y: 0 },
            grab: POINT { x: 0, y: 0 },
            dragging: false,
        })
    })
}

pub fn register_class(hinstance: HINSTANCE) {
    unsafe {
        let class_name = wide(CLASS_NAME);
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            // CS_DBLCLKS: sin esto no llega WM_LBUTTONDBLCLK y el
            // doble clic sobre una tarjeta no abriría la nota.
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: crate::icon::app_icon_large(),
            hCursor: cursor,
            hbrBackground: null_mut(), // pintamos todo nosotros
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: crate::icon::app_icon(),
        };
        RegisterClassExW(&wc);
    }
}

/// Abre la ventana en la lista de notas (o, si ya está abierta, la trae
/// al frente).
pub fn show() {
    open(View::Notes);
}

/// Abre la ventana en "Configuración y sincronización".
pub fn show_settings() {
    open(View::Settings);
}

fn open(view: View) {
    let existing = state().lock().unwrap().hwnd;
    if existing != 0 {
        let hwnd = existing as HWND;
        crate::win::show_normal(hwnd);
        set_view(hwnd, view);
        refresh_list(hwnd);
        return;
    }
    create();
    let hwnd = state().lock().unwrap().hwnd as HWND;
    if !hwnd.is_null() {
        set_view(hwnd, view);
    }
}

/// Pasa de la lista a la configuración, o al revés.
fn set_view(hwnd: HWND, view: View) {
    let edit = {
        let mut s = state().lock().unwrap();
        if s.view == view {
            return;
        }
        if view == View::Settings {
            s.notes_scroll = s.scroll;
            s.scroll = 0;
        } else {
            s.scroll = s.notes_scroll;
        }
        s.view = view;
        s.hover = -1;
        s.hover_top_btn = false;
        s.settings_hover = settings::Hit::None;
        s.edit as HWND
    };
    unsafe {
        if !edit.is_null() {
            ShowWindow(edit, if view == View::Settings { SW_HIDE } else { SW_SHOW });
        }
        SetFocus(if view == View::Settings || edit.is_null() { hwnd } else { edit });
    }
    update_title(hwnd);
    update_scrollbar(hwnd);
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

fn view() -> View {
    state().lock().unwrap().view
}

fn update_title(hwnd: HWND) {
    let title = match view() {
        View::Notes => format!("Todas las notas ({})", app().lock().unwrap().notes.len()),
        View::Settings => "Configuración y sincronización".to_string(),
    };
    let w = wide(&title);
    unsafe { SetWindowTextW(hwnd, w.as_ptr()) };
}

fn create() {
    let hinstance = { app().lock().unwrap().hinstance } as HINSTANCE;
    let class_name = wide(CLASS_NAME);
    let title = wide("Todas las notas");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            392,
            560,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        )
    };
    if !hwnd.is_null() {
        state().lock().unwrap().hwnd = hwnd as isize;
        // La barra de título oscura hay que pedirla ANTES de mostrar la
        // ventana, si no se ve un destello blanco.
        theme::apply_title_bar(hwnd);
        theme::apply_scrollbars(hwnd);
        crate::win::show_normal(hwnd);
    }
}

/// Algo cambió en las notas (se llama desde `save_all`): si la ventana
/// está abierta, se refresca — en cola, nunca en el acto, porque quien
/// avisa puede estar en el medio de cualquier cosa.
pub fn notify_changed() {
    let hwnd = STATE.get().map(|s| s.lock().unwrap().hwnd).unwrap_or(0);
    if hwnd != 0 {
        unsafe { PostMessageW(hwnd as HWND, WM_APP_REFRESH, 0, 0) };
    }
}

/// `true` si `hwnd` es esta ventana o algo adentro (para Ctrl+N).
pub fn owns(hwnd: HWND) -> bool {
    let me = STATE.get().map(|s| s.lock().unwrap().hwnd).unwrap_or(0);
    me != 0 && (hwnd as isize == me || unsafe { IsChild(me as HWND, hwnd) } != 0)
}

/// Cambió el tema: recolorear todo.
pub fn apply_theme() {
    let (hwnd, old_brush) = {
        let mut s = state().lock().unwrap();
        let old = s.search_brush;
        s.search_brush = if s.hwnd != 0 { (unsafe { CreateSolidBrush(theme::chrome().surface) }) as isize } else { 0 };
        (s.hwnd as HWND, old)
    };
    unsafe {
        if old_brush != 0 {
            DeleteObject(old_brush as HGDIOBJ);
        }
        if hwnd.is_null() {
            return;
        }
        theme::apply_title_bar(hwnd);
        theme::apply_scrollbars(hwnd);
        // La barra de título no se repinta sola al cambiarle el modo.
        SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
    }
}

// -----------------------------------------------------------------
// Datos
// -----------------------------------------------------------------

fn get_window_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    unsafe {
        let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0) as usize;
        let mut buf: Vec<u16> = vec![0u16; len + 1];
        SendMessageW(hwnd, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize);
        from_wide(&buf)
    }
}

/// El texto de la vista previa: todo lo que no es el título. Si la
/// nota tiene nombre propio, el cuerpo entero; si no, desde la segunda
/// línea (la primera ya se muestra como título).
pub(crate) fn preview_of(data: &NoteData) -> String {
    let skip = if data.title.trim().is_empty() { 1 } else { 0 };
    let body: String = data
        .text
        .lines()
        .skip(skip)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    body.chars().take(160).collect()
}

fn refresh_list(hwnd: HWND) {
    let edit = { state().lock().unwrap().edit as HWND };
    let query = get_window_text(edit).to_lowercase();

    let mut items: Vec<Item> = {
        let a = app().lock().unwrap();
        let mut v: Vec<(u32, Item)> = a
            .notes
            .values()
            .map(|nr| {
                (
                    nr.data.id,
                    Item {
                        id: nr.data.id,
                        hidden: nr.data.hidden,
                        locked: nr.data.locked,
                        color: nr.data.color,
                        pinned: nr.data.layer == Layer::AlwaysOnTop,
                        rolled: nr.data.rolled,
                        title: note::display_title(&nr.data),
                        preview: preview_of(&nr.data),
                    },
                )
            })
            .collect();
        v.sort_by_key(|(id, _)| *id);
        v.into_iter().map(|(_, it)| it).collect()
    };
    if !query.is_empty() {
        // Busca en todo: nombre y cuerpo.
        items.retain(|it| it.title.to_lowercase().contains(&query) || it.preview.to_lowercase().contains(&query));
    }

    let count = items.len();
    {
        let mut s = state().lock().unwrap();
        // Conservar la tarjeta seleccionada aunque cambie de lugar.
        let selected = if s.sel >= 0 { s.items.get(s.sel as usize).map(|it| it.id) } else { None };
        s.items = items;
        s.hover = -1;
        s.sel = selected.and_then(|id| s.items.iter().position(|it| it.id == id)).map(|i| i as i32).unwrap_or(-1);
        if count > 0 && s.sel < 0 {
            s.sel = 0;
        }
    }

    if !hwnd.is_null() {
        update_title(hwnd);
        update_scrollbar(hwnd);
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

// -----------------------------------------------------------------
// Layout
// -----------------------------------------------------------------

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    (rc.right, rc.bottom)
}

/// Dónde arranca la grilla (debajo del buscador, que no se desplaza).
fn grid_top() -> i32 {
    PAD + SEARCH_H + PAD
}

/// El botón de arriba: el engranaje, a la derecha del buscador; en la
/// configuración, "volver", a la izquierda.
fn top_btn_rect(cw: i32) -> RECT {
    if view() == View::Settings {
        RECT { left: PAD, top: PAD, right: PAD + SEARCH_H, bottom: PAD + SEARCH_H }
    } else {
        RECT { left: cw - PAD - SEARCH_H, top: PAD, right: cw - PAD, bottom: PAD + SEARCH_H }
    }
}

/// Alto de lo que se desplaza: la grilla o la configuración.
fn content_height(cw: i32) -> i32 {
    let s = state().lock().unwrap();
    match s.view {
        View::Notes => grid_layout(&s.items, cw).1,
        View::Settings => {
            drop(s);
            settings::height(cw)
        }
    }
}

fn search_rect(cw: i32) -> RECT {
    RECT { left: PAD, top: PAD, right: (cw - PAD - SEARCH_H - 8).max(PAD + 40), bottom: PAD + SEARCH_H }
}

/// Rectángulos de las tarjetas **relativos al origen de la grilla**
/// (sin el desplazamiento del scroll), más el alto total del contenido.
/// Cada fila es tan alta como su tarjeta más alta, y las más bajitas
/// (las enrolladas) quedan alineadas arriba — como en el diseño.
fn grid_layout(items: &[Item], client_w: i32) -> (Vec<RECT>, i32) {
    let avail = client_w - 2 * PAD;
    let card_w = ((avail - (COLS - 1) * GAP) / COLS).max(48);
    let mut rects = Vec::with_capacity(items.len());
    let mut y = 0;
    let mut i = 0usize;
    while i < items.len() {
        let end = (i + COLS as usize).min(items.len());
        let row_h = items[i..end]
            .iter()
            .map(|it| if it.rolled { CARD_H_ROLLED } else { CARD_H })
            .max()
            .unwrap_or(CARD_H);
        for (k, it) in items[i..end].iter().enumerate() {
            let x = PAD + (k as i32) * (card_w + GAP);
            let h = if it.rolled { CARD_H_ROLLED } else { CARD_H };
            rects.push(RECT { left: x, top: y, right: x + card_w, bottom: y + h });
        }
        y += row_h + GAP;
        i = end;
    }
    let total = if rects.is_empty() { 0 } else { y - GAP + PAD };
    (rects, total)
}

fn update_scrollbar(hwnd: HWND) {
    let (cw, ch) = client_size(hwnd);
    let total = content_height(cw);
    let view = (ch - grid_top()).max(1);
    let max_scroll = (total - view).max(0);
    let scroll = {
        let mut s = state().lock().unwrap();
        s.scroll = s.scroll.clamp(0, max_scroll);
        s.scroll
    };

    let mut si: SCROLLINFO = unsafe { std::mem::zeroed() };
    si.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
    si.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
    si.nMin = 0;
    si.nMax = (total - 1).max(0);
    si.nPage = view as u32;
    si.nPos = scroll;
    unsafe { SetScrollInfo(hwnd, SB_VERT, &si, 1) };
}

fn set_scroll(hwnd: HWND, value: i32) {
    let (cw, ch) = client_size(hwnd);
    let total = content_height(cw);
    let max_scroll = (total - (ch - grid_top()).max(1)).max(0);
    let next = value.clamp(0, max_scroll);
    let changed = {
        let mut s = state().lock().unwrap();
        let changed = s.scroll != next;
        s.scroll = next;
        changed
    };
    if changed {
        update_scrollbar(hwnd);
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

/// Tarjeta bajo (`x`, `y`) (coordenadas de cliente), su rectángulo en
/// pantalla de cliente, o -1.
fn hit_test(hwnd: HWND, x: i32, y: i32) -> (i32, RECT) {
    let none = (-1, RECT { left: 0, top: 0, right: 0, bottom: 0 });
    if y < grid_top() || view() != View::Notes {
        return none;
    }
    let (cw, _) = client_size(hwnd);
    let s = state().lock().unwrap();
    let (rects, _) = grid_layout(&s.items, cw);
    let off = grid_top() - s.scroll;
    for (i, r) in rects.iter().enumerate() {
        let rr = RECT { left: r.left, top: r.top + off, right: r.right, bottom: r.bottom + off };
        if x >= rr.left && x < rr.right && y >= rr.top && y < rr.bottom {
            return (i as i32, rr);
        }
    }
    none
}

fn in_rect(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Deja visible la tarjeta seleccionada (para las flechas del teclado).
fn scroll_into_view(hwnd: HWND, idx: i32) {
    if idx < 0 {
        return;
    }
    let (cw, ch) = client_size(hwnd);
    let (rects, _) = {
        let s = state().lock().unwrap();
        grid_layout(&s.items, cw)
    };
    let Some(r) = rects.get(idx as usize) else { return };
    let view = (ch - grid_top()).max(1);
    let scroll = { state().lock().unwrap().scroll };
    if r.top < scroll {
        set_scroll(hwnd, r.top - GAP);
    } else if r.bottom > scroll + view {
        set_scroll(hwnd, r.bottom - view + GAP);
    }
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

/// Rectángulo de esquinas redondeadas en un GraphicsPath de GDI+ (la
/// API no trae uno de fábrica, pero sí cuatro arcos de 90°).
unsafe fn add_round_rect(path: *mut GpPath, x: i32, y: i32, w: i32, h: i32, r: i32) {
    let d = (r * 2).min(w).min(h);
    GdipAddPathArcI(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArcI(path, x + w - d, y, d, d, 270.0, 90.0);
    GdipAddPathArcI(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
    GdipAddPathArcI(path, x, y + h - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
}

unsafe fn fill_round_rect(g: *mut GpGraphics, r: &RECT, radius: i32, argb_color: u32) {
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    add_round_rect(path, r.left, r.top, r.right - r.left, r.bottom - r.top, radius);
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(argb_color, &mut brush);
    GdipFillPath(g, brush as *mut GpBrush, path);
    GdipDeleteBrush(brush as *mut GpBrush);
    GdipDeletePath(path);
}

unsafe fn stroke_round_rect(g: *mut GpGraphics, r: &RECT, radius: i32, argb_color: u32, width: f32) {
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    add_round_rect(path, r.left, r.top, r.right - r.left, r.bottom - r.top, radius);
    let mut pen: *mut GpPen = null_mut();
    GdipCreatePen1(argb_color, width, UnitPixel, &mut pen);
    GdipDrawPath(g, pen, path);
    GdipDeletePen(pen);
    GdipDeletePath(path);
}

fn argb(a: u8, c: u32) -> u32 {
    (note::colorref_to_argb(c) & 0x00ff_ffff) | ((a as u32) << 24)
}

/// Mezcla dos COLORREF (0x00BBGGRR) canal por canal.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, i: u32| ((c >> (i * 8)) & 0xff) as f32;
    let out = |v: f32| (v.clamp(0.0, 255.0) as u32) & 0xff;
    let r = out(ch(a, 0) + (ch(b, 0) - ch(a, 0)) * t);
    let g = out(ch(a, 1) + (ch(b, 1) - ch(a, 1)) * t);
    let bl = out(ch(a, 2) + (ch(b, 2) - ch(a, 2)) * t);
    r | (g << 8) | (bl << 16)
}

unsafe fn round_pen(color: u32, width: f32) -> *mut GpPen {
    let mut pen: *mut GpPen = null_mut();
    GdipCreatePen1(color, width, UnitPixel, &mut pen);
    GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapFlat);
    pen
}

/// La lupa del buscador, como en el diseño.
unsafe fn draw_magnifier(g: *mut GpGraphics, cx: i32, cy: i32, color: u32) {
    let pen = round_pen(argb(0xff, color), 1.6);
    GdipDrawEllipseI(g, pen, cx - 5, cy - 6, 10, 10);
    GdipDrawLineI(g, pen, cx + 3, cy + 3, cx + 6, cy + 6);
    GdipDeletePen(pen);
}

/// El mismo pin del encabezado de la nota, en chiquito, para marcar
/// las que están en "siempre encima".
unsafe fn draw_pin(g: *mut GpGraphics, cx: i32, cy: i32, ink: u32) {
    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(argb(0xff, ink), &mut brush);
    GdipFillEllipseI(g, brush as *mut GpBrush, cx - 4, cy - 4, 8, 8);
    GdipDeleteBrush(brush as *mut GpBrush);
    let pen = round_pen(argb(0xff, ink), 1.6);
    GdipDrawLineI(g, pen, cx, cy + 4, cx, cy + 9);
    GdipDeletePen(pen);
}

/// Con el mod "Translucent Windows" de Windhawk la ventana es de vidrio,
/// y cada píxel se compone según su alfa: lo que pinta GDI (el fondo, el
/// cuadro de texto del buscador) lleva alfa 0 y se ve como vidrio
/// teñido, y lo de GDI+ lleva alfa entero y se ve macizo. La píldora del
/// buscador rodea al cuadro de texto: si no se compone igual que él,
/// adentro se ve un rectángulo de otro tono. Así que la barra de arriba
/// va toda "como GDI". (Sin el mod, el alfa no se usa.)
unsafe fn like_gdi(bits: *mut u32, w: i32, h: i32, r: &RECT) {
    GdiFlush();
    for y in r.top.max(0)..r.bottom.min(h) {
        for x in r.left.max(0)..r.right.min(w) {
            *bits.add((y * w + x) as usize) &= 0x00ff_ffff;
        }
    }
}

fn on_paint(hwnd: HWND) {
    let (cw, ch) = client_size(hwnd);
    let (ui_font, title_font, hover_btn, view, settings_hover, scroll) = {
        let s = state().lock().unwrap();
        (s.ui_font as HFONT, s.title_font as HFONT, s.hover_top_btn, s.view, s.settings_hover, s.scroll)
    };
    let c = theme::chrome();
    let (w, h) = (cw.max(1), ch.max(1));

    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        // Doble buffer: la grilla se repinta entera en cada scroll y en
        // cada cambio del buscador; pintando directo sobre el DC de la
        // ventana se vería el parpadeo de siempre. En un DIB, para poder
        // tocar el alfa (ver `like_gdi`).
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut std::ffi::c_void = null_mut();
        let bmp = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            EndPaint(hwnd, &ps);
            return;
        }
        let bits = bits as *mut u32;
        let mem = CreateCompatibleDC(hdc);
        let old_bmp = SelectObject(mem, bmp);

        let bg = CreateSolidBrush(c.window);
        let full = RECT { left: 0, top: 0, right: cw, bottom: ch };
        FillRect(mem, &full, bg);
        DeleteObject(bg);
        SetBkMode(mem, TRANSPARENT as i32);

        // La barra de arriba: buscador y engranaje, o volver y el título.
        let br = top_btn_rect(cw);
        let sr = search_rect(cw);
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(mem, &mut g) == Ok && !g.is_null() {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            if view == View::Notes {
                // Píldora del buscador (el EDIT real vive adentro).
                fill_round_rect(g, &sr, SEARCH_R, argb(0xff, c.surface));
            }
            fill_round_rect(g, &br, SEARCH_R, argb(0xff, if hover_btn { c.surface_hover } else { c.surface }));
            GdipDeleteGraphics(g);
        }
        if view == View::Notes {
            like_gdi(bits, w, h, &sr);
        }
        like_gdi(bits, w, h, &br);
        let mut g: *mut GpGraphics = null_mut();
        if view == View::Notes && GdipCreateFromHDC(mem, &mut g) == Ok && !g.is_null() {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            draw_magnifier(g, sr.left + 18, sr.top + SEARCH_H / 2, c.muted);
            GdipDeleteGraphics(g);
        }
        let glyph = if view == View::Notes { 0xE713 } else { 0xE72B }; // engranaje / volver
        flyout::draw_glyph(mem, flyout::icon_font(-16), glyph, &br, c.text);

        match view {
            View::Notes => draw_cards(mem, cw, ch, ui_font, title_font),
            View::Settings => {
                let mut title = RECT { left: br.right + 10, top: br.top, right: cw - PAD, bottom: br.bottom };
                let old = SelectObject(mem, title_font);
                SetTextColor(mem, c.text);
                let t = wide("Configuración y sincronización");
                DrawTextW(mem, t.as_ptr(), -1, &mut title, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS);
                SelectObject(mem, old);
                // Lo que se desplaza no pasa por encima de la barra.
                let clip = CreateRectRgn(0, grid_top(), cw, ch);
                SelectClipRgn(mem, clip);
                settings::paint(mem, cw, grid_top() - scroll, settings_hover);
                SelectClipRgn(mem, null_mut());
                DeleteObject(clip);
            }
        }

        BitBlt(hdc, 0, 0, cw, ch, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

unsafe fn draw_cards(hdc: HDC, cw: i32, ch: i32, ui_font: HFONT, title_font: HFONT) {
    let c = theme::chrome();
    let s = state().lock().unwrap();
    let (rects, _) = grid_layout(&s.items, cw);
    let off = grid_top() - s.scroll;

    if s.items.is_empty() {
        let edit = s.edit as HWND;
        drop(s);
        let searching = !get_window_text(edit).is_empty();
        let mut rc = RECT { left: PAD * 2, top: grid_top() + 24, right: cw - PAD * 2, bottom: ch };
        let old = SelectObject(hdc, ui_font);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, c.muted);
        let msg = wide(if searching {
            "Ninguna nota coincide con la búsqueda."
        } else {
            "Todavía no hay notas. Clic en el icono de la bandeja, o Ctrl+N, para crear una."
        });
        DrawTextW(hdc, msg.as_ptr(), -1, &mut rc, DT_CENTER | DT_WORDBREAK);
        SelectObject(hdc, old);
        return;
    }

    // La grilla se desplaza; el buscador no. Recortar evita que una
    // tarjeta pase por encima de la píldora al hacer scroll.
    let clip = CreateRectRgn(0, grid_top(), cw, ch);
    SelectClipRgn(hdc, clip);
    SetBkMode(hdc, TRANSPARENT as i32);

    for (i, r) in rects.iter().enumerate() {
        let rr = RECT { left: r.left, top: r.top + off, right: r.right, bottom: r.bottom + off };
        if rr.bottom < grid_top() || rr.top > ch {
            continue; // fuera de la vista
        }
        let it = &s.items[i];
        let (header, body, ink) = theme::note_colors(it.color);
        // Oculta (solo vive acá): la tarjeta, apagada.
        let faded = |col: u32| if it.hidden { mix(col, c.window, 0.5) } else { col };
        let ink = faded(ink);
        // La tarjeta que se está arrastrando queda "vacía" en su lugar.
        let dragged = s.dragging && i as i32 == s.press;

        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(hdc, &mut g) == Ok && !g.is_null() {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            if dragged {
                stroke_round_rect(g, &rr, CARD_R, argb(0x80, c.muted), 1.4);
                GdipDeleteGraphics(g);
                continue;
            }
            // En oscuro el cuerpo es casi negro: la tarjeta lleva el
            // color del encabezado, para que se reconozca de un vistazo.
            let fill = faded(if theme::is_dark() { mix(body, header, 0.55) } else { body });
            fill_round_rect(g, &rr, CARD_R, argb(0xff, fill));
            if i as i32 == s.sel {
                stroke_round_rect(g, &rr, CARD_R, argb(0xff, ink), 2.0);
            } else if i as i32 == s.hover {
                stroke_round_rect(g, &rr, CARD_R, argb(0x55, ink), 1.6);
            }
            if it.pinned {
                draw_pin(g, rr.right - 13, rr.top + 12, ink);
            }
            GdipDeleteGraphics(g);
        }

        let fill = faded(if theme::is_dark() { mix(body, header, 0.55) } else { body });
        let marks = it.pinned as i32 + it.hidden as i32 + it.locked as i32;
        // Marcas a la izquierda del pin: oculta, bloqueada.
        let mut x = rr.right - 20 - if it.pinned { 14 } else { 0 };
        for (on, glyph) in [(it.hidden, 0xED1Au16), (it.locked, 0xE72E)] {
            if on {
                flyout::draw_glyph(hdc, flyout::icon_font(-12), glyph, &RECT { left: x, top: rr.top + 4, right: x + 16, bottom: rr.top + 20 }, ink);
                x -= 14;
            }
        }
        let text_right = rr.right - 8 - marks * 14;
        let old = SelectObject(hdc, title_font);
        SetTextColor(hdc, ink);
        let mut title_rc = RECT { left: rr.left + 8, top: rr.top + 7, right: text_right, bottom: rr.top + 7 + 17 };
        let wtitle = wide(&it.title);
        DrawTextW(hdc, wtitle.as_ptr(), -1, &mut title_rc, DT_SINGLELINE | DT_END_ELLIPSIS | DT_LEFT);
        SelectObject(hdc, old);

        if !it.rolled && !it.preview.is_empty() {
            let old = SelectObject(hdc, ui_font);
            SetTextColor(hdc, mix(ink, fill, 0.35));
            let mut body_rc = RECT { left: rr.left + 8, top: rr.top + 27, right: rr.right - 8, bottom: rr.bottom - 6 };
            let wbody = wide(&it.preview);
            DrawTextW(hdc, wbody.as_ptr(), -1, &mut body_rc, DT_WORDBREAK | DT_END_ELLIPSIS | DT_LEFT);
            SelectObject(hdc, old);
        }
    }

    SelectClipRgn(hdc, null_mut());
    DeleteObject(clip);
}

// -----------------------------------------------------------------
// Interacción
// -----------------------------------------------------------------

fn set_sel(hwnd: HWND, idx: i32) {
    let changed = {
        let mut s = state().lock().unwrap();
        let changed = s.sel != idx;
        s.sel = idx;
        changed
    };
    if changed {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

fn set_hover(hwnd: HWND, idx: i32, theme_btn: bool) {
    let changed = {
        let mut s = state().lock().unwrap();
        let changed = s.hover != idx || s.hover_top_btn != theme_btn;
        s.hover = idx;
        s.hover_top_btn = theme_btn;
        changed
    };
    if changed {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

/// Mueve la selección por la grilla con el teclado (±1 en la fila,
/// ±COLS entre filas).
fn move_sel(delta: i32) {
    let hwnd = { state().lock().unwrap().hwnd as HWND };
    if hwnd.is_null() {
        return;
    }
    let next = {
        let s = state().lock().unwrap();
        if s.items.is_empty() {
            return;
        }
        (s.sel + delta).clamp(0, s.items.len() as i32 - 1)
    };
    set_sel(hwnd, next);
    scroll_into_view(hwnd, next);
}

/// La nota seleccionada: su número y si está oculta.
fn selected_note() -> Option<(u32, bool)> {
    let s = state().lock().unwrap();
    if s.sel < 0 {
        return None;
    }
    s.items.get(s.sel as usize).map(|it| (it.id, it.hidden))
}

fn selected_locked() -> bool {
    let s = state().lock().unwrap();
    s.sel >= 0 && s.items.get(s.sel as usize).is_some_and(|it| it.locked)
}

fn open_selected() {
    if let Some((id, _)) = selected_note() {
        note::open_by_id(id);
    }
}

fn rename_selected() {
    if let Some((id, _)) = selected_note() {
        let h = note::open_by_id(id);
        if !h.is_null() {
            crate::rename::begin(h);
        }
    }
}

fn show_card_menu(hwnd: HWND, idx: i32) {
    set_sel(hwnd, idx);
    let Some((target, hidden)) = selected_note() else { return };
    // Bloqueada: solo abrirla (se desbloquea con el candado de la nota).
    let locked = selected_locked();
    let off = if locked { MF_GRAYED | MF_DISABLED } else { 0 };
    let choice = unsafe {
        let menu = CreatePopupMenu();
        let open = wide("Abrir");
        let hide = wide("Ocultar");
        let rename = wide("Cambiar nombre\tF2");
        let delete = wide(if locked { "Eliminar nota (está bloqueada)" } else { "Eliminar nota" });
        AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
        if !hidden {
            AppendMenuW(menu, MF_STRING | off, MENU_HIDE, hide.as_ptr());
        }
        AppendMenuW(menu, MF_STRING | off, MENU_RENAME, rename.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING | off, MENU_DELETE, delete.as_ptr());
        SetMenuDefaultItem(menu, MENU_OPEN as u32, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let choice = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
        DestroyMenu(menu);
        choice as usize
    };
    match choice {
        MENU_OPEN => {
            note::open_by_id(target);
        }
        MENU_HIDE => note::hide_by_id(target),
        MENU_RENAME => rename_selected(),
        MENU_DELETE => note::delete_by_id(target, hwnd),
        _ => {}
    }
}

fn layout(hwnd: HWND) {
    let (cw, _) = client_size(hwnd);
    let edit = { state().lock().unwrap().edit as HWND };
    if edit.is_null() {
        return;
    }
    // El EDIT va adentro de la píldora dibujada, después de la lupa.
    let sr = search_rect(cw);
    let x = sr.left + 34;
    let w = (sr.right - 10 - x).max(10);
    let h = 18;
    unsafe { SetWindowPos(edit, null_mut(), x, PAD + (SEARCH_H - h) / 2, w, h, SWP_NOZORDER) };
}

// -----------------------------------------------------------------
// Arrastrar una tarjeta al escritorio
// -----------------------------------------------------------------

/// Dónde está el cursor, en pantalla, y si soltar ahí cuenta como
/// "soltar en el escritorio" (= fuera de esta ventana).
fn drop_point(hwnd: HWND) -> (POINT, bool) {
    let mut pt = POINT { x: 0, y: 0 };
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        GetCursorPos(&mut pt);
        GetWindowRect(hwnd, &mut rc);
    }
    let outside = !(pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom);
    (pt, outside)
}

fn begin_drag(hwnd: HWND) {
    let (color, title) = {
        let mut s = state().lock().unwrap();
        let idx = s.press;
        let Some(it) = s.items.get(idx as usize) else { return };
        let info = (it.color, it.title.clone());
        s.dragging = true;
        info
    };
    let grab = { state().lock().unwrap().grab };
    let (pt, outside) = drop_point(hwnd);
    crate::ghost::show(color, &title, pt.x - grab.x, pt.y - grab.y);
    crate::ghost::move_to(pt.x - grab.x, pt.y - grab.y, outside);
    unsafe {
        SetCursor(LoadCursorW(null_mut(), IDC_SIZEALL));
        InvalidateRect(hwnd, null(), 0);
    }
}

/// Termina el arrastre. `drop`: soltar (si cayó afuera) o cancelar.
fn end_drag(hwnd: HWND, drop: bool) {
    let (dragging, target, grab) = {
        let mut s = state().lock().unwrap();
        let info = (s.dragging, s.items.get(s.press.max(0) as usize).map(|it| it.id), s.grab);
        s.dragging = false;
        s.press = -1;
        info
    };
    if !dragging {
        return;
    }
    crate::ghost::hide();
    unsafe { InvalidateRect(hwnd, null(), 0) };
    let (pt, outside) = drop_point(hwnd);
    if let (true, true, Some(target)) = (drop, outside, target) {
        // La nota cae donde estaba el fantasma: su encabezado bajo el
        // cursor, igual que se la venía viendo.
        note::drop_by_id(target, pt.x - grab.x, pt.y - grab.y);
    }
}

fn on_lbuttondown(hwnd: HWND, x: i32, y: i32) {
    let (cw, _) = client_size(hwnd);
    if in_rect(&top_btn_rect(cw), x, y) {
        set_view(hwnd, if view() == View::Notes { View::Settings } else { View::Notes });
        return;
    }
    if view() == View::Settings {
        settings_click(hwnd, x, y);
        return;
    }
    let (idx, card) = hit_test(hwnd, x, y);
    unsafe { SetFocus(hwnd_edit()) };
    if idx < 0 {
        return;
    }
    set_sel(hwnd, idx);
    {
        let mut s = state().lock().unwrap();
        s.press = idx;
        s.press_pt = POINT { x, y };
        // Dónde agarró el usuario la tarjeta, trasladado al fantasma:
        // se lo sostiene más o menos del mismo lugar, pero siempre del
        // encabezado (así la nota cae con la barra bajo el cursor).
        let gx = ((x - card.left) * crate::ghost::W / (card.right - card.left).max(1)).clamp(16, crate::ghost::W - 16);
        s.grab = POINT { x: gx, y: 15 };
        s.dragging = false;
    }
    unsafe { SetCapture(hwnd) };
}

fn on_mousemove(hwnd: HWND, x: i32, y: i32) {
    let (press, press_pt, dragging) = {
        let s = state().lock().unwrap();
        (s.press, s.press_pt, s.dragging)
    };
    if press >= 0 && unsafe { GetCapture() } == hwnd {
        if !dragging {
            let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            if (x - press_pt.x).abs() > dx || (y - press_pt.y).abs() > dy {
                begin_drag(hwnd);
            }
        } else {
            let grab = { state().lock().unwrap().grab };
            let (pt, outside) = drop_point(hwnd);
            crate::ghost::move_to(pt.x - grab.x, pt.y - grab.y, outside);
            unsafe {
                SetCursor(LoadCursorW(null_mut(), if outside { IDC_SIZEALL } else { IDC_NO }));
            }
        }
        return;
    }
    let (cw, _) = client_size(hwnd);
    let on_btn = in_rect(&top_btn_rect(cw), x, y);
    set_hover(hwnd, if on_btn { -1 } else { hit_test(hwnd, x, y).0 }, on_btn);
    let over = if on_btn { settings::Hit::None } else { settings_hit(hwnd, x, y) };
    let changed = {
        let mut s = state().lock().unwrap();
        std::mem::replace(&mut s.settings_hover, over) != over
    };
    if changed {
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
    // Sin TrackMouseEvent nunca llega WM_MOUSELEAVE y la tarjeta se
    // queda resaltada al salir de la ventana.
    unsafe {
        let mut tme: TRACKMOUSEEVENT = std::mem::zeroed();
        tme.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
        tme.dwFlags = TME_LEAVE;
        tme.hwndTrack = hwnd;
        TrackMouseEvent(&mut tme);
    }
}

/// Qué hay de la configuración en (`x`, `y`) (coordenadas de cliente).
fn settings_hit(hwnd: HWND, x: i32, y: i32) -> settings::Hit {
    if view() != View::Settings || y < grid_top() {
        return settings::Hit::None;
    }
    let (cw, _) = client_size(hwnd);
    let scroll = state().lock().unwrap().scroll;
    settings::hit(cw, x, y - grid_top() + scroll)
}

fn settings_click(hwnd: HWND, x: i32, y: i32) {
    let hit = settings_hit(hwnd, x, y);
    if hit == settings::Hit::None {
        return;
    }
    settings::click(hit);
    // Puede haber cambiado lo que se muestra (y cuánto mide).
    if state().lock().unwrap().hwnd == hwnd as isize {
        update_scrollbar(hwnd);
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

fn on_lbuttonup(hwnd: HWND) {
    let dragging = { state().lock().unwrap().dragging };
    if dragging {
        end_drag(hwnd, true);
    } else {
        state().lock().unwrap().press = -1;
    }
    // Después de limpiar el estado: ReleaseCapture manda
    // WM_CAPTURECHANGED, que cancela cualquier arrastre en curso.
    unsafe { ReleaseCapture() };
}

// -----------------------------------------------------------------
// Ciclo de vida
// -----------------------------------------------------------------

fn hwnd_edit() -> HWND {
    state().lock().unwrap().edit as HWND
}

fn on_create(hwnd: HWND) {
    let hinstance = { app().lock().unwrap().hinstance } as HINSTANCE;
    let face = wide("Segoe UI");
    let make_font = |weight: i32, height: i32| unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH as u32) | (FF_DONTCARE as u32),
            face.as_ptr(),
        )
    };
    let ui_font = make_font(FW_NORMAL as i32, -13);
    let title_font = make_font(FW_SEMIBOLD as i32, -13);
    let brush = unsafe { CreateSolidBrush(theme::chrome().surface) };

    let edit = unsafe {
        let edit_class = wide("EDIT");
        // Sin WS_EX_CLIENTEDGE: el marco 3D de Windows 95 no pinta
        // nada al lado de la píldora del diseño. El fondo se resuelve
        // en WM_CTLCOLOREDIT.
        let edit = CreateWindowExW(
            0,
            edit_class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | (ES_AUTOHSCROLL as u32),
            0,
            0,
            10,
            10,
            hwnd,
            ID_SEARCH as HMENU,
            hinstance,
            null(),
        );
        SendMessageW(edit, WM_SETFONT, ui_font as usize, 1);
        edit
    };

    // Subclassing del EDIT: para el texto de ayuda ("Buscar notas…")
    // y para que Enter/Esc/flechas/F2 manejen la grilla en vez de
    // pitar. El wndproc original se guarda en GWLP_USERDATA del propio
    // EDIT, no en el estado de acá: los últimos mensajes del EDIT
    // (WM_DESTROY, WM_NCDESTROY) llegan cuando la ventana madre ya
    // pasó por su WM_DESTROY.
    unsafe {
        let prev = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        SetWindowLongPtrW(edit, GWLP_USERDATA, prev);
    }

    {
        let mut s = state().lock().unwrap();
        s.edit = edit as isize;
        s.search_brush = brush as isize;
        s.ui_font = ui_font as isize;
        s.title_font = title_font as isize;
        s.scroll = 0;
        s.sel = -1;
        s.hover = -1;
        s.press = -1;
        s.dragging = false;
    }

    layout(hwnd);
    refresh_list(hwnd);
    unsafe { SetFocus(edit) };
}

/// En WM_NCDESTROY (el último mensaje, con los hijos ya destruidos),
/// no en WM_DESTROY: si se borraba antes, el EDIT todavía recibía
/// mensajes y buscaba su estado en una ventana a medio desarmar.
fn on_ncdestroy() {
    crate::ghost::hide();
    let mut s = state().lock().unwrap();
    unsafe {
        for h in [s.search_brush, s.ui_font, s.title_font] {
            if h != 0 {
                DeleteObject(h as HGDIOBJ);
            }
        }
    }
    s.hwnd = 0;
    s.edit = 0;
    s.search_brush = 0;
    s.ui_font = 0;
    s.title_font = 0;
    s.items.clear();
    s.sel = -1;
    s.hover = -1;
    s.scroll = 0;
    s.press = -1;
    s.dragging = false;
    s.view = View::Notes;
    s.notes_scroll = 0;
    s.settings_hover = settings::Hit::None;
    drop(s);
    settings::free_fonts();
}

/// Wndproc del EDIT del buscador (ver `on_create`).
unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let original: WNDPROC = std::mem::transmute(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    let parent = GetParent(hwnd);

    match msg {
        WM_KEYDOWN => match wparam as u16 {
            VK_RETURN => {
                open_selected();
                return 0;
            }
            VK_ESCAPE => {
                let dragging = { state().lock().unwrap().dragging };
                if dragging {
                    end_drag(parent, false);
                    ReleaseCapture();
                } else if !parent.is_null() {
                    DestroyWindow(parent);
                }
                return 0;
            }
            VK_F2 => {
                rename_selected();
                return 0;
            }
            VK_DOWN => {
                move_sel(COLS);
                return 0;
            }
            VK_UP => {
                move_sel(-COLS);
                return 0;
            }
            VK_TAB => {
                move_sel(if GetKeyState(VK_SHIFT as i32) < 0 { -1 } else { 1 });
                return 0;
            }
            _ => {}
        },
        // Sin esto el EDIT hace "beep" con Enter, Esc y Tab.
        WM_CHAR if wparam == 13 || wparam == 27 || wparam == 9 => return 0,
        WM_PAINT => {
            let r = CallWindowProcW(original, hwnd, msg, wparam, lparam);
            if GetWindowTextLengthW(hwnd) == 0 {
                let font = { state().lock().unwrap().ui_font as HFONT };
                let hdc = GetDC(hwnd);
                let old = SelectObject(hdc, font);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, theme::chrome().muted);
                let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(hwnd, &mut rc);
                rc.left += 1;
                let hint = wide("Buscar notas…");
                DrawTextW(hdc, hint.as_ptr(), -1, &mut rc, DT_SINGLELINE | DT_VCENTER | DT_LEFT);
                SelectObject(hdc, old);
                ReleaseDC(hwnd, hdc);
            }
            return r;
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        }
        _ => {}
    }
    CallWindowProcW(original, hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    match msg {
        WM_CREATE => {
            on_create(hwnd);
            0
        }
        WM_SIZE => {
            layout(hwnd);
            update_scrollbar(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_ERASEBKGND => 1, // todo el fondo lo pinta WM_PAINT (doble buffer)
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_CTLCOLOREDIT => {
            let brush = { state().lock().unwrap().search_brush };
            let c = theme::chrome();
            SetBkColor(wparam as HDC, c.surface);
            SetTextColor(wparam as HDC, c.text);
            brush as LRESULT
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as usize;
            let code = ((wparam >> 16) & 0xffff) as u32;
            if id == ID_SEARCH && code == EN_CHANGE {
                state().lock().unwrap().scroll = 0;
                refresh_list(hwnd);
            }
            0
        }
        WM_APP_REFRESH => {
            // Durante un arrastre la lista no se toca (el índice de la
            // tarjeta en la mano tiene que seguir siendo el mismo).
            if !state().lock().unwrap().dragging {
                refresh_list(hwnd);
            }
            0
        }
        WM_MOUSEMOVE => {
            on_mousemove(hwnd, x, y);
            0
        }
        WM_MOUSELEAVE => {
            set_hover(hwnd, -1, false);
            if std::mem::replace(&mut state().lock().unwrap().settings_hover, settings::Hit::None) != settings::Hit::None {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_SETCURSOR => {
            let (dragging, over, on_btn) = {
                let s = state().lock().unwrap();
                (s.dragging, s.settings_hover, s.hover_top_btn)
            };
            if dragging {
                return 1; // el cursor lo maneja on_mousemove
            }
            if (over != settings::Hit::None || on_btn) && (lparam & 0xffff) as u32 == HTCLIENT {
                SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_KEYDOWN if wparam as u16 == VK_ESCAPE && view() == View::Settings => {
            set_view(hwnd, View::Notes);
            0
        }
        WM_LBUTTONDOWN => {
            on_lbuttondown(hwnd, x, y);
            0
        }
        WM_LBUTTONUP => {
            on_lbuttonup(hwnd);
            0
        }
        WM_CAPTURECHANGED => {
            // Otra ventana se quedó con el mouse (o Esc): se cancela.
            end_drag(hwnd, false);
            state().lock().unwrap().press = -1;
            0
        }
        WM_LBUTTONDBLCLK => {
            let (idx, _) = hit_test(hwnd, x, y);
            if idx >= 0 {
                set_sel(hwnd, idx);
                open_selected();
            } else if view() == View::Settings {
                // El segundo clic de un doble clic también cuenta (un
                // interruptor tocado dos veces vuelve a donde estaba).
                settings_click(hwnd, x, y);
            }
            0
        }
        WM_RBUTTONUP => {
            let (idx, _) = hit_test(hwnd, x, y);
            if idx >= 0 {
                show_card_menu(hwnd, idx);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as i16 as i32;
            let scroll = { state().lock().unwrap().scroll };
            set_scroll(hwnd, scroll - delta / 120 * 48);
            0
        }
        WM_VSCROLL => {
            let (_, ch) = client_size(hwnd);
            let page = (ch - grid_top()).max(1);
            let scroll = { state().lock().unwrap().scroll };
            let next = match (wparam & 0xffff) as i32 {
                SB_LINEUP => scroll - 24,
                SB_LINEDOWN => scroll + 24,
                SB_PAGEUP => scroll - page,
                SB_PAGEDOWN => scroll + page,
                SB_THUMBTRACK | SB_THUMBPOSITION => {
                    let mut si: SCROLLINFO = std::mem::zeroed();
                    si.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
                    si.fMask = SIF_TRACKPOS;
                    GetScrollInfo(hwnd, SB_VERT, &mut si);
                    si.nTrackPos
                }
                _ => scroll,
            };
            set_scroll(hwnd, next);
            0
        }
        WM_ACTIVATE => {
            // Refresca al volver a esta ventana (por ejemplo con
            // Alt+Tab), no todo el tiempo: antes había un timer que
            // rehacía la lista entera cada 1 s sin ningún control de
            // parpadeo, y visiblemente titilaba todo el rato.
            if (wparam & 0xffff) != 0 {
                refresh_list(hwnd);
            }
            0
        }
        WM_NCDESTROY => {
            on_ncdestroy();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
