//! Simpcky con Raven: la misma app de notas, con la interfaz hecha en
//! Raven (C + Vulkan) en lugar de Win32/GDI + RichEdit.
//!
//! Es una demo para comparar: notas con encabezado de color, arrastre,
//! enrollado Manual/Auto, siempre encima, bloqueo, colores, renombrar,
//! duplicar, ocultar, eliminar, "Todas las notas" con búsqueda, tema claro
//! u oscuro, translucidez y guardado en el mismo `notes.json`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod win32;

use raven::*;
use std::cell::RefCell;
use std::ffi::c_void;
use std::os::raw::c_char;
use store::*;

// -----------------------------------------------------------------
// Aspecto
// -----------------------------------------------------------------

const HEADER_H: i32 = 34;

/// (encabezado, cuerpo, tinta), la misma paleta que `theme.rs`.
const LIGHT: [(u32, u32, u32); 6] = [
    (0xFDE68A, 0xFEF9C3, 0x78350F),
    (0xFBCFE8, 0xFCE7F3, 0x831843),
    (0xBBF7D0, 0xDCFCE7, 0x065F46),
    (0xBFDBFE, 0xDBEAFE, 0x1E3A8A),
    (0xE9D5FF, 0xF3E8FF, 0x581C87),
    (0xE2E8F0, 0xF1F5F9, 0x334155),
];
const DARK: [(u32, u32, u32); 6] = [
    (0x6B5514, 0x2A2515, 0xFDE68A),
    (0x74284F, 0x2B1822, 0xFBCFE8),
    (0x1D5E3E, 0x15261C, 0xBBF7D0),
    (0x234A80, 0x151F30, 0xBFDBFE),
    (0x52307F, 0x201830, 0xE9D5FF),
    (0x3E4652, 0x1E2227, 0xE2E8F0),
];
const COLOR_NAMES: [&str; 6] = ["Amarillo", "Rosa", "Verde", "Azul", "Morado", "Gris"];

// Íconos de Segoe Fluent Icons / MDL2 (los mismos que usa Simpcky).
const ICON_ADD: &str = "\u{E710}";
const ICON_PIN: &str = "\u{E718}";
const ICON_PINNED: &str = "\u{E840}";
const ICON_LOCK: &str = "\u{E72E}";
const ICON_UNLOCK: &str = "\u{E785}";
const ICON_MORE: &str = "\u{E712}";

fn colors(idx: u8, dark: bool) -> (u32, u32, u32) {
    let i = idx as usize % 6;
    if dark { DARK[i] } else { LIGHT[i] }
}

fn rgb(c: u32) -> (f32, f32, f32) {
    (((c >> 16) & 0xFF) as f32 / 255.0, ((c >> 8) & 0xFF) as f32 / 255.0, (c & 0xFF) as f32 / 255.0)
}

// -----------------------------------------------------------------
// Estado
// -----------------------------------------------------------------

struct Note {
    data: NoteData,
    win: Rid,
    root: Rid,
    header: Rid,
    title: Rid,
    title_input: Rid,
    buttons: Rid,
    pin: Rid,
    lock: Rid,
    more: Rid,
    editor: Rid,
    hwnd: usize,
    roll_timer: Rid,
    /// Mientras el mouse está encima se mira cada 150 ms si salió: el
    /// aviso de salida no llega si el cursor salta afuera de golpe.
    hover_timer: Rid,
    hovered: bool,
    renaming: bool,
    /// Dónde se apretó el botón sobre el encabezado (para distinguir un
    /// clic, que enrolla, de un arrastre, que mueve la nota).
    press: Option<(f32, f32)>,
}

struct AllNotes {
    win: Rid,
    search: Rid,
    list: Rid,
    footer: Rid,
}

// (`R_INVALID_ID` no es 0: nada de `#[derive(Default)]` con `Rid`.)
impl Default for AllNotes {
    fn default() -> Self { AllNotes { win: R_INVALID_ID, search: R_INVALID_ID, list: R_INVALID_ID, footer: R_INVALID_ID } }
}

impl Default for Flyout {
    fn default() -> Self { Flyout { win: R_INVALID_ID, note: 0, timer: R_INVALID_ID, actions: Vec::new() } }
}

struct App {
    notes: Vec<Note>,
    settings: Settings,
    next_id: u32,
    save_timer: Rid,
    icons: Rid,
    all: AllNotes,
    flyout: Flyout,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Segoe UI para el texto (el editor de Raven trae una monoespaciada).
    /// (texto, títulos en seminegrita).
    static TEXT_FONT: std::cell::Cell<(Rid, Rid)> = const { std::cell::Cell::new((R_INVALID_ID, R_INVALID_ID)) };
}

/// Acceso al estado. Ninguna llamada a Raven que pueda volver a llamarnos
/// se hace adentro: se copian los `Rid` y se suelta antes.
fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

fn with_note<R>(id: u32, f: impl FnOnce(&mut Note) -> R) -> Option<R> {
    with(|a| a.notes.iter_mut().find(|n| n.data.id == id).map(f)).flatten()
}

fn ud(v: u32) -> *mut c_void { v as usize as *mut c_void }
fn from_ud(p: *mut c_void) -> u32 { p as usize as u32 }

fn s(text: &str) -> std::ffi::CString { cstr(text) }

// -----------------------------------------------------------------
// Ventana de una nota
// -----------------------------------------------------------------

unsafe fn flat_icon_button(parent: Rid, icon: &str, tooltip: &str, icons: Rid, action: u32, id: u32) -> Rid {
    let b = Rwidget_add_child(parent, Rbutton_create(s(icon).as_ptr()));
    Rbutton_set_style(b, R_BUTTON_FLAT);
    // Sin el relleno interno: en 28 px el botón dejaba a la etiqueta sin
    // lugar y el ícono salía recortado.
    Rwidget_set_metric(b, R_METRIC_PADDING_X, 0.0);
    Rwidget_set_metric(b, R_METRIC_PADDING_Y, 0.0);
    if icons != R_INVALID_ID {
        // El texto del botón es una etiqueta hija: la fuente va en ella.
        let label = Rwidget_get_child(b, 0);
        if label != R_INVALID_ID {
            Rlabel_set_font(label, icons);
            Rlabel_set_text_size(label, 14.0);
        }
    }
    Rwidget_set_tooltip(b, s(tooltip).as_ptr());
    Rwidget_set_size(b, 28.0, 26.0);
    Rslot_set_valign(b, R_VALIGN_CENTER);
    Rbutton_set_clicked_callback(b, Some(on_button), ud(id << 8 | action));
    b
}

