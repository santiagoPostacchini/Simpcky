//! Ventana de una nota individual: creación, encabezado (título y
//! botones), arrastre, menú "⋯", enrollado (manual/auto) y autoguardado.
//! El cuerpo (texto con formato, listas, emojis) es de `editor.rs`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicI32, Ordering};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Controls::{TOOLTIPS_CLASSW, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_NEWTOOLRECTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    DragDetect, GetCapture, GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_F2, VK_MENU, VK_OEM_COMMA, VK_OEM_PERIOD, VK_OEM_PLUS,
    VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::{app, save_all};
use crate::editor;
use crate::flyout::{self, Entry};
use crate::persist::{Layer, NoteData, RollMode};
use crate::win::wide;

/// Alto del encabezado a 100 % (96 ppp). Todas las medidas de la nota
/// están pensadas a 100 % y se escalan con el ppp del monitor (ver
/// `px`): a 150 %, sin esto, el texto (que el RichEdit sí escala) quedaba
/// enorme al lado de una barra y unos botones diminutos.
const HEADER_H: i32 = 40;
const RICHEDIT_ID: usize = 100;
const TIMER_HOVER: usize = 1;
const TIMER_AUTOSAVE: usize = 2;
const TIMER_DESKTOP_WATCH: usize = 3;
const HOVER_POLL_MS: u32 = 150;
/// Mientras el mouse está sobre la nota: cuándo se fue (y hay que
/// esconder los botones del encabezado).
const TIMER_HOT: usize = 6;
const HOT_POLL_MS: u32 = 100;
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

const ID_COLOR_BASE: u32 = 2000;
const ID_DELETE_NOTE: u32 = 2105;
const ID_DUPLICATE_NOTE: u32 = 2106;
const ID_RENAME: u32 = 2107;
/// "Siempre encima" (prendido) o widget de escritorio (apagado).
const ID_TOGGLE_TOP: u32 = 2109;
/// Enrollado automático (prendido) o manual (apagado).
const ID_TOGGLE_AUTOROLL: u32 = 2110;
const ID_ALL_NOTES: u32 = 2111;
/// Ocultar: la nota queda solo en "Todas las notas".
const ID_HIDE: u32 = 2112;
/// Bloquear: solo lectura, hasta tocar el candado.
const ID_LOCK: u32 = 2113;
const TIMER_PEEK_END: usize = 4;
/// Acomodar otra vez las notas del escritorio: un momento después de que
/// se active una, y un rato después de arrancar (ver `settle_widgets_with`).
const TIMER_SETTLE: usize = 7;

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

/// Registra una nota en el estado global (nueva o cargada desde disco)
/// y crea su ventana — salvo que esté guardada en "Todas las notas"
/// (`hidden`): entonces devuelve nulo, y la ventana aparece recién al
/// abrirla (`show_hidden`).
pub fn create_note(data: NoteData) -> HWND {
    {
        let mut a = app().lock().unwrap();
        a.notes.insert(
            data.id,
            crate::app::NoteRuntime {
                data: data.clone(),
                hwnd: 0,
                edit: 0,
                peeking: false,
                deleting: false,
                last_active: 0,
            },
        );
    }
    if data.hidden {
        return null_mut();
    }
    create_window(&data)
}

/// La ventana de una nota ya registrada.
fn create_window(data: &NoteData) -> HWND {
    let id = data.id;
    let hinstance = { app().lock().unwrap().hinstance };
    let class_name = wide(NOTE_CLASS);
    let header = flyout::px(flyout::dpi_at(data.x, data.y) * scale_pct() / 100, HEADER_H);
    let h = if data.rolled { header } else { data.h };
    // Donde se dibuja, no donde "vive": una nota que viene de otra compu
    // (o de un monitor que ya no está) puede tener coordenadas fuera de
    // esta pantalla. Se muestra ajustada, pero el dato queda intacto:
    // si se lo reescribiera, esta compu le "corregiría" la posición a
    // las demás en la próxima sincronización.
    let (x, y) = clamp_to_work_area(data.x, data.y, data.w, header);
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

    // Modo vidrio: `WS_POPUPWINDOW` (con borde y menú de sistema, que
    // igual no se ven: ver WM_NCCALCSIZE) es lo que el mod de Windhawk
    // reconoce como ventana, y lo mira al crearla.
    let style = if crate::glass::active() { WS_POPUPWINDOW } else { WS_POPUP };
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            null(),
            // Sin WS_VISIBLE: una ventana que nace visible se activa, y al
            // arrancar la app eso ponía las notas adelante de las
            // aplicaciones. Se muestran al final, sin activarlas.
            style | WS_CLIPCHILDREN,
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
        // Sin el borde gris de 1 px que Windows 11 le pone alrededor a
        // las ventanas redondeadas: la nota es su color (y en modo vidrio
        // el desenfoque), sin marco.
        let border: u32 = DWMWA_COLOR_NONE;
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &pref as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR as u32,
                &border as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
            // Modo vidrio: el `WS_BORDER` de `WS_POPUPWINDOW` hizo falta
            // para que el mod la reconociera al crearla, y ya aplicó su
            // desenfoque; de acá en más solo dibujaría un marco gris de
            // 1 px (el del vidrio extendido a toda la ventana).
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            if style & WS_BORDER != 0 {
                SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !WS_BORDER) as i32 as isize);
                SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            }
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    hwnd
}

fn note_id(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32 }
}

/// Escala de las notas elegida en Configuración (100, 125 o 150 %).
/// Aparte del estado global: se consulta en cada medida, también con
/// ese estado tomado.
static SCALE: AtomicI32 = AtomicI32::new(100);

pub fn scale_pct() -> i32 {
    SCALE.load(Ordering::Relaxed)
}

fn pct_of(level: u8) -> i32 {
    match level {
        1 => 125,
        2 => 150,
        _ => 100,
    }
}

/// Al arrancar, con los ajustes ya cargados.
pub fn load_scale() {
    let level = app().lock().unwrap().settings.scale;
    SCALE.store(pct_of(level), Ordering::Relaxed);
}

/// Otra escala: las notas se agrandan o se achican enteras (letra,
/// encabezado y también la ventana, en proporción) y se rehacen.
pub fn set_scale(level: u8) {
    let (old, new) = (scale_pct(), pct_of(level));
    app().lock().unwrap().settings.scale = level;
    crate::app::save_settings();
    if old == new {
        return;
    }
    SCALE.store(new, Ordering::Relaxed);
    flush_all();
    {
        let mut a = app().lock().unwrap();
        for nr in a.notes.values_mut() {
            nr.data.w = nr.data.w * new / old;
            nr.data.h = nr.data.h * new / old;
        }
    }
    save_all();
    recreate_all();
}

/// El ppp con el que se dibuja la nota: el del monitor, por la escala.
pub fn note_dpi(hwnd: HWND) -> i32 {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96) as i32;
    dpi * scale_pct() / 100
}

/// `v` píxeles a 100 %, al ppp y la escala de la nota.
fn px(hwnd: HWND, v: i32) -> i32 {
    v * note_dpi(hwnd) / 96
}

/// Alto del encabezado de esta nota (y de la nota entera, enrollada).
fn header_h(hwnd: HWND) -> i32 {
    px(hwnd, HEADER_H)
}

/// El tamaño de una nota nueva, al ppp del monitor en (`x`, `y`).
pub fn default_size(x: i32, y: i32) -> (i32, i32) {
    let dpi = flyout::dpi_at(x, y) * scale_pct() / 100;
    (flyout::px(dpi, 280), flyout::px(dpi, 320))
}

/// Fuente semibold del título (la del cuadro de renombrar, y la de
/// respaldo si DirectWrite fallara).
pub fn header_font(hwnd: HWND) -> HFONT {
    flyout::font(-px(hwnd, 15), FW_SEMIBOLD as i32, 0, "Segoe UI")
}

fn apply_body_style(edit: HWND, color_idx: u8) {
    let (_, body, ink) = palette_entry(color_idx);
    editor::set_colors(edit, body, ink);
}

/// Deja aire entre el texto y el borde de la nota (el RichEdit por
/// defecto arranca a escribir pegado al canto).
///
/// Solo a los costados: el aire de arriba y de abajo lo pone la nota
/// (el RichEdit arranca `TEXT_PAD_Y` más abajo y termina `TEXT_PAD_Y`
/// antes, ver `edit_rect`). Con un margen de abajo adentro del RichEdit,
/// la línea que queda a medias al final se dibujaba en ese margen y, al
/// desplazar el texto, los pedazos de líneas viejas quedaban ahí pegados.
fn apply_text_padding(edit: HWND, w: i32, edit_h: i32) {
    editor::set_padding(edit, w, edit_h, px(edit, TEXT_PAD_X), 0);
}

/// Dónde va el RichEdit en una nota de `w`×`total_h`: debajo del
/// encabezado, con aire arriba y abajo, y más ancho que la nota justo lo
/// que mide su barra de desplazamiento, que así queda recortada (la que
/// se ve es la fina que dibuja `editor.rs`).
fn edit_rect(hwnd: HWND, w: i32, total_h: i32) -> RECT {
    let hh = header_h(hwnd);
    let pad = px(hwnd, TEXT_PAD_Y);
    RECT { left: 0, top: hh + pad, right: w + editor::hidden_bar_width(hwnd), bottom: (total_h - pad).max(hh + pad) }
}

