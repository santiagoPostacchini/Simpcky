//! El cuerpo de una nota: un RichEdit con todo lo que le falta para ser
//! un editor de notas.
//!
//! - **Formato**: negrita, cursiva, subrayado y tachado (Ctrl+B/I/U/T o
//!   clic derecho). Se lee y se escribe como `richtext::Styles`, no como
//!   RTF (ver `richtext.rs`).
//! - **Listas**: viñetas y tareas con casilla, como prefijos del párrafo
//!   (`• `, `☐ `, `☑ `). Enter sigue la lista, Enter en un ítem vacío o
//!   Retroceso al lado de la marca la terminan, `[] ` y `- ` al comienzo
//!   de una línea la empiezan, y un clic en la casilla la tilda.
//! - **Emojis en color y casillas lindas**: el RichEdit dibuja con GDI
//!   (emojis en blanco y negro, casillas como un carácter de fuente); después
//!   de cada WM_PAINT se pintan encima (ver `paint_overlay`).
//! - **Pegar sin arrastrar formato ajeno**: lo pegado conserva negrita &
//!   cía., pero toma la letra, el tamaño y los colores de la nota.
//! - **Barra de desplazamiento fina**, del color de la nota: la de Windows
//!   queda fuera de la vista (ver `hidden_bar_width`) y se dibuja otra
//!   encima, que se ensancha con el mouse y se puede arrastrar.
//!
//! Los cambios "de presentación" (sangría de las listas, tareas hechas
//! en gris y tachadas, limpiar lo pegado) se hacen con el deshacer del
//! RichEdit suspendido (TOM, `ITextDocument::Undo(tomSuspend)`): así
//! Ctrl+Z deshace lo que hizo el usuario, no lo que acomodó la app.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Controls::{EM_CHARFROMPOS, EM_POSFROMCHAR};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::flyout::{self, Entry, Glyph, Tool};
use crate::richtext::{self, List, Styles, BOLD, ITALIC, STRIKE, STYLES, UNDERLINE};
use crate::win::wide;

// -----------------------------------------------------------------
// RichEdit a mano (Richedit.h: windows-sys no lo trae)
// -----------------------------------------------------------------

const EM_EXGETSEL: u32 = WM_USER + 52;
const EM_EXLINEFROMCHAR: u32 = WM_USER + 54;
const EM_EXSETSEL: u32 = WM_USER + 55;
const EM_GETCHARFORMAT: u32 = WM_USER + 58;
const EM_GETOLEINTERFACE: u32 = WM_USER + 60;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;
const EM_SETCHARFORMAT: u32 = WM_USER + 68;
const EM_SETEVENTMASK: u32 = WM_USER + 69;
const EM_SETPARAFORMAT: u32 = WM_USER + 71;
const EM_CANPASTE: u32 = WM_USER + 50;
const EM_GETTEXTEX: u32 = WM_USER + 94;
const EM_GETTEXTLENGTHEX: u32 = WM_USER + 95;
const EM_SETTYPOGRAPHYOPTIONS: u32 = WM_USER + 202;
const EM_GETSCROLLPOS: u32 = WM_USER + 221;
const EM_SETSCROLLPOS: u32 = WM_USER + 222;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_GETRECT: u32 = 0x00B2;
const EM_SETRECT: u32 = 0x00B3;
const EM_LINEINDEX: u32 = 0x00BB;
const EM_SETMODIFY: u32 = 0x00B9;
const EM_EMPTYUNDOBUFFER: u32 = 0x00CD;

const ENM_CHANGE: isize = 0x0001;
const TO_ADVANCEDTYPOGRAPHY: usize = 0x0001;
const SCF_SELECTION: usize = 0x0001;
const SCF_ALL: usize = 0x0004;
const ES_NOOLEDRAGDROP: u32 = 0x0008;
const ES_DISABLENOSCROLL: u32 = 0x2000;
const WM_MOUSELEAVE: u32 = 0x02A3;

const CFM_BOLD: u32 = 0x0000_0001;
const CFM_ITALIC: u32 = 0x0000_0002;
const CFM_UNDERLINE: u32 = 0x0000_0004;
const CFM_STRIKEOUT: u32 = 0x0000_0008;
const CFM_PROTECTED: u32 = 0x0000_0010;
const CFM_LINK: u32 = 0x0000_0020;
const CFM_SMALLCAPS: u32 = 0x0000_0040;
const CFM_ALLCAPS: u32 = 0x0000_0080;
const CFM_HIDDEN: u32 = 0x0000_0100;
const CFM_SUBSCRIPT: u32 = 0x0003_0000; // y superíndice
const CFM_BACKCOLOR: u32 = 0x0400_0000;
const CFM_OFFSET: u32 = 0x1000_0000;
const CFM_FACE: u32 = 0x2000_0000;
const CFM_COLOR: u32 = 0x4000_0000;
const CFM_SIZE: u32 = 0x8000_0000;
const CFE_AUTOBACKCOLOR: u32 = CFM_BACKCOLOR;
/// Los cuatro estilos, en el orden de `richtext` (b, i, u, s).
const STYLE_MASKS: [u32; STYLES] = [CFM_BOLD, CFM_ITALIC, CFM_UNDERLINE, CFM_STRIKEOUT];

const PFM_STARTINDENT: u32 = 0x0001;
const PFM_RIGHTINDENT: u32 = 0x0002;
const PFM_OFFSET: u32 = 0x0004;
const PFM_ALIGNMENT: u32 = 0x0008;
const PFM_NUMBERING: u32 = 0x0020;
const PFM_SPACEBEFORE: u32 = 0x0040;
const PFM_SPACEAFTER: u32 = 0x0080;
const PFM_LINESPACING: u32 = 0x0100;
const PFA_LEFT: u16 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct CharRange {
    min: i32,
    max: i32,
}

/// `CHARFORMAT2W`: mismo orden y tipos que el struct de Win32 (116 bytes).
#[repr(C)]
struct CharFormat {
    cb_size: u32,
    mask: u32,
    effects: u32,
    height: i32,
    offset: i32,
    color: u32,
    charset: u8,
    pitch_family: u8,
    face: [u16; 32],
    weight: u16,
    spacing: i16,
    back_color: u32,
    lcid: u32,
    reserved: u32,
    style: i16,
    kerning: u16,
    underline_type: u8,
    animation: u8,
    rev_author: u8,
    underline_color: u8,
}

impl CharFormat {
    fn new(mask: u32, effects: u32) -> Self {
        let mut cf: CharFormat = unsafe { std::mem::zeroed() };
        cf.cb_size = std::mem::size_of::<CharFormat>() as u32;
        cf.mask = mask;
        cf.effects = effects;
        cf
    }
}

/// `PARAFORMAT2` (188 bytes).
#[repr(C)]
struct ParaFormat {
    cb_size: u32,
    mask: u32,
    numbering: u16,
    effects: u16,
    start_indent: i32,
    right_indent: i32,
    offset: i32,
    alignment: u16,
    tab_count: i16,
    tabs: [i32; 32],
    space_before: i32,
    space_after: i32,
    line_spacing: i32,
    style: i16,
    line_spacing_rule: u8,
    outline_level: u8,
    shading_weight: u16,
    shading_style: u16,
    numbering_start: u16,
    numbering_style: u16,
    numbering_tab: u16,
    border_space: u16,
    border_width: u16,
    borders: u16,
}

#[repr(C)]
struct GetTextEx {
    cb: u32,
    flags: u32,
    codepage: u32,
    default_char: *const u8,
    used_default: *mut i32,
}

#[repr(C)]
struct GetTextLengthEx {
    flags: u32,
    codepage: u32,
}

// TOM (tom.h): solo tres métodos de ITextDocument, por su lugar en la
// tabla virtual (IUnknown 0-2, IDispatch 3-6, y después en orden).
const IID_ITEXTDOCUMENT: GUID = GUID::from_u128(0x8CC497C0_A1DF_11CE_8098_00AA0047BE5D);
const TOM_FREEZE: usize = 18;
const TOM_UNFREEZE: usize = 19;
const TOM_UNDO: usize = 22;
const TOM_SUSPEND: i32 = -9_999_995;
const TOM_RESUME: i32 = -9_999_994;