const ACT_NEW: u32 = 1;
const ACT_PIN: u32 = 2;
const ACT_LOCK: u32 = 3;
const ACT_MORE: u32 = 4;

unsafe fn create_note(data: NoteData, settings: &Settings, icons: Rid) -> Note {
    let id = data.id;
    let h = if data.rolled { HEADER_H } else { data.h.max(HEADER_H + 40) };
    let win = Rwindow_create(s(&data.display_title()).as_ptr(), data.w.max(160), h, (R_WINDOW_BORDERLESS | R_WINDOW_HIDDEN) as u32);
    Rwindow_set_min_size(win, 160, HEADER_H);
    Rwindow_set_position(win, data.x, data.y);

    let root = Rwidget_create(R_WIDGET_BORDER);
    let vbox = Rwidget_add_child(root, Rwidget_create(R_WIDGET_VBOX));

    // Encabezado: título y botones (que solo se ven con el mouse encima).
    let header = Rwidget_add_child(vbox, Rwidget_create(R_WIDGET_BORDER));
    Rwidget_set_size(header, -1.0, HEADER_H as f32);
    Rborder_set_padding(header, 10.0, 0.0, 4.0, 0.0);
    Rwidget_set_cursor(header, R_CURSOR_ARROW);
    let hbox = Rwidget_add_child(header, Rwidget_create(R_WIDGET_HBOX));
    let title = Rwidget_add_child(hbox, Rlabel_create(s(&data.display_title()).as_ptr()));
    Rslot_set_fill(title, 1.0);
    Rslot_set_valign(title, R_VALIGN_CENTER);
    let title_input = Rwidget_add_child(hbox, Rwidget_create(R_WIDGET_TEXT_INPUT));
    Rslot_set_fill(title_input, 1.0);
    Rslot_set_valign(title_input, R_VALIGN_CENTER);
    Rwidget_set_visibility(title_input, R_COLLAPSED);
    Rtext_input_set_committed_callback(title_input, Some(on_rename_commit), ud(id));
    // `committed` solo llega con Enter: el nombre se guarda mientras se escribe.
    Rtext_input_set_changed_callback(title_input, Some(on_rename_typed), ud(id));
    let buttons = Rwidget_add_child(hbox, Rwidget_create(R_WIDGET_HBOX));
    Rslot_set_valign(buttons, R_VALIGN_CENTER);
    flat_icon_button(buttons, ICON_ADD, "Nota nueva (Ctrl+N)", icons, ACT_NEW, id);
    let pin = flat_icon_button(buttons, ICON_PIN, "Siempre encima (Ctrl+Mayús+T)", icons, ACT_PIN, id);
    let lock = flat_icon_button(buttons, ICON_UNLOCK, "Bloquear", icons, ACT_LOCK, id);
    let more = flat_icon_button(buttons, ICON_MORE, "Color y más opciones", icons, ACT_MORE, id);
    Rwidget_set_visibility(buttons, R_HIDDEN);
    Rwidget_set_event_callback(header, Some(on_header_event), ud(id));

    // Cuerpo: el editor de Raven, con Markdown (negrita, listas, casillas).
    let editor = Rwidget_add_child(vbox, Rtext_editor_create(s(&data.text).as_ptr()));
    Rslot_set_fill(editor, 1.0);
    Rtext_editor_set_word_wrap(editor, 1);
    Rtext_editor_set_line_numbers(editor, 0);
    Rtext_editor_set_markdown(editor, 1);
    let (font, bold) = TEXT_FONT.with(|f| f.get());
    if font != R_INVALID_ID {
        Rtext_editor_set_font(editor, font, 16.0);
    }
    if bold != R_INVALID_ID {
        Rlabel_set_font(title, bold);
    }
    Rlabel_set_text_size(title, 15.0);
    Rtext_editor_set_placeholder(editor, s("Escribí acá… (**negrita**, *cursiva*, - [ ] tareas)").as_ptr());
    Rtext_editor_set_changed_callback(editor, Some(on_text_changed), ud(id));
    if data.rolled {
        Rwidget_set_visibility(editor, R_COLLAPSED);
    }

    Rwidget_set_event_callback(root, Some(on_root_event), ud(id));
    Rwindow_set_content(win, root);
    Rwindow_set_close_callback(win, Some(on_close), ud(id));
    for (key, mods, action) in [
        (R_KEY_N, R_MOD_CTRL, "nota.nueva"),
        (R_KEY_R, R_MOD_CTRL, "nota.enrollar"),
        (R_KEY_W, R_MOD_CTRL, "nota.ocultar"),
        (R_KEY_T, R_MOD_CTRL | R_MOD_SHIFT, "nota.encima"),
        (R_KEY_F2, 0, "nota.renombrar"),
    ] {
        Rwindow_register_shortcut(win, key, mods as u32, s(action).as_ptr());
    }

    let mut native: RNativeWindow = std::mem::zeroed();
    let hwnd = if Rwindow_get_native_handle(win, &mut native) != 0 { native.window } else { 0 };
    let note = Note {
        data,
        win,
        root,
        header,
        title,
        title_input,
        buttons,
        pin,
        lock,
        more,
        editor,
        hwnd,
        roll_timer: R_INVALID_ID,
        hover_timer: R_INVALID_ID,
        hovered: false,
        renaming: false,
        press: None,
    };
    paint_note(&note, settings.dark);
    Rwindow_set_opacity(win, settings.opacity);
    if !note.data.hidden {
        Rwindow_show(win);
    }
    apply_layer(&note);
    apply_lock(&note);
    note
}