pub fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    // La marca de una lista no es parte del título.
    let line = line.trim_start_matches(['\u{2610}', '\u{2611}', '\u{2022}']).trim_start();
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
    let t = layout_of(hwnd).title_rect;
    let h = px(hwnd, 24);
    let top = (header_h(hwnd) - h) / 2;
    Some((title, header, ink, RECT { left: t.left - px(hwnd, 2), top, right: t.right, bottom: top + h }))
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
// Encabezado: título y botones
// -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    /// "+": una nota nueva al lado de esta.
    Add,
    /// Siempre encima / widget de escritorio.
    Pin,
    /// Enrollar / desenrollar (solo en modo manual: en automático se
    /// enrolla sola).
    Chevron,
    /// "⋯": colores y el resto de las opciones.
    More,
    /// El candado de una nota bloqueada: la desbloquea.
    Lock,
    /// El resto de la barra: arrastrar mueve la nota, doble clic la
    /// renombra.
    Bar,
    None,
}

const BTN_W: i32 = 32;
const BTN_H: i32 = 30;

struct HeaderLayout {
    /// Alto del encabezado, ya escalado.
    height: i32,
    /// El ícono de "no se pudo guardar", antes del título (ver
    /// `app::save_failing`).
    warn: Option<RECT>,
    add: RECT,
    pin: RECT,
    chevron: Option<RECT>,
    more: RECT,
    title_rect: RECT,
    /// Los botones se ven solo con el mouse encima o mientras se escribe
    /// en la nota; el resto del tiempo la barra es nada más el título
    /// (un widget de escritorio no necesita cuatro íconos a la vista).
    buttons: bool,
    /// Nota bloqueada: en lugar de los botones, solo el candado, siempre
    /// a la vista.
    lock: Option<RECT>,
}

impl HeaderLayout {
    fn new(width: i32, buttons: bool, chevron: bool, dpi: i32, warn: bool, locked: bool) -> Self {
        let p = |v| flyout::px(dpi, v);
        let height = p(HEADER_H);
        let (bw, bh, edge) = (p(BTN_W), p(BTN_H), p(6));
        let top = (height - bh) / 2;
        let slot = |i: i32| RECT { left: width - edge - bw * (i + 1), top, right: width - edge - bw * i, bottom: top + bh };
        let more = slot(0);
        let (chevron, pin, add) = if chevron { (Some(slot(1)), slot(2), slot(3)) } else { (None, slot(1), slot(2)) };
        let buttons = buttons && !locked;
        let lock = locked.then(|| slot(0));
        let right = match lock {
            Some(r) => r.left - p(4),
            None if buttons => add.left - p(4),
            None => width - p(14),
        };
        let warn = warn.then(|| RECT { left: p(8), top, right: p(8) + bh, bottom: top + bh });
        let left = warn.map(|r| r.right + p(2)).unwrap_or(p(14));
        let title_rect = RECT { left, top: 0, right: right.max(left), bottom: height };
        HeaderLayout { height, warn, add, pin, chevron, more, title_rect, buttons, lock }
    }

    fn hit(&self, x: i32, y: i32) -> Hit {
        if y < 0 || y >= self.height {
            return Hit::None;
        }
        let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        if self.lock.as_ref().is_some_and(inside) {
            return Hit::Lock;
        }
        if self.buttons {
            if inside(&self.more) {
                return Hit::More;
            }
            if self.chevron.as_ref().is_some_and(inside) {
                return Hit::Chevron;
            }
            if inside(&self.pin) {
                return Hit::Pin;
            }
            if inside(&self.add) {
                return Hit::Add;
            }
        }
        Hit::Bar
    }
}

/// Estado del encabezado de cada nota que no hace falta guardar: si el
/// mouse está encima, qué botón está resaltado, y su ventana de ayudas.
#[derive(Default)]
struct HeaderState {
    hover: bool,
    hot: Option<Hit>,
    tips: isize,
}

thread_local! {
    static HEADERS: RefCell<HashMap<isize, HeaderState>> = RefCell::new(HashMap::new());
}

fn header_state<R>(hwnd: HWND, f: impl FnOnce(&mut HeaderState) -> R) -> R {
    HEADERS.with(|m| f(m.borrow_mut().entry(hwnd as isize).or_default()))
}

fn has_focus(hwnd: HWND) -> bool {
    let f = unsafe { GetFocus() };
    !f.is_null() && (f == hwnd || unsafe { IsChild(hwnd, f) } != 0)
}

fn layout_of(hwnd: HWND) -> HeaderLayout {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let id = note_id(hwnd);
    let auto = app().lock().unwrap().notes.get(&id).is_some_and(|nr| nr.data.roll_mode == RollMode::Auto);
    let hover = header_state(hwnd, |s| s.hover);
    let buttons = hover || has_focus(hwnd) || crate::rename::is_renaming(hwnd);
    HeaderLayout::new(rc.right, buttons, !auto, px(hwnd, 96), crate::app::save_failing(), is_locked(hwnd))
}

/// Repinta el encabezado de todas las notas (cambió algo que muestran
/// todas, como el aviso de que no se puede guardar).
pub fn repaint_headers() {
    let list: Vec<HWND> = app().lock().unwrap().notes.values().map(|nr| nr.hwnd as HWND).filter(|h| !h.is_null()).collect();
    for h in list {
        invalidate_header(h);
    }
}

/// Lo que cambia con el mouse encima o el foco: el encabezado (botones)
/// y la agarradera de la esquina.
fn invalidate_header(hwnd: HWND) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        GetClientRect(hwnd, &mut rc);
        rc.bottom = header_h(hwnd);
        InvalidateRect(hwnd, &rc, 0);
        let grip = grip_rect(hwnd);
        InvalidateRect(hwnd, &grip, 0);
    }
}

/// El mouse pasó por la nota (el encabezado, o su texto: ver
/// `editor.rs`): aparecen los botones, hasta que se vaya.
pub fn on_hover(hwnd: HWND) {
    let newly = header_state(hwnd, |s| !std::mem::replace(&mut s.hover, true));
    if newly {
        unsafe { SetTimer(hwnd, TIMER_HOT, HOT_POLL_MS, None) };
        invalidate_header(hwnd);
    }
}

/// Entró o salió el foco del texto: con foco, los botones quedan a mano.
pub fn on_focus_change(hwnd: HWND) {
    if !hwnd.is_null() {
        invalidate_header(hwnd);
    }
}

fn set_hot_button(hwnd: HWND, hit: Hit) {
    let hot = match hit {
        Hit::Bar | Hit::None => None,
        h => Some(h),
    };
    let changed = header_state(hwnd, |s| std::mem::replace(&mut s.hot, hot) != hot);
    if changed {
        invalidate_header(hwnd);
    }
}

/// ¿Sigue el mouse sobre la nota? (Sin TrackMouseEvent: el mouse pasa
/// del encabezado al texto, que es otra ventana, y eso contaría como
/// "se fue".)
fn hot_tick(hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    let under = unsafe { WindowFromPoint(pt) };
    let over = under == hwnd || unsafe { IsChild(hwnd, under) } != 0 || crate::seltool::is_over(under, hwnd);
    if over {
        let mut c = pt;
        unsafe { ScreenToClient(hwnd, &mut c) };
        set_hot_button(hwnd, layout_of(hwnd).hit(c.x, c.y));
        return;
    }
    if flyout::is_open() {
        return; // con el menú de la nota abierto, los botones siguen
    }
    unsafe { KillTimer(hwnd, TIMER_HOT) };
    header_state(hwnd, |s| {
        s.hover = false;
        s.hot = None;
    });
    invalidate_header(hwnd);
}

const TIPS: [(usize, &str); 6] = [
    (1, "Nota nueva (Ctrl+N)"),
    (2, "Siempre encima (Ctrl+Mayús+T)"),
    (3, "Enrollar o desenrollar (Ctrl+R)"),
    (4, "Color y más opciones"),
    (5, "No se pudo guardar en el disco: Simpcky reintenta solo (ver el ícono de la bandeja)"),
    (6, "Bloqueada: solo para leer y copiar. Clic para desbloquearla"),
];

/// Las ayudas que aparecen al dejar el mouse sobre un botón.
fn create_tooltips(hwnd: HWND) {
    unsafe {
        let hinstance = app().lock().unwrap().hinstance;
        // Sin dueño: una nota anclada es hija del escritorio, y el dueño
        // terminaría siendo una ventana de Explorer.
        let tips = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            TOOLTIPS_CLASSW,
            null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            null_mut(),
            null_mut(),
            hinstance as HINSTANCE,
            null(),
        );
        if tips.is_null() {
            return;
        }
        for (id, text) in TIPS {
            let w = wide(text);
            let mut ti: TTTOOLINFOW = std::mem::zeroed();
            ti.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
            ti.uFlags = TTF_SUBCLASS;
            ti.hwnd = hwnd;
            ti.uId = id;
            ti.lpszText = w.as_ptr() as *mut u16;
            SendMessageW(tips, TTM_ADDTOOLW, 0, &ti as *const TTTOOLINFOW as isize);
        }
        header_state(hwnd, |s| s.tips = tips as isize);
    }
}