// Comandos del menú del clic derecho (llegan a la nota como WM_COMMAND
// y ella se los pasa a `command`).
pub const ID_FIRST: u32 = 3000;
const ID_STYLE: u32 = 3000; // + BOLD/ITALIC/UNDERLINE/STRIKE
const ID_BULLETS: u32 = 3010;
const ID_TODO: u32 = 3011;
const ID_CUT: u32 = 3020;
const ID_COPY: u32 = 3021;
const ID_PASTE: u32 = 3022;
const ID_SELECT_ALL: u32 = 3023;
const ID_EMOJI: u32 = 3024;
pub const ID_LAST: u32 = 3099;

struct EditState {
    doc: *mut c_void,
    body: u32,
    ink: u32,
    /// El WM_CHAR que sigue a una tecla que ya se resolvió acá (Enter,
    /// Retroceso) y que el RichEdit no tiene que volver a procesar.
    swallow: Option<u16>,
    /// Qué párrafo es qué clase de lista, la última vez que se acomodó
    /// la presentación (ver `refresh`).
    lists: Vec<(usize, List)>,
    /// Dónde estaban los emojis la última vez que se les puso su letra
    /// (ver `fix_faces`).
    emojis: Vec<(usize, usize)>,
    /// El mouse está sobre la barra de desplazamiento propia.
    bar_hover: bool,
    /// Arrastrando la barra: a qué altura del pulgar se lo agarró.
    bar_drag: Option<i32>,
    /// Modo vidrio (ver `glass.rs`): opacidad del color de la nota sobre
    /// el desenfoque de Windhawk. `None`: liso.
    glass: Option<u8>,
}

/// Temporizador del RichEdit para acomodar la presentación un momento
/// después de cada cambio (ver `schedule_refresh`).
const TIMER_REFRESH: usize = 0x5E1C;

thread_local! {
    static EDITS: RefCell<HashMap<isize, EditState>> = RefCell::new(HashMap::new());
}

fn with_state<R>(edit: HWND, f: impl FnOnce(&mut EditState) -> R) -> Option<R> {
    EDITS.with(|m| m.borrow_mut().get_mut(&(edit as isize)).map(f))
}

unsafe fn send(edit: HWND, msg: u32, w: usize, l: isize) -> isize {
    SendMessageW(edit, msg, w, l)
}

// -----------------------------------------------------------------
// Creación
// -----------------------------------------------------------------

pub fn create(parent: HWND, hinstance: HINSTANCE, id: usize, rc: RECT) -> HWND {
    let class = wide("RICHEDIT50W");
    unsafe {
        let edit = CreateWindowExW(
            // Transparente: el RichEdit pinta solo el texto, y el fondo lo
            // pone `paint_background` (liso, o translúcido en modo vidrio).
            WS_EX_TRANSPARENT,
            class.as_ptr(),
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | (ES_MULTILINE as u32)
                | (ES_AUTOVSCROLL as u32)
                | (ES_WANTRETURN as u32)
                // La barra de Windows siempre presente (deshabilitada si no
                // hace falta): así el ancho del texto no cambia cuando
                // aparece, y la nota la deja fuera de la vista.
                | ES_DISABLENOSCROLL
                // Arrastrar texto desde otra app traería su formato (y no
                // pasa por `on_paste`); adentro de una nota casi no se usa.
                | ES_NOOLEDRAGDROP,
            rc.left,
            rc.top,
            rc.right - rc.left,
            (rc.bottom - rc.top).max(0),
            parent,
            id as HMENU,
            hinstance,
            null(),
        );
        if edit.is_null() {
            return edit;
        }
        // Mide bien los emojis (sin esto quedan montados sobre la letra
        // que sigue) y los textos complejos.
        send(edit, EM_SETTYPOGRAPHYOPTIONS, TO_ADVANCEDTYPOGRAPHY, TO_ADVANCEDTYPOGRAPHY as isize);
        // Sin esto el RichEdit no avisa EN_CHANGE, y el autoguardado de la
        // nota nunca se enteraba de lo que se escribía.
        send(edit, EM_SETEVENTMASK, 0, ENM_CHANGE);
        let mut ole: *mut c_void = null_mut();
        let mut doc: *mut c_void = null_mut();
        if send(edit, EM_GETOLEINTERFACE, 0, &mut ole as *mut *mut c_void as isize) != 0 && !ole.is_null() {
            let qi: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32 = method(ole, 0);
            qi(ole, &IID_ITEXTDOCUMENT, &mut doc);
            com_release(ole);
        }
        EDITS.with(|m| {
            m.borrow_mut().insert(edit as isize, EditState {
                    doc,
                    body: 0xffffff,
                    ink: 0,
                    swallow: None,
                    lists: Vec::new(),
                    emojis: Vec::new(),
                    bar_hover: false,
                    bar_drag: None,
                    glass: None,
                })
        });
        SetWindowSubclass(edit, Some(subclass_proc), 1, 0);
        edit
    }
}

unsafe fn method<F: Copy>(obj: *mut c_void, index: usize) -> F {
    let vtbl = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(index))
}

unsafe fn com_release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = method(obj, 2);
        f(obj);
    }
}

fn tom(edit: HWND, index: usize, arg: i32) {
    let doc = with_state(edit, |s| s.doc).unwrap_or(null_mut());
    if doc.is_null() {
        return;
    }
    unsafe {
        let mut count: i32 = 0;
        if index == TOM_UNDO {
            let f: unsafe extern "system" fn(*mut c_void, i32, *mut i32) -> i32 = method(doc, index);
            f(doc, arg, &mut count);
        } else {
            let f: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = method(doc, index);
            f(doc, &mut count);
        }
    }
}

/// Para cambios que el usuario no hizo: sin redibujar a mitad de camino,
/// sin entrar en el deshacer, sin avisos, y con la selección y el
/// desplazamiento como estaban.
struct Quiet {
    edit: HWND,
    sel: CharRange,
    scroll: POINT,
    mask: isize,
}

impl Quiet {
    fn new(edit: HWND) -> Quiet {
        let mut sel = CharRange { min: 0, max: 0 };
        let mut scroll = POINT { x: 0, y: 0 };
        unsafe {
            send(edit, EM_EXGETSEL, 0, &mut sel as *mut CharRange as isize);
            send(edit, EM_GETSCROLLPOS, 0, &mut scroll as *mut POINT as isize);
        }
        let mask = unsafe { send(edit, EM_SETEVENTMASK, 0, 0) };
        tom(edit, TOM_FREEZE, 0);
        tom(edit, TOM_UNDO, TOM_SUSPEND);
        Quiet { edit, sel, scroll, mask }
    }
}

impl Drop for Quiet {
    fn drop(&mut self) {
        unsafe {
            send(self.edit, EM_EXSETSEL, 0, &self.sel as *const CharRange as isize);
            send(self.edit, EM_SETSCROLLPOS, 0, &self.scroll as *const POINT as isize);
        }
        tom(self.edit, TOM_UNDO, TOM_RESUME);
        tom(self.edit, TOM_UNFREEZE, 0);
        unsafe { send(self.edit, EM_SETEVENTMASK, 0, self.mask) };
    }
}

// -----------------------------------------------------------------
// Texto y formato
// -----------------------------------------------------------------

/// El texto tal cual lo cuenta el RichEdit: párrafos separados por `\r`,
/// posiciones en unidades UTF-16.
fn units(edit: HWND) -> Vec<u16> {
    unsafe {
        let gtl = GetTextLengthEx { flags: 2 | 8, codepage: 1200 }; // GTL_PRECISE | GTL_NUMCHARS
        let len = send(edit, EM_GETTEXTLENGTHEX, &gtl as *const GetTextLengthEx as usize, 0).max(0) as usize;
        let mut buf = vec![0u16; len + 2];
        let gt = GetTextEx { cb: (buf.len() * 2) as u32, flags: 0, codepage: 1200, default_char: null(), used_default: null_mut() };
        let n = send(edit, EM_GETTEXTEX, &gt as *const GetTextEx as usize, buf.as_mut_ptr() as isize).max(0) as usize;
        buf.truncate(n.min(len + 1));
        buf
    }
}

fn select(edit: HWND, a: usize, b: usize) {
    let r = CharRange { min: a as i32, max: b as i32 };
    unsafe { send(edit, EM_EXSETSEL, 0, &r as *const CharRange as isize) };
}

