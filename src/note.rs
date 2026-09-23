//! Ventana de una nota individual: creación, dibujo del encabezado,
//! arrastre, menú "⋯", enrollado (manual/auto) y autoguardado.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Controls::EM_SETRECT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    DragDetect, GetDoubleClickTime, GetKeyState, ReleaseCapture, SetFocus, VK_CONTROL, VK_F2, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{app, save_all};
use crate::persist::{Layer, NoteData, RollMode};
use crate::win::{from_wide, wide};

pub const HEADER_H: i32 = 44;
const RICHEDIT_ID: usize = 100;
const TIMER_HOVER: usize = 1;
const TIMER_AUTOSAVE: usize = 2;
const TIMER_DESKTOP_WATCH: usize = 3;
const HOVER_POLL_MS: u32 = 150;
const AUTOSAVE_DEBOUNCE_MS: u32 = 600;
/// El anclaje al escritorio usa un truco no documentado de Explorer
/// (ver `desktop.rs`) que un reinicio de `explorer.exe` puede
/// deshacer; cada tanto se vuelve a intentar mientras la nota siga
/// anclada. No hace falta que sea muy seguido.
const DESKTOP_WATCH_MS: u32 = 4000;
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
/// "Widget de escritorio" en el menú — pone `Layer::Desktop` (ver
/// `persist::Layer`: `Normal` y `Desktop` son lo mismo por dentro).
const ID_LAYER_DESKTOP: u32 = 2103;
const ID_LAYER_ALWAYS_ON_TOP: u32 = 2104;
const ID_DELETE_NOTE: u32 = 2105;
const ID_DUPLICATE_NOTE: u32 = 2106;

const ID_RENAME: u32 = 2107;
const ID_DARK_MODE: u32 = 2108;
const TIMER_PEEK_END: usize = 4;
/// Un clic sin arrastre en la barra (expandida) enrolla la nota, pero
/// recién cuando pasa el tiempo de doble clic sin un segundo clic: si
/// no, el doble clic para renombrar primero la enrollaba.
const TIMER_BAR_CLICK: usize = 5;

/// (encabezado, cuerpo, tinta) del color de la nota en el tema actual
/// (ver `theme.rs`: la misma nota tiene su versión clara y oscura).
fn palette_entry(idx: u8) -> (u32, u32, u32) {
    crate::theme::note_colors(idx)
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
            // CS_DBLCLKS: sin esto Windows nunca manda
            // WM_LBUTTONDBLCLK, solo dos WM_LBUTTONDOWN sueltos — y
            // el doble clic para editar el título no funcionaría.
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW | CS_DBLCLKS,
            lpfnWndProc: Some(note_wndproc),
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
                peeking: false,
                deleting: false,
            },
        );
        hinstance
    };

    let class_name = wide(NOTE_CLASS);
    let h = if data.rolled { HEADER_H } else { data.h };
    // Donde se dibuja, no donde "vive": una nota que viene de otra compu
    // (o de un monitor que ya no está) puede tener coordenadas fuera de
    // esta pantalla. Se muestra ajustada, pero el dato queda intacto:
    // si se lo reescribiera, esta compu le "corregiría" la posición a
    // las demás en la próxima sincronización.
    let (x, y) = clamp_to_work_area(data.x, data.y, data.w, HEADER_H);
    // Siempre encima: ventana normal, con botón en la barra de tareas
    // (WS_EX_APPWINDOW) y por encima de todo (WS_EX_TOPMOST). Widget
    // de escritorio: oculta de la barra de tareas (WS_EX_TOOLWINDOW) —
    // aunque en rigor ni hace falta, porque al anclarla pasa a ser
    // hija de Progman y las ventanas hijas nunca aparecen ahí.
    let ex_style = if data.layer == Layer::AlwaysOnTop {
        WS_EX_APPWINDOW | WS_EX_TOPMOST
    } else {
        WS_EX_TOOLWINDOW
    };

    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            null(),
            WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
            x,
            y,
            data.w,
            h,
            null_mut(),
            null_mut(),
            hinstance as HINSTANCE,
            id as usize as *const c_void,
        )
    };

    if hwnd.is_null() {
        // No se pudo crear la ventana (Windows sin recursos, o el
        // escritorio a mitad de reiniciarse). Los datos NO se tiran:
        // antes esto borraba la nota del mapa, el guardado siguiente la
        // borraba de `notes.json`, y con la sincronización encima esa
        // "desaparición" viajaría como borrado a todas las compus. La
        // nota queda sin ventana (como cuando Explorer se reinicia) y se
        // vuelve a intentar en el próximo `recreate_lost` o arranque.
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

/// Fuente semibold para el título del encabezado (nota enrollada) —
/// un poco más chica que la del cuerpo, para que lea como título.
pub fn header_font() -> HFONT {
    let mut a = app().lock().unwrap();
    if a.header_font == 0 {
        let face = wide("Segoe UI");
        let f = unsafe {
            CreateFontW(
                -15,
                0,
                0,
                0,
                FW_SEMIBOLD as i32,
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
        a.header_font = f as isize;
    }
    a.header_font as HFONT
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

pub fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        "Nota".to_string()
    } else {
        line.to_string()
    }
}

/// El nombre que se muestra de una nota: el que le puso el usuario
/// (ver `rename.rs`) o, si no le puso ninguno, la primera línea del
/// texto — y si la nota está vacía, "Nota".
pub fn display_title(data: &NoteData) -> String {
    let t = data.title.trim();
    if t.is_empty() {
        first_line(&data.text)
    } else {
        t.to_string()
    }
}

/// Todo lo que necesita `rename::begin`: el nombre actual, los colores
/// del encabezado y dónde va el cuadro de texto (sobre el título).
pub fn rename_info(hwnd: HWND) -> Option<(String, u32, u32, RECT)> {
    let id = note_id(hwnd);
    let (title, color) = {
        let a = app().lock().unwrap();
        let nr = a.notes.get(&id)?;
        (display_title(&nr.data), nr.data.color)
    };
    let (header, _, ink) = palette_entry(color);
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let t = HeaderLayout::new(rc.right).title_rect;
    let h = 22;
    let top = (HEADER_H - h) / 2;
    Some((title, header, ink, RECT { left: t.left - 2, top, right: t.right, bottom: top + h }))
}

/// Pone el nombre elegido por el usuario (vacío = volver a usar la
/// primera línea del texto).
pub fn set_title(hwnd: HWND, title: &str) {
    let id = note_id(hwnd);
    {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) => nr.data.title = title.to_string(),
            None => return,
        }
    }
    unsafe { InvalidateRect(hwnd, null(), 1) };
    save_all();
}