/// Colores de la nota según su color y el tema.
unsafe fn paint_note(n: &Note, dark: bool) {
    let (head, body, ink) = colors(n.data.color, dark);
    let (hr, hg, hb) = rgb(head);
    let (br, bg, bb) = rgb(body);
    let (ir, ig, ib) = rgb(ink);
    Rborder_set_background(n.root, br, bg, bb, 1.0);
    Rborder_set_outline(n.root, 1.0, hr, hg, hb, 1.0);
    Rborder_set_background(n.header, hr, hg, hb, 1.0);
    Rlabel_set_color(n.title, ir, ig, ib, 1.0);
    for w in [n.editor, n.title_input] {
        Rwidget_set_color(w, R_COLOR_INPUT, br, bg, bb, 1.0);
        Rwidget_set_color(w, R_COLOR_TEXT, ir, ig, ib, 1.0);
        Rwidget_set_color(w, R_COLOR_TEXT_PLACEHOLDER, ir, ig, ib, 0.45);
    }
    for b in [n.pin, n.lock, n.more] {
        Rwidget_set_color(b, R_COLOR_TEXT, ir, ig, ib, 1.0);
    }
    Rwidget_set_color(n.buttons, R_COLOR_TEXT, ir, ig, ib, 1.0);
}

unsafe fn apply_layer(n: &Note) {
    let mut native: RNativeWindow = std::mem::zeroed();
    let on_top = n.data.layer == 2;
    if Rwindow_get_native_handle(n.win, &mut native) != 0 {
        win32::own_by_desktop(native.window, !on_top);
        win32::round_corners(native.window);
    }
    Rwindow_set_topmost(n.win, on_top as i32);
    Rbutton_set_text(n.pin, s(if on_top { ICON_PINNED } else { ICON_PIN }).as_ptr());
}

unsafe fn apply_lock(n: &Note) {
    Rtext_editor_set_read_only(n.editor, n.data.locked as i32);
    Rbutton_set_text(n.lock, s(if n.data.locked { ICON_LOCK } else { ICON_UNLOCK }).as_ptr());
    Rwidget_set_tooltip(n.lock, s(if n.data.locked { "Desbloquear" } else { "Bloquear" }).as_ptr());
}

/// Enrolla (solo el encabezado) o despliega una nota.
fn set_rolled(id: u32, rolled: bool) {
    let Some((win, was)) = with_note(id, |n| (n.win, n.data.rolled)) else { return };
    if was == rolled {
        return;
    }
    // Antes de enrollar, el tamaño actual (pudo cambiar desde el último guardado).
    if rolled {
        let (mut cw, mut ch) = (0, 0);
        unsafe { Rwindow_get_size(win, &mut cw, &mut ch) };
        with_note(id, |n| {
            if cw > 0 {
                n.data.w = cw;
            }
            if ch > HEADER_H {
                n.data.h = ch;
            }
        });
    }
    let Some((editor, w, h)) = with_note(id, |n| {
        n.data.rolled = rolled;
        (n.editor, n.data.w, n.data.h)
    }) else { return };
    unsafe {
        if rolled {
            Rwidget_set_visibility(editor, R_COLLAPSED);
            Rwindow_set_size(win, w, HEADER_H);
        } else {
            Rwindow_set_size(win, w, h);
            Rwidget_set_visibility(editor, R_VISIBLE);
        }
    }
    touch(id, 3);
}

/// Marca una parte de la nota como cambiada y programa el guardado.
fn touch(id: u32, part: usize) {
    with_note(id, |n| n.data.t[part] = now_ms());
    schedule_save();
}

fn schedule_save() {
    let pending = with(|a| a.save_timer != R_INVALID_ID).unwrap_or(true);
    if !pending {
        let t = unsafe { Rtimer_create(0.8, 0, Some(on_save_timer), std::ptr::null_mut()) };
        with(|a| a.save_timer = t);
    }
}

/// Lee posición y tamaño de las ventanas y guarda todo.
fn save_now() {
    let wins: Vec<(u32, Rid)> = with(|a| a.notes.iter().map(|n| (n.data.id, n.win)).collect()).unwrap_or_default();
    for (id, win) in wins {
        let (mut x, mut y, mut w, mut h) = (0, 0, 0, 0);
        unsafe {
            Rwindow_get_position(win, &mut x, &mut y);
            Rwindow_get_size(win, &mut w, &mut h);
        }
        with_note(id, |n| {
            if (n.data.x, n.data.y) != (x, y) {
                n.data.x = x;
                n.data.y = y;
                n.data.t[2] = now_ms();
            }
            if w > 0 {
                n.data.w = w;
            }
            if !n.data.rolled && h > HEADER_H {
                n.data.h = h;
            }
        });
    }
    with(|a| {
        let notes: Vec<NoteData> = a.notes.iter().map(|n| n.data.clone()).collect();
        save_notes(&notes);
    });
}

// -----------------------------------------------------------------
// Acciones
// -----------------------------------------------------------------

fn new_note(near: Option<u32>) {
    let pos = near.and_then(|id| with_note(id, |n| (n.data.x + 32, n.data.y + 32, n.data.color)));
    let Some((data, settings, icons)) = with(|a| {
        let id = a.next_id;
        a.next_id += 1;
        let (x, y, color) = pos.unwrap_or((120 + (id as i32 % 8) * 30, 120 + (id as i32 % 8) * 30, 0));
        (NoteData::new(id, x, y, color), a.settings, a.icons)
    }) else { return };
    let id = data.id;
    let note = unsafe { create_note(data, &settings, icons) };
    let editor = note.editor;
    with(|a| a.notes.push(note));
    unsafe { Rwidget_focus(editor) };
    touch(id, 0);
    refresh_all_notes();
}

fn duplicate(id: u32) {
    let Some(mut data) = with_note(id, |n| n.data.clone()) else { return };
    let Some((settings, icons, new_id)) = with(|a| {
        let i = a.next_id;
        a.next_id += 1;
        (a.settings, a.icons, i)
    }) else { return };
    data.id = new_id;
    data.uid = new_uid();
    data.x += 32;
    data.y += 32;
    data.hidden = false;
    data.t = [now_ms(); 4];
    let note = unsafe { create_note(data, &settings, icons) };
    with(|a| a.notes.push(note));
    schedule_save();
    refresh_all_notes();
}