/// Las ayudas, sobre los botones que se ven en este momento.
fn update_tooltips(hwnd: HWND, layout: &HeaderLayout) {
    let tips = header_state(hwnd, |s| s.tips) as HWND;
    if tips.is_null() {
        return;
    }
    let none = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let rects = [
        (1, if layout.buttons { layout.add } else { none }),
        (2, if layout.buttons { layout.pin } else { none }),
        (3, if layout.buttons { layout.chevron.unwrap_or(none) } else { none }),
        (4, if layout.buttons { layout.more } else { none }),
        (5, layout.warn.unwrap_or(none)),
        (6, layout.lock.unwrap_or(none)),
    ];
    for (id, rect) in rects {
        let mut ti: TTTOOLINFOW = unsafe { std::mem::zeroed() };
        ti.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
        ti.hwnd = hwnd;
        ti.uId = id;
        ti.rect = rect;
        unsafe { SendMessageW(tips, TTM_NEWTOOLRECTW, 0, &ti as *const TTTOOLINFOW as isize) };
    }
}

fn forget_header_state(hwnd: HWND) {
    let tips = HEADERS.with(|m| m.borrow_mut().remove(&(hwnd as isize)).map(|s| s.tips)).unwrap_or(0);
    if tips != 0 {
        unsafe { DestroyWindow(tips as HWND) };
    }
}

// -----------------------------------------------------------------
// Dibujo
// -----------------------------------------------------------------

/// `COLORREF` (0x00BBGGRR, lo que usa GDI) → ARGB opaco (0xAARRGGBB,
/// lo que espera GDI+).
pub fn colorref_to_argb(c: u32) -> u32 {
    let r = c & 0xff;
    let g = (c >> 8) & 0xff;
    let b = (c >> 16) & 0xff;
    0xff000000 | (r << 16) | (g << 8) | b
}

/// `a` sobre `b`, con `alpha` de 0 a 255 (colores GDI).
fn blend(a: u32, b: u32, alpha: u32) -> u32 {
    let mix = |sh: u32| (((a >> sh) & 0xff) * alpha + ((b >> sh) & 0xff) * (255 - alpha)) / 255;
    mix(0) | (mix(8) << 8) | (mix(16) << 16)
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
    let (header_color, body_color, ink) = palette_entry(color);
    let layout = layout_of(hwnd);
    let hot = header_state(hwnd, |s| s.hot);
    update_tooltips(hwnd, &layout);

    unsafe {
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut client);
        let width = client.right - client.left;
        // En memoria y después de una: los botones aparecen y se
        // resaltan con el mouse, y repintar directo parpadeaba.
        let hh = layout.height;
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, width.max(1), hh);
        let old_bmp = SelectObject(mem, bmp);

        let header_rect = RECT { left: 0, top: 0, right: width, bottom: hh };
        if let Some((ha, ba)) = crate::glass::opacity() {
            SelectObject(mem, old_bmp);
            DeleteObject(bmp);
            DeleteDC(mem);
            paint_glass_header(hwnd, hdc, &layout, width, hh, header_color, ink, ha, (pinned, rolled, hot), &title, renaming);
            if client.bottom > hh {
                crate::glass::fill(hdc, RECT { left: 0, top: hh, right: width, bottom: client.bottom }, body_color, ba);
                if layout.buttons {
                    paint_grip(hwnd, hdc, body_color, ink, Some(ba));
                }
            }
            return;
        }
        let brush = CreateSolidBrush(header_color);
        FillRect(mem, &header_rect, brush);
        DeleteObject(brush);
        SetBkMode(mem, TRANSPARENT as i32);

        if layout.buttons {
            let mut g: *mut GpGraphics = null_mut();
            GdipCreateFromHDC(mem, &mut g);
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            let hover_bg = blend(ink, header_color, 0x24);
            let mut buttons = vec![
                (Hit::Add, layout.add, 0xE710u16),
                (Hit::Pin, layout.pin, if pinned { 0xE842 } else { 0xE718 }),
                (Hit::More, layout.more, 0xE712),
            ];
            if let Some(r) = layout.chevron {
                buttons.push((Hit::Chevron, r, if rolled { 0xE70D } else { 0xE70E }));
            }
            for (hit, rect, glyph) in buttons {
                if hot == Some(hit) {
                    flyout::fill_round(g, &rect, px(hwnd, 5) as f32, hover_bg);
                }
                flyout::draw_glyph(mem, flyout::icon_font(-px(hwnd, 15)), glyph, &rect, ink);
            }
            GdipDeleteGraphics(g);
        }
        if let Some(rect) = layout.lock {
            if hot == Some(Hit::Lock) {
                let mut g: *mut GpGraphics = null_mut();
                GdipCreateFromHDC(mem, &mut g);
                GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
                flyout::fill_round(g, &rect, px(hwnd, 5) as f32, blend(ink, header_color, 0x24));
                GdipDeleteGraphics(g);
            }
            flyout::draw_glyph(mem, flyout::icon_font(-px(hwnd, 15)), LOCK_GLYPH, &rect, ink);
        }

        if let Some(r) = layout.warn {
            flyout::draw_glyph(mem, flyout::icon_font(-px(hwnd, 16)), 0xE7BA, &r, ink);
        }

        // El título (el nombre, o la primera línea del texto) se ve
        // siempre en el encabezado, esté enrollada o no.
        if !renaming && !crate::d2d::draw_title(mem, layout.title_rect, &title, (ink, 0xff), crate::d2d::Bg::Solid(header_color), px(hwnd, 15)) {
            // Sin DirectWrite (no debería pasar): con GDI, emojis en gris.
            let old_font = SelectObject(mem, header_font(hwnd));
            SetTextColor(mem, ink);
            let mut text_rc = layout.title_rect;
            let wtext = wide(&title);
            DrawTextW(mem, wtext.as_ptr(), -1, &mut text_rc, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_LEFT | DT_NOPREFIX);
            SelectObject(mem, old_font);
        }

        BitBlt(hdc, 0, 0, width, hh, mem, 0, 0, SRCCOPY);
        // Debajo del encabezado: el margen alrededor del texto (el
        // RichEdit tapa el resto; WS_CLIPCHILDREN evita pisarlo).
        if client.bottom > hh {
            fill_body(hdc, RECT { left: 0, top: hh, right: width, bottom: client.bottom }, body_color);
            if layout.buttons {
                paint_grip(hwnd, hdc, body_color, ink, None);
            }
        }
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
    }
}

// -----------------------------------------------------------------
// Enrollado (manual/auto)
// -----------------------------------------------------------------

fn apply_rolled_state(hwnd: HWND, edit: HWND, w: i32, full_h: i32, rolled: bool) {
    let new_h = if rolled { header_h(hwnd) } else { full_h };
    if rolled {
        crate::seltool::hide_for(edit);
    }
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
    let hovered = under == hwnd || unsafe { IsChild(hwnd, under) != 0 } || crate::seltool::is_over(under, hwnd);
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
        if !rolled {
            raise_above_notes(hwnd);
        }
    }
}