// -----------------------------------------------------------------
// Layout del encabezado
// -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Chevron,
    Ellipsis,
    Pin,
    /// El punto de color del encabezado: en el diseño no es un adorno,
    /// es el selector de color (un clic abre la paleta de 6).
    Dot,
    /// El resto de la barra: qué hace depende de si está enrollada o
    /// no, se decide en `on_lbuttondown` (ver comentario ahí).
    Bar,
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

    /// "⋯" y el pin son iguales enrollada o no. El chevron siempre
    /// enrolla/desenrolla (el sentido que corresponda). El resto de
    /// la barra (`Hit::Bar`) es ambiguo a propósito: `on_lbuttondown`
    /// decide entre arrastrar y enrollar según el estado.
    fn hit(&self, x: i32, y: i32) -> Hit {
        let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        // El punto es chico: se le da un área de clic algo más
        // generosa que su dibujo, si no hay que apuntar con lupa.
        let dot_zone = RECT { left: self.dot.left - 6, top: 0, right: self.dot.right + 6, bottom: HEADER_H };
        if inside(&self.ellipsis) {
            Hit::Ellipsis
        } else if inside(&self.pin) {
            Hit::Pin
        } else if inside(&self.chevron) {
            Hit::Chevron
        } else if inside(&dot_zone) {
            Hit::Dot
        } else if y < HEADER_H {
            Hit::Bar
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

    // Punto de color: no un punto de tinta sólido, sino un tinte
    // suave del propio color del encabezado, como una hendidura hacia
    // adentro — un arco de sombra arriba y uno de brillo abajo, en vez
    // de un relleno opaco de contraste fuerte.
    let d = &layout.dot;
    let (dw, dh) = (d.right - d.left, d.bottom - d.top);
    let mut well_brush: *mut GpSolidFill = null_mut();
    GdipCreateSolidFill(argb(0x2E, 0, 0, 0), &mut well_brush);
    GdipFillEllipseI(graphics, well_brush as *mut GpBrush, d.left, d.top, dw, dh);
    GdipDeleteBrush(well_brush as *mut GpBrush);

    let mut shadow_pen: *mut GpPen = null_mut();
    GdipCreatePen1(argb(0x55, 0, 0, 0), 1.3, UnitPixel, &mut shadow_pen);
    GdipDrawArcI(graphics, shadow_pen, d.left, d.top, dw, dh, 180.0, 180.0);
    GdipDeletePen(shadow_pen);

    let mut highlight_pen: *mut GpPen = null_mut();
    GdipCreatePen1(argb(0x60, 0xff, 0xff, 0xff), 1.3, UnitPixel, &mut highlight_pen);
    GdipDrawArcI(graphics, highlight_pen, d.left, d.top, dw, dh, 0.0, 180.0);
    GdipDeletePen(highlight_pen);

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
pub fn colorref_to_argb(c: u32) -> u32 {
    let r = c & 0xff;
    let g = (c >> 8) & 0xff;
    let b = (c >> 16) & 0xff;
    0xff000000 | (r << 16) | (g << 8) | b
}

/// ARGB con alfa explícito, para los tintes semitransparentes del
/// punto de color "hundido".
fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn on_paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        paint(hwnd, hdc);
        EndPaint(hwnd, &ps);
    }
}