fn toggle_pin(id: u32) {
    with_note(id, |n| {
        n.data.layer = if n.data.layer == 2 { 1 } else { 2 };
        unsafe { apply_layer(n) };
    });
    touch(id, 3);
}

fn toggle_lock(id: u32) {
    with_note(id, |n| {
        n.data.locked = !n.data.locked;
        unsafe { apply_lock(n) };
    });
    schedule_save();
}

fn set_color(id: u32, color: u8) {
    let dark = with(|a| a.settings.dark).unwrap_or(false);
    with_note(id, |n| {
        n.data.color = color;
        unsafe { paint_note(n, dark) };
    });
    touch(id, 1);
    refresh_all_notes();
}

fn hide(id: u32) {
    let Some(win) = with_note(id, |n| {
        n.data.hidden = true;
        n.win
    }) else { return };
    unsafe { Rwindow_hide(win) };
    schedule_save();
    // Sin bandeja del sistema (Raven no la tiene): si no queda ninguna
    // nota a la vista, se abre "Todas las notas" para poder volver.
    let any_visible = with(|a| a.notes.iter().any(|n| !n.data.hidden)).unwrap_or(false);
    if !any_visible {
        open_all_notes();
    } else {
        refresh_all_notes();
    }
}

fn show_note(id: u32) {
    let Some((win, editor)) = with_note(id, |n| {
        n.data.hidden = false;
        (n.win, n.editor)
    }) else { return };
    set_rolled(id, false);
    unsafe {
        Rwindow_show(win);
        Rwidget_focus(editor);
    }
    schedule_save();
    refresh_all_notes();
}

fn delete(id: u32) {
    let Some(win) = with_note(id, |n| n.win) else { return };
    let answer = unsafe { Rmessage_box(win, s("Simpcky").as_ptr(), s("¿Eliminar esta nota? No se puede deshacer.").as_ptr(), R_MESSAGE_YES_NO) };
    if answer != R_MESSAGE_RESULT_YES {
        return;
    }
    let removed = with(|a| {
        let i = a.notes.iter().position(|n| n.data.id == id)?;
        Some(a.notes.remove(i))
    })
    .flatten();
    if let Some(n) = removed {
        unsafe {
            for t in [n.roll_timer, n.hover_timer] {
                if t != R_INVALID_ID {
                    Rtimer_destroy(t);
                }
            }
            Rwindow_destroy(n.win);
        }
    }
    schedule_save();
    // Si era la última a la vista, que quede por dónde volver a las ocultas.
    if !with(|a| a.notes.iter().any(|n| !n.data.hidden)).unwrap_or(false) {
        open_all_notes();
    } else {
        refresh_all_notes();
    }
}

fn start_rename(id: u32) {
    let Some((title, input, text)) = with_note(id, |n| {
        n.renaming = true;
        (n.title, n.title_input, n.data.display_title())
    }) else { return };
    unsafe {
        Rwidget_set_visibility(title, R_COLLAPSED);
        Rwidget_set_visibility(input, R_VISIBLE);
        Rtext_input_set_text(input, s(&text).as_ptr());
        Rwidget_focus(input);
        Rtext_input_select_all(input);
    }
}

fn set_dark(dark: bool) {
    unsafe { Rlook_set(if dark { R_LOOK_WINDOWS_DARK } else { R_LOOK_WINDOWS_LIGHT }) };
    with(|a| {
        a.settings.dark = dark;
        save_settings(&a.settings);
        for n in &a.notes {
            unsafe { paint_note(n, dark) };
        }
    });
    refresh_all_notes();
}

fn set_opacity(opacity: f32) {
    with(|a| {
        a.settings.opacity = opacity;
        save_settings(&a.settings);
        for n in &a.notes {
            unsafe { Rwindow_set_opacity(n.win, opacity) };
        }
    });
}

fn quit() {
    save_now();
    unsafe { Rquit(0) };
}

// -----------------------------------------------------------------
// Menú "⋯": una ventanita aparte, como el flyout de Simpcky. Los menús
// de Raven (`Rmenu_open`) se pintan dentro de la ventana que los abre, y
// una nota de 300 px los recorta.
// -----------------------------------------------------------------

const M_COLOR: u32 = 0; // 0..=5
const M_TOP: u32 = 10;
const M_AUTO: u32 = 11;
const M_RENAME: u32 = 12;
const M_DUP: u32 = 13;
const M_HIDE: u32 = 14;
const M_ALL: u32 = 15;
const M_DELETE: u32 = 16;
const M_DARK: u32 = 17;
const M_QUIT: u32 = 18;
const M_OPACITY: u32 = 20; // pasa a la siguiente translucidez
const M_DELAY: u32 = 30; // pasa a la siguiente espera
const OPACITIES: [f32; 4] = [1.0, 0.92, 0.85, 0.75];

struct Flyout {
    win: Rid,
    note: u32,
    timer: Rid,
    actions: Vec<u32>,
}

fn close_flyout() {
    let Some(f) = with(|a| std::mem::take(&mut a.flyout)) else { return };
    unsafe {
        if f.timer != R_INVALID_ID {
            Rtimer_destroy(f.timer);
        }
        if f.win != R_INVALID_ID && Rwindow_is_valid(f.win) != 0 {
            Rwindow_destroy(f.win);
        }
    }
}