fn selection(edit: HWND) -> (usize, usize) {
    let mut r = CharRange { min: 0, max: 0 };
    unsafe { send(edit, EM_EXGETSEL, 0, &mut r as *mut CharRange as isize) };
    (r.min.max(0) as usize, r.max.max(0) as usize)
}

fn set_format(edit: HWND, scope: usize, cf: &CharFormat) {
    unsafe { send(edit, EM_SETCHARFORMAT, scope, cf as *const CharFormat as isize) };
}

fn get_format(edit: HWND) -> CharFormat {
    let mut cf = CharFormat::new(0, 0);
    unsafe { send(edit, EM_GETCHARFORMAT, SCF_SELECTION, &mut cf as *mut CharFormat as isize) };
    cf
}

fn replace(edit: HWND, a: usize, b: usize, with: &str, undoable: bool) {
    select(edit, a, b);
    let w = wide(with);
    unsafe { send(edit, EM_REPLACESEL, undoable as usize, w.as_ptr() as isize) };
}

/// Un tono entre la tinta y el fondo: para las tareas hechas.
fn faded(ink: u32, body: u32) -> u32 {
    let mix = |sh: u32| (((ink >> sh) & 0xff) * 45 + ((body >> sh) & 0xff) * 55) / 100;
    mix(0) | (mix(8) << 8) | (mix(16) << 16)
}

/// Letra, tamaño y color de la nota para [a, b) (o todo), sin tocar
/// negrita/cursiva/subrayado/tachado.
fn base_format(edit: HWND, range: Option<(usize, usize)>, ink: u32) {
    let mut cf = CharFormat::new(
        CFM_FACE | CFM_SIZE | CFM_COLOR | CFM_BACKCOLOR | CFM_OFFSET | CFM_LINK | CFM_PROTECTED | CFM_HIDDEN | CFM_SUBSCRIPT | CFM_SMALLCAPS | CFM_ALLCAPS,
        CFE_AUTOBACKCOLOR,
    );
    let face = wide("Segoe UI");
    cf.face[..face.len()].copy_from_slice(&face);
    cf.height = 240; // 12 pt: los 16 px de siempre
    cf.color = ink;
    match range {
        Some((a, b)) => {
            select(edit, a, b);
            set_format(edit, SCF_SELECTION, &cf);
        }
        None => set_format(edit, SCF_ALL, &cf),
    }
}

/// Sangría, espaciado y alineación normales en los párrafos de [a, b].
fn base_paragraphs(edit: HWND, a: usize, b: usize) {
    let mut pf: ParaFormat = unsafe { std::mem::zeroed() };
    pf.cb_size = std::mem::size_of::<ParaFormat>() as u32;
    pf.mask = PFM_STARTINDENT | PFM_RIGHTINDENT | PFM_OFFSET | PFM_ALIGNMENT | PFM_NUMBERING | PFM_SPACEBEFORE | PFM_SPACEAFTER | PFM_LINESPACING;
    pf.alignment = PFA_LEFT;
    select(edit, a, b);
    unsafe { send(edit, EM_SETPARAFORMAT, 0, &pf as *const ParaFormat as isize) };
}

fn pos(edit: HWND, cp: usize) -> POINT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { send(edit, EM_POSFROMCHAR, &mut p as *mut POINT as usize, cp as isize) };
    p
}

fn twips(edit: HWND, px: i32) -> i32 {
    let dpi = unsafe { GetDpiForWindow(edit) }.max(96) as i32;
    px * 1440 / dpi
}

/// Acomoda la presentación de los párrafos de `paras`: sangría colgante
/// en las listas (el texto que pasa de línea queda alineado después de
/// la marca) y las tareas hechas en gris y tachadas.
fn decorate(edit: HWND, text: &[u16], paras: &[(usize, usize)], ink: u32, body: u32) {
    for &(a, b) in paras {
        let (kind, plen) = richtext::list_of(&text[a..b]);
        let mut pf: ParaFormat = unsafe { std::mem::zeroed() };
        pf.cb_size = std::mem::size_of::<ParaFormat>() as u32;
        pf.mask = PFM_STARTINDENT | PFM_OFFSET;
        if kind != List::None {
            let hang = pos(edit, a + plen).x - pos(edit, a).x;
            pf.offset = twips(edit, hang.max(0));
        }
        select(edit, a, a);
        unsafe { send(edit, EM_SETPARAFORMAT, 0, &pf as *const ParaFormat as isize) };
        if let List::Todo { done } = kind {
            if b > a + plen {
                select(edit, a + plen, b);
                let mut cf = CharFormat::new(CFM_COLOR | CFM_STRIKEOUT, if done { CFM_STRIKEOUT } else { 0 });
                cf.color = if done { faded(ink, body) } else { ink };
                if done {
                    set_format(edit, SCF_SELECTION, &cf);
                } else {
                    // Destildada: vuelve el color, pero el tachado solo si
                    // lo había puesto la tarea (ver `toggle_todo`).
                    cf.mask = CFM_COLOR;
                    set_format(edit, SCF_SELECTION, &cf);
                }
            }
        }
    }
}

/// Todo el texto en Segoe UI y cada emoji en Segoe UI Emoji. Si no, el
/// RichEdit elige por su cuenta de qué letra sacar cada emoji (❤ sale de
/// Segoe UI Symbol, en versión texto) y le reserva ese ancho, más angosto
/// que el del emoji en color que se pinta encima: quedaban recortados.
fn fix_faces(edit: HWND, text: &[u16]) -> Vec<(usize, usize)> {
    let face = |name: &str| {
        let mut cf = CharFormat::new(CFM_FACE, 0);
        let w = wide(name);
        cf.face[..w.len()].copy_from_slice(&w);
        cf
    };
    set_format(edit, SCF_ALL, &face("Segoe UI"));
    let clusters = richtext::emoji_clusters(text);
    let emoji = face("Segoe UI Emoji");
    for &(a, b) in &clusters {
        select(edit, a, b);
        set_format(edit, SCF_SELECTION, &emoji);
    }
    clusters
}

/// Lo que se escriba a continuación, con la letra del texto (no la del
/// emoji que quedó justo antes del cursor).
fn plain_typing_face(edit: HWND) {
    let (a, b) = selection(edit);
    if a == b {
        let mut cf = CharFormat::new(CFM_FACE, 0);
        let w = wide("Segoe UI");
        cf.face[..w.len()].copy_from_slice(&w);
        set_format(edit, SCF_SELECTION, &cf);
    }
}

fn list_signature(text: &[u16]) -> Vec<(usize, List)> {
    richtext::paragraphs(text)
        .into_iter()
        .enumerate()
        .map(|(i, (a, b))| (i, richtext::list_of(&text[a..b]).0))
        .filter(|(_, k)| *k != List::None)
        .collect()
}

fn colors(edit: HWND) -> (u32, u32) {
    with_state(edit, |s| (s.ink, s.body)).unwrap_or((0, 0xffffff))
}

/// Pone el contenido de una nota (texto y estilos) en el RichEdit, con
/// la letra y los colores de la nota. Deja el cursor al principio y el
/// deshacer vacío: lo cargado no es algo que se pueda "deshacer".
pub fn load(edit: HWND, text: &str, fmt: &str) {
    let (ink, body) = colors(edit);
    {
        let _q = Quiet::new(edit);
        let w = wide(&text.replace('\n', "\r"));
        unsafe { send(edit, WM_SETTEXT, 0, w.as_ptr() as isize) };
        base_format(edit, None, ink);
        let all = units(edit);
        base_paragraphs(edit, 0, all.len());
        let st = richtext::styles_from_str(fmt, all.len() as u32);
        for (k, spans) in st.spans.iter().enumerate() {
            for &(s, l) in spans {
                select(edit, s as usize, (s + l) as usize);
                set_format(edit, SCF_SELECTION, &CharFormat::new(STYLE_MASKS[k], STYLE_MASKS[k]));
            }
        }
        let paras = richtext::paragraphs(&all);
        decorate(edit, &all, &paras, ink, body);
        let emojis = fix_faces(edit, &all);
        with_state(edit, |s| {
            s.lists = list_signature(&all);
            s.emojis = emojis;
        });
    }
    // Cursor y vista al principio: una nota larga se ve desde arriba.
    select(edit, 0, 0);
    unsafe { send(edit, EM_SETSCROLLPOS, 0, &POINT { x: 0, y: 0 } as *const POINT as isize) };
    plain_typing_face(edit);
    unsafe {
        send(edit, EM_SETMODIFY, 0, 0);
        send(edit, EM_EMPTYUNDOBUFFER, 0, 0);
    }
}