/// Dibuja el encabezado en `hdc`: el de WM_PAINT, o el que manda
/// WM_PRINTCLIENT (capturas de pantalla, miniaturas, `PrintWindow`).
/// Sin esto último, una nota anclada al escritorio salía en negro en
/// cualquier captura.
fn paint(hwnd: HWND, hdc: HDC) {
    let id = note_id(hwnd);
    let (color, rolled, pinned, title) = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => (nr.data.color, nr.data.rolled, nr.data.layer == Layer::AlwaysOnTop, display_title(&nr.data)),
            None => return,
        }
    };
    let renaming = crate::rename::is_renaming(hwnd);
    let (header_color, _, ink) = palette_entry(color);

    unsafe {
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

        // El título (la primera línea) se ve siempre en el
        // encabezado, esté enrollada o no — no solo cuando está
        // enrollada.
        //
        // Sin este SelectObject, DrawTextW usa la fuente por defecto
        // del DC (la bitmap "System" de toda la vida) en vez de Segoe
        // UI — se nota mucho al lado del resto de la nota.
        if !renaming {
            let old_font = SelectObject(hdc, header_font());
            let mut text_rc = layout.title_rect;
            let wtext = wide(&title);
            DrawTextW(hdc, wtext.as_ptr(), -1, &mut text_rc, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_LEFT);
            SelectObject(hdc, old_font);
        }
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

/// Enrolla/desenrolla (el sentido que corresponda) — la acción del
/// chevron, y también de un clic sin arrastre sobre la barra cuando
/// está expandida (ver `on_bar_click`). Si el resultado es
/// desenrollarla, además deja el cursor en el texto, listo para
/// escribir.
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
        if !rolled && !edit.is_null() {
            unsafe { SetFocus(edit) };
        }
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

/// Marca un ítem de menú con la viñeta redonda de "opción elegida" en
/// vez del tilde de "activado". `MF_CHECKED` solo sabe dibujar el
/// tilde; el círculo (que es lo que corresponde cuando las opciones
/// son excluyentes, como en el diseño) se pide con `MFT_RADIOCHECK`.
pub fn mark_radio(menu: HMENU, id: u32, selected: bool) {
    unsafe {
        let mut mii: MENUITEMINFOW = std::mem::zeroed();
        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        mii.fMask = MIIM_FTYPE | MIIM_STATE;
        mii.fType = MFT_RADIOCHECK;
        mii.fState = if selected { MFS_CHECKED } else { MFS_UNCHECKED };
        SetMenuItemInfoW(menu, id, 0, &mii);
    }
}

/// Paleta de 6 colores como submenú (o como menú suelto, cuando se
/// abre desde el punto de color del encabezado).
unsafe fn build_color_menu(current: u8) -> (HMENU, Vec<Vec<u16>>) {
    let menu = CreatePopupMenu();
    let mut labels = Vec::with_capacity(crate::theme::PALETTE_LEN);
    for (i, name) in crate::theme::COLOR_NAMES.iter().enumerate() {
        labels.push(wide(name));
        AppendMenuW(menu, MF_STRING, (ID_COLOR_BASE as usize) + i, labels[i].as_ptr());
        mark_radio(menu, ID_COLOR_BASE + i as u32, current as usize == i);
    }
    (menu, labels)
}

/// Clic en el punto de color del encabezado: abre la paleta justo
/// debajo, como en el diseño ("Selector de color — clic para elegir
/// entre 6 colores").
fn show_color_menu(hwnd: HWND) {
    let id = note_id(hwnd);
    let color = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => nr.data.color,
            None => return,
        }
    };
    unsafe {
        let (menu, _labels) = build_color_menu(color);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        let layout = HeaderLayout::new(rc.right);
        let mut pt = POINT { x: layout.dot.left, y: HEADER_H };
        ClientToScreen(hwnd, &mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(menu, TPM_LEFTBUTTON | TPM_LEFTALIGN, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}

/// El menú "⋯", con los mismos ítems que el diseño: color, enrollado,
/// las dos capas como interruptores, duplicar y eliminar.
fn show_note_menu(hwnd: HWND) {
    let id = note_id(hwnd);
    let (color, roll_mode, layer) = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => (nr.data.color, nr.data.roll_mode, nr.data.layer),
            None => return,
        }
    };

    unsafe {
        let menu = CreatePopupMenu();
        let (color_menu, _color_labels) = build_color_menu(color);
        let color_label = wide("Color");
        AppendMenuW(menu, MF_POPUP, color_menu as usize, color_label.as_ptr());

        let roll_menu = CreatePopupMenu();
        let manual_label = wide("Manual");
        let auto_label = wide("Auto");
        AppendMenuW(roll_menu, MF_STRING, ID_ROLL_MANUAL as usize, manual_label.as_ptr());
        AppendMenuW(roll_menu, MF_STRING, ID_ROLL_AUTO as usize, auto_label.as_ptr());
        mark_radio(roll_menu, ID_ROLL_MANUAL, roll_mode == RollMode::Manual);
        mark_radio(roll_menu, ID_ROLL_AUTO, roll_mode == RollMode::Auto);
        let roll_label = wide("Enrollar");
        AppendMenuW(menu, MF_POPUP, roll_menu as usize, roll_label.as_ptr());

        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        // Las dos capas, como los dos interruptores del diseño. Son
        // excluyentes: "Normal" y "Desktop" son la misma cosa por
        // dentro (ver `persist::Layer`), así que la nota siempre está
        // en una de estas dos.
        let top_label = wide("Siempre encima");
        let widget_label = wide("Anclar al escritorio");
        AppendMenuW(
            menu,
            MF_STRING | if layer == Layer::AlwaysOnTop { MF_CHECKED } else { MF_UNCHECKED },
            ID_LAYER_ALWAYS_ON_TOP as usize,
            top_label.as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING | if layer != Layer::AlwaysOnTop { MF_CHECKED } else { MF_UNCHECKED },
            ID_LAYER_DESKTOP as usize,
            widget_label.as_ptr(),
        );

        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        let rename_label = wide("Cambiar nombre\tF2");
        AppendMenuW(menu, MF_STRING, ID_RENAME as usize, rename_label.as_ptr());
        let dark_label = wide("Modo oscuro");
        AppendMenuW(
            menu,
            MF_STRING | if crate::theme::is_dark() { MF_CHECKED } else { MF_UNCHECKED },
            ID_DARK_MODE as usize,
            dark_label.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());

        let duplicate_label = wide("Duplicar nota");
        AppendMenuW(menu, MF_STRING, ID_DUPLICATE_NOTE as usize, duplicate_label.as_ptr());
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

/// Copia la nota (texto, color, modo y tamaño) en una nota nueva,
/// corrida un poco para que se vea que son dos.
fn duplicate_note(hwnd: HWND) {
    let id = note_id(hwnd);
    // El texto vivo está en el RichEdit, que puede ir por delante de
    // lo guardado si el autoguardado todavía no disparó.
    commit_text_and_save(hwnd);
    let source = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => nr.data.clone(),
            None => return,
        }
    };
    let mut copy = source;
    copy.id = crate::app::next_note_id();
    // Otra nota, no la misma: identidad nueva y sin historia (save_all
    // le pone la hora al guardarla).
    copy.uid = crate::persist::new_uid();
    copy.t = [0; crate::persist::PARTS];
    copy.x += 28;
    copy.y += 28;
    copy.rolled = false;
    let dup = create_note(copy);
    save_all();
    // Igual que una nota nueva: al frente, a la vista.
    if !dup.is_null() {
        open_note(dup);
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

/// Cambia la capa de la nota, deshaciendo primero lo que corresponda
/// del estado anterior (bajar el "siempre encima" o desanclar del
/// escritorio) antes de aplicar el nuevo.
fn set_layer(hwnd: HWND, layer: Layer) {
    let id = note_id(hwnd);
    // En los hechos hay dos capas: "siempre encima" o widget de
    // escritorio (`Normal`, de archivos viejos, es lo mismo que
    // `Desktop`). Antes solo se desanclaba si la capa previa era
    // exactamente `Desktop`, y una nota vieja en `Normal` quedaba
    // anclada aunque se la pasara a "siempre encima".
    let to_top = layer == Layer::AlwaysOnTop;
    let (was_top, peeking) = {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) => {
                let was_top = nr.data.layer == Layer::AlwaysOnTop;
                let peeking = nr.peeking;
                nr.data.layer = if to_top { Layer::AlwaysOnTop } else { Layer::Desktop };
                nr.peeking = false;
                (was_top, peeking)
            }
            None => return,
        }
    };
    if was_top == to_top {
        // Misma capa. Lo único que puede quedar por hacer: un widget
        // "asomado" al frente al que se le pide anclarse, vuelve ya.
        if peeking && !to_top {
            anchor_widget(hwnd);
        }
        return;
    }

    switch_layer_window(hwnd, to_top);
    save_all();
}