fn open_flyout(id: u32) {
    close_flyout();
    let Some((data, settings, hwnd)) = with(|a| {
        let settings = a.settings;
        a.notes.iter().find(|n| n.data.id == id).map(|n| (n.data.clone(), settings, n.hwnd))
    })
    .flatten() else { return };
    let check = |on: bool, t: &str| format!("{}{}", if on { "✓   " } else { "     " }, t);
    let delay = ROLL_DELAYS_MS[settings.roll_delay as usize % 5];
    let opacity = OPACITIES.iter().position(|o| (o - settings.opacity).abs() < 0.01).unwrap_or(0);
    let rows: Vec<(String, u32)> = vec![
        (check(data.layer == 2, "Siempre encima          Ctrl+Mayús+T"), M_TOP),
        (check(data.roll_mode == 1, "Enrollar al quitar el mouse"), M_AUTO),
        (check(false, &format!("Esperar antes: {}", if delay == 0 { "no".to_string() } else { format!("{:.1} s", delay as f32 / 1000.0) })), M_DELAY),
        (check(false, "Cambiar nombre                         F2"), M_RENAME),
        (check(false, "Duplicar"), M_DUP),
        (check(false, "Ocultar                              Ctrl+W"), M_HIDE),
        (check(false, "Todas las notas"), M_ALL),
        (check(settings.dark, "Tema oscuro"), M_DARK),
        (check(false, &format!("Translucidez: {}", if opacity == 0 { "apagada".to_string() } else { format!("{} %", (OPACITIES[opacity] * 100.0).round()) })), M_OPACITY),
        (check(false, "Eliminar nota"), M_DELETE),
        (check(false, "Salir de Simpcky"), M_QUIT),
    ];
    const ROW: f32 = 30.0;
    let height = (14.0 + 34.0 + 10.0 + rows.len() as f32 * ROW + 12.0) as i32;
    unsafe {
        let win = Rwindow_create(s("Opciones").as_ptr(), 270, height, (R_WINDOW_BORDERLESS | R_WINDOW_HIDDEN | R_WINDOW_NO_RESIZE) as u32);
        let root = Rwidget_create(R_WIDGET_BORDER);
        Rborder_set_padding(root, 8.0, 10.0, 8.0, 8.0);
        let vb = Rwidget_add_child(root, Rwidget_create(R_WIDGET_VBOX));
        // Fila de colores.
        let swatches = Rwidget_add_child(vb, Rwidget_create(R_WIDGET_HBOX));
        Rslot_set_padding(swatches, 8.0, 0.0, 0.0, 8.0);
        for i in 0..6u8 {
            let (head, _, ink) = colors(i, settings.dark);
            let sw = Rwidget_add_child(swatches, Rwidget_create(R_WIDGET_BORDER));
            Rwidget_set_size(sw, 28.0, 28.0);
            Rslot_set_padding(sw, 0.0, 0.0, 10.0, 0.0);
            let (r, g, b) = rgb(head);
            Rborder_set_background(sw, r, g, b, 1.0);
            Rborder_set_corner_radius(sw, 14.0, 14.0, 14.0, 14.0);
            let (r, g, b) = rgb(ink);
            Rborder_set_outline(sw, if i == data.color { 2.5 } else { 1.0 }, r, g, b, if i == data.color { 1.0 } else { 0.25 });
            Rwidget_set_cursor(sw, R_CURSOR_HAND);
            Rwidget_set_tooltip(sw, s(COLOR_NAMES[i as usize]).as_ptr());
            Rwidget_set_event_callback(sw, Some(on_swatch), ud(id << 8 | i as u32));
        }
        Rwidget_add_child(vb, Rwidget_create(R_WIDGET_SEPARATOR));
        let list = Rwidget_add_child(vb, Rwidget_create(R_WIDGET_LIST_VIEW));
        Rslot_set_fill(list, 1.0);
        Rwidget_set_metric(list, R_METRIC_ROW_HEIGHT, ROW);
        for (text, _) in &rows {
            Rlist_view_add_item(list, s(text).as_ptr());
        }
        Rlist_view_set_selection_changed_callback(list, Some(on_flyout_row), ud(id));
        Rwindow_set_content(win, root);

        // Debajo del botón "⋯" (a la derecha del encabezado).
        let mut native: RNativeWindow = std::mem::zeroed();
        if let Some((x, y, w)) = win32::window_rect(hwnd) {
            Rwindow_set_position(win, (x + w - 270).max(0), y + HEADER_H + 2);
        }
        Rwindow_set_topmost(win, 1);
        Rwindow_show(win);
        if Rwindow_get_native_handle(win, &mut native) != 0 {
            win32::round_corners(native.window);
        }
        // Se cierra al perder el foco (un clic afuera, Esc).
        let timer = Rtimer_create(0.12, 1, Some(on_flyout_watch), std::ptr::null_mut());
        Rwindow_register_shortcut(win, R_KEY_ESCAPE, 0, s("menu.cerrar").as_ptr());
        let actions = rows.iter().map(|r| r.1).collect();
        with(|a| a.flyout = Flyout { win, note: id, timer, actions });
    }
}

unsafe extern "C" fn on_flyout_watch(_t: Rid, _u: *mut c_void) {
    let win = with(|a| a.flyout.win).unwrap_or(R_INVALID_ID);
    if win == R_INVALID_ID || Rwindow_is_valid(win) == 0 || Rwindow_is_active(win) == 0 {
        close_flyout();
    }
}

unsafe extern "C" fn on_flyout_row(list: Rid, index: i32, user: *mut c_void) {
    let id = from_ud(user);
    if index < 0 {
        return;
    }
    let action = with(|a| a.flyout.actions.get(index as usize).copied()).flatten();
    Rlist_view_set_selected(list, -1);
    close_flyout();
    if let Some(action) = action {
        run_action(id, action);
    }
}

unsafe extern "C" fn on_swatch(_w: Rid, ev: *const REvent, user: *mut c_void) -> i32 {
    if (*ev).type_ == R_EVENT_MOUSE_UP && (*ev).button == R_MOUSE_LEFT {
        let v = from_ud(user);
        close_flyout();
        set_color(v >> 8, (v & 0xFF) as u8);
        return 1;
    }
    0
}

// -----------------------------------------------------------------
// "Todas las notas"
// -----------------------------------------------------------------

