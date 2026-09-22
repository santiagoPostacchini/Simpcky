//! Ventana de una nota individual: creación, dibujo del encabezado,
//! arrastre, menú "⋯", enrollado (manual/auto) y autoguardado.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Controls::EM_SETRECT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{app, save_all};
use crate::persist::{Layer, NoteData, RollMode};
use crate::win::{from_wide, rgb, wide};

pub const HEADER_H: i32 = 44;
const RICHEDIT_ID: usize = 100;
const TIMER_HOVER: usize = 1;
const TIMER_AUTOSAVE: usize = 2;
const HOVER_POLL_MS: u32 = 150;
const AUTOSAVE_DEBOUNCE_MS: u32 = 600;
/// Margen interno del cuerpo de texto respecto del borde de la nota,
/// para que el texto no arranque pegado al canto (como en el diseño).
const TEXT_PAD_X: i32 = 14;
const TEXT_PAD_Y: i32 = 12;

// RichEdit (Msftedit.dll) no viene en windows-sys: son solo un puñado de
// constantes de Winuser/Richedit.h, así que se definen a mano.
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;
const EM_SETCHARFORMAT: u32 = WM_USER + 68;
const SCF_SELECTION: usize = 0x0001;
const SCF_ALL: usize = 0x0004;
const CFM_COLOR: u32 = 0x4000_0000;

/// Réplica a mano de `CHARFORMAT2W` (Richedit.h): mismo orden y tipo de
/// campo que el struct real de Win32, para que el layout en memoria
/// coincida byte a byte. Solo se usa para forzar el color del texto
/// (`dwMask = CFM_COLOR`), el resto de los campos van a cero.
#[repr(C)]
struct CharFormat2W {
    cb_size: u32,
    dw_mask: u32,
    dw_effects: u32,
    y_height: i32,
    y_offset: i32,
    cr_text_color: u32,
    b_char_set: u8,
    b_pitch_and_family: u8,
    sz_face_name: [u16; 32],
    w_weight: u16,
    s_spacing: i16,
    cr_back_color: u32,
    lcid: u32,
    dw_reserved: u32,
    s_style: i16,
    w_kerning: u16,
    b_underline_type: u8,
    b_animation: u8,
    b_rev_author: u8,
    b_reserved1: u8,
}

const ID_COLOR_BASE: u32 = 2000;
const ID_ROLL_MANUAL: u32 = 2100;
const ID_ROLL_AUTO: u32 = 2101;
const ID_TOGGLE_PIN: u32 = 2102;
const ID_DELETE_NOTE: u32 = 2103;

/// (encabezado, cuerpo, tinta) para cada color de la paleta.
pub const PALETTE: [(u32, u32, u32); 6] = [
    (rgb(0xFD, 0xE6, 0x8A), rgb(0xFE, 0xF9, 0xC3), rgb(0x78, 0x35, 0x0F)), // amarillo
    (rgb(0xFB, 0xCF, 0xE8), rgb(0xFC, 0xE7, 0xF3), rgb(0x83, 0x18, 0x43)), // rosa
    (rgb(0xBB, 0xF7, 0xD0), rgb(0xDC, 0xFC, 0xE7), rgb(0x06, 0x5F, 0x46)), // verde
    (rgb(0xBF, 0xDB, 0xFE), rgb(0xDB, 0xEA, 0xFE), rgb(0x1E, 0x3A, 0x8A)), // azul
    (rgb(0xE9, 0xD5, 0xFF), rgb(0xF3, 0xE8, 0xFF), rgb(0x58, 0x1C, 0x87)), // morado
    (rgb(0xE2, 0xE8, 0xF0), rgb(0xF1, 0xF5, 0xF9), rgb(0x33, 0x41, 0x55)), // gris
];

const COLOR_NAMES: [&str; 6] = ["Amarillo", "Rosa", "Verde", "Azul", "Morado", "Gris"];

fn palette_entry(idx: u8) -> (u32, u32, u32) {
    PALETTE[(idx as usize) % PALETTE.len()]
}

const NOTE_CLASS: &str = "SimpckyNote";