/// Lo que cambia en la ventana al pasar entre "siempre encima" y widget
/// de escritorio (sin guardar nada: eso lo decide el que llama).
fn switch_layer_window(hwnd: HWND, to_top: bool) {
    unsafe {
        if to_top {
            KillTimer(hwnd, TIMER_DESKTOP_WATCH);
            KillTimer(hwnd, TIMER_PEEK_END);
            crate::desktop::detach(hwnd);
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        } else {
            SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            anchor_widget(hwnd);
        }
        apply_taskbar_visibility(hwnd, if to_top { Layer::AlwaysOnTop } else { Layer::Desktop });
        InvalidateRect(hwnd, null(), 1);
    }
}

/// Una nota cambió en otra compu (ver `sync::apply`): se reemplazan sus
/// datos y se actualiza la ventana en lo que haga falta. No es un cambio
/// local: se marca como ya guardado, así no se le pone la hora de ahora
/// y no vuelve a subir.
pub fn apply_remote(new: NoteData) {
    let id = new.id;
    crate::app::mark_saved(&new);
    let (hwnd, edit, old) = {
        let mut a = app().lock().unwrap();
        let Some(nr) = a.notes.get_mut(&id) else { return };
        let old = std::mem::replace(&mut nr.data, new.clone());
        (nr.hwnd as HWND, nr.edit as HWND, old)
    };
    if hwnd.is_null() {
        return; // sin ventana ahora: se va a crear con los datos nuevos
    }
    unsafe {
        if !edit.is_null() && old.text != new.text {
            let text = wide(&new.text);
            SendMessageW(edit, WM_SETTEXT, 0, text.as_ptr() as isize);
        }
        if !edit.is_null() && (old.text != new.text || old.color != new.color) {
            apply_body_style(edit, new.color);
        }
        let (was_top, to_top) = (old.layer == Layer::AlwaysOnTop, new.layer == Layer::AlwaysOnTop);
        if was_top != to_top {
            if !to_top {
                // Si estaba asomada, deja de estarlo: vuelve al escritorio.
                let mut a = app().lock().unwrap();
                if let Some(nr) = a.notes.get_mut(&id) {
                    nr.peeking = false;
                }
            }
            switch_layer_window(hwnd, to_top);
        }
        if old.roll_mode != new.roll_mode {
            if new.roll_mode == RollMode::Auto {
                SetTimer(hwnd, TIMER_HOVER, HOVER_POLL_MS, None);
            } else {
                KillTimer(hwnd, TIMER_HOVER);
            }
        }
        if (old.x, old.y) != (new.x, new.y) {
            let (x, y) = clamp_to_work_area(new.x, new.y, new.w, HEADER_H);
            crate::desktop::move_to_screen(hwnd, x, y);
        }
        if (old.w, old.h, old.rolled) != (new.w, new.h, new.rolled) {
            apply_rolled_state(hwnd, edit, new.w, new.h, new.rolled);
        }
        InvalidateRect(hwnd, null(), 1);
    }
}

/// La nota se borró en otra compu: se va de acá también, sin preguntar
/// (la confirmación ya la dio el usuario allá).
pub fn delete_silently(id: u32) {
    crate::app::forget_saved(id);
    let hwnd = {
        let mut a = app().lock().unwrap();
        match a.notes.get_mut(&id) {
            Some(nr) => {
                nr.deleting = true;
                nr.hwnd as HWND
            }
            None => return,
        }
    };
    if hwnd.is_null() {
        app().lock().unwrap().notes.remove(&id);
        save_all();
    } else {
        unsafe { DestroyWindow(hwnd) };
    }
}

/// Ancla la nota detrás de los íconos y deja al vigilante encargado de
/// reanclarla si algo la suelta (o de reintentar, si Explorer todavía
/// no estaba listo: la nota no se pierde, queda como ventana suelta
/// hasta que se pueda).
fn anchor_widget(hwnd: HWND) {
    let id = note_id(hwnd);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.peeking = false;
        }
    }
    unsafe {
        KillTimer(hwnd, TIMER_PEEK_END);
        crate::desktop::anchor(hwnd);
        SetTimer(hwnd, TIMER_DESKTOP_WATCH, DESKTOP_WATCH_MS, None);
    }
}

/// Siempre encima → ventana normal, con botón en la barra de tareas
/// (`WS_EX_APPWINDOW`). Cualquier otra cosa → oculta de ahí
/// (`WS_EX_TOOLWINDOW`) — aunque, al quedar anclada como hija de
/// Progman, ya ni podría aparecer sin esto (las ventanas hijas nunca
/// están en la barra de tareas).
fn apply_taskbar_visibility(hwnd: HWND, layer: Layer) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let new_ex = if layer == Layer::AlwaysOnTop {
            (ex & !WS_EX_TOOLWINDOW) | WS_EX_APPWINDOW
        } else {
            (ex & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW
        };
        if new_ex == ex {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex as isize);
        // Windows no refresca el botón de la barra de tareas solo con
        // el cambio de estilo: hay que ocultar y volver a mostrar la
        // ventana (sin activarla) para que lo note.
        ShowWindow(hwnd, SW_HIDE);
        SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        ShowWindow(hwnd, SW_SHOWNA);
    }
}

pub fn confirm_delete(hwnd: HWND) {
    delete_note_confirm(hwnd, hwnd);
}