fn open_all_notes() {
    let existing = with(|a| a.all.win).unwrap_or(R_INVALID_ID);
    unsafe {
        if existing != R_INVALID_ID && Rwindow_is_valid(existing) != 0 {
            Rwindow_show(existing);
            refresh_all_notes();
            return;
        }
        let win = window_from_xml(
            r#"<Window Title="Todas las notas — Simpcky (Raven)" Width="420" Height="560" MinWidth="300" MinHeight="260">
                <VBox Padding="14" Spacing="10">
                    <HBox Spacing="8">
                        <TextInput Name="search" Placeholder="Buscar en las notas" Slot.Fill="1"/>
                        <Button Text="Nota nueva" Style="Primary" OnClicked="todas.nueva"/>
                    </HBox>
                    <ScrollBox Slot.Fill="1"><VBox Name="list" Spacing="8"/></ScrollBox>
                    <HBox Spacing="8">
                        <Label Name="footer" Opacity="0.7" Slot.Fill="1" Slot.VAlign="Center"/>
                        <Button Text="Salir" Style="Flat" OnClicked="todas.salir"/>
                    </HBox>
                </VBox>
            </Window>"#,
        );
        let search = widget_find(win, "search");
        let list = widget_find(win, "list");
        let footer = widget_find(win, "footer");
        Rtext_input_set_changed_callback(search, Some(on_search), std::ptr::null_mut());
        Rwindow_set_close_callback(win, Some(on_all_close), std::ptr::null_mut());
        with(|a| a.all = AllNotes { win, search, list, footer });
        Rwindow_show(win);
    }
    refresh_all_notes();
}

fn refresh_all_notes() {
    let Some((all, cards, dark)) = with(|a| {
        let cards: Vec<NoteData> = a.notes.iter().map(|n| n.data.clone()).collect();
        (std::mem::take(&mut a.all), cards, a.settings.dark)
    }) else { return };
    let AllNotes { win, search, list, footer } = all;
    with(|a| a.all = AllNotes { win, search, list, footer });
    unsafe {
        if win == R_INVALID_ID || Rwindow_is_valid(win) == 0 {
            return;
        }
        let query = text(Rtext_input_get_text(search)).to_lowercase();
        Rwidget_clear_children(list);
        let mut shown = 0;
        for d in &cards {
            if !query.is_empty() && !d.display_title().to_lowercase().contains(&query) && !d.text.to_lowercase().contains(&query) {
                continue;
            }
            shown += 1;
            let (head, body, ink) = colors(d.color, dark);
            let card = Rwidget_add_child(list, Rwidget_create(R_WIDGET_BORDER));
            let (r, g, b) = rgb(body);
            Rborder_set_background(card, r, g, b, 1.0);
            let (r, g, b) = rgb(head);
            Rborder_set_outline(card, 1.0, r, g, b, 1.0);
            Rborder_set_corner_radius(card, 8.0, 8.0, 8.0, 8.0);
            Rborder_set_padding(card, 12.0, 10.0, 12.0, 10.0);
            Rwidget_set_cursor(card, R_CURSOR_HAND);
            Rwidget_set_event_callback(card, Some(on_card_event), ud(d.id));
            let vb = Rwidget_add_child(card, Rwidget_create(R_WIDGET_VBOX));
            let (r, g, b) = rgb(ink);
            let state = if d.hidden { " · oculta" } else if d.rolled { " · enrollada" } else { "" };
            let t = Rwidget_add_child(vb, Rlabel_create(s(&format!("{}{}", d.display_title(), state)).as_ptr()));
            Rlabel_set_color(t, r, g, b, 1.0);
            Rlabel_set_text_size(t, 15.0);
            let preview: String = d.text.lines().filter(|l| !l.trim().is_empty()).take(3).collect::<Vec<_>>().join("\n");
            if !preview.is_empty() {
                let p = Rwidget_add_child(vb, Rlabel_create(s(&preview).as_ptr()));
                Rlabel_set_wrap(p, 1);
                Rlabel_set_color(p, r, g, b, 0.75);
            }
        }
        let (ws, private) = win32::process_memory();
        let mut stats: RMemoryStats = std::mem::zeroed();
        Rmemory_get_stats(&mut stats);
        let info = format!(
            "{} de {} notas · proceso {:.1} MB (privada {:.1} MB) · heap de Raven {:.1} MB",
            shown,
            cards.len(),
            ws as f64 / 1048576.0,
            private as f64 / 1048576.0,
            stats.used_bytes as f64 / 1048576.0
        );
        Rlabel_set_text(footer, s(&info).as_ptr());
    }
}

// -----------------------------------------------------------------
// Llamadas de Raven
// -----------------------------------------------------------------

unsafe extern "C" fn on_button(_b: Rid, user: *mut c_void) {
    let v = from_ud(user);
    let (id, action) = (v >> 8, v & 0xFF);
    match action {
        ACT_NEW => new_note(Some(id)),
        ACT_PIN => toggle_pin(id),
        ACT_LOCK => toggle_lock(id),
        ACT_MORE => open_flyout(id),
        _ => {}
    }
}

fn run_action(id: u32, action: u32) {
    match action {
        a if a < M_COLOR + 6 => set_color(id, (a - M_COLOR) as u8),
        M_TOP => toggle_pin(id),
        M_AUTO => {
            let auto = with_note(id, |n| {
                n.data.roll_mode = 1 - n.data.roll_mode.min(1);
                n.data.roll_mode == 1
            });
            if auto == Some(false) {
                set_rolled(id, false);
            }
            touch(id, 3);
        }
        M_RENAME => start_rename(id),
        M_DUP => duplicate(id),
        M_HIDE => hide(id),
        M_ALL => open_all_notes(),
        M_DELETE => delete(id),
        M_DARK => set_dark(!with(|a| a.settings.dark).unwrap_or(false)),
        M_QUIT => quit(),
        M_OPACITY => {
            let cur = with(|a| a.settings.opacity).unwrap_or(1.0);
            let i = OPACITIES.iter().position(|o| (o - cur).abs() < 0.01).unwrap_or(0);
            set_opacity(OPACITIES[(i + 1) % OPACITIES.len()]);
            open_flyout(id);
        }
        M_DELAY => {
            with(|app| {
                app.settings.roll_delay = (app.settings.roll_delay + 1) % ROLL_DELAYS_MS.len() as u8;
                save_settings(&app.settings);
            });
            open_flyout(id);
        }
        _ => {}
    }
}