/// Registra la clase de ventana de las notas. Llamar una sola vez al
/// arrancar, antes de crear la primera nota.
pub fn register_class(hinstance: HINSTANCE) {
    unsafe {
        let class_name = wide(NOTE_CLASS);
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
            lpfnWndProc: Some(note_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: cursor,
            hbrBackground: null_mut(), // pintamos todo nosotros
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

/// Carga Msftedit.dll (RichEdit 5.0). Llamar una sola vez al arrancar,
/// antes de crear la primera nota.
pub fn load_richedit_library() {
    unsafe {
        let lib = wide("Msftedit.dll");
        windows_sys::Win32::System::LibraryLoader::LoadLibraryW(lib.as_ptr());
    }
}

/// Crea la ventana de una nota a partir de sus datos (nueva o cargada
/// desde disco) y la registra en el estado global.
pub fn create_note(data: NoteData) -> HWND {
    let id = data.id;
    let hinstance = {
        let mut a = app().lock().unwrap();
        let hinstance = a.hinstance;
        a.notes.insert(
            id,
            crate::app::NoteRuntime {
                data: data.clone(),
                hwnd: 0,
                edit: 0,
            },
        );
        hinstance
    };

    let class_name = wide(NOTE_CLASS);
    let h = if data.rolled { HEADER_H } else { data.h };
    let mut ex_style = WS_EX_TOOLWINDOW;
    if data.layer == Layer::AlwaysOnTop {
        ex_style |= WS_EX_TOPMOST;
    }

    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            null(),
            WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
            data.x,
            data.y,
            data.w,
            h,
            null_mut(),
            null_mut(),
            hinstance as HINSTANCE,
            id as usize as *const c_void,
        )
    };

    if hwnd.is_null() {
        app().lock().unwrap().notes.remove(&id);
    } else {
        // WS_POPUP no entra en el redondeo automático de esquinas de
        // Windows 11 (eso solo lo aplica a ventanas "normales" con
        // caption), así que se pide a mano; si no, la nota se ve como
        // un rectángulo a secas en vez de una tarjeta.
        let pref: i32 = DWMWCP_ROUND;
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &pref as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }
    hwnd
}

fn note_id(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32 }
}

fn app_font() -> HFONT {
    let mut a = app().lock().unwrap();
    if a.font == 0 {
        let face = wide("Segoe UI");
        let f = unsafe {
            CreateFontW(
                -16,
                0,
                0,
                0,
                FW_NORMAL as i32,
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
        a.font = f as isize;
    }
    a.font as HFONT
}

fn create_richedit(parent: HWND, hinstance: HINSTANCE, w: i32, content_h: i32, text: &str) -> HWND {
    let class = wide("RICHEDIT50W");
    let txt = wide(text);
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            txt.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | (ES_MULTILINE as u32) | (ES_AUTOVSCROLL as u32) | (ES_WANTRETURN as u32),
            0,
            HEADER_H,
            w,
            content_h.max(0),
            parent,
            RICHEDIT_ID as HMENU,
            hinstance,
            null(),
        )
    }
}

fn apply_body_style(edit: HWND, color_idx: u8) {
    let (_, body, ink) = palette_entry(color_idx);
    unsafe {
        SendMessageW(edit, EM_SETBKGNDCOLOR, 0, body as isize);
        SendMessageW(edit, WM_SETFONT, app_font() as usize, 1);
    }
    // El RichEdit no hereda el color de texto del tema del sistema: si
    // no se fuerza acá, el texto puede quedar con muy poco contraste
    // sobre el pastel del cuerpo. Se aplica a lo ya escrito (SCF_ALL) y
    // a lo que se escriba de ahora en más (SCF_SELECTION, con el caret
    // vacío equivale a fijar el formato por defecto).
    let mut cf: CharFormat2W = unsafe { std::mem::zeroed() };
    cf.cb_size = std::mem::size_of::<CharFormat2W>() as u32;
    cf.dw_mask = CFM_COLOR;
    cf.cr_text_color = ink;
    unsafe {
        SendMessageW(edit, EM_SETCHARFORMAT, SCF_ALL, &cf as *const CharFormat2W as isize);
        SendMessageW(edit, EM_SETCHARFORMAT, SCF_SELECTION, &cf as *const CharFormat2W as isize);
    }
}

/// Deja aire entre el texto y el borde de la nota (el RichEdit por
/// defecto arranca a escribir pegado al canto).
fn apply_text_padding(edit: HWND, w: i32, content_h: i32) {
    let mut rc = RECT {
        left: TEXT_PAD_X,
        top: TEXT_PAD_Y,
        right: (w - TEXT_PAD_X).max(TEXT_PAD_X),
        bottom: (content_h - TEXT_PAD_Y).max(TEXT_PAD_Y),
    };
    unsafe {
        SendMessageW(edit, EM_SETRECT as u32, 0, &mut rc as *mut RECT as isize);
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        "Nota".to_string()
    } else {
        line.to_string()
    }
}