/// "¿Eliminar esta nota?" con el cuadro de diálogo sobre `owner` (la
/// propia nota, o "Todas las notas" si se borra desde ahí).
pub fn delete_note_confirm(hwnd: HWND, owner: HWND) {
    let msg = wide("¿Eliminar esta nota? Esta acción no se puede deshacer.");
    let title = wide("Simpcky");
    let res = unsafe { MessageBoxW(owner, msg.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONWARNING) };
    if res == IDYES {
        // Marcar ANTES de destruir: on_destroy solo borra los datos de
        // una nota si el usuario lo pidió (ver `NoteRuntime::deleting`).
        let id = note_id(hwnd);
        if let Some(nr) = app().lock().unwrap().notes.get_mut(&id) {
            nr.deleting = true;
        }
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

/// "Abrir" desde "Todas las notas" (doble clic, Enter): desenrolla la
/// nota (si el modo es manual — en Auto se volvería a enrollar sola en
/// el próximo tick) y la trae adelante DE VERDAD, con el cursor en el
/// texto.
///
/// Un widget de escritorio vive detrás de todas las ventanas a
/// propósito, así que "traerlo al frente" dentro del escritorio no
/// servía de nada si había algo abierto encima (que es casi siempre).
/// Ahora se "asoma": se suelta del escritorio y flota adelante como
/// una ventana normal mientras se la usa, y apenas se hace clic en
/// cualquier otra cosa vuelve sola a su lugar detrás de los íconos
/// (ver `TIMER_PEEK_END`).
pub fn open_note(hwnd: HWND) {
    let id = note_id(hwnd);
    let (unroll, widget, peeking, edit) = {
        let mut a = app().lock().unwrap();
        let Some(nr) = a.notes.get_mut(&id) else { return };
        let unroll = if nr.data.rolled && nr.data.roll_mode == RollMode::Manual {
            nr.data.rolled = false;
            Some((nr.data.w, nr.data.h))
        } else {
            None
        };
        (unroll, nr.data.layer != Layer::AlwaysOnTop, nr.peeking, nr.edit as HWND)
    };
    if let Some((w, h)) = unroll {
        apply_rolled_state(hwnd, edit, w, h, false);
        save_all();
    }
    unsafe {
        if widget && !peeking {
            KillTimer(hwnd, TIMER_PEEK_END);
            {
                let mut a = app().lock().unwrap();
                if let Some(nr) = a.notes.get_mut(&id) {
                    nr.peeking = true;
                }
            }
            crate::desktop::detach(hwnd);
        }
        ShowWindow(hwnd, SW_SHOW);
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        SetForegroundWindow(hwnd);
        if !edit.is_null() {
            SetFocus(edit);
        }
        if widget {
            // Si Windows no le dio el foco (a veces niega el primer
            // plano a una app que no lo tenía), la nota nunca recibiría
            // el "perdiste el foco" que la devuelve al escritorio, y se
            // quedaría suelta para siempre. Este control lo cubre: si
            // para entonces no es la ventana activa, vuelve a anclarse.
            SetTimer(hwnd, TIMER_PEEK_END, 3000, None);
        }
    }
}

/// Soltar una tarjeta de "Todas las notas" sobre el escritorio: la
/// nota pasa a ser un widget, en ese lugar. `x`/`y` son coordenadas
/// de pantalla de la esquina superior izquierda.
///
/// Si donde cayó hay otra ventana tapando el escritorio, además se
/// asoma al frente un momento para que se vea dónde quedó (y vuelve a
/// su lugar al hacer clic en otra cosa, como cualquier nota asomada) —
/// si no, soltarla sobre el navegador la hacía "desaparecer".
pub fn drop_on_desktop(hwnd: HWND, x: i32, y: i32) {
    let Some((x, y)) = place_on_desktop(hwnd, x, y) else { return };
    // Se mira un punto del encabezado ya ubicado: si el escritorio se
    // ve ahí, la ventana que aparece en ese punto es la propia nota
    // (hija del escritorio); si no, es lo que la tapa.
    if !crate::desktop::is_desktop_visible_at(x + 24, y + HEADER_H / 2) {
        open_note(hwnd);
    }
}

/// Ancla la nota al escritorio en (`x`, `y`) — ajustado para que el
/// encabezado quede dentro de la pantalla — y devuelve dónde quedó.
fn place_on_desktop(hwnd: HWND, x: i32, y: i32) -> Option<(i32, i32)> {
    let id = note_id(hwnd);
    let (w, was_top) = {
        let a = app().lock().unwrap();
        let nr = a.notes.get(&id)?;
        (nr.data.w, nr.data.layer == Layer::AlwaysOnTop)
    };
    // Que no quede fuera de la pantalla: al menos el encabezado entero
    // tiene que caer dentro del área de trabajo del monitor.
    let (x, y) = clamp_to_work_area(x, y, w, HEADER_H);
    if was_top {
        set_layer(hwnd, Layer::Desktop);
    } else {
        anchor_widget(hwnd);
    }
    crate::desktop::move_to_screen(hwnd, x, y);
    crate::desktop::raise(hwnd);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.data.x = x;
            nr.data.y = y;
        }
    }
    save_all();
    Some((x, y))
}

/// Ajusta una posición de pantalla para que un rectángulo de `w`×`h`
/// quede dentro del área de trabajo (sin la barra de tareas) del
/// monitor donde cae.
pub fn clamp_to_work_area(x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return (x, y);
        }
        let r = info.rcWork;
        let nx = x.min(r.right - w).max(r.left);
        let ny = y.min(r.bottom - h).max(r.top);
        (nx, ny)
    }
}

/// Repinta todas las notas con el tema actual (claro/oscuro).
pub fn apply_theme_all() {
    let list: Vec<(HWND, HWND, u8)> = {
        let a = app().lock().unwrap();
        a.notes.values().map(|nr| (nr.hwnd as HWND, nr.edit as HWND, nr.data.color)).collect()
    };
    for (hwnd, edit, color) in list {
        if hwnd.is_null() {
            continue;
        }
        // Un renombrado a medias quedaría con los colores viejos.
        crate::rename::finish(hwnd, true);
        if !edit.is_null() {
            apply_body_style(edit, color);
            crate::theme::apply_scrollbars(edit);
        }
        unsafe { InvalidateRect(hwnd, null(), 1) };
    }
}