/// Clic en la barra: enrolla o despliega (en Manual). Arrastrar: mueve.
/// Doble clic: cambia el nombre.
unsafe extern "C" fn on_header_event(_w: Rid, ev: *const REvent, user: *mut c_void) -> i32 {
    let ev = &*ev;
    let id = from_ud(user);
    if ev.type_ != R_EVENT_MOUSE_MOVE {
        log(&format!("encabezado {id}: evento {:#x} botón {} clics {}", ev.type_, ev.button, ev.clicks));
    }
    match ev.type_ {
        R_EVENT_MOUSE_DOWN if ev.button == R_MOUSE_LEFT => {
            if ev.clicks >= 2 {
                with_note(id, |n| n.press = None);
                start_rename(id);
            } else {
                with_note(id, |n| n.press = Some((ev.x, ev.y)));
            }
            1
        }
        R_EVENT_MOUSE_MOVE => {
            let moved = with_note(id, |n| match n.press {
                Some((x, y)) if (ev.x - x).abs() + (ev.y - y).abs() > 4.0 => {
                    n.press = None;
                    Some(n.win)
                }
                _ => None,
            })
            .flatten();
            if let Some(win) = moved {
                Rwindow_begin_move(win);
                schedule_save();
            }
            0
        }
        R_EVENT_MOUSE_UP if ev.button == R_MOUSE_LEFT => {
            let click = with_note(id, |n| (n.press.take().is_some(), n.data.rolled, n.data.roll_mode)).unwrap_or((false, false, 0));
            if click.0 && click.2 == 0 {
                set_rolled(id, !click.1);
            }
            1
        }
        _ => 0,
    }
}

/// Mouse encima: se ven los botones y, en Auto, la nota se despliega.
unsafe extern "C" fn on_root_event(_w: Rid, ev: *const REvent, user: *mut c_void) -> i32 {
    if (*ev).type_ == R_EVENT_MOUSE_ENTER || (*ev).type_ == R_EVENT_MOUSE_MOVE {
        on_enter(from_ud(user));
    }
    0
}

fn on_enter(id: u32) {
    let Some((buttons, auto, timer, was)) = with_note(id, |n| {
        let was = n.hovered;
        n.hovered = true;
        let t = n.roll_timer;
        n.roll_timer = R_INVALID_ID;
        (n.buttons, n.data.roll_mode == 1, t, was)
    }) else { return };
    if was {
        return;
    }
    unsafe {
        if timer != R_INVALID_ID {
            Rtimer_destroy(timer);
        }
        Rwidget_set_visibility(buttons, R_VISIBLE);
        let poll = Rtimer_create(0.15, 1, Some(on_hover_poll), ud(id));
        with_note(id, |n| n.hover_timer = poll);
    }
    if auto {
        set_rolled(id, false);
    }
}

/// ¿Sigue el mouse sobre la nota (o sobre su menú abierto)?
unsafe extern "C" fn on_hover_poll(_t: Rid, user: *mut c_void) {
    let id = from_ud(user);
    let flyout_open = with(|a| a.flyout.note == id && a.flyout.win != R_INVALID_ID).unwrap_or(false);
    let Some(hwnd) = with_note(id, |n| n.hwnd) else { return };
    if flyout_open || win32::cursor_inside(hwnd) {
        return;
    }
    on_leave(id);
}

fn on_leave(id: u32) {
    let Some((buttons, auto, poll, renaming)) = with_note(id, |n| {
        n.hovered = false;
        let p = n.hover_timer;
        n.hover_timer = R_INVALID_ID;
        (n.buttons, n.data.roll_mode == 1, p, n.renaming)
    }) else { return };
    unsafe {
        if poll != R_INVALID_ID {
            Rtimer_destroy(poll);
        }
        if !renaming {
            Rwidget_set_visibility(buttons, R_HIDDEN);
        }
        if auto && !renaming {
            let delay = with(|a| ROLL_DELAYS_MS[a.settings.roll_delay as usize % 5]).unwrap_or(0);
            if delay == 0 {
                set_rolled(id, true);
            } else {
                let t = Rtimer_create(delay as f64 / 1000.0, 0, Some(on_roll_timer), ud(id));
                with_note(id, |n| n.roll_timer = t);
            }
        }
    }
}

unsafe extern "C" fn on_roll_timer(_t: Rid, user: *mut c_void) {
    let id = from_ud(user);
    let go = with_note(id, |n| {
        n.roll_timer = R_INVALID_ID;
        !n.hovered && n.data.roll_mode == 1
    });
    if go == Some(true) {
        set_rolled(id, true);
    }
}

unsafe extern "C" fn on_text_changed(editor: Rid, user: *mut c_void) {
    let id = from_ud(user);
    let txt = text(Rtext_editor_get_text(editor));
    let title = with_note(id, |n| {
        n.data.text = txt;
        (n.title, n.data.display_title(), n.win)
    });
    if let Some((label, t, win)) = title {
        Rlabel_set_text(label, s(&t).as_ptr());
        Rwindow_set_title(win, s(&t).as_ptr());
    }
    touch(id, 0);
}

unsafe extern "C" fn on_rename_typed(_input: Rid, value: *const c_char, user: *mut c_void) {
    let id = from_ud(user);
    let value = text(value);
    let changed = with_note(id, |n| {
        if n.renaming {
            n.data.title = value.trim().to_string();
        }
        n.renaming
    });
    if changed == Some(true) {
        touch(id, 0);
    }
}

unsafe extern "C" fn on_rename_commit(input: Rid, value: *const c_char, user: *mut c_void) {
    let id = from_ud(user);
    let value = text(value);
    let Some((label, editor)) = with_note(id, |n| {
        if !n.renaming {
            return None;
        }
        n.renaming = false;
        n.data.title = value.trim().to_string();
        Some((n.title, n.editor))
    })
    .flatten() else { return };
    let shown = with_note(id, |n| n.data.display_title()).unwrap_or_default();
    Rlabel_set_text(label, s(&shown).as_ptr());
    Rwidget_set_visibility(input, R_COLLAPSED);
    Rwidget_set_visibility(label, R_VISIBLE);
    Rwidget_focus(editor);
    touch(id, 0);
    refresh_all_notes();
}