// -----------------------------------------------------------------
// Layout del encabezado
// -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Chevron,
    Ellipsis,
    Drag,
    None,
}

struct HeaderLayout {
    dot: RECT,
    pin: RECT,
    chevron: RECT,
    ellipsis: RECT,
    title_rect: RECT,
}

impl HeaderLayout {
    fn new(width: i32) -> Self {
        let cy = HEADER_H / 2;
        let dot = RECT { left: 14, top: cy - 8, right: 30, bottom: cy + 8 };
        let ellipsis = RECT { left: width - 46, top: 0, right: width - 14, bottom: HEADER_H };
        let chevron = RECT { left: width - 82, top: 0, right: width - 50, bottom: HEADER_H };
        let pin = RECT { left: width - 118, top: 0, right: width - 86, bottom: HEADER_H };
        let title_rect = RECT { left: dot.right + 10, top: 0, right: (pin.left - 8).max(dot.right + 10), bottom: HEADER_H };
        HeaderLayout { dot, pin, chevron, ellipsis, title_rect }
    }

    fn hit(&self, x: i32, y: i32) -> Hit {
        let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        if inside(&self.chevron) {
            Hit::Chevron
        } else if inside(&self.ellipsis) {
            Hit::Ellipsis
        } else if y < HEADER_H {
            Hit::Drag
        } else {
            Hit::None
        }
    }
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

/// GDI clásico no suaviza líneas ni círculos (`Polyline`/`Ellipse` salen
/// dentadas a este tamaño): por eso los íconos del encabezado se dibujan
/// con GDI+ sobre el mismo HDC, con antialiasing y puntas/uniones
/// redondeadas — el mismo `gdiplus.dll` que trae Windows desde XP, sin
/// sumar ninguna dependencia ni un gramo de peso al binario.
unsafe fn draw_header_icons(hdc: HDC, layout: &HeaderLayout, ink: u32, pinned: bool, rolled: bool) {
    let mut graphics: *mut GpGraphics = null_mut();
    if GdipCreateFromHDC(hdc, &mut graphics) != Ok || graphics.is_null() {
        return;
    }
    GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);

    let ink_argb = colorref_to_argb(ink);
    let mut pen: *mut GpPen = null_mut();
    GdipCreatePen1(ink_argb, 1.8, UnitPixel, &mut pen);
    GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapFlat);
    GdipSetPenLineJoin(pen, LineJoinRound);

    let mut brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(ink_argb, &mut brush);
    let brush = brush as *mut GpBrush;

    // Punto de color
    let d = &layout.dot;
    GdipFillEllipseI(graphics, brush, d.left, d.top, d.right - d.left, d.bottom - d.top);

    // Fijar (relleno si está siempre encima, contorno si no)
    let pcx = (layout.pin.left + layout.pin.right) / 2;
    let pcy = HEADER_H / 2 - 3;
    let r = 6;
    if pinned {
        GdipFillEllipseI(graphics, brush, pcx - r, pcy - r, r * 2, r * 2);
    } else {
        GdipDrawEllipseI(graphics, pen, pcx - r, pcy - r, r * 2, r * 2);
    }
    GdipDrawLineI(graphics, pen, pcx, pcy + r, pcx, pcy + r + 7);

    // Chevron: abajo si está expandida, hacia la derecha si está enrollada
    let ccx = (layout.chevron.left + layout.chevron.right) / 2;
    let ccy = HEADER_H / 2;
    let chevron_pts = if rolled {
        [Point { X: ccx - 4, Y: ccy - 6 }, Point { X: ccx + 4, Y: ccy }, Point { X: ccx - 4, Y: ccy + 6 }]
    } else {
        [Point { X: ccx - 6, Y: ccy - 4 }, Point { X: ccx, Y: ccy + 4 }, Point { X: ccx + 6, Y: ccy - 4 }]
    };
    GdipDrawLinesI(graphics, pen, chevron_pts.as_ptr(), 3);

    // Ellipsis "⋯"
    let ecx = (layout.ellipsis.left + layout.ellipsis.right) / 2;
    let ecy = HEADER_H / 2;
    for dx in [-8, 0, 8] {
        GdipFillEllipseI(graphics, brush, ecx + dx - 3, ecy - 3, 6, 6);
    }

    GdipDeleteBrush(brush);
    GdipDeletePen(pen);
    GdipDeleteGraphics(graphics);
}