/// Recrea las ventanas de las notas que se quedaron sin ventana (ver
/// `on_destroy`): típicamente, Explorer se reinició y se llevó puesto
/// a Progman con todas sus hijas.
pub fn recreate_lost() {
    let lost: Vec<NoteData> = {
        let a = app().lock().unwrap();
        a.notes.values().filter(|nr| nr.hwnd == 0).map(|nr| nr.data.clone()).collect()
    };
    for data in lost {
        create_note(data);
    }
}

/// El texto del RichEdit, con los saltos de línea siempre como `\n`:
/// RichEdit los devuelve como `\r\n` (o `\r`), y si el mismo texto
/// pudiera quedar guardado de dos formas, cada compu lo vería como un
/// cambio y se lo pasarían de una a otra para siempre.
fn read_text(edit: HWND) -> String {
    let len = unsafe { SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0) } as usize;
    let mut buf: Vec<u16> = vec![0u16; len + 1];
    unsafe { SendMessageW(edit, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize) };
    from_wide(&buf).replace("\r\n", "\n").replace('\r', "\n")
}

/// Pasa a los datos lo que haya en los cuadros de texto que el
/// autoguardado todavía no levantó (se llama antes de sincronizar: lo
/// que se está escribiendo en este momento también cuenta). Un solo
/// guardado al final, no uno por nota.
pub fn commit_all_text() {
    let edits: Vec<(u32, HWND)> = {
        let a = app().lock().unwrap();
        a.notes.values().filter(|nr| nr.edit != 0).map(|nr| (nr.data.id, nr.edit as HWND)).collect()
    };
    let texts: Vec<(u32, String)> = edits.into_iter().map(|(id, edit)| (id, read_text(edit))).collect();
    let changed = {
        let mut a = app().lock().unwrap();
        let mut changed = false;
        for (id, text) in texts {
            if let Some(nr) = a.notes.get_mut(&id) {
                if nr.data.text != text {
                    nr.data.text = text;
                    changed = true;
                }
            }
        }
        changed
    };
    if changed {
        save_all();
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
    let text = read_text(edit);
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
    let (hinstance, color, w, content_h, rolled, text, roll_mode, layer) = {
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
            nr.data.layer,
        )
    };
    let edit = create_richedit(hwnd, hinstance as HINSTANCE, w, content_h, &text);
    apply_body_style(edit, color);
    crate::theme::apply_scrollbars(edit);
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
    if layer != Layer::AlwaysOnTop {
        // "Normal" (dato viejo) y "Desktop" son, en los hechos, el
        // mismo caso: widget de escritorio, anclado detrás de los
        // íconos. Si falla (Explorer todavía no listo, versión rara)
        // la nota no se pierde: queda como ventana suelta y el
        // vigilante reintenta solo.
        anchor_widget(hwnd);
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

/// Fin de un arrastre o de un resize (WM_EXITSIZEMOVE cubre ambos).
/// El alto solo se guarda si no está enrollada: `data.h` siempre
/// representa el alto "desenrollado", y mientras está enrollada la
/// ventana mide `HEADER_H` de verdad, que no es lo que hay que
/// recordar.
fn on_move_end(hwnd: HWND) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut rc) };
    let id = note_id(hwnd);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.data.x = rc.left;
            nr.data.y = rc.top;
            nr.data.w = (rc.right - rc.left).max(160);
            if !nr.data.rolled {
                nr.data.h = (rc.bottom - rc.top).max(HEADER_H + 80);
            }
        }
    }
    save_all();
}

fn on_destroy(hwnd: HWND) {
    let id = note_id(hwnd);
    unsafe {
        KillTimer(hwnd, TIMER_HOVER);
        KillTimer(hwnd, TIMER_AUTOSAVE);
        KillTimer(hwnd, TIMER_DESKTOP_WATCH);
        KillTimer(hwnd, TIMER_PEEK_END);
        KillTimer(hwnd, TIMER_BAR_CLICK);
    }
    crate::rename::abort_for(hwnd);

    let (deleting, edit) = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            // Puede ser una ventana vieja de una nota que ya se recreó
            // con otro HWND: esa no tiene nada que tocar.
            Some(nr) if nr.hwnd == hwnd as isize => (nr.deleting, nr.edit as HWND),
            _ => return,
        }
    };
    if deleting {
        app().lock().unwrap().notes.remove(&id);
        save_all();
        return;
    }

    // No fue el usuario: la ventana se va porque se la llevó otro
    // (Explorer reiniciándose destruye a Progman y a todas sus hijas,
    // o Windows cerrando la sesión). Antes esto BORRABA la nota de
    // `notes.json`. Ahora se rescata el texto (durante el WM_DESTROY
    // del padre el RichEdit todavía existe), se conservan los datos y
    // se pide que se vuelva a crear la ventana.
    let text = if edit.is_null() {
        None
    } else {
        Some(read_text(edit))
    };
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            if let Some(t) = text {
                nr.data.text = t;
            }
            nr.hwnd = 0;
            nr.edit = 0;
            nr.peeking = false;
        }
    }
    save_all();
    if !crate::app::is_shutting_down() {
        crate::tray::request_recreate();
    }
}

/// Doble clic sobre la barra: cambiar el nombre de la nota ahí mismo
/// (ver `rename.rs`). El primer clic del par no llegó a enrollarla:
/// se cancela acá (ver TIMER_BAR_CLICK).
fn on_dblclick(hwnd: HWND, lparam: LPARAM) {
    unsafe { KillTimer(hwnd, TIMER_BAR_CLICK) };
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let layout = HeaderLayout::new(rc.right);
    if layout.hit(x, y) == Hit::Bar {
        crate::rename::begin(hwnd);
    }
}

const RESIZE_MARGIN: i32 = 6;