/// Alt+F4 sobre una nota la oculta (queda en "Todas las notas").
unsafe extern "C" fn on_close(_win: Rid, user: *mut c_void) -> i32 {
    hide(from_ud(user));
    0
}

unsafe extern "C" fn on_all_close(_win: Rid, _user: *mut c_void) -> i32 {
    let any_visible = with(|a| a.notes.iter().any(|n| !n.data.hidden)).unwrap_or(false);
    if any_visible {
        with(|a| a.all = AllNotes::default());
        1
    } else {
        // Es la única ventana a la vista: cerrarla es salir.
        quit();
        1
    }
}

unsafe extern "C" fn on_card_event(_w: Rid, ev: *const REvent, user: *mut c_void) -> i32 {
    if (*ev).type_ == R_EVENT_MOUSE_UP && (*ev).button == R_MOUSE_LEFT {
        show_note(from_ud(user));
        return 1;
    }
    0
}

unsafe extern "C" fn on_search(_input: Rid, _text: *const c_char, _user: *mut c_void) {
    refresh_all_notes();
}

unsafe extern "C" fn on_save_timer(_t: Rid, _user: *mut c_void) {
    with(|a| a.save_timer = R_INVALID_ID);
    save_now();
}

/// Errores de Raven, a  (con SIMPCKY_RAVEN_LOG).
unsafe extern "C" fn on_raven_error(code: RError, _sev: RSeverity, detail: *const c_char) {
    log(&format!("{}: {}", text(Rerror_to_string(code)), text(detail)));
}

fn log(line: &str) {
    use std::io::Write;
    if std::env::var_os("SIMPCKY_RAVEN_LOG").is_none() {
        return;
    }
    let path = std::env::temp_dir().join("simpcky-raven.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Atajos de teclado: `sender` es la ventana (o un widget de ella).
fn note_of(sender: Rid) -> Option<u32> {
    let win = unsafe { Rwidget_get_window(sender) };
    with(|a| a.notes.iter().find(|n| n.win == sender || n.win == win).map(|n| n.data.id)).flatten()
}

fn main() {
    unsafe {
        if std::env::var_os("SIMPCKY_RAVEN_LOG").is_some() {
            Rerror_set_callback(Some(on_raven_error));
        }
        if Rinit(R_BACKEND_VULKAN) == 0 {
            eprintln!("Raven no pudo iniciar Vulkan");
            std::process::exit(1);
        }
    }
    let settings = load_settings();
    unsafe { Rlook_set(if settings.dark { R_LOOK_WINDOWS_DARK } else { R_LOOK_WINDOWS_LIGHT }) };

    // Íconos: la fuente del sistema de Windows 11 (o la de Windows 10).
    let icons = ["C:\\Windows\\Fonts\\SegoeIcons.ttf", "C:\\Windows\\Fonts\\segmdl2.ttf"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(|p| unsafe { Rfont_create_from_file(s(p).as_ptr()) })
        .unwrap_or(R_INVALID_ID);
    // Segoe UI para el texto, con emojis de respaldo, como en el RichEdit.
    unsafe {
        let emoji = Rfont_create_from_file(s("C:\\Windows\\Fonts\\seguiemj.ttf").as_ptr());
        let text_font = Rfont_create_from_file(s("C:\\Windows\\Fonts\\segoeui.ttf").as_ptr());
        let bold = Rfont_create_from_file(s("C:\\Windows\\Fonts\\seguisb.ttf").as_ptr());
        if emoji != R_INVALID_ID {
            for f in [Rtheme_get_font(), text_font, bold] {
                if f != R_INVALID_ID {
                    Rfont_set_fallback(f, emoji);
                }
            }
        }
        TEXT_FONT.with(|f| f.set((text_font, bold)));
    }

    let mut notes = load_notes();
    if notes.is_empty() {
        let mut welcome = NoteData::new(1, 160, 140, 0);
        welcome.title = "Bienvenida".into();
        welcome.text = "## Simpcky con Raven\n\nEsta nota está dibujada con **Raven** (Vulkan), no con GDI.\n\n- [x] Arrastrá la barra para moverla\n- [ ] Un clic en la barra la enrolla\n- [ ] Doble clic la renombra\n- [ ] El menú ⋯ tiene colores, *siempre encima*, tema oscuro…\n\nCtrl+N crea otra nota.".into();
        notes.push(welcome);
    }
    let next_id = notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;

    APP.with(|cell| {
        *cell.borrow_mut() = Some(App { notes: Vec::new(), settings, next_id, save_timer: R_INVALID_ID, icons, all: AllNotes::default(), flyout: Flyout::default() });
    });
    for data in notes {
        let note = unsafe { create_note(data, &settings, icons) };
        with(|a| a.notes.push(note));
    }

    action_bind("nota.nueva", |sender, _| new_note(note_of(sender)));
    action_bind("nota.enrollar", |sender, _| {
        if let Some(id) = note_of(sender) {
            let rolled = with_note(id, |n| n.data.rolled).unwrap_or(false);
            set_rolled(id, !rolled);
        }
    });
    action_bind("nota.ocultar", |sender, _| {
        if let Some(id) = note_of(sender) {
            hide(id)
        }
    });
    action_bind("nota.encima", |sender, _| {
        if let Some(id) = note_of(sender) {
            toggle_pin(id)
        }
    });
    action_bind("nota.renombrar", |sender, _| {
        if let Some(id) = note_of(sender) {
            start_rename(id)
        }
    });
    action_bind("todas.nueva", |_, _| new_note(None));
    action_bind("todas.salir", |_, _| quit());
    action_bind("menu.cerrar", |_, _| close_flyout());

    if !with(|a| a.notes.iter().any(|n| !n.data.hidden)).unwrap_or(false) {
        open_all_notes();
    }
    if std::env::var_os("SIMPCKY_RAVEN_TODAS").is_some() {
        open_all_notes();
    }

    unsafe {
        while Rwait_frame(-1.0) != 0 {}
    }
    save_now();
    unsafe { Rshutdown() };
}