/// La que se desenrolla pasa adelante de las otras notas (si no, otra
/// enrollada encima le tapaba el texto), sin pasar adelante de ninguna
/// aplicación ni robar el foco.
fn raise_above_notes(hwnd: HWND) {
    unsafe {
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD != 0 {
            // Anclada: hija del escritorio, arriba de sus hermanas.
            crate::desktop::raise(hwnd);
        } else if glass_widget(hwnd) {
            // Widget en modo vidrio: justo encima del más alto de los
            // otros widgets (que están debajo de las aplicaciones). Mandar
            // al fondo uno por uno no sirve para ordenarlos: con ventanas
            // "poseídas" por el escritorio, Windows no respeta ese orden.
            place_above(hwnd, widget_base(hwnd));
        } else {
            // "Siempre encima" (o asomada): al tope de su capa.
            SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
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

/// El menú de la nota (el botón "⋯"), el único lugar donde se elige el
/// color. `below`: el botón del que cuelga, en coordenadas de pantalla.
fn show_note_menu(hwnd: HWND, below: RECT) {
    let id = note_id(hwnd);
    let (color, roll_mode, layer) = {
        let a = app().lock().unwrap();
        match a.notes.get(&id) {
            Some(nr) => (nr.data.color, nr.data.roll_mode, nr.data.layer),
            None => return,
        }
    };
    let entries = vec![
        Entry::Swatches { base: ID_COLOR_BASE, current: color },
        Entry::Separator,
        Entry::toggle(ID_TOGGLE_TOP, 0xE718, "Siempre encima", "Ctrl+Mayús+T", layer == Layer::AlwaysOnTop),
        Entry::toggle(ID_TOGGLE_AUTOROLL, 0xE70E, "Enrollar al quitar el mouse", "", roll_mode == RollMode::Auto),
        Entry::item(ID_LOCK, LOCK_GLYPH, "Bloquear (solo lectura)", ""),
        Entry::item(ID_RENAME, 0xE8AC, "Cambiar nombre", "F2"),
        Entry::item(ID_DUPLICATE_NOTE, 0xE8C8, "Duplicar", ""),
        Entry::item(ID_HIDE, 0xED1A, "Ocultar", "Ctrl+W"),
        Entry::item(ID_ALL_NOTES, 0xE70B, "Todas las notas", ""),
        Entry::Separator,
        Entry::item(ID_DELETE_NOTE, 0xE74D, "Eliminar nota", "").danger(),
    ];
    flyout::show(hwnd, entries, flyout::Anchor::Below(below));
}

/// "+" del encabezado: una nota nueva al lado de esta (a la derecha, o a
/// la izquierda si no entra).
fn new_note_beside(hwnd: HWND) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut rc) };
    let (w, _) = default_size(rc.left, rc.top);
    let mut x = rc.right + 12;
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 && x + w > info.rcWork.right {
            x = rc.left - 12 - w;
        }
    }
    crate::tray::spawn_note_at(x, rc.top);
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
            if crate::glass::active() {
                crate::desktop::disown(hwnd);
            } else {
                crate::desktop::detach(hwnd);
            }
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        } else {
            // Si se la está usando, no se va de golpe al escritorio: se
            // queda adelante, asomada, hasta que el foco pase a otra cosa
            // (ver TIMER_PEEK_END). Si no, vuelve ya.
            let active = GetForegroundWindow() == hwnd;
            if active {
                let id = note_id(hwnd);
                if let Some(nr) = app().lock().unwrap().notes.get_mut(&id) {
                    nr.peeking = true;
                }
            }
            SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            if !active {
                anchor_widget(hwnd);
            }
        }
        let active = GetForegroundWindow() == hwnd;
        apply_taskbar_visibility(hwnd, if to_top { Layer::AlwaysOnTop } else { Layer::Desktop });
        // Refrescar el botón de la barra de tareas oculta y muestra la
        // ventana, y eso le saca el foco: se le devuelve.
        if active && GetForegroundWindow() != hwnd {
            SetForegroundWindow(hwnd);
        }
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
        if !edit.is_null() && old.color != new.color {
            apply_body_style(edit, new.color);
        }
        if !edit.is_null() && (old.text != new.text || old.fmt != new.fmt) {
            editor::load(edit, &new.text, &new.fmt);
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
            let (x, y) = clamp_to_work_area(new.x, new.y, new.w, header_h(hwnd));
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
        if crate::glass::active() {
            crate::desktop::own(hwnd);
        } else {
            crate::desktop::anchor(hwnd);
        }
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
    if ask_delete(owner) {
        // Marcar ANTES de destruir: on_destroy solo borra los datos de
        // una nota si el usuario lo pidió (ver `NoteRuntime::deleting`).
        let id = note_id(hwnd);
        if let Some(nr) = app().lock().unwrap().notes.get_mut(&id) {
            nr.deleting = true;
        }
        unsafe { DestroyWindow(hwnd) };
    }
}

fn ask_delete(owner: HWND) -> bool {
    let msg = wide("¿Eliminar esta nota? Esta acción no se puede deshacer.");
    let title = wide("Simpcky");
    unsafe { MessageBoxW(owner, msg.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONWARNING) == IDYES }
}

/// "Eliminar nota" desde "Todas las notas": con ventana o guardada.
pub fn delete_by_id(id: u32, owner: HWND) {
    let hwnd = app().lock().unwrap().notes.get(&id).map(|nr| nr.hwnd as HWND);
    match hwnd {
        Some(h) if !h.is_null() => delete_note_confirm(h, owner),
        Some(_) if ask_delete(owner) => {
            app().lock().unwrap().notes.remove(&id);
            save_all();
        }
        _ => {}
    }
}

// -----------------------------------------------------------------
// Guardar en "Todas las notas" (ocultar) y volver a mostrar
// -----------------------------------------------------------------

/// El candado del encabezado (Segoe Fluent Icons / MDL2: "Lock").
const LOCK_GLYPH: u16 = 0xE72E;

pub fn is_locked(hwnd: HWND) -> bool {
    let id = note_id(hwnd);
    app().lock().unwrap().notes.get(&id).is_some_and(|nr| nr.data.locked)
}

/// Bloquea la nota (solo lectura: se puede leer, seleccionar y copiar,
/// moverla y nada más; en la barra queda solo el candado) o la libera
/// (tocando el candado).
pub fn set_locked(hwnd: HWND, on: bool) {
    let id = note_id(hwnd);
    let edit = {
        let mut a = app().lock().unwrap();
        let Some(nr) = a.notes.get_mut(&id) else { return };
        nr.data.locked = on;
        nr.edit as HWND
    };
    if on {
        flyout::close();
        crate::rename::finish(hwnd, true);
    }
    if !edit.is_null() {
        editor::set_locked(edit, on);
    }
    header_state(hwnd, |s| s.hot = None);
    // Cambia solo la barra (y la agarradera): repintar la nota entera
    // borraba el texto hasta el próximo repintado del RichEdit.
    invalidate_header(hwnd);
    save_all();
}

/// "Ocultar" (menú, Ctrl+W o cerrar la ventana): la nota deja el
/// escritorio y queda guardada en "Todas las notas", de donde vuelve con
/// doble clic o arrastrándola afuera.
pub fn hide_note(hwnd: HWND) {
    let id = note_id(hwnd);
    flyout::close();
    crate::rename::finish(hwnd, true);
    commit_text_and_save(hwnd);
    {
        let mut a = app().lock().unwrap();
        let Some(nr) = a.notes.get_mut(&id) else { return };
        nr.data.hidden = true;
    }
    // No se borra nada: on_destroy guarda el texto y los datos, y como
    // está guardada no se la vuelve a crear (ver `recreate_lost`).
    unsafe { DestroyWindow(hwnd) };
    // La primera vez, dónde quedó.
    static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
        crate::tray::notify_action(
            "La nota quedó en \"Todas las notas\"",
            "Para volver a verla en el escritorio, abrila con doble clic o arrastrala afuera.",
            crate::allnotes::show,
        );
    }
}

/// "Ocultar" desde "Todas las notas".
pub fn hide_by_id(id: u32) {
    let hwnd = app().lock().unwrap().notes.get(&id).map(|nr| nr.hwnd as HWND).unwrap_or(null_mut());
    if !hwnd.is_null() {
        hide_note(hwnd);
    }
}

/// La ventana de la nota `id`, creándola si estaba guardada (en su lugar
/// de siempre, o en `at`). Nulo si la nota no existe o no se pudo.
fn show_hidden(id: u32, at: Option<(i32, i32)>) -> HWND {
    let data = {
        let mut a = app().lock().unwrap();
        let Some(nr) = a.notes.get_mut(&id) else { return null_mut() };
        if nr.hwnd != 0 {
            return nr.hwnd as HWND;
        }
        nr.data.hidden = false;
        if let Some((x, y)) = at {
            (nr.data.x, nr.data.y) = (x, y);
        }
        nr.data.clone()
    };
    let hwnd = create_window(&data);
    save_all();
    hwnd
}

/// "Abrir" desde "Todas las notas", esté o no en el escritorio.
pub fn open_by_id(id: u32) -> HWND {
    let hwnd = show_hidden(id, None);
    if !hwnd.is_null() {
        open_note(hwnd);
    }
    hwnd
}

/// Soltar una tarjeta de "Todas las notas" en el escritorio.
pub fn drop_by_id(id: u32, x: i32, y: i32) {
    let hwnd = show_hidden(id, Some((x, y)));
    if !hwnd.is_null() {
        drop_on_desktop(hwnd, x, y);
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
            // En vidrio ya es de primer nivel: asomarse es solo pasar
            // adelante (ver WM_WINDOWPOSCHANGING).
            if !crate::glass::active() {
                crate::desktop::detach(hwnd);
            }
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
    if !crate::desktop::is_desktop_visible_at(x + px(hwnd, 24), y + header_h(hwnd) / 2) {
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
    let (x, y) = clamp_to_work_area(x, y, w, header_h(hwnd));
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
            // En modo vidrio, la opacidad depende también del tema.
            editor::set_glass(edit, crate::glass::opacity().map(|(_, b)| b));
        }
        unsafe { InvalidateRect(hwnd, null(), 1) };
    }
    flyout::close();
}

/// Recrea las ventanas de las notas que se quedaron sin ventana (ver
/// `on_destroy`): típicamente, Explorer se reinició y se llevó puesto
/// a Progman con todas sus hijas.
pub fn recreate_lost() {
    let lost: Vec<NoteData> = {
        let a = app().lock().unwrap();
        a.notes.values().filter(|nr| nr.hwnd == 0 && !nr.data.hidden).map(|nr| nr.data.clone()).collect()
    };
    for data in lost {
        create_window(&data);
    }
}