/// Colores de la nota (fondo y tinta), para el color o el tema nuevos.
pub fn set_colors(edit: HWND, body: u32, ink: u32) {
    with_state(edit, |s| {
        s.body = body;
        s.ink = ink;
    });
    unsafe { send(edit, EM_SETBKGNDCOLOR, 0, body as isize) };
    let _q = Quiet::new(edit);
    let mut cf = CharFormat::new(CFM_COLOR, 0);
    cf.color = ink;
    set_format(edit, SCF_ALL, &cf);
    let all = units(edit);
    let paras = richtext::paragraphs(&all);
    decorate(edit, &all, &paras, ink, body);
    // Lo que se escriba de ahora en más, también con la tinta nueva.
    drop(_q);
    let (a, b) = selection(edit);
    if a == b {
        set_format(edit, SCF_SELECTION, &cf);
    }
    unsafe { InvalidateRect(edit, null(), 1) };
}

/// Si cambió qué párrafos son listas (se tildó con Ctrl+Z, se borró una
/// marca a mano…) o dónde hay emojis, vuelve a acomodar la presentación.
pub fn refresh(edit: HWND) {
    let all = units(edit);
    let sig = list_signature(&all);
    let emojis = richtext::emoji_clusters(&all);
    let (lists_changed, emojis_changed) = with_state(edit, |s| (s.lists != sig, s.emojis != emojis)).unwrap_or((false, false));
    if !lists_changed && !emojis_changed {
        return;
    }
    {
        let _q = Quiet::new(edit);
        if lists_changed {
            let (ink, body) = colors(edit);
            let paras = richtext::paragraphs(&all);
            decorate(edit, &all, &paras, ink, body);
        }
        if emojis_changed {
            fix_faces(edit, &all);
        }
        with_state(edit, |s| {
            s.lists = sig;
            s.emojis = emojis;
        });
    }
    plain_typing_face(edit);
}

/// Hubo un cambio en el texto: la presentación se acomoda apenas se deja
/// de escribir (no en cada tecla).
pub fn schedule_refresh(edit: HWND) {
    unsafe {
        SetTimer(edit, TIMER_REFRESH, 80, None);
        paint_overlay(edit);
        // De vacía a con texto (o al revés): la ayuda en gris ocupa varias
        // líneas, y el RichEdit solo repintaría la primera.
        let gtl = GetTextLengthEx { flags: 2 | 8, codepage: 1200 };
        if send(edit, EM_GETTEXTLENGTHEX, &gtl as *const GetTextLengthEx as usize, 0) <= 1 {
            InvalidateRect(edit, null(), 1);
        }
    }
}