/// Deja que Windows redimensione la ventana como si tuviera un borde
/// grueso normal (aunque no lo tenga: es un WS_POPUP sin marco), con
/// solo decirle en qué borde/esquina cae el cursor. Deshabilitado
/// mientras está enrollada (el alto queda fijo al del encabezado).
fn on_nchittest(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let id = note_id(hwnd);
    let rolled = {
        let a = app().lock().unwrap();
        a.notes.get(&id).map(|nr| nr.data.rolled).unwrap_or(false)
    };
    if rolled {
        return unsafe { DefWindowProcW(hwnd, WM_NCHITTEST, 0, lparam) };
    }
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut rc) };
    let left = x < rc.left + RESIZE_MARGIN;
    let right = x >= rc.right - RESIZE_MARGIN;
    let top = y < rc.top + RESIZE_MARGIN;
    let bottom = y >= rc.bottom - RESIZE_MARGIN;
    let hit = if top && left {
        HTTOPLEFT
    } else if top && right {
        HTTOPRIGHT
    } else if bottom && left {
        HTBOTTOMLEFT
    } else if bottom && right {
        HTBOTTOMRIGHT
    } else if left {
        HTLEFT
    } else if right {
        HTRIGHT
    } else if top {
        HTTOP
    } else if bottom {
        HTBOTTOM
    } else {
        0
    };
    if hit != 0 {
        hit as LRESULT
    } else {
        unsafe { DefWindowProcW(hwnd, WM_NCHITTEST, 0, lparam) }
    }
}

/// Tamaño mínimo al redimensionar: que siempre quede lugar para el
/// encabezado completo y algo de cuerpo debajo.
fn on_getminmaxinfo(lparam: LPARAM) {
    let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
    info.ptMinTrackSize.x = 160;
    info.ptMinTrackSize.y = HEADER_H + 80;
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
        Hit::Pin => toggle_always_on_top(hwnd),
        Hit::Dot => show_color_menu(hwnd),
        Hit::Bar => on_bar_click(hwnd, x, y),
        Hit::None => {}
    }
}

/// La nota a la que pertenece `hwnd` (la nota misma, su RichEdit, o el
/// cuadro de renombrar), o nulo si no es de ninguna.
///
/// Antes esto se resolvía con `GetAncestor(GA_ROOT)`, que para una
/// nota anclada devuelve… Progman (el escritorio es su padre de
/// verdad): los atajos nunca funcionaban en los widgets.
pub fn owning_note(mut h: HWND) -> HWND {
    for _ in 0..4 {
        if h.is_null() {
            break;
        }
        let id = note_id(h);
        let is_note = {
            let a = app().lock().unwrap();
            a.notes.get(&id).map(|nr| nr.hwnd == h as isize).unwrap_or(false)
        };
        if is_note {
            return h;
        }
        h = unsafe { GetAncestor(h, GA_PARENT) };
    }
    null_mut()
}

/// Atajos de teclado de la especificación (sección 7), resueltos en
/// el bucle de mensajes (`main.rs`) antes de que el RichEdit se coma
/// la tecla. `target` es la ventana que recibió la tecla. Devuelve
/// `true` si consumió el atajo.
///
/// Son atajos de la app, no del sistema: valen mientras el foco esté
/// en una nota (Ctrl+N también en "Todas las notas"). Registrarlos
/// como *hotkeys* globales le robaría Ctrl+N a todas las demás
/// aplicaciones de Windows.
///
/// El diseño también lista `Supr` para eliminar la nota; queda a
/// propósito sin implementar, porque el foco normal de una nota es el
/// cuadro de texto y ahí `Supr` tiene que borrar caracteres.
pub fn handle_shortcut(target: HWND, vk: u32, repeat: bool) -> bool {
    if target.is_null() {
        return false;
    }
    let down = |k: u16| unsafe { GetKeyState(k as i32) } < 0;
    let ctrl = down(VK_CONTROL);
    let shift = down(VK_SHIFT);
    let note = owning_note(target);

    #[derive(Clone, Copy)]
    enum Action {
        NewNote,
        Rename,
        Roll,
        AlwaysOnTop,
        Desktop,
    }
    let action = if note.is_null() {
        // Fuera de una nota, solo Ctrl+N en "Todas las notas".
        (ctrl && !shift && vk == b'N' as u32 && crate::allnotes::owns(target)).then_some(Action::NewNote)
    } else if vk == VK_F2 as u32 && !ctrl {
        Some(Action::Rename)
    } else if !ctrl {
        None
    } else {
        match (vk as u8, shift) {
            (b'N', false) => Some(Action::NewNote),
            (b'R', false) => Some(Action::Roll),
            (b'T', true) => Some(Action::AlwaysOnTop),
            (b'D', true) => Some(Action::Desktop),
            _ => None,
        }
    };
    let Some(action) = action else { return false };
    // Tecla mantenida apretada: el atajo ya se ejecutó con la primera
    // pulsación; las repeticiones se tragan sin hacer nada (ni pasarle
    // la tecla al texto).
    if repeat {
        return true;
    }
    match action {
        Action::NewNote => crate::tray::spawn_new_note(),
        Action::Rename => crate::rename::begin(note),
        Action::Roll => toggle_roll_manual(note),
        Action::AlwaysOnTop => toggle_always_on_top(note),
        Action::Desktop => set_layer(note, Layer::Desktop),
    }
    true
}

/// El pin del encabezado: mismo efecto que "Capa → Siempre encima"
/// del menú "⋯", pero con un clic.
fn toggle_always_on_top(hwnd: HWND) {
    let id = note_id(hwnd);
    let current = {
        let a = app().lock().unwrap();
        a.notes.get(&id).map(|nr| nr.data.layer).unwrap_or(Layer::Normal)
    };
    let next = if current == Layer::AlwaysOnTop { Layer::Desktop } else { Layer::AlwaysOnTop };
    set_layer(hwnd, next);
}