/// `COLORREF` (0x00BBGGRR, lo que usa GDI) → ARGB opaco (0xAARRGGBB,
/// lo que espera GDI+).
fn colorref_to_argb(c: u32) -> u32 {
    let r = c & 0xff;
    let g = (c >> 8) & 0xff;
    let b = (c >> 16) & 0xff;
    0xff000000 | (r << 16) | (g << 8) | b
}

fn on_paint(hwnd: HWND) {
    let id = note_id(hwnd);
    let (color, rolled, pinned, title) = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => (nr.data.color, nr.data.rolled, nr.data.layer == Layer::AlwaysOnTop, first_line(&nr.data.text)),
            None => return,
        }
    };
    let (header_color, _, ink) = palette_entry(color);

    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        let layout = HeaderLayout::new(width);

        let header_rect = RECT { left: 0, top: 0, right: width, bottom: HEADER_H };
        let brush = CreateSolidBrush(header_color);
        FillRect(hdc, &header_rect, brush);
        DeleteObject(brush);

        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, ink);

        draw_header_icons(hdc, &layout, ink, pinned, rolled);

        if rolled {
            let mut text_rc = layout.title_rect;
            let wtext = wide(&title);
            DrawTextW(hdc, wtext.as_ptr(), -1, &mut text_rc, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_LEFT);
        }

        EndPaint(hwnd, &ps);
    }
}

// -----------------------------------------------------------------
// Enrollado (manual/auto)
// -----------------------------------------------------------------

fn apply_rolled_state(hwnd: HWND, edit: HWND, w: i32, full_h: i32, rolled: bool) {
    let new_h = if rolled { HEADER_H } else { full_h };
    unsafe {
        SetWindowPos(hwnd, null_mut(), 0, 0, w, new_h, SWP_NOMOVE | SWP_NOZORDER);
        if !edit.is_null() {
            ShowWindow(edit, if rolled { SW_HIDE } else { SW_SHOW });
        }
        InvalidateRect(hwnd, null(), 1);
    }
}

fn toggle_roll_manual(hwnd: HWND) {
    let id = note_id(hwnd);
    let change = {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) if nr.data.roll_mode == RollMode::Manual => {
                nr.data.rolled = !nr.data.rolled;
                Some((nr.data.rolled, nr.data.w, nr.data.h, nr.edit as HWND))
            }
            _ => None,
        }
    };
    if let Some((rolled, w, h, edit)) = change {
        apply_rolled_state(hwnd, edit, w, h, rolled);
        save_all();
    }
}

fn handle_hover_tick(hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let under = unsafe { WindowFromPoint(pt) };
    let hovered = under == hwnd || unsafe { IsChild(hwnd, under) != 0 };
    let id = note_id(hwnd);

    let change = {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) if nr.data.roll_mode == RollMode::Auto && hovered && nr.data.rolled => {
                nr.data.rolled = false;
                Some((false, nr.data.w, nr.data.h, nr.edit as HWND))
            }
            Some(nr) if nr.data.roll_mode == RollMode::Auto && !hovered && !nr.data.rolled => {
                nr.data.rolled = true;
                Some((true, nr.data.w, nr.data.h, nr.edit as HWND))
            }
            _ => None,
        }
    };
    if let Some((rolled, w, h, edit)) = change {
        apply_rolled_state(hwnd, edit, w, h, rolled);
    }
}

fn set_roll_mode(hwnd: HWND, mode: RollMode) {
    let id = note_id(hwnd);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.data.roll_mode = mode;
        }
    }
    unsafe {
        match mode {
            RollMode::Auto => {
                SetTimer(hwnd, TIMER_HOVER, HOVER_POLL_MS, None);
            }
            RollMode::Manual => {
                KillTimer(hwnd, TIMER_HOVER);
            }
        }
    }
    save_all();
}

// -----------------------------------------------------------------
// Menú "⋯" de la nota
// -----------------------------------------------------------------