/// El contenido para guardar: el texto (con `\n`) y sus estilos.
pub fn read(edit: HWND) -> (String, String) {
    let all = units(edit);
    let text = String::from_utf16_lossy(&all).replace('\r', "\n");
    let n = all.len();
    if n == 0 {
        return (text, String::new());
    }
    let mut st = Styles::default();
    {
        let _q = Quiet::new(edit);
        // Recorre el texto por tramos de formato parejo: agranda el tramo
        // al doble mientras siga parejo y después ajusta con búsqueda
        // binaria. Unos pocos mensajes por tramo, no uno por letra.
        let uniform = |a: usize, b: usize| -> Option<u32> {
            select(edit, a, b);
            let cf = get_format(edit);
            let all_masks = CFM_BOLD | CFM_ITALIC | CFM_UNDERLINE | CFM_STRIKEOUT;
            (cf.mask & all_masks == all_masks).then_some(cf.effects & all_masks)
        };
        let mut p = 0;
        while p < n {
            let state = uniform(p, p + 1).unwrap_or(0);
            let (mut lo, mut hi, mut step) = (p + 1, n + 1, 1);
            while lo < n {
                let q = (lo + step).min(n);
                if uniform(p, q) == Some(state) {
                    lo = q;
                    step *= 2;
                } else {
                    hi = q;
                    break;
                }
            }
            while hi > lo + 1 {
                let mid = (lo + hi) / 2;
                if uniform(p, mid) == Some(state) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            for (k, m) in STYLE_MASKS.iter().enumerate() {
                if state & m != 0 {
                    st.spans[k].push((p as u32, (lo - p) as u32));
                }
            }
            p = lo;
        }
    }
    // El tachado de las tareas hechas es de la tarea, no del texto.
    for (a, b) in richtext::paragraphs(&all) {
        let (kind, plen) = richtext::list_of(&all[a..b]);
        if kind == (List::Todo { done: true }) {
            st.clear_range(STRIKE, (a + plen) as u32, b as u32);
        }
    }
    st.normalize(n as u32);
    (text, richtext::styles_to_string(&st))
}

/// Qué estilos tiene la selección (todos parejos), para los botones del
/// menú.
fn style_state(edit: HWND) -> [bool; STYLES] {
    let cf = get_format(edit);
    let mut out = [false; STYLES];
    for (k, m) in STYLE_MASKS.iter().enumerate() {
        out[k] = cf.mask & m != 0 && cf.effects & m != 0;
    }
    out
}

/// Avisa a la nota que el contenido cambió (el RichEdit no manda
/// EN_CHANGE por un cambio de formato solo).
fn changed(edit: HWND) {
    unsafe {
        let parent = GetParent(edit);
        let id = GetDlgCtrlID(edit) as usize;
        SendMessageW(parent, WM_COMMAND, id | ((EN_CHANGE as usize) << 16), edit as isize);
    }
}

/// Negrita, cursiva, subrayado o tachado en la selección (o para lo que
/// se escriba a continuación, si no hay selección).
pub fn toggle_style(edit: HWND, style: usize) {
    let on = style_state(edit)[style];
    set_format(edit, SCF_SELECTION, &CharFormat::new(STYLE_MASKS[style], if on { 0 } else { STYLE_MASKS[style] }));
    changed(edit);
}

/// El párrafo (inicio, fin) que contiene `cp`.
fn paragraph_at(text: &[u16], cp: usize) -> (usize, usize) {
    richtext::paragraphs(text).into_iter().find(|&(a, b)| cp >= a && cp <= b).unwrap_or((text.len(), text.len()))
}

/// Viñetas o tareas en los párrafos seleccionados; si ya lo eran todos,
/// las saca.
pub fn toggle_list(edit: HWND, want: List) {
    let text = units(edit);
    let (s, e) = selection(edit);
    let paras: Vec<(usize, usize)> =
        richtext::paragraphs(&text).into_iter().filter(|&(a, b)| (a <= e && b >= s) || (s == e && s >= a && s <= b)).collect();
    if paras.is_empty() {
        return;
    }
    let same = |k: List| matches!((k, want), (List::Bullet, List::Bullet) | (List::Todo { .. }, List::Todo { .. }));
    let all_same = paras.iter().all(|&(a, b)| same(richtext::list_of(&text[a..b]).0));
    let (mut ns, mut ne) = (s as i64, e as i64);
    for &(a, b) in paras.iter().rev() {
        let (_, plen) = richtext::list_of(&text[a..b]);
        let new = if all_same { "" } else { want.prefix() };
        replace(edit, a, a + plen, new, true);
        let delta = richtext::utf16_len(new) as i64 - plen as i64;
        if (a as i64) < ns || (a as usize == s && s == e) {
            ns = (ns + delta).max(a as i64);
        }
        if (a as i64) < ne || (a as usize == e) {
            ne = (ne + delta).max(a as i64);
        }
    }
    after_list_change(edit);
    select(edit, ns.max(0) as usize, ne.max(ns).max(0) as usize);
    changed(edit);
}

/// Tras sumar o sacar marcas de lista: presentación de todo al día y la
/// tinta normal para lo que se escriba (el ítem nuevo no hereda el gris
/// de una tarea hecha).
fn after_list_change(edit: HWND) {
    let (ink, body) = colors(edit);
    let all = units(edit);
    {
        let _q = Quiet::new(edit);
        let paras = richtext::paragraphs(&all);
        decorate(edit, &all, &paras, ink, body);
    }
    with_state(edit, |s| s.lists = list_signature(&all));
}

/// Tilda o destilda la tarea del párrafo que empieza en `a`.
fn toggle_todo(edit: HWND, a: usize) {
    let text = units(edit);
    let (_, b) = paragraph_at(&text, a);
    let (kind, plen) = richtext::list_of(&text[a..b]);
    let List::Todo { done } = kind else { return };
    let (s, e) = selection(edit);
    replace(edit, a, a + 1, &List::Todo { done: !done }.prefix()[..3], true);
    if done && b > a + plen {
        // Destildar saca el tachado que había puesto la tarea.
        let _q = Quiet::new(edit);
        select(edit, a + plen, b);
        set_format(edit, SCF_SELECTION, &CharFormat::new(CFM_STRIKEOUT, 0));
    }
    after_list_change(edit);
    select(edit, s, e);
    changed(edit);
}

/// Enter en una lista: sigue la lista, o la termina si el ítem está
/// vacío. `false` si no es una lista (y el Enter es uno común).
fn on_enter(edit: HWND) -> bool {
    let text = units(edit);
    let (s, e) = selection(edit);
    let (a, b) = paragraph_at(&text, s);
    let (kind, plen) = richtext::list_of(&text[a..b]);
    if kind == List::None || s < a + plen {
        return false;
    }
    let empty = text[a + plen..b].iter().all(|&c| c == ' ' as u16);
    if empty && s == e {
        replace(edit, a, b, "", true);
        after_list_change(edit);
        select(edit, a, a);
        changed(edit);
        return true;
    }
    let next = match kind {
        List::Todo { .. } => List::Todo { done: false },
        k => k,
    };
    replace(edit, s, e, &format!("\r{}", next.prefix()), true);
    let caret = s + 1 + richtext::utf16_len(next.prefix()) as usize;
    let (ink, _) = colors(edit);
    {
        // El ítem nuevo, limpio: sin el gris ni el tachado de una tarea
        // hecha que se partió al medio.
        let _q = Quiet::new(edit);
        select(edit, s + 1, caret);
        let mut cf = CharFormat::new(CFM_COLOR | CFM_STRIKEOUT, 0);
        cf.color = ink;
        set_format(edit, SCF_SELECTION, &cf);
    }
    after_list_change(edit);
    select(edit, caret, caret);
    let mut cf = CharFormat::new(CFM_COLOR | CFM_STRIKEOUT, 0);
    cf.color = ink;
    set_format(edit, SCF_SELECTION, &cf);
    true
}

/// Retroceso justo después de la marca: saca la marca (el párrafo sigue,
/// como texto común). Suprimir al final de un párrafo que sigue con un
/// ítem: lo une sin arrastrar la marca de ese ítem.
fn on_delete_key(edit: HWND, back: bool) -> bool {
    let text = units(edit);
    let (s, e) = selection(edit);
    if s != e {
        return false;
    }
    let (a, b) = paragraph_at(&text, s);
    if back {
        let (kind, plen) = richtext::list_of(&text[a..b]);
        if kind == List::None || s != a + plen {
            return false;
        }
        replace(edit, a, a + plen, "", true);
        after_list_change(edit);
        select(edit, a, a);
        changed(edit);
        return true;
    }
    if s != b || b >= text.len() {
        return false;
    }
    let (na, nb) = paragraph_at(&text, b + 1);
    let (kind, plen) = richtext::list_of(&text[na..nb]);
    if kind == List::None {
        return false;
    }
    replace(edit, b, na + plen, "", true);
    after_list_change(edit);
    select(edit, b, b);
    changed(edit);
    true
}

/// Espacio después de `[]`, `-`… al comienzo de una línea: la vuelve una
/// lista.
fn on_space(edit: HWND) -> bool {
    let text = units(edit);
    let (s, e) = selection(edit);
    if s != e {
        return false;
    }
    let (a, b) = paragraph_at(&text, s);
    if richtext::list_of(&text[a..b]).0 != List::None {
        return false;
    }
    let Some(kind) = richtext::shortcut_list(&text[a..s]) else { return false };
    replace(edit, a, s, kind.prefix(), true);
    let caret = a + richtext::utf16_len(kind.prefix()) as usize;
    after_list_change(edit);
    select(edit, caret, caret);
    changed(edit);
    true
}

/// Pegar: lo pegado toma la letra, el tamaño y los colores de la nota
/// (se quedan la negrita y compañía).
fn on_paste(edit: HWND) {
    let before = units(edit).len();
    let (s, e) = selection(edit);
    unsafe { DefSubclassProc(edit, WM_PASTE, 0, 0) };
    let after = units(edit).len();
    let end = (s + (after + (e - s)).saturating_sub(before)).min(after);
    if end <= s {
        return;
    }
    let (ink, body) = colors(edit);
    {
        let _q = Quiet::new(edit);
        base_format(edit, Some((s, end)), ink);
        base_paragraphs(edit, s, end);
        let all = units(edit);
        let paras = richtext::paragraphs(&all);
        decorate(edit, &all, &paras, ink, body);
        with_state(edit, |st| st.lists = list_signature(&all));
    }
    changed(edit);
}

/// La casilla de tarea en el punto (`x`, `y`) del RichEdit: el inicio de
/// su párrafo.
fn checkbox_at(edit: HWND, x: i32, y: i32) -> Option<usize> {
    let pt = POINT { x, y };
    let cp = unsafe { send(edit, EM_CHARFROMPOS, 0, &pt as *const POINT as isize) }.max(0) as usize;
    let text = units(edit);
    let (a, b) = paragraph_at(&text, cp.min(text.len()));
    let (kind, _) = richtext::list_of(&text[a..b]);
    if !matches!(kind, List::Todo { .. }) {
        return None;
    }
    let p0 = pos(edit, a);
    let p1 = pos(edit, a + 1);
    let right = if p1.y == p0.y && p1.x > p0.x { p1.x } else { p0.x + 16 };
    let bottom = line_bottom(edit, a, p0.y);
    (x >= p0.x - 4 && x < right + 2 && y >= p0.y && y < bottom).then_some(a)
}

// -----------------------------------------------------------------
// Menú del clic derecho y comandos
// -----------------------------------------------------------------

fn show_menu(edit: HWND, screen: Option<POINT>) {
    let at = match screen {
        Some(p) => {
            // Clic derecho fuera de la selección: el cursor va ahí (como
            // en Word), así el formato se aplica donde se hizo clic.
            let mut c = p;
            unsafe { ScreenToClient(edit, &mut c) };
            let cp = unsafe { send(edit, EM_CHARFROMPOS, 0, &c as *const POINT as isize) }.max(0) as usize;
            let (s, e) = selection(edit);
            if cp < s || cp > e || s == e {
                select(edit, cp, cp);
            }
            p
        }
        None => {
            let (s, _) = selection(edit);
            let mut p = pos(edit, s);
            p.y = line_bottom(edit, s, p.y);
            unsafe { ClientToScreen(edit, &mut p) };
            p
        }
    };
    let (s, e) = selection(edit);
    let styles = style_state(edit);
    let text = units(edit);
    let (a, b) = paragraph_at(&text, s);
    let kind = richtext::list_of(&text[a..b]).0;
    let has_sel = s != e;
    let can_paste = unsafe { send(edit, EM_CANPASTE, 0, 0) } != 0;
    let tool = |id: u32, glyph: Glyph, active: bool| Tool { id, glyph, active };
    let entries = vec![
        Entry::Tools(vec![
            tool(ID_STYLE + BOLD as u32, Glyph::Letter('B', BOLD), styles[BOLD]),
            tool(ID_STYLE + ITALIC as u32, Glyph::Letter('I', ITALIC), styles[ITALIC]),
            tool(ID_STYLE + UNDERLINE as u32, Glyph::Letter('U', UNDERLINE), styles[UNDERLINE]),
            tool(ID_STYLE + STRIKE as u32, Glyph::Letter('S', STRIKE), styles[STRIKE]),
            tool(ID_BULLETS, Glyph::Icon(0xE8FD), kind == List::Bullet),
            tool(ID_TODO, Glyph::Icon(0xE73A), matches!(kind, List::Todo { .. })),
        ]),
        Entry::Separator,
        Entry::item(ID_CUT, 0xE8C6, "Cortar", "Ctrl+X").enabled(has_sel),
        Entry::item(ID_COPY, 0xE8C8, "Copiar", "Ctrl+C").enabled(has_sel),
        Entry::item(ID_PASTE, 0xE77F, "Pegar", "Ctrl+V").enabled(can_paste),
        Entry::item(ID_SELECT_ALL, 0xE8B3, "Seleccionar todo", "Ctrl+A"),
        Entry::Separator,
        Entry::item(ID_EMOJI, 0xE76E, "Emoji", "Win+."),
    ];
    let owner = unsafe { GetParent(edit) };
    flyout::show(owner, entries, flyout::Anchor::Point(at.x, at.y));
}

/// Un comando del menú del clic derecho (o de un atajo).
pub fn command(edit: HWND, id: u32) {
    unsafe {
        match id {
            _ if (ID_STYLE..ID_STYLE + STYLES as u32).contains(&id) => toggle_style(edit, (id - ID_STYLE) as usize),
            ID_BULLETS => toggle_list(edit, List::Bullet),
            ID_TODO => toggle_list(edit, List::Todo { done: false }),
            ID_CUT => {
                send(edit, WM_CUT, 0, 0);
            }
            ID_COPY => {
                send(edit, WM_COPY, 0, 0);
            }
            ID_PASTE => {
                send(edit, WM_PASTE, 0, 0);
            }
            ID_SELECT_ALL => select(edit, 0, units(edit).len()),
            ID_EMOJI => open_emoji_panel(edit),
            _ => {}
        }
    }
}

pub fn toggle_bullets(edit: HWND) {
    toggle_list(edit, List::Bullet);
}

pub fn toggle_todos(edit: HWND) {
    toggle_list(edit, List::Todo { done: false });
}

/// El panel de emojis de Windows (Win+.), para el texto de la nota.
fn open_emoji_panel(edit: HWND) {
    unsafe {
        SetFocus(edit);
        let key = |vk: u16, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
            },
        };
        let inputs = [key(VK_LWIN, false), key(VK_OEM_PERIOD, false), key(VK_OEM_PERIOD, true), key(VK_LWIN, true)];
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

// -----------------------------------------------------------------
// Dibujo encima del RichEdit: emojis en color y casillas
// -----------------------------------------------------------------

/// Tamaño de letra del texto, en píxeles (los 12 pt de `base_format`).
fn em_px(edit: HWND) -> i32 {
    let dpi = unsafe { GetDpiForWindow(edit) }.max(96) as i32;
    (240 * dpi / 1440).max(8)
}

/// Dónde termina la línea que empieza (o contiene) `cp` y arranca en `top`.
fn line_bottom(edit: HWND, cp: usize, top: i32) -> i32 {
    unsafe {
        let line = send(edit, EM_EXLINEFROMCHAR, 0, cp as isize);
        let next = send(edit, EM_LINEINDEX, (line + 1) as usize, 0);
        if next > 0 {
            let y = pos(edit, next as usize).y;
            if y > top {
                return y;
            }
        }
    }
    top + em_px(edit) * 4 / 3
}

unsafe fn paint_overlay(edit: HWND) {
    if IsWindowVisible(edit) == 0 {
        return; // nota enrollada
    }
    paint_marks(edit);
    paint_bar(edit);
}

// -----------------------------------------------------------------
// Barra de desplazamiento propia
// -----------------------------------------------------------------

/// Ancho de la barra de Windows del RichEdit: la nota lo hace más ancho
/// en esa medida, así la barra queda del otro lado del borde, recortada.
/// El RichEdit la sigue manejando (rueda, teclado, rangos); acá solo se
/// dibuja una más fina encima.
pub fn hidden_bar_width(hwnd: HWND) -> i32 {
    unsafe {
        windows_sys::Win32::UI::HiDpi::GetSystemMetricsForDpi(SM_CXVSCROLL, GetDpiForWindow(hwnd).max(96))
    }
}

fn scale(edit: HWND, v: i32) -> i32 {
    v * unsafe { GetDpiForWindow(edit) }.max(96) as i32 / 96
}

/// La franja de la barra (a la derecha, dentro del margen del texto), el
/// recorrido y el pulgar, o `None` si el texto entra entero.
struct Bar {
    strip: RECT,
    track: RECT,
    thumb: RECT,
    min: i32,
    range: i32,
    page: i32,
}

fn bar(edit: HWND) -> (RECT, Option<Bar>) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(edit, &mut rc) };
    let strip = RECT { left: rc.right - scale(edit, 14), top: 0, right: rc.right, bottom: rc.bottom };
    let mut si: SCROLLINFO = unsafe { std::mem::zeroed() };
    si.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
    si.fMask = SIF_ALL;
    if unsafe { GetScrollInfo(edit, SB_VERT as i32, &mut si) } == 0 {
        return (strip, None);
    }
    let range = si.nMax - si.nMin + 1;
    let page = si.nPage as i32;
    if page <= 0 || range <= page {
        return (strip, None);
    }
    let track = RECT { left: strip.left, top: scale(edit, 2), right: strip.right, bottom: rc.bottom - scale(edit, 2) };
    let track_h = (track.bottom - track.top).max(1);
    let th = (track_h * page / range).max(scale(edit, 28)).min(track_h);
    let ty = track.top + (track_h - th) * (si.nPos - si.nMin).clamp(0, range - page) / (range - page).max(1);
    let thumb = RECT { left: strip.left, top: ty, right: strip.right, bottom: ty + th };
    (strip, Some(Bar { strip, track, thumb, min: si.nMin, range, page }))
}