/// El resto de la barra, sin la ambigüedad resuelta todavía:
/// - **Enrollada**: arrastra, como cualquier título de ventana.
/// - **Expandida**: un clic SIN arrastre la enrolla; si el usuario
///   sí mueve el mouse, se convierte en un arrastre normal.
///   `DragDetect` es el mecanismo estándar de Windows para distinguir
///   ambos casos a partir del mismo botón apretado.
fn on_bar_click(hwnd: HWND, x: i32, y: i32) {
    let id = note_id(hwnd);
    let rolled = {
        let a = app().lock().unwrap();
        a.notes.get(&id).map(|nr| nr.data.rolled).unwrap_or(false)
    };
    unsafe {
        if rolled {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
            return;
        }
        let mut pt = POINT { x, y };
        ClientToScreen(hwnd, &mut pt);
        if DragDetect(hwnd, pt) != 0 {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
        } else {
            // Todavía no: puede ser el primer clic de un doble clic
            // (renombrar). Ver TIMER_BAR_CLICK.
            SetTimer(hwnd, TIMER_BAR_CLICK, GetDoubleClickTime(), None);
        }
    }
}

fn on_command(hwnd: HWND, wparam: WPARAM) {
    let low = (wparam & 0xffff) as u32;
    let high = ((wparam >> 16) & 0xffff) as u32;
    if high == 0 {
        match low {
            ID_ROLL_MANUAL => set_roll_mode(hwnd, RollMode::Manual),
            ID_ROLL_AUTO => set_roll_mode(hwnd, RollMode::Auto),
            ID_LAYER_DESKTOP => set_layer(hwnd, Layer::Desktop),
            ID_LAYER_ALWAYS_ON_TOP => set_layer(hwnd, Layer::AlwaysOnTop),
            ID_DELETE_NOTE => confirm_delete(hwnd),
            ID_DUPLICATE_NOTE => duplicate_note(hwnd),
            ID_RENAME => crate::rename::begin(hwnd),
            ID_DARK_MODE => crate::theme::set_dark(!crate::theme::is_dark()),
            id if (ID_COLOR_BASE..ID_COLOR_BASE + crate::theme::PALETTE_LEN as u32).contains(&id) => {
                set_color(hwnd, (id - ID_COLOR_BASE) as u8)
            }
            _ => {}
        }
    } else if low as usize == RICHEDIT_ID && high == EN_CHANGE {
        unsafe { SetTimer(hwnd, TIMER_AUTOSAVE, AUTOSAVE_DEBOUNCE_MS, None) };
    }
}

fn is_peeking(hwnd: HWND) -> bool {
    let id = note_id(hwnd);
    app().lock().unwrap().notes.get(&id).map(|nr| nr.peeking).unwrap_or(false)
}

fn on_timer(hwnd: HWND, wparam: WPARAM) {
    match wparam {
        TIMER_HOVER => handle_hover_tick(hwnd),
        TIMER_DESKTOP_WATCH => {
            // Antes esto llamaba a anchor() (SetParent + cambio de
            // estilo) en CADA tick, sin importar si hacía falta — y
            // eso le cortaba el foco al RichEdit si justo estabas
            // escribiendo (por eso se perdía el foco cada ~4 s, sin
            // tocar nada). Ahora solo toca algo si de verdad se
            // desancló (por ejemplo, `explorer.exe` se reinició) — y
            // nunca mientras la nota está "asomada" al frente.
            if !is_peeking(hwnd) && !crate::desktop::is_anchored(hwnd) {
                crate::desktop::anchor(hwnd);
            }
        }
        TIMER_PEEK_END => {
            unsafe { KillTimer(hwnd, TIMER_PEEK_END) };
            if !is_peeking(hwnd) {
                return;
            }
            // El foco se fue a otro lado. Si fue a algo nuestro (el
            // "¿Eliminar?" de esta misma nota, por ejemplo) se sigue
            // asomando; si no, vuelve a su lugar en el escritorio.
            let fg = unsafe { GetForegroundWindow() };
            let ours = fg == hwnd || unsafe { GetWindow(fg, GW_OWNER) } == hwnd || unsafe { IsChild(hwnd, fg) } != 0;
            if !ours {
                anchor_widget(hwnd);
            }
        }
        TIMER_AUTOSAVE => {
            unsafe { KillTimer(hwnd, TIMER_AUTOSAVE) };
            commit_text_and_save(hwnd);
        }
        TIMER_BAR_CLICK => {
            // Pasó el tiempo de doble clic sin un segundo clic: era un
            // clic simple en la barra, que enrolla.
            unsafe { KillTimer(hwnd, TIMER_BAR_CLICK) };
            toggle_roll_manual(hwnd);
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
        WM_PRINTCLIENT => {
            paint(hwnd, wparam as HDC);
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
        WM_LBUTTONDBLCLK => {
            on_dblclick(hwnd, lparam);
            0
        }
        WM_NCHITTEST => on_nchittest(hwnd, lparam),
        WM_GETMINMAXINFO => {
            on_getminmaxinfo(lparam);
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
        // Clic en cualquier parte de una nota anclada (también en su
        // texto): pasa adelante de las otras notas. Las ventanas hijas
        // no cambian de orden solas.
        WM_MOUSEACTIVATE => {
            crate::desktop::raise(hwnd);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ACTIVATE => {
            if (wparam & 0xffff) as u32 == WA_INACTIVE {
                // Una nota asomada vuelve al escritorio al perder el
                // foco — con un respiro, para no reaccionar a
                // parpadeos de activación (ver TIMER_PEEK_END).
                if is_peeking(hwnd) {
                    SetTimer(hwnd, TIMER_PEEK_END, 250, None);
                }
                0
            } else {
                // Al activarse, el foco va al texto (DefWindowProc lo
                // dejaría en la ventana de la nota, sin cursor).
                if !crate::rename::is_renaming(hwnd) {
                    let id = note_id(hwnd);
                    let edit = { app().lock().unwrap().notes.get(&id).map(|nr| nr.edit as HWND) };
                    if let Some(edit) = edit.filter(|e| !e.is_null()) {
                        SetFocus(edit);
                        return 0;
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_CTLCOLOREDIT => match crate::rename::ctl_color(wparam as HDC, lparam as HWND) {
            Some(brush) => brush,
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        },
        crate::rename::WM_RENAME_DONE => {
            crate::rename::finish(hwnd, wparam != 0);
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