fn show_note_menu(hwnd: HWND) {
    let id = note_id(hwnd);
    let (color, roll_mode, pinned) = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => (nr.data.color, nr.data.roll_mode, nr.data.layer == Layer::AlwaysOnTop),
            None => return,
        }
    };

    unsafe {
        let menu = CreatePopupMenu();
        let color_menu = CreatePopupMenu();
        let mut color_labels = Vec::with_capacity(COLOR_NAMES.len());
        for (i, name) in COLOR_NAMES.iter().enumerate() {
            let flags = MF_STRING | if color as usize == i { MF_CHECKED } else { MF_UNCHECKED };
            color_labels.push(wide(name));
            AppendMenuW(color_menu, flags, (ID_COLOR_BASE as usize) + i, color_labels[i].as_ptr());
        }
        let color_label = wide("Color");
        AppendMenuW(menu, MF_POPUP, color_menu as usize, color_label.as_ptr());

        let roll_menu = CreatePopupMenu();
        let manual_label = wide("Manual");
        let auto_label = wide("Auto");
        AppendMenuW(
            roll_menu,
            MF_STRING | if roll_mode == RollMode::Manual { MF_CHECKED } else { MF_UNCHECKED },
            ID_ROLL_MANUAL as usize,
            manual_label.as_ptr(),
        );
        AppendMenuW(
            roll_menu,
            MF_STRING | if roll_mode == RollMode::Auto { MF_CHECKED } else { MF_UNCHECKED },
            ID_ROLL_AUTO as usize,
            auto_label.as_ptr(),
        );
        let roll_label = wide("Enrollar");
        AppendMenuW(menu, MF_POPUP, roll_menu as usize, roll_label.as_ptr());

        let pin_label = wide("Siempre encima");
        AppendMenuW(
            menu,
            MF_STRING | if pinned { MF_CHECKED } else { MF_UNCHECKED },
            ID_TOGGLE_PIN as usize,
            pin_label.as_ptr(),
        );

        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        let delete_label = wide("Eliminar nota");
        AppendMenuW(menu, MF_STRING, ID_DELETE_NOTE as usize, delete_label.as_ptr());

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_LEFTALIGN, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}

fn set_color(hwnd: HWND, idx: u8) {
    let id = note_id(hwnd);
    let edit = {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) => {
                nr.data.color = idx;
                nr.edit as HWND
            }
            None => return,
        }
    };
    apply_body_style(edit, idx);
    unsafe { InvalidateRect(hwnd, null(), 1) };
    save_all();
}

fn toggle_pin(hwnd: HWND) {
    let id = note_id(hwnd);
    let now_pinned = {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) => {
                let pinned = nr.data.layer != Layer::AlwaysOnTop;
                nr.data.layer = if pinned { Layer::AlwaysOnTop } else { Layer::Normal };
                pinned
            }
            None => return,
        }
    };
    unsafe {
        let insert_after: HWND = if now_pinned { HWND_TOPMOST } else { HWND_NOTOPMOST };
        SetWindowPos(hwnd, insert_after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        InvalidateRect(hwnd, null(), 1);
    }
    save_all();
}

fn confirm_delete(hwnd: HWND) {
    let msg = wide("¿Eliminar esta nota? Esta acción no se puede deshacer.");
    let title = wide("Simpcky");
    let res = unsafe { MessageBoxW(hwnd, msg.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONWARNING) };
    if res == IDYES {
        unsafe { DestroyWindow(hwnd) };
    }
}

// -----------------------------------------------------------------
// Autoguardado del texto
// -----------------------------------------------------------------

/// Fuerza a volcar a disco el texto de todas las notas abiertas,
/// aunque su temporizador de autoguardado todavía no haya disparado
/// (se llama justo antes de salir de la app).
pub fn flush_all() {
    let hwnds: Vec<HWND> = {
        let a = app().lock().unwrap();
        a.notes.values().map(|nr| nr.hwnd as HWND).filter(|h| !h.is_null()).collect()
    };
    for hwnd in hwnds {
        commit_text_and_save(hwnd);
    }
}

fn commit_text_and_save(hwnd: HWND) {
    let id = note_id(hwnd);
    let edit: HWND = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => nr.edit as HWND,
            None => return,
        }
    };
    if edit.is_null() {
        return;
    }
    let len = unsafe { SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0) } as usize;
    let mut buf: Vec<u16> = vec![0u16; len + 1];
    unsafe { SendMessageW(edit, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize) };
    let text = from_wide(&buf);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.data.text = text;
        }
    }
    save_all();
}

// -----------------------------------------------------------------
// Procedimiento de ventana
// -----------------------------------------------------------------