/// Tapa la franja (el pulgar anterior se corre con el texto cuando el
/// RichEdit desplaza la ventana) y dibuja el pulgar donde va: una
/// rayita del color de la tinta, más ancha con el mouse encima.
unsafe fn paint_bar(edit: HWND) {
    let (strip, bar) = bar(edit);
    let (ink, body) = colors(edit);
    let hdc = GetDC(edit);
    let thumb = bar.map(|b| {
        let (hover, drag) = with_state(edit, |s| (s.bar_hover, s.bar_drag.is_some())).unwrap_or((false, false));
        let wide = hover || drag;
        let w = if wide { scale(edit, 6) } else { scale(edit, 3) };
        let cx = b.strip.right - scale(edit, 8);
        let r = RECT { left: cx - w / 2, top: b.thumb.top, right: cx - w / 2 + w, bottom: b.thumb.bottom };
        (r, w, if wide { 60u32 } else { 35 })
    });
    if let Some(alpha) = glass(edit) {
        // Vidrio: la franja y el pulgar sobre un lienzo con alfa.
        if let Some(c) = crate::glass::Canvas::new(strip.right - strip.left, strip.bottom - strip.top) {
            c.fill(RECT { left: 0, top: 0, right: strip.right - strip.left, bottom: strip.bottom - strip.top }, body, alpha);
            if let Some((r, w, a)) = thumb {
                let r = RECT { left: r.left - strip.left, right: r.right - strip.left, ..r };
                c.with_graphics(|g| flyout::fill_round_argb(g, &r, w as f32 / 2.0, crate::glass::argb(ink, (a * 255 / 100) as u8)));
            }
            c.blit(hdc, strip.left, strip.top);
        }
    } else {
        fill_bg(edit, hdc, strip);
        if let Some((r, w, a)) = thumb {
            let mix = |sh: u32| (((ink >> sh) & 0xff) * a + ((body >> sh) & 0xff) * (100 - a)) / 100;
            let color = mix(0) | (mix(8) << 8) | (mix(16) << 16);
            let mut g: *mut GpGraphics = null_mut();
            if GdipCreateFromHDC(hdc, &mut g) == Ok && !g.is_null() {
                GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
                flyout::fill_round(g, &r, w as f32 / 2.0, color);
                GdipDeleteGraphics(g);
            }
        }
    }
    ReleaseDC(edit, hdc);
}