/// El contenido del RichEdit: texto (con los saltos de línea siempre
/// como `\n`: si el mismo texto pudiera quedar guardado de dos formas,
/// cada compu lo vería como un cambio y se lo pasarían de una a otra
/// para siempre) y formato.
fn read_text(edit: HWND) -> (String, String) {
    editor::refresh(edit);
    editor::read(edit)
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
    let texts: Vec<(u32, (String, String))> = edits.into_iter().map(|(id, edit)| (id, read_text(edit))).collect();
    let changed = {
        let mut a = app().lock().unwrap();
        let mut changed = false;
        for (id, (text, fmt)) in texts {
            if let Some(nr) = a.notes.get_mut(&id) {
                if nr.data.text != text || nr.data.fmt != fmt {
                    nr.data.text = text;
                    nr.data.fmt = fmt;
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
    let (text, fmt) = read_text(edit);
    {
        let mut a = app().lock().unwrap();
        if let Some(nr) = a.notes.get_mut(&id) {
            nr.data.text = text;
            nr.data.fmt = fmt;
        }
    }
    save_all();
}

// -----------------------------------------------------------------
// Procedimiento de ventana
// -----------------------------------------------------------------

fn on_create(hwnd: HWND) {
    let id = note_id(hwnd);
    let (hinstance, color, w, content_h, rolled, text, fmt, roll_mode, layer, locked) = {
        let a = app().lock().unwrap();
        let nr = a.notes.get(&id).expect("nota registrada antes de crear la ventana");
        (
            a.hinstance,
            nr.data.color,
            nr.data.w,
            nr.data.h - header_h(hwnd),
            nr.data.rolled,
            nr.data.text.clone(),
            nr.data.fmt.clone(),
            nr.data.roll_mode,
            nr.data.layer,
            nr.data.locked,
        )
    };
    let rc = edit_rect(hwnd, w, header_h(hwnd) + content_h);
    let edit = editor::create(hwnd, hinstance as HINSTANCE, RICHEDIT_ID, rc);
    apply_body_style(edit, color);
    editor::load(edit, &text, &fmt);
    crate::theme::apply_scrollbars(edit);
    apply_text_padding(edit, w, rc.bottom - rc.top);
    create_tooltips(hwnd);
    editor::set_glass(edit, crate::glass::opacity().map(|(_, b)| b));
    if locked {
        editor::set_locked(edit, true);
    }
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
    let rc = edit_rect(hwnd, w, total_h);
    unsafe {
        SetWindowPos(edit, null_mut(), rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOCOPYBITS);
    }
    apply_text_padding(edit, w, rc.bottom - rc.top);
    // Pintar ya, no "cuando se pueda": en un cambio de tamaño rápido
    // Windows muestra la parte nueva de la ventana antes de que la app la
    // pinte (en blanco, o la imagen vieja estirada), y el WM_PAINT recién
    // llegaba cuando el mouse se quedaba quieto.
    unsafe { RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW | RDW_ALLCHILDREN) };
}

/// Esquinas redondeadas. Suelta (siempre encima, o asomada), las pone
/// Windows 11 (`DWMWCP_ROUND`, con borde suave y sombra); pero Windows
/// solo redondea ventanas de primer nivel, y anclada al escritorio la nota
/// es hija de Progman: ahí se recorta con una región, con el mismo radio
/// de 8 px (el borde queda un poco más escalonado que el de Windows).
fn update_shape(hwnd: HWND) {
    unsafe {
        let child = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD != 0;
        if !child {
            let mut box_ = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            if GetWindowRgnBox(hwnd, &mut box_) != 0 {
                SetWindowRgn(hwnd, null_mut(), 1);
            }
            return;
        }
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(hwnd, &mut rc);
        let d = px(hwnd, 16);
        // El sistema se queda con la región: no se borra acá.
        let rgn = CreateRoundRectRgn(0, 0, rc.right - rc.left + 1, rc.bottom - rc.top + 1, d, d);
        SetWindowRgn(hwnd, rgn, 1);
    }
}

/// El fondo, con los colores de la nota (encabezado y cuerpo): lo que se
/// vea antes de que llegue el dibujo de verdad tiene que ser la nota, no
/// un rectángulo blanco.
fn on_erase(hwnd: HWND, hdc: HDC) {
    let id = note_id(hwnd);
    let Some(color) = app().lock().unwrap().notes.get(&id).map(|nr| nr.data.color) else { return };
    let (header, body, _) = palette_entry(color);
    let hh = header_h(hwnd);
    unsafe {
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        let b = CreateSolidBrush(header);
        FillRect(hdc, &RECT { bottom: hh.min(rc.bottom), ..rc }, b);
        DeleteObject(b);
        if rc.bottom > hh {
            fill_body(hdc, RECT { top: hh, ..rc }, body);
        }
    }
}

/// El cuerpo de la nota en `r` (coordenadas de la nota): liso o, en modo
/// vidrio, el color con su opacidad sobre el desenfoque de Windhawk.
fn fill_body(hdc: HDC, r: RECT, body: u32) {
    unsafe {
        if let Some((_, ba)) = crate::glass::opacity() {
            crate::glass::fill(hdc, r, body, ba);
            return;
        }
        let b = CreateSolidBrush(body);
        FillRect(hdc, &r, b);
        DeleteObject(b);
    }
}

/// El encabezado en modo vidrio: todo sobre un lienzo con alfa (el color
/// de la nota con su opacidad, los botones y el título), copiado de una.
#[allow(clippy::too_many_arguments)]
unsafe fn paint_glass_header(
    hwnd: HWND,
    hdc: HDC,
    layout: &HeaderLayout,
    width: i32,
    hh: i32,
    header_color: u32,
    ink: u32,
    alpha: u8,
    (pinned, rolled, hot): (bool, bool, Option<Hit>),
    title: &str,
    renaming: bool,
) {
    let Some(c) = crate::glass::Canvas::new(width.max(1), hh) else { return };
    c.fill(RECT { left: 0, top: 0, right: width, bottom: hh }, header_color, alpha);
    let center = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;
    if layout.buttons {
        let mut buttons = vec![
            (Hit::Add, layout.add, 0xE710u16),
            (Hit::Pin, layout.pin, if pinned { 0xE842 } else { 0xE718 }),
            (Hit::More, layout.more, 0xE712),
        ];
        if let Some(r) = layout.chevron {
            buttons.push((Hit::Chevron, r, if rolled { 0xE70D } else { 0xE70E }));
        }
        if let Some((_, rect, _)) = buttons.iter().find(|(h, ..)| hot == Some(*h)) {
            let rect = *rect;
            c.with_graphics(|g| flyout::fill_round_argb(g, &rect, px(hwnd, 5) as f32, crate::glass::argb(ink, 0x30)));
        }
        for (_, rect, glyph) in buttons {
            c.text(rect, &String::from_utf16_lossy(&[glyph]), flyout::icon_font(-px(hwnd, 15)), ink, center);
        }
    }
    if let Some(rect) = layout.lock {
        if hot == Some(Hit::Lock) {
            c.with_graphics(|g| flyout::fill_round_argb(g, &rect, px(hwnd, 5) as f32, crate::glass::argb(ink, 0x30)));
        }
        c.text(rect, &String::from_utf16_lossy(&[LOCK_GLYPH]), flyout::icon_font(-px(hwnd, 15)), ink, center);
    }
    if let Some(r) = layout.warn {
        c.text(r, &String::from_utf16_lossy(&[0xE7BA]), flyout::icon_font(-px(hwnd, 16)), ink, center);
    }
    if !renaming {
        // Con DirectWrite (emojis en color); si no, texto GDI con su alfa.
        // El título, semitransparente como el resto de la nota.
        let ink_alpha = crate::glass::title_alpha();
        if !crate::d2d::draw_title(c.dc, layout.title_rect, title, (ink, ink_alpha), crate::d2d::Bg::Glass(header_color, alpha), px(hwnd, 15)) {
            let format = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_LEFT | DT_NOPREFIX;
            c.text_alpha(layout.title_rect, title, header_font(hwnd), ink, ink_alpha, format);
        }
    }
    c.blit(hdc, 0, 0);
}

/// Modo vidrio y widget de escritorio (no siempre encima, no asomada):
/// su lugar es justo encima del escritorio, debajo de las aplicaciones.
fn glass_widget(hwnd: HWND) -> bool {
    if !crate::glass::active() {
        return false;
    }
    let id = note_id(hwnd);
    app().lock().unwrap().notes.get(&id).is_some_and(|nr| nr.data.layer != Layer::AlwaysOnTop && !nr.peeking)
}

/// Los widgets en modo vidrio (menos `except`), de arriba hacia abajo.
fn widgets_top_down(except: HWND) -> Vec<HWND> {
    let set: Vec<HWND> = {
        let a = app().lock().unwrap();
        a.notes
            .values()
            .filter(|nr| nr.hwnd != 0 && nr.hwnd != except as isize && nr.data.layer != Layer::AlwaysOnTop && !nr.peeking)
            .map(|nr| nr.hwnd as HWND)
            .collect()
    };
    let mut out = Vec::new();
    unsafe {
        let mut w = GetTopWindow(null_mut());
        while !w.is_null() && out.len() < set.len() {
            if set.contains(&w) {
                out.push(w);
            }
            w = GetWindow(w, GW_HWNDNEXT);
        }
    }
    out
}

/// La sombra que Windows le pone abajo a cada ventana emergente es una
/// ventana aparte ("SysShadow"): no cuenta para ubicar las notas.
fn is_shadow(w: HWND) -> bool {
    let mut buf = [0u16; 16];
    let n = unsafe { GetClassNameW(w, buf.as_mut_ptr(), buf.len() as i32) } as usize;
    String::from_utf16_lossy(&buf[..n]) == "SysShadow"
}

/// La ventana justo arriba de `w` en el orden, sin contar sombras ni
/// `skip`.
unsafe fn window_above(w: HWND, skip: HWND) -> HWND {
    let mut a = GetWindow(w, GW_HWNDPREV);
    while !a.is_null() && (a == skip || is_shadow(a)) {
        a = GetWindow(a, GW_HWNDPREV);
    }
    a
}

/// Sobre qué va un widget en modo vidrio: el más alto de los otros
/// widgets, o el escritorio mismo si no hay otro.
fn widget_base(hwnd: HWND) -> HWND {
    widgets_top_down(hwnd).first().copied().unwrap_or_else(crate::desktop::find_host)
}

/// Dónde va un widget en modo vidrio cuando Windows lo mueve en el orden
/// de las ventanas (al tocarlo, al activarse): justo encima de los otros
/// widgets, que están justo encima del escritorio; nunca encima de una
/// aplicación. `None`: no hay escritorio donde apoyarlo.
fn widget_slot(hwnd: HWND) -> Option<HWND> {
    let base = widget_base(hwnd);
    if base.is_null() {
        return None;
    }
    let after = unsafe { window_above(base, hwnd) };
    Some(if after.is_null() { HWND_TOP } else { after })
}

/// Pone `hwnd` justo encima de `base` (otro widget, o el escritorio):
/// "insertar después de" la ventana que hoy está arriba de `base`. Nada
/// de HWND_BOTTOM: con ventanas "poseídas" por el escritorio, Windows no
/// las manda al fondo (quedaban adelante de las aplicaciones).
unsafe fn place_above(hwnd: HWND, base: HWND) {
    if base.is_null() {
        return;
    }
    let after = window_above(base, hwnd);
    RESTACKING.set(true);
    SetWindowPos(hwnd, if after.is_null() { HWND_TOP } else { after }, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    RESTACKING.set(false);
}

thread_local! {
    /// Acomodando los widgets a propósito (`settle_widgets`,
    /// `raise_above_notes`): el ajuste
    /// de WM_WINDOWPOSCHANGING no se mete.
    static RESTACKING: Cell<bool> = const { Cell::new(false) };
}

/// Todas las notas del escritorio (modo vidrio) juntas, justo encima de
/// él y en el orden que tenían —con `first` arriba de todas—, así ninguna
/// queda adelante de una aplicación. De arriba hacia abajo, cada una se
/// apoya directo sobre el escritorio, debajo de las anteriores.
///
/// Hace falta después de que se active una: Windows sube juntas todas las
/// ventanas "poseídas" por el escritorio (las notas, y hasta una ventana
/// suya del IME), y quedaban todas adelante de las aplicaciones.
fn settle_widgets_with(first: HWND) {
    if !crate::glass::active() {
        return;
    }
    let host = crate::desktop::find_host();
    let first = (!first.is_null() && glass_widget(first)).then_some(first);
    for h in first.into_iter().chain(widgets_top_down(first.unwrap_or(null_mut()))) {
        unsafe { place_above(h, host) };
    }
}

pub fn settle_widgets() {
    settle_widgets_with(null_mut());
}

/// Acomodar las notas del escritorio en cuanto se pueda (no en medio de
/// una activación): con `hwnd` arriba de las otras.
const WM_APP_SETTLE: u32 = WM_APP + 40;

pub fn settle_soon(hwnd: HWND) {
    unsafe { PostMessageW(hwnd, WM_APP_SETTLE, 0, 0) };
}

/// Al arrancar: acomodar las notas del escritorio en cuanto arranque el
/// bucle de mensajes (y otra vez un rato después, por si Windows activa
/// una más tarde).
pub fn settle_later() {
    let any = app().lock().unwrap().notes.values().find(|nr| nr.hwnd != 0).map(|nr| nr.hwnd as HWND);
    if let Some(h) = any {
        settle_soon(h);
        unsafe { SetTimer(h, TIMER_SETTLE, 1500, None) };
    }
}

/// Cambió el nivel de transparencia: se repintan (la opacidad de cada
/// parte, y el texto).
pub fn refresh_glass() {
    let list: Vec<(HWND, HWND)> =
        app().lock().unwrap().notes.values().filter(|nr| nr.hwnd != 0).map(|nr| (nr.hwnd as HWND, nr.edit as HWND)).collect();
    let body = crate::glass::opacity().map(|(_, b)| b);
    for (h, edit) in list {
        if !edit.is_null() {
            editor::set_glass(edit, body);
        }
        unsafe { RedrawWindow(h, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW) };
    }
}

/// Se prendió o apagó el modo vidrio (o apareció o se fue el mod): las
/// ventanas de las notas son de otra clase (poseídas y con otro estilo, o
/// hijas del escritorio), y el mod solo las mira al crearlas: se rehacen.
/// Los datos no se tocan.
pub fn recreate_all() {
    flush_all();
    let list: Vec<HWND> = app().lock().unwrap().notes.values().map(|nr| nr.hwnd as HWND).filter(|h| !h.is_null()).collect();
    for h in list {
        unsafe { DestroyWindow(h) };
    }
    recreate_lost();
    settle_widgets();
}

/// Fin de un arrastre o de un resize (WM_EXITSIZEMOVE cubre ambos).
/// El alto solo se guarda si no está enrollada: `data.h` siempre
/// representa el alto "desenrollado", y mientras está enrollada la
/// ventana mide el alto del encabezado, que no es lo que hay que
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
            nr.data.w = (rc.right - rc.left).max(px(hwnd, 160));
            if !nr.data.rolled {
                nr.data.h = (rc.bottom - rc.top).max(header_h(hwnd) + px(hwnd, 80));
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
        KillTimer(hwnd, TIMER_HOT);
    }
    crate::rename::abort_for(hwnd);
    forget_header_state(hwnd);

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
            if let Some((t, f)) = text {
                nr.data.text = t;
                nr.data.fmt = f;
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
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    if layout_of(hwnd).hit(x, y) == Hit::Bar && !is_locked(hwnd) {
        crate::rename::begin(hwnd);
    }
}

/// Franja de los bordes que cambia el tamaño (a 100 %), y cuánto de
/// cada lado ocupa una esquina: las esquinas son redondeadas y chicas,
/// así que se agarran también desde un poco más allá.
const RESIZE_MARGIN: i32 = 8;
const RESIZE_CORNER: i32 = 18;

/// Qué borde o esquina de la nota hay en (`x`, `y`) (pantalla), o 0. Lo
/// usa también el texto (`editor.rs`), que llega hasta los costados y
/// deja pasar ahí el mouse a la nota. Nada mientras está enrollada (el
/// alto queda fijo al del encabezado).
pub fn resize_hit(hwnd: HWND, x: i32, y: i32) -> u32 {
    let id = note_id(hwnd);
    if app().lock().unwrap().notes.get(&id).is_none_or(|nr| nr.data.rolled || nr.data.locked) {
        return 0;
    }
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(hwnd, &mut rc) };
    let (m, c) = (px(hwnd, RESIZE_MARGIN), px(hwnd, RESIZE_CORNER));
    let near = |v: i32, edge: i32, reach: i32, inward: bool| if inward { v >= edge && v < edge + reach } else { v < edge && v >= edge - reach };
    let (left, right) = (near(x, rc.left, m, true), near(x, rc.right, m, false));
    let (top, bottom) = (near(y, rc.top, m, true), near(y, rc.bottom, m, false));
    let (left_c, right_c) = (near(x, rc.left, c, true), near(x, rc.right, c, false));
    let (top_c, bottom_c) = (near(y, rc.top, c, true), near(y, rc.bottom, c, false));
    if (top && left_c) || (left && top_c) {
        HTTOPLEFT
    } else if (top && right_c) || (right && top_c) {
        HTTOPRIGHT
    } else if (bottom && left_c) || (left && bottom_c) {
        HTBOTTOMLEFT
    } else if (bottom && right_c) || (right && bottom_c) || in_grip(hwnd, x, y) {
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
    }
}

/// Windows redimensiona la ventana como si tuviera un borde grueso (o lo
/// hace `begin_sizing`), con solo decirle en qué borde o esquina cae el
/// cursor.
fn on_nchittest(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    match resize_hit(hwnd, x, y) {
        0 => unsafe { DefWindowProcW(hwnd, WM_NCHITTEST, 0, lparam) },
        hit => hit as LRESULT,
    }
}

/// La agarradera de la esquina de abajo a la derecha (en coordenadas de
/// la nota): tres puntitos en diagonal, a la vista con el mouse encima o
/// el foco en la nota, como los botones del encabezado.
fn grip_rect(hwnd: HWND) -> RECT {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    let s = px(hwnd, 14);
    RECT { left: rc.right - s, top: rc.bottom - s, right: rc.right, bottom: rc.bottom }
}

fn in_grip(hwnd: HWND, x: i32, y: i32) -> bool {
    let mut p = POINT { x, y };
    unsafe { ScreenToClient(hwnd, &mut p) };
    let g = grip_rect(hwnd);
    p.x >= g.left && p.x < g.right && p.y >= g.top && p.y < g.bottom
}

/// Dibuja la agarradera (sobre el cuerpo ya pintado). `glass`: la
/// opacidad del cuerpo en modo vidrio.
unsafe fn paint_grip(hwnd: HWND, hdc: HDC, body: u32, ink: u32, glass: Option<u8>) {
    let g = grip_rect(hwnd);
    let (w, h) = (g.right - g.left, g.bottom - g.top);
    let (d, r) = (px(hwnd, 4) as f32, px(hwnd, 1).max(1) as f32 * 1.1);
    // Tres puntitos de la esquina hacia adentro, lejos del redondeo.
    let corner = (w as f32 - px(hwnd, 6) as f32, h as f32 - px(hwnd, 6) as f32);
    let dots = [(0.0, 0.0), (-d, 0.0), (0.0, -d), (-2.0 * d, 0.0), (-d, -d), (0.0, -2.0 * d)];
    let draw = |gr: *mut GpGraphics, color: u32| {
        let mut brush: *mut GpSolidFill = null_mut();
        GdipCreateSolidFill(color, &mut brush);
        for (dx, dy) in dots {
            GdipFillEllipse(gr, brush as *mut GpBrush, corner.0 + dx - r, corner.1 + dy - r, r * 2.0, r * 2.0);
        }
        GdipDeleteBrush(brush as *mut GpBrush);
    };
    match glass {
        Some(alpha) => {
            if let Some(c) = crate::glass::Canvas::new(w, h) {
                c.fill(RECT { left: 0, top: 0, right: w, bottom: h }, body, alpha);
                c.with_graphics(|gr| draw(gr, crate::glass::argb(ink, 0x99)));
                c.blit(hdc, g.left, g.top);
            }
        }
        None => {
            let mut gr: *mut GpGraphics = null_mut();
            if GdipCreateFromHDC(hdc, &mut gr) == Ok && !gr.is_null() {
                GdipSetSmoothingMode(gr, SmoothingModeAntiAlias);
                GdipTranslateWorldTransform(gr, g.left as f32, g.top as f32, MatrixOrderPrepend);
                draw(gr, (colorref_to_argb(ink) & 0x00ff_ffff) | 0x9900_0000);
                GdipDeleteGraphics(gr);
            }
        }
    }
}

// -----------------------------------------------------------------
// Cambio de tamaño a mano
// -----------------------------------------------------------------

/// Windows solo cambia el tamaño desde el borde (lo que dice
/// `on_nchittest`) a las ventanas hijas y a las que tienen marco grueso
/// (`WS_THICKFRAME`), que se vería. Una nota de primer nivel (siempre
/// encima, asomada, o cualquiera en modo vidrio) lo hace acá: se toma el
/// mouse y se acompaña el borde hasta soltar.
struct Sizing {
    hwnd: HWND,
    edge: u32,
    start: POINT,
    rect: RECT,
}

thread_local! {
    static SIZING: RefCell<Option<Sizing>> = const { RefCell::new(None) };
}

/// Empieza a cambiar el tamaño desde `edge` (un `HTLEFT`…`HTBOTTOMRIGHT`).
/// `false` si es una nota anclada: eso lo hace Windows.
fn begin_sizing(hwnd: HWND, edge: u32) -> bool {
    unsafe {
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD != 0 {
            return false;
        }
        let mut start = POINT { x: 0, y: 0 };
        GetCursorPos(&mut start);
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(hwnd, &mut rect);
        SIZING.with(|s| *s.borrow_mut() = Some(Sizing { hwnd, edge, start, rect }));
        SetCapture(hwnd);
        SendMessageW(hwnd, WM_ENTERSIZEMOVE, 0, 0);
    }
    true
}

fn sizing_cursor(edge: u32) -> *const u16 {
    match edge {
        HTLEFT | HTRIGHT => IDC_SIZEWE,
        HTTOP | HTBOTTOM => IDC_SIZENS,
        HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE,
        _ => IDC_SIZENESW,
    }
}

/// El mouse se movió: si se está cambiando el tamaño, el borde lo sigue
/// (sin achicarse de la medida mínima). `true` si era eso.
fn on_sizing_move(hwnd: HWND) -> bool {
    let Some((edge, start, r)) = SIZING.with(|s| s.borrow().as_ref().filter(|z| z.hwnd == hwnd).map(|z| (z.edge, z.start, z.rect))) else {
        return false;
    };
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    let (dx, dy) = (p.x - start.x, p.y - start.y);
    let (min_w, min_h) = (px(hwnd, 160), header_h(hwnd) + px(hwnd, 80));
    let (mut left, mut top, mut right, mut bottom) = (r.left, r.top, r.right, r.bottom);
    if matches!(edge, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
        left = (r.left + dx).min(r.right - min_w);
    }
    if matches!(edge, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT) {
        right = (r.right + dx).max(r.left + min_w);
    }
    if matches!(edge, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
        top = (r.top + dy).min(r.bottom - min_h);
    }
    if matches!(edge, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT) {
        bottom = (r.bottom + dy).max(r.top + min_h);
    }
    unsafe {
        SetCursor(LoadCursorW(null_mut(), sizing_cursor(edge)));
        SetWindowPos(hwnd, null_mut(), left, top, right - left, bottom - top, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    true
}

/// Se soltó el botón (o se perdió el mouse): termina y se guarda.
fn end_sizing(hwnd: HWND) -> bool {
    let was = SIZING.with(|s| {
        let mut s = s.borrow_mut();
        let mine = s.as_ref().is_some_and(|z| z.hwnd == hwnd);
        if mine {
            *s = None;
        }
        mine
    });
    if was {
        unsafe {
            if GetCapture() == hwnd {
                ReleaseCapture();
            }
            SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
        }
    }
    was
}

/// Tamaño mínimo al redimensionar: que siempre quede lugar para el
/// encabezado completo y algo de cuerpo debajo.
fn on_getminmaxinfo(hwnd: HWND, lparam: LPARAM) {
    let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
    info.ptMinTrackSize.x = px(hwnd, 160);
    info.ptMinTrackSize.y = header_h(hwnd) + px(hwnd, 80);
}

fn on_lbuttondown(hwnd: HWND, lparam: LPARAM) {
    let x = (lparam & 0xffff) as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as i16 as i32;
    let layout = layout_of(hwnd);
    match layout.hit(x, y) {
        Hit::Add => new_note_beside(hwnd),
        Hit::Chevron => toggle_roll_manual(hwnd),
        Hit::More => {
            let mut r = layout.more;
            unsafe {
                ClientToScreen(hwnd, &mut r as *mut RECT as *mut POINT);
                ClientToScreen(hwnd, (&mut r as *mut RECT as *mut POINT).add(1));
            }
            show_note_menu(hwnd, r);
        }
        Hit::Pin => toggle_always_on_top(hwnd),
        Hit::Lock => set_locked(hwnd, false),
        Hit::Bar => on_bar_click(hwnd, x, y),
        Hit::None => {
            let id = note_id(hwnd);
            let edit = app().lock().unwrap().notes.get(&id).map(|nr| nr.edit as HWND);
            if let Some(edit) = edit.filter(|e| !e.is_null()) {
                unsafe { SetFocus(edit) };
            }
        }
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
        Hide,
        Style(usize),
        Bullets,
        Todos,
        /// Atajos propios del RichEdit que cambian cosas que la nota no
        /// guarda (alineación, interlineado, tamaño de letra…): se
        /// tragan, así lo que se ve es siempre lo que queda guardado.
        Swallow,
    }
    let alt = down(VK_MENU);
    let in_text = !note.is_null() && is_body(note, target);
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
            (b'W', false) => Some(Action::Hide),
            _ if !in_text || alt => None,
            (b'B', false) => Some(Action::Style(crate::richtext::BOLD)),
            (b'I', false) => Some(Action::Style(crate::richtext::ITALIC)),
            (b'U', false) => Some(Action::Style(crate::richtext::UNDERLINE)),
            // Como en las Sticky Notes de Microsoft.
            (b'T', false) => Some(Action::Style(crate::richtext::STRIKE)),
            (b'L', true) => Some(Action::Bullets),
            (b'C', true) => Some(Action::Todos),
            (b'E' | b'J' | b'L' | b'1' | b'2' | b'5', _) => Some(Action::Swallow),
            _ if [VK_OEM_PLUS, VK_OEM_PERIOD, VK_OEM_COMMA].contains(&(vk as u16)) => Some(Action::Swallow),
            _ => None,
        }
    };
    let Some(action) = action else { return false };
    // Bloqueada: nada que la cambie (sí una nota nueva). La tecla se
    // traga igual, para que tampoco la tome el texto.
    if !note.is_null() && is_locked(note) && !matches!(action, Action::NewNote) {
        return true;
    }
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
        Action::Hide => hide_note(note),
        Action::Style(k) => {
            editor::toggle_style(target, k);
            crate::seltool::refresh(target);
        }
        Action::Bullets => editor::toggle_bullets(target),
        Action::Todos => editor::toggle_todos(target),
        Action::Swallow => {}
    }
    true
}

/// `true` si `target` es el cuadro de texto de la nota `note` (y no, por
/// ejemplo, el de renombrar).
fn is_body(note: HWND, target: HWND) -> bool {
    let id = note_id(note);
    app().lock().unwrap().notes.get(&id).is_some_and(|nr| nr.edit == target as isize)
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

/// El resto de la barra: arrastrar mueve la nota (un clic suelto no
/// hace nada — antes la enrollaba, y un clic para traerla al frente o
/// el primero de un doble clic la enrollaban sin querer). Para enrollar
/// está el botón, y Ctrl+R.
///
/// `DragDetect` distingue el clic del arrastre sin comerse el doble clic
/// (que es para renombrar).
fn on_bar_click(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let mut pt = POINT { x, y };
        ClientToScreen(hwnd, &mut pt);
        if DragDetect(hwnd, pt) != 0 {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
        } else {
            // Un clic en la barra de una nota en la que se estaba
            // escribiendo: el foco vuelve al texto.
            let id = note_id(hwnd);
            let edit = app().lock().unwrap().notes.get(&id).map(|nr| nr.edit as HWND);
            if let Some(edit) = edit.filter(|e| !e.is_null()) {
                SetFocus(edit);
            }
        }
    }
}

fn on_command(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let low = (wparam & 0xffff) as u32;
    let high = ((wparam >> 16) & 0xffff) as u32;
    if high == 0 {
        match low {
            ID_TOGGLE_TOP => toggle_always_on_top(hwnd),
            ID_TOGGLE_AUTOROLL => {
                let id = note_id(hwnd);
                let auto = app().lock().unwrap().notes.get(&id).is_some_and(|nr| nr.data.roll_mode == RollMode::Auto);
                set_roll_mode(hwnd, if auto { RollMode::Manual } else { RollMode::Auto });
                unsafe { InvalidateRect(hwnd, null(), 0) };
            }
            ID_DELETE_NOTE => confirm_delete(hwnd),
            ID_DUPLICATE_NOTE => duplicate_note(hwnd),
            ID_RENAME => crate::rename::begin(hwnd),
            ID_ALL_NOTES => crate::allnotes::show(),
            ID_HIDE => hide_note(hwnd),
            ID_LOCK => set_locked(hwnd, true),
            id if (ID_COLOR_BASE..ID_COLOR_BASE + crate::theme::PALETTE_LEN as u32).contains(&id) => {
                set_color(hwnd, (id - ID_COLOR_BASE) as u8)
            }
            id if (editor::ID_FIRST..=editor::ID_LAST).contains(&id) => {
                let id_note = note_id(hwnd);
                let edit = app().lock().unwrap().notes.get(&id_note).map(|nr| nr.edit as HWND);
                if let Some(edit) = edit.filter(|e| !e.is_null()) {
                    editor::command(edit, id);
                }
            }
            _ => {}
        }
    } else if low as usize == RICHEDIT_ID && high == EN_CHANGE {
        unsafe { SetTimer(hwnd, TIMER_AUTOSAVE, AUTOSAVE_DEBOUNCE_MS, None) };
        editor::schedule_refresh(lparam as HWND);
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
            let layer_top = {
                let id = note_id(hwnd);
                app().lock().unwrap().notes.get(&id).is_some_and(|nr| nr.data.layer == Layer::AlwaysOnTop)
            };
            if crate::glass::active() {
                if !layer_top && !is_peeking(hwnd) && !crate::desktop::is_owned(hwnd) {
                    crate::desktop::own(hwnd);
                }
            } else if !is_peeking(hwnd) && !crate::desktop::is_anchored(hwnd) {
                crate::desktop::anchor(hwnd);
            }
            // ¿Se prendió o se apagó el mod de Windhawk?
            crate::tray::check_glass_soon();
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
            let ours = fg == hwnd || unsafe { GetWindow(fg, GW_OWNER) } == hwnd || unsafe { IsChild(hwnd, fg) } != 0 || flyout::is_open();
            if !ours {
                anchor_widget(hwnd);
            }
        }
        TIMER_AUTOSAVE => {
            unsafe { KillTimer(hwnd, TIMER_AUTOSAVE) };
            commit_text_and_save(hwnd);
        }
        TIMER_HOT => hot_tick(hwnd),
        TIMER_SETTLE => {
            unsafe { KillTimer(hwnd, TIMER_SETTLE) };
            settle_widgets_with(hwnd);
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
        WM_ERASEBKGND => {
            on_erase(hwnd, wparam as HDC);
            1
        }
        WM_SIZE => {
            update_shape(hwnd);
            on_size(hwnd, lparam);
            0
        }
        // Se ancló al escritorio o se soltó (SetParent + WS_CHILD).
        WM_STYLECHANGED => {
            if wparam as i32 == GWL_STYLE {
                update_shape(hwnd);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        // Modo vidrio, mientras se crea: el borde de `WS_POPUPWINDOW` no
        // se dibuja, todo es área de la nota (después se le saca).
        WM_NCCALCSIZE if wparam != 0 && GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_BORDER != 0 => 0,
        // Modo vidrio: un widget se queda sobre el escritorio aunque lo
        // activen (si no, un clic lo pasaba adelante de las aplicaciones).
        WM_WINDOWPOSCHANGING => {
            let wp = &mut *(lparam as *mut WINDOWPOS);
            if wp.flags & SWP_NOZORDER == 0 && !RESTACKING.get() && glass_widget(hwnd) {
                match widget_slot(hwnd) {
                    Some(after) => wp.hwndInsertAfter = after,
                    None => wp.flags |= SWP_NOZORDER,
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
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
            on_getminmaxinfo(hwnd, lparam);
            0
        }
        // El menú de la nota se abre solo con "⋯" (antes también con
        // clic derecho en la barra: dos caminos para lo mismo).
        WM_RBUTTONUP => 0,
        WM_NCLBUTTONDOWN if (HTLEFT..=HTBOTTOMRIGHT).contains(&(wparam as u32)) && begin_sizing(hwnd, wparam as u32) => 0,
        WM_MOUSEMOVE if on_sizing_move(hwnd) => 0,
        WM_LBUTTONUP if end_sizing(hwnd) => 0,
        WM_CAPTURECHANGED => {
            end_sizing(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            on_hover(hwnd);
            let x = (lparam & 0xffff) as i16 as i32;
            let y = ((lparam >> 16) & 0xffff) as i16 as i32;
            set_hot_button(hwnd, layout_of(hwnd).hit(x, y));
            0
        }
        WM_COMMAND => {
            on_command(hwnd, wparam, lparam);
            0
        }
        WM_TIMER => {
            on_timer(hwnd, wparam);
            0
        }
        WM_DPICHANGED => {
            let r = &*(lparam as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            InvalidateRect(hwnd, null(), 1);
            0
        }
        WM_EXITSIZEMOVE => {
            on_move_end(hwnd);
            0
        }
        WM_MOVE => {
            let edit = app().lock().unwrap().notes.get(&note_id(hwnd)).map(|nr| nr.edit as HWND);
            if let Some(edit) = edit {
                crate::seltool::hide_for(edit);
            }
            0
        }
        WM_APP_SETTLE => {
            settle_widgets_with(hwnd);
            0
        }
        // Clic en cualquier parte de una nota anclada (también en su
        // texto): pasa adelante de las otras notas. Las ventanas hijas
        // no cambian de orden solas.
        WM_MOUSEACTIVATE => {
            crate::desktop::raise(hwnd);
            // Modo vidrio: al hacer clic, Windows sube la nota —y con ella
            // todas las poseídas por el escritorio— apenas vuelve de acá.
            // El mensaje en cola se atiende después: ahí se acomodan.
            if glass_widget(hwnd) {
                settle_soon(hwnd);
            }
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
                let id = note_id(hwnd);
                if let Some(nr) = app().lock().unwrap().notes.get_mut(&id) {
                    nr.last_active = crate::persist::now_ms();
                }
                if glass_widget(hwnd) {
                    // Windows sube el grupo recién después de activarla (y
                    // sin avisarle a las ventanas): se acomoda un momento
                    // más tarde.
                    SetTimer(hwnd, TIMER_SETTLE, 60, None);
                }
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
        // Alt+F4, o "Cerrar ventana" en la barra de tareas (siempre
        // encima): se guarda en "Todas las notas", no se borra.
        WM_CLOSE => {
            if !is_locked(hwnd) {
                hide_note(hwnd);
            }
            0
        }
        WM_DESTROY => {
            on_destroy(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