fn on_create(hwnd: HWND) {
    let id = note_id(hwnd);
    let (hinstance, color, w, content_h, rolled, text, roll_mode) = {
        let a = app().lock().unwrap();
        let nr = a.notes.get(&id).expect("nota registrada antes de crear la ventana");
        (
            a.hinstance,
            nr.data.color,
            nr.data.w,
            nr.data.h - HEADER_H,
            nr.data.rolled,
            nr.data.text.clone(),
            nr.data.roll_mode,
        )
    };
    let edit = create_richedit(hwnd, hinstance as HINSTANCE, w, content_h, &text);
    apply_body_style(edit, color);
    apply_text_padding(edit, w, content_h);
    if rolled && !edit.is_null() {
        unsafe { ShowWindow(edit, SW_HIDE) };
    }
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.hwnd = hwnd as isize;
            nr.edit = edit as isize;
        }
    }
    if roll_mode == RollMode::Auto {
        unsafe { SetTimer(hwnd, TIMER_HOVER, HOVER_POLL_MS, None) };
    }
}

fn on_size(hwnd: HWND, lparam: LPARAM) {
    let id = note_id(hwnd);
    let edit: HWND = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => nr.edit as HWND,
            None => return,
        }
    };
    if edit.is_null() {
        return;
    }
    let w = (lparam & 0xffff) as i32;
    let total_h = ((lparam >> 16) & 0xffff) as i32;
    let eh = (total_h - HEADER_H).max(0);
    unsafe { SetWindowPos(edit, null_mut(), 0, HEADER_H, w, eh, SWP_NOZORDER) };
    apply_text_padding(edit, w, eh);
}

fn on_move_end(hwnd: HWND) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut rc) };
    let id = note_id(hwnd);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.data.x = rc.left;
            nr.data.y = rc.top;
        }
    }
    save_all();
}

fn on_destroy(hwnd: HWND) {
    let id = note_id(hwnd);
    unsafe {
        KillTimer(hwnd, TIMER_HOVER);
        KillTimer(hwnd, TIMER_AUTOSAVE);
    }
    {
        let mut a = app().lock().unwrap();
        a.notes.remove(&id);
    }
    save_all();
}

fn on_lbuttondown(hwnd: HWND, lparam: LPARAM) {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let layout = HeaderLayout::new(rc.right);
    match layout.hit(x, y) {
        Hit::Chevron => toggle_roll_manual(hwnd),
        Hit::Ellipsis => show_note_menu(hwnd),
        Hit::Drag => unsafe {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
        },
        Hit::None => {}
    }
}

fn on_command(hwnd: HWND, wparam: WPARAM) {
    let low = (wparam & 0xffff) as u32;
    let high = ((wparam >> 16) & 0xffff) as u32;
    if high == 0 {
        match low {
            ID_ROLL_MANUAL => set_roll_mode(hwnd, RollMode::Manual),
            ID_ROLL_AUTO => set_roll_mode(hwnd, RollMode::Auto),
            ID_TOGGLE_PIN => toggle_pin(hwnd),
            ID_DELETE_NOTE => confirm_delete(hwnd),
            id if (ID_COLOR_BASE..ID_COLOR_BASE + PALETTE.len() as u32).contains(&id) => {
                set_color(hwnd, (id - ID_COLOR_BASE) as u8)
            }
            _ => {}
        }
    } else if low as usize == RICHEDIT_ID && high == EN_CHANGE {
        unsafe { SetTimer(hwnd, TIMER_AUTOSAVE, AUTOSAVE_DEBOUNCE_MS, None) };
    }
}

fn on_timer(hwnd: HWND, wparam: WPARAM) {
    match wparam {
        TIMER_HOVER => handle_hover_tick(hwnd),
        TIMER_AUTOSAVE => {
            unsafe { KillTimer(hwnd, TIMER_AUTOSAVE) };
            commit_text_and_save(hwnd);
        }
        _ => {}
    }
}

pub unsafe extern "system" fn note_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let cs = &*(lparam as *const CREATESTRUCTW);
            let id = cs.lpCreateParams as usize as u32;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, id as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            on_create(hwnd);
            0
        }
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            on_size(hwnd, lparam);
            0
        }
        WM_LBUTTONDOWN => {
            on_lbuttondown(hwnd, lparam);
            0
        }
        WM_RBUTTONUP => {
            show_note_menu(hwnd);
            0
        }
        WM_COMMAND => {
            on_command(hwnd, wparam);
            0
        }
        WM_TIMER => {
            on_timer(hwnd, wparam);
            0
        }
        WM_EXITSIZEMOVE => {
            on_move_end(hwnd);
            0
        }
        WM_CLOSE => {
            confirm_delete(hwnd);
            0
        }
        WM_DESTROY => {
            on_destroy(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