/// Lleva el pulgar a que su borde de arriba quede en `top`.
fn drag_bar_to(edit: HWND, top: i32) {
    let (_, Some(b)) = bar(edit) else { return };
    let span = (b.track.bottom - b.track.top) - (b.thumb.bottom - b.thumb.top);
    let frac = (top - b.track.top).clamp(0, span.max(0));
    let pos = b.min + ((b.range - b.page) as i64 * frac as i64 / span.max(1) as i64) as i32;
    let mut cur = POINT { x: 0, y: 0 };
    unsafe {
        send(edit, EM_GETSCROLLPOS, 0, &mut cur as *mut POINT as isize);
        let p = POINT { x: cur.x, y: pos };
        send(edit, EM_SETSCROLLPOS, 0, &p as *const POINT as isize);
    }
}

/// ¿El punto cae en la franja de la barra, y hay algo que desplazar?
fn on_bar(edit: HWND, x: i32) -> Option<Bar> {
    let (strip, bar) = bar(edit);
    if x >= strip.left {
        bar
    } else {
        None
    }
}

unsafe fn paint_marks(edit: HWND) {
    let text = units(edit);
    if text.is_empty() {
        paint_placeholder(edit);
        return;
    }
    let clusters = richtext::emoji_clusters(&text);
    let todos: Vec<(usize, usize, bool)> = richtext::paragraphs(&text)
        .into_iter()
        .filter_map(|(a, b)| match richtext::list_of(&text[a..b]) {
            (List::Todo { done }, plen) => Some((a, plen, done)),
            _ => None,
        })
        .collect();
    if clusters.is_empty() && todos.is_empty() {
        return;
    }
    let (ink, body) = colors(edit);
    let mut clip = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    send(edit, EM_GETRECT, 0, &mut clip as *mut RECT as isize);
    let em = em_px(edit);
    let hdc = GetDC(edit);
    HideCaret(edit);
    IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);

    if !todos.is_empty() {
        let mut g: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(hdc, &mut g) == Ok && !g.is_null() {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            for (a, plen, done) in todos {
                let p0 = pos(edit, a);
                if p0.y >= clip.bottom || p0.y + em * 2 < clip.top {
                    continue;
                }
                let p1 = pos(edit, a + 1);
                let right = if p1.y == p0.y && p1.x > p0.x { p1.x } else { p0.x + em };
                let pe = pos(edit, a + plen);
                let cover = if pe.y == p0.y && pe.x > right { pe.x } else { right };
                let bottom = line_bottom(edit, a, p0.y);
                let glyph = RECT { left: p0.x, top: p0.y, right, bottom };
                match glass(edit) {
                    Some(alpha) => {
                        // Vidrio: la casilla sobre un lienzo con alfa.
                        let area = RECT { right: cover, ..glyph };
                        if let Some(c) = crate::glass::Canvas::new(area.right - area.left, area.bottom - area.top) {
                            c.fill(RECT { left: 0, top: 0, right: area.right - area.left, bottom: area.bottom - area.top }, body, alpha);
                            let local = RECT { left: 0, top: 0, right: right - area.left, bottom: area.bottom - area.top };
                            c.with_graphics(|cg| draw_checkbox(cg, local, em, done, ink, body));
                            c.blit(hdc, area.left, area.top);
                        }
                    }
                    None => {
                        fill_bg(edit, hdc, RECT { right: cover, ..glyph });
                        draw_checkbox(g, glyph, em, done, ink, body);
                    }
                }
            }
            GdipDeleteGraphics(g);
        }
    }

    let (s, e) = selection(edit);
    let selected_visible = s != e && GetFocus() == edit;
    for (a, b) in clusters {
        let p0 = pos(edit, a);
        if p0.y >= clip.bottom || p0.y + em * 2 < clip.top {
            continue;
        }
        let p1 = pos(edit, b);
        let right = if p1.y == p0.y && p1.x > p0.x { p1.x } else { p0.x + em * 5 / 4 };
        let cell = RECT { left: p0.x, top: p0.y, right, bottom: line_bottom(edit, a, p0.y) };
        let bg = if selected_visible && a >= s && b <= e {
            crate::d2d::Bg::Solid(GetSysColor(COLOR_HIGHLIGHT))
        } else if let Some(alpha) = glass(edit) {
            crate::d2d::Bg::Glass(body, alpha)
        } else {
            crate::d2d::Bg::Solid(body)
        };
        crate::d2d::draw_emoji(hdc, cell, clip, &text[a..b], em as u32, bg);
    }

    ShowCaret(edit);
    ReleaseDC(edit, hdc);
}

/// Una nota vacía no es un rectángulo en blanco: muestra en gris qué va
/// ahí y cómo empezar una lista.
unsafe fn paint_placeholder(edit: HWND) {
    let (ink, body) = colors(edit);
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    send(edit, EM_GETRECT, 0, &mut rc as *mut RECT as isize);
    let hdc = GetDC(edit);
    HideCaret(edit);
    let old = SelectObject(hdc, flyout::font(-em_px(edit), 400, 0, "Segoe UI"));
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, faded(ink, body));
    let hint = wide("Escribí una nota…\r\n\r\n[] y un espacio: lista de tareas\r\n- y un espacio: viñetas");
    DrawTextW(hdc, hint.as_ptr(), -1, &mut rc, DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX);
    SelectObject(hdc, old);
    ShowCaret(edit);
    ReleaseDC(edit, hdc);
}

/// Una casilla redondeada en lugar del ☐/☑ de la fuente, centrada en
/// `cell`: contorno suave si está pendiente, llena y con tilde si está
/// hecha. (El fondo de abajo ya está pintado.)
unsafe fn draw_checkbox(g: *mut GpGraphics, cell: RECT, em: i32, done: bool, ink: u32, body: u32) {
    let size = (em * 7 / 8) as f32;
    let cx = (cell.left + cell.right) as f32 / 2.0;
    let cy = (cell.top + cell.bottom) as f32 / 2.0;
    let (x, y) = (cx - size / 2.0, cy - size / 2.0);
    let r = size / 4.5;
    let mut path: *mut GpPath = null_mut();
    GdipCreatePath(FillModeAlternate, &mut path);
    let d = r * 2.0;
    GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
    GdipAddPathArc(path, x + size - d, y, d, d, 270.0, 90.0);
    GdipAddPathArc(path, x + size - d, y + size - d, d, d, 0.0, 90.0);
    GdipAddPathArc(path, x, y + size - d, d, d, 90.0, 90.0);
    GdipClosePathFigure(path);
    let argb = crate::note::colorref_to_argb;
    if done {
        let mut fill: *mut GpSolidFill = null_mut();
        GdipCreateSolidFill(argb(faded(ink, body)), &mut fill);
        GdipFillPath(g, fill as *mut GpBrush, path);
        GdipDeleteBrush(fill as *mut GpBrush);
        let mut pen: *mut GpPen = null_mut();
        GdipCreatePen1(argb(body), size / 7.0, UnitPixel, &mut pen);
        GdipSetPenLineCap197819(pen, LineCapRound, LineCapRound, DashCapFlat);
        GdipSetPenLineJoin(pen, LineJoinRound);
        let pts = [
            PointF { X: x + size * 0.24, Y: y + size * 0.52 },
            PointF { X: x + size * 0.43, Y: y + size * 0.70 },
            PointF { X: x + size * 0.77, Y: y + size * 0.32 },
        ];
        GdipDrawLines(g, pen, pts.as_ptr(), 3);
        GdipDeletePen(pen);
    } else {
        let mut pen: *mut GpPen = null_mut();
        GdipCreatePen1((argb(ink) & 0x00ff_ffff) | 0xB0 << 24, 1.6, UnitPixel, &mut pen);
        GdipDrawPath(g, pen, path);
        GdipDeletePen(pen);
    }
    GdipDeletePath(path);
}

// -----------------------------------------------------------------
// Subclase del RichEdit
// -----------------------------------------------------------------

fn ctrl() -> bool {
    unsafe { GetKeyState(VK_CONTROL as i32) < 0 }
}

unsafe extern "system" fn subclass_proc(edit: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_background(edit);
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            paint_overlay(edit);
            r
        }
        WM_ERASEBKGND => 1,
        WM_KEYUP | WM_IME_CHAR | WM_IME_COMPOSITION | WM_UNDO | WM_CUT | WM_CLEAR => {
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            paint_overlay(edit);
            r
        }
        WM_KEYDOWN => {
            let vk = wparam as u16;
            let alt = GetKeyState(VK_MENU as i32) < 0;
            let shift = GetKeyState(VK_SHIFT as i32) < 0;
            // Pegar con el teclado pasa por `on_paste` (el RichEdit lo
            // resuelve por dentro, sin mandarse WM_PASTE).
            let paste = (ctrl() && !alt && !shift && vk == 0x56) || (shift && !ctrl() && !alt && vk == VK_INSERT);
            let handled = if paste {
                SendMessageW(edit, WM_PASTE, 0, 0);
                true
            } else if alt || ctrl() {
                false
            } else {
                match vk {
                    VK_RETURN if !shift => on_enter(edit),
                    VK_BACK => on_delete_key(edit, true),
                    VK_DELETE => on_delete_key(edit, false),
                    _ => false,
                }
            };
            if handled {
                paint_overlay(edit);
                // El WM_CHAR que TranslateMessage ya generó para esta tecla.
                let swallow = match vk {
                    VK_RETURN => Some('\r' as u16),
                    VK_BACK => Some(8),
                    0x56 => Some(0x16), // Ctrl+V
                    _ => None,
                };
                with_state(edit, |s| s.swallow = swallow);
                return 0;
            }
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            paint_overlay(edit);
            r
        }
        WM_CHAR => {
            let c = wparam as u16;
            let swallow = with_state(edit, |s| s.swallow.take()).flatten();
            if swallow == Some(c) {
                return 0;
            }
            if c == ' ' as u16 && on_space(edit) {
                paint_overlay(edit);
                return 0;
            }
            // Al escribir, el RichEdit redibuja la línea en el acto (sin
            // WM_PAINT), con el ☐ de la fuente y los emojis en gris: sin
            // esto la casilla "cambiaba de tamaño" con cada letra.
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            paint_overlay(edit);
            r
        }
        WM_PASTE => {
            on_paste(edit);
            0
        }
        WM_TIMER if wparam == TIMER_REFRESH => {
            KillTimer(edit, TIMER_REFRESH);
            refresh(edit);
            paint_overlay(edit);
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam & 0xffff) as i16 as i32;
            let y = ((lparam >> 16) & 0xffff) as i16 as i32;
            if let Some(b) = on_bar(edit, x) {
                if y >= b.thumb.top && y < b.thumb.bottom {
                    with_state(edit, |s| s.bar_drag = Some(y - b.thumb.top));
                    SetCapture(edit);
                } else {
                    // Clic en el recorrido: una página para ese lado.
                    let dir = if y < b.thumb.top { SB_PAGEUP } else { SB_PAGEDOWN };
                    SendMessageW(edit, WM_VSCROLL, dir as usize, 0);
                }
                paint_overlay(edit);
                return 0;
            }
            if let Some(a) = checkbox_at(edit, x, y) {
                SetFocus(edit);
                toggle_todo(edit, a);
                return 0;
            }
            DefSubclassProc(edit, msg, wparam, lparam)
        }
        WM_SETCURSOR => {
            let mut p = POINT { x: 0, y: 0 };
            GetCursorPos(&mut p);
            ScreenToClient(edit, &mut p);
            if (lparam & 0xffff) as u32 == HTCLIENT && on_bar(edit, p.x).is_some() {
                SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
                return 1;
            }
            if (lparam & 0xffff) as u32 == HTCLIENT && checkbox_at(edit, p.x, p.y).is_some() {
                SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                return 1;
            }
            DefSubclassProc(edit, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            crate::note::on_hover(GetParent(edit));
            let x = (lparam & 0xffff) as i16 as i32;
            let y = ((lparam >> 16) & 0xffff) as i16 as i32;
            if let Some(grab) = with_state(edit, |s| s.bar_drag).flatten() {
                drag_bar_to(edit, y - grab);
                paint_overlay(edit);
                return 0;
            }
            let hover = on_bar(edit, x).is_some();
            if with_state(edit, |s| std::mem::replace(&mut s.bar_hover, hover)) != Some(hover) {
                if hover {
                    // Para enterarse cuando el mouse se va (WM_MOUSELEAVE).
                    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: edit, dwHoverTime: 0 };
                    TrackMouseEvent(&mut tme);
                }
                paint_bar(edit);
            }
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            // Seleccionando con el mouse, el RichEdit también dibuja solo.
            if wparam & 0x0001 != 0 {
                paint_overlay(edit);
            }
            r
        }
        WM_LBUTTONUP => {
            if with_state(edit, |s| s.bar_drag.take()).flatten().is_some() {
                ReleaseCapture();
                paint_overlay(edit);
                return 0;
            }
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            paint_overlay(edit);
            r
        }
        WM_CAPTURECHANGED => {
            if with_state(edit, |s| s.bar_drag.take()).flatten().is_some() {
                paint_bar(edit);
            }
            DefSubclassProc(edit, msg, wparam, lparam)
        }
        WM_MOUSELEAVE => {
            if with_state(edit, |s| std::mem::replace(&mut s.bar_hover, false)) == Some(true) {
                paint_bar(edit);
            }
            DefSubclassProc(edit, msg, wparam, lparam)
        }
        // El borde de la nota sigue sirviendo para cambiarle el tamaño,
        // aunque el texto llegue hasta ahí.
        WM_NCHITTEST => {
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            if r as u32 == HTCLIENT {
                let mut p = POINT { x: (lparam & 0xffff) as i16 as i32, y: ((lparam >> 16) & 0xffff) as i16 as i32 };
                ScreenToClient(edit, &mut p);
                let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(edit, &mut rc);
                let edge = scale(edit, 4);
                if p.x < edge || p.x >= rc.right - edge {
                    return HTTRANSPARENT as LRESULT;
                }
            }
            r
        }
        WM_CONTEXTMENU => {
            let p = if lparam == -1 {
                None
            } else {
                Some(POINT { x: (lparam & 0xffff) as i16 as i32, y: ((lparam >> 16) & 0xffff) as i16 as i32 })
            };
            show_menu(edit, p);
            0
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            let r = DefSubclassProc(edit, msg, wparam, lparam);
            crate::note::on_focus_change(GetParent(edit));
            r
        }
        WM_NCDESTROY => {
            let doc = EDITS.with(|m| m.borrow_mut().remove(&(edit as isize)).map(|s| s.doc));
            if let Some(doc) = doc {
                com_release(doc);
            }
            RemoveWindowSubclass(edit, Some(subclass_proc), 1);
            DefSubclassProc(edit, msg, wparam, lparam)
        }
        _ => DefSubclassProc(edit, msg, wparam, lparam),
    }
}

/// Modo vidrio para el texto: la opacidad del color de la nota sobre el
/// desenfoque de Windhawk, o `None` para liso.
pub fn set_glass(edit: HWND, opacity: Option<u8>) {
    with_state(edit, |s| s.glass = opacity);
    unsafe { InvalidateRect(edit, null(), 0) };
}

fn glass(edit: HWND) -> Option<u8> {
    with_state(edit, |s| s.glass).flatten()
}

/// El fondo del texto en `r` (coordenadas del RichEdit).
unsafe fn fill_bg(edit: HWND, hdc: HDC, r: RECT) {
    let (_, body) = colors(edit);
    if let Some(alpha) = glass(edit) {
        crate::glass::fill(hdc, r, body, alpha);
        return;
    }
    let brush = CreateSolidBrush(body);
    FillRect(hdc, &r, brush);
    DeleteObject(brush);
}

/// Antes de que el RichEdit pinte (solo texto: es transparente), el fondo
/// de lo que va a repintar.
unsafe fn paint_background(edit: HWND) {
    let rgn = CreateRectRgn(0, 0, 0, 0);
    let kind = GetUpdateRgn(edit, rgn, 0);
    if kind != NULLREGION && kind != RGN_ERROR {
        let hdc = GetDC(edit);
        SelectClipRgn(hdc, rgn);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(edit, &mut rc);
        fill_bg(edit, hdc, rc);
        ReleaseDC(edit, hdc);
    }
    DeleteObject(rgn);
}

/// Márgenes internos del texto.
pub fn set_padding(edit: HWND, w: i32, h: i32, pad_x: i32, pad_y: i32) {
    let mut rc = RECT { left: pad_x, top: pad_y, right: (w - pad_x).max(pad_x), bottom: (h - pad_y).max(pad_y) };
    unsafe { send(edit, EM_SETRECT, 0, &mut rc as *mut RECT as isize) };
}
