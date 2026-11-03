//! Simpcky dibujado con Direct2D propio: un solo dispositivo de GPU para
//! todas las notas (D3D11 → Direct2D → DirectComposition), texto con
//! DirectWrite, celdas y pilas de `common::snap`, y vidrio nativo de Windows.
//! Ver `demos/SPEC.md`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod editor;
mod gfx;
mod note;
mod overlay;

use std::cell::RefCell;
use std::time::{Duration, Instant};

use common::pump;
use common::snap::{Host, Info, Snap};
use common::store::{self, NoteData, Settings};
use common::win::{self, wide};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use gfx::Gfx;
use note::{Button, Fonts, NoteWin, BAR};
use overlay::Overlay;

const NOTE_CLASS: &str = "SimpckyD2DNota";
const MSG_CLASS: &str = "SimpckyD2DMensajes";
const SAVE_TIMER: usize = 1;
const SAVE_MS: u32 = 800;

// -----------------------------------------------------------------
// Las notas (lo que ve el arrastre)
// -----------------------------------------------------------------

pub struct Notes {
    g: Gfx,
    fonts: Fonts,
    list: Vec<NoteWin>,
    settings: Settings,
    next_id: u32,
    msg_hwnd: HWND,
}

impl Notes {
    fn get(&self, id: u32) -> Option<&NoteWin> {
        self.list.iter().find(|n| n.data.id == id)
    }
    fn get_mut(&mut self, id: u32) -> Option<&mut NoteWin> {
        self.list.iter_mut().find(|n| n.data.id == id)
    }
    fn by_hwnd(&self, hwnd: HWND) -> Option<u32> {
        self.list.iter().find(|n| n.hwnd == hwnd).map(|n| n.data.id)
    }

    fn paint(&mut self, id: u32) {
        let (dark, glass) = (self.settings.dark, self.settings.glass);
        let Notes { g, fonts, list, .. } = self;
        if let Some(n) = list.iter_mut().find(|n| n.data.id == id) {
            n.paint(g, fonts, dark, glass);
        }
    }

    fn paint_all(&mut self) {
        let ids: Vec<u32> = self.list.iter().map(|n| n.data.id).collect();
        for id in ids {
            self.paint(id);
        }
    }

    fn schedule_save(&self) {
        unsafe { SetTimer(self.msg_hwnd, SAVE_TIMER, SAVE_MS, None) };
    }

    fn save_now(&mut self) {
        for n in &mut self.list {
            n.data.text = n.ed.string();
        }
        let data: Vec<NoteData> = self.list.iter().map(|n| n.data.clone()).collect();
        store::save_notes(&data);
    }

    fn create(&mut self, data: NoteData) -> u32 {
        let id = data.id;
        let rolled_h = if data.rolled { BAR } else { data.h };
        let hwnd = unsafe {
            let class = wide(NOTE_CLASS);
            CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW, class.as_ptr(), wide(&data.display_title()).as_ptr(), WS_POPUP, data.x, data.y, data.w, rolled_h, std::ptr::null_mut(), std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null())
        };
        let layer = data.layer;
        let note = NoteWin::new(&self.g, hwnd, data);
        self.g.commit();
        win::round_corners(hwnd, true);
        win::set_layer(hwnd, layer != 2);
        win::set_blur(hwnd, self.settings.glass > 0);
        self.list.push(note);
        self.paint(id);
        if !self.get(id).is_some_and(|n| n.data.hidden) {
            unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        }
        id
    }

    fn new_note(&mut self, near: Option<u32>) -> u32 {
        let (x, y, color) = near
            .and_then(|k| self.get(k))
            .map(|n| (n.data.x + 32, n.data.y + 32, n.data.color))
            .unwrap_or_else(|| {
                let p = win::cursor();
                (p.x - 140, p.y - 20, 0)
            });
        let id = self.next_id;
        self.next_id += 1;
        let id = self.create(NoteData::new(id, x, y, color));
        if let Some(n) = self.get(id) {
            unsafe { SetForegroundWindow(n.hwnd) };
        }
        self.schedule_save();
        id
    }

    /// Cambia el tamaño de la superficie si hace falta (y repinta).
    fn fit_surface(&mut self, id: u32, w: i32, h: i32) {
        let Notes { g, fonts, list, settings, .. } = self;
        let Some(n) = list.iter_mut().find(|n| n.data.id == id) else { return };
        let (sw, sh) = n.surf.size;
        // La superficie es la de la nota abierta; si la ventana entra, no se repinta.
        let want_h = n.data.h.max(h).max(BAR) as u32;
        if sw as i32 != w || sh < h as u32 || (sh != want_h && n.data.h >= h) {
            let _ = n.surf.resize(g, w as u32, want_h);
            n.ed.layout = None;
            n.paint(g, fonts, settings.dark, settings.glass);
        }
    }
}

impl Host for Notes {
    fn notes_on(&self, monitor: HMONITOR) -> Vec<Info> {
        self.list
            .iter()
            .filter(|n| !n.data.hidden && n.data.layer != 2 && unsafe { IsWindowVisible(n.hwnd) } != 0)
            .map(|n| Info { id: n.data.id, x: n.data.x, y: n.data.y, w: n.data.w, h: n.data.h, rolled: n.data.rolled, bar: BAR })
            .filter(|n| win::monitor_of(n.x + n.w / 2, n.y + n.bar / 2).0 == monitor)
            .collect()
    }

    fn window_rect(&self, id: u32) -> RECT {
        self.get(id).map(|n| win::window_rect(n.hwnd)).unwrap_or(win::rect(0, 0, 0, 0))
    }

    fn place(&mut self, id: u32, pos: Option<(i32, i32)>, size: Option<(i32, i32)>) {
        let Some(hwnd) = self.get(id).map(|n| n.hwnd) else { return };
        if pos.is_none() && size.is_none() {
            return;
        }
        if let Some((w, h)) = size {
            self.fit_surface(id, w, h);
        }
        let (x, y) = pos.unwrap_or((0, 0));
        let (w, h) = size.unwrap_or((0, 0));
        let mut flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW;
        if pos.is_none() {
            flags |= SWP_NOMOVE;
        }
        if size.is_none() {
            flags |= SWP_NOSIZE;
        }
        unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, flags) };
        common::framelog::mark("paso", id, Duration::ZERO);
    }

    fn state(&self, id: u32) -> (i32, i32, bool, bool, bool) {
        self.get(id).map(|n| (n.data.w, n.data.h, n.data.rolled, n.data.roll_mode == 1, n.data.locked)).unwrap_or((280, 320, false, false, false))
    }

    fn set_shape(&mut self, id: u32, size: Option<(i32, i32)>, rolled: bool) {
        if let Some(n) = self.get_mut(id) {
            if let Some((w, h)) = size {
                (n.data.w, n.data.h) = (w, h);
            }
            n.data.rolled = rolled;
            n.data.t[3] = store::now_ms();
        }
        self.schedule_save();
    }

    fn set_auto(&mut self, id: u32, auto: bool) {
        if let Some(n) = self.get_mut(id) {
            n.data.roll_mode = auto as u8;
        }
    }

    fn store(&mut self, id: u32, x: i32, y: i32, size: Option<(i32, i32)>) {
        if let Some(n) = self.get_mut(id) {
            (n.data.x, n.data.y) = (x, y);
            if let Some((w, h)) = size {
                (n.data.w, n.data.h) = (w, h);
            }
            n.data.t[2] = store::now_ms();
        }
    }

    fn raise(&mut self, id: u32) {
        if let Some(n) = self.get(id) {
            unsafe { SetWindowPos(n.hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        }
    }

    fn trash(&mut self, id: u32) {
        if let Some(i) = self.list.iter().position(|n| n.data.id == id) {
            let n = self.list.remove(i);
            unsafe { DestroyWindow(n.hwnd) };
        }
        self.schedule_save();
    }

    fn save(&mut self) {
        self.schedule_save();
    }

    fn bar(&self) -> i32 {
        BAR
    }

    fn dark(&self) -> bool {
        self.settings.dark
    }
}

// -----------------------------------------------------------------
// La aplicación
// -----------------------------------------------------------------

struct App {
    notes: Notes,
    snap: Snap,
    overlay: Option<Overlay>,
    frames: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Acceso al estado. Si ya está tomado (un mensaje que llega en medio de
/// otro, p. ej. `SetWindowPos` → `WM_SIZE`), `None`.
fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

impl App {
    fn wake(&mut self) {
        if !self.frames {
            self.frames = true;
            pump::run_frames(true);
        }
    }

    /// Un cuadro: el arrastre, las transiciones, la capa y el modo Auto.
    fn frame(&mut self) {
        pump::frame_taken();
        let was = self.snap.busy();
        let moving = self.snap.frame(&mut self.notes);
        // Esc o el botón soltado fuera de la ventana terminaron el arrastre acá:
        // soltar también el mouse.
        if was && !self.snap.busy() {
            unsafe { ReleaseCapture() };
        }
        let l = self.snap.layers();
        if l.visible() {
            let work = l.work;
            let same = self.overlay.as_ref().map(|o| o.work()).is_some_and(|w| (w.left, w.top, w.right, w.bottom) == (work.left, work.top, work.right, work.bottom));
            if !same {
                self.overlay = Some(Overlay::new(&self.notes.g, work));
            }
            let l = self.snap.layers().clone();
            if let Some(o) = self.overlay.as_mut() {
                o.update(&self.notes.g, &self.notes.fonts, &l);
            }
        } else if let Some(o) = self.overlay.as_mut() {
            o.hide();
        }
        // Auto: enrollar las que el mouse dejó.
        let now = Instant::now();
        let due: Vec<u32> = self.notes.list.iter().filter(|n| n.roll_at.is_some_and(|t| t <= now)).map(|n| n.data.id).collect();
        for id in &due {
            if let Some(n) = self.notes.get_mut(*id) {
                n.roll_at = None;
            }
            self.snap.reshape(&mut self.notes, *id, None, true);
        }
        let waiting = self.notes.list.iter().any(|n| n.roll_at.is_some());
        if !moving && !waiting && due.is_empty() && !self.snap.motion.busy() {
            self.frames = false;
            pump::run_frames(false);
        }
    }

    fn toggle_roll(&mut self, id: u32) {
        let rolled = self.notes.state(id).2;
        self.snap.reshape(&mut self.notes, id, None, !rolled);
        self.wake();
    }

    fn set_glass(&mut self, level: u8) {
        self.notes.settings.glass = level % 4;
        store::save_settings(&self.notes.settings);
        for n in &self.notes.list {
            win::set_blur(n.hwnd, level % 4 > 0);
        }
        self.notes.paint_all();
    }

    fn set_dark(&mut self, dark: bool) {
        self.notes.settings.dark = dark;
        store::save_settings(&self.notes.settings);
        self.notes.paint_all();
    }

    fn toggle_pin(&mut self, id: u32) {
        if let Some(n) = self.notes.get_mut(id) {
            n.data.layer = if n.data.layer == 2 { 1 } else { 2 };
            win::set_layer(n.hwnd, n.data.layer != 2);
        }
        self.notes.paint(id);
        self.notes.schedule_save();
    }
}

fn lparam_xy(l: LPARAM) -> (i32, i32) {
    ((l & 0xffff) as i16 as i32, ((l >> 16) & 0xffff) as i16 as i32)
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

unsafe extern "system" fn note_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = with(|app| note_message(app, hwnd, msg, wp, lp)).flatten();
    match handled {
        Some(r) => r,
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// `Some(resultado)` si el mensaje se atendió acá.
fn note_message(app: &mut App, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let id = app.notes.by_hwnd(hwnd)?;
    match msg {
        WM_NCHITTEST => {
            // Bordes para cambiar el tamaño (abajo y a la derecha), como una nota.
            let (sx, sy) = lparam_xy(lp);
            let r = win::window_rect(hwnd);
            let (x, y) = (sx - r.left, sy - r.top);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let rolled = app.notes.state(id).2;
            let e = 6;
            let hit = if rolled {
                if x >= w - e { HTRIGHT } else { HTCLIENT }
            } else if x >= w - 12 && y >= h - 12 {
                HTBOTTOMRIGHT
            } else if x >= w - e {
                HTRIGHT
            } else if y >= h - e {
                HTBOTTOM
            } else {
                HTCLIENT
            };
            Some(hit as LRESULT)
        }
        WM_SIZE => {
            let (w, h) = ((lp & 0xffff) as i32, ((lp >> 16) & 0xffff) as i32);
            app.notes.fit_surface(id, w, h);
            Some(0)
        }
        WM_EXITSIZEMOVE => {
            let r = win::window_rect(hwnd);
            let rolled = app.notes.state(id).2;
            let size = if rolled { (r.right - r.left, app.notes.state(id).1) } else { (r.right - r.left, r.bottom - r.top) };
            app.notes.store(id, r.left, r.top, Some(size));
            app.notes.schedule_save();
            Some(0)
        }
        WM_MOUSEACTIVATE => Some(MA_ACTIVATE as LRESULT),
        WM_SETFOCUS | WM_KILLFOCUS => {
            set_focus(app, if msg == WM_SETFOCUS { Some(id) } else { None }, id);
            Some(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_xy(lp);
            // `SetFocus` manda WM_SETFOCUS/WM_KILLFOCUS ya, con el estado tomado
            // (y `with` los descarta): el foco se anota acá a mano.
            unsafe { SetFocus(hwnd) };
            set_focus(app, Some(id), id);
            if y < BAR {
                let button = app.notes.get(id).and_then(|n| n.button_at(x, y));
                if let Some(b) = button {
                    if let Some(n) = app.notes.get_mut(id) {
                        n.pressed = Some(b);
                    }
                    return Some(0);
                }
                if key_down(VK_CONTROL) && key_down(VK_SHIFT) {
                    app.snap.make_pile(&mut app.notes, id);
                    app.wake();
                    return Some(0);
                }
                unsafe { SetCapture(hwnd) };
                app.snap.begin_move(&mut app.notes, id, POINT { x, y });
                app.wake();
            } else {
                let (ox, oy) = NoteWin::text_origin();
                let shift = key_down(VK_SHIFT);
                if let Some(n) = app.notes.get_mut(id) {
                    n.ed.click(x as f32 - ox, y as f32 - oy, shift);
                    n.selecting = true;
                }
                unsafe { SetCapture(hwnd) };
                app.notes.paint(id);
            }
            Some(0)
        }
        WM_MOUSEMOVE => {
            if app.snap.dragging() == Some(id) {
                app.snap.step(&mut app.notes);
                return Some(0);
            }
            let (x, y) = lparam_xy(lp);
            let mut repaint = false;
            let mut unroll = false;
            let hushed = app.snap.hush(id, true);
            if let Some(n) = app.notes.get_mut(id) {
                if !n.hovered {
                    n.hovered = true;
                    repaint = true;
                    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    unsafe { TrackMouseEvent(&mut t) };
                    n.roll_at = None;
                    unroll = n.data.roll_mode == 1 && n.data.rolled && !hushed;
                }
                if n.selecting {
                    let (ox, oy) = NoteWin::text_origin();
                    n.ed.click(x as f32 - ox, y as f32 - oy, true);
                    repaint = true;
                }
                let hot = n.button_at(x, y);
                if hot != n.hot {
                    n.hot = hot;
                    repaint = true;
                }
            }
            if repaint {
                app.notes.paint(id);
            }
            if unroll && !app.snap.busy() {
                app.snap.reshape(&mut app.notes, id, None, false);
                app.wake();
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            app.snap.hush(id, false);
            let delay = store::ROLL_DELAYS_MS[app.notes.settings.roll_delay as usize % 5];
            if let Some(n) = app.notes.get_mut(id) {
                n.hovered = false;
                n.hot = None;
                if n.data.roll_mode == 1 && !n.data.rolled && !n.focused {
                    n.roll_at = Some(Instant::now() + Duration::from_millis(delay as u64));
                }
            }
            app.notes.paint(id);
            app.wake();
            Some(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_xy(lp);
            if app.snap.dragging() == Some(id) {
                let click = !app.snap.moved();
                app.snap.end(&mut app.notes, false);
                unsafe { ReleaseCapture() };
                if click && app.notes.state(id).3 == false {
                    app.toggle_roll(id);
                }
                app.wake();
                return Some(0);
            }
            let pressed = app.notes.get_mut(id).and_then(|n| n.pressed.take());
            if let Some(b) = pressed {
                if app.notes.get(id).and_then(|n| n.button_at(x, y)) == Some(b) {
                    match b {
                        Button::New => {
                            app.notes.new_note(Some(id));
                        }
                        Button::Pin => app.toggle_pin(id),
                    }
                }
            }
            if let Some(n) = app.notes.get_mut(id) {
                n.selecting = false;
            }
            unsafe { ReleaseCapture() };
            Some(0)
        }
        WM_KEYDOWN => {
            let ctrl = key_down(VK_CONTROL);
            let shift = key_down(VK_SHIFT);
            let vk = wp as u16;
            match (ctrl, vk) {
                (true, 0x4E) => {
                    app.notes.new_note(Some(id));
                }
                (true, 0x52) => app.toggle_roll(id),
                (true, 0x47) => {
                    let g = app.notes.settings.glass + 1;
                    app.set_glass(g);
                }
                (true, 0x44) => {
                    let d = !app.notes.settings.dark;
                    app.set_dark(d);
                }
                (true, 0x54) if shift => app.toggle_pin(id),
                (true, 0x41) => app.notes.get_mut(id).map(|n| n.ed.select_all()).unwrap_or(()),
                (true, 0x43) | (true, 0x58) => {
                    let sel = app.notes.get(id).map(|n| n.ed.selected()).unwrap_or_default();
                    copy(hwnd, &sel);
                    if vk == 0x58 && !locked(app, id) {
                        if let Some(n) = app.notes.get_mut(id) {
                            n.ed.insert(&[]);
                        }
                        edited(app, id);
                    }
                }
                (true, 0x56) if !locked(app, id) => {
                    let s = paste(hwnd);
                    if let Some(n) = app.notes.get_mut(id) {
                        n.ed.insert(&s);
                    }
                    edited(app, id);
                }
                _ => {
                    if locked(app, id) && (vk == VK_BACK || vk == VK_DELETE) {
                        return Some(0);
                    }
                    let Some(n) = app.notes.get_mut(id) else { return Some(0) };
                    let ed = &mut n.ed;
                    let mut changed = false;
                    match vk {
                        VK_LEFT => ed.left(shift, ctrl),
                        VK_RIGHT => ed.right(shift, ctrl),
                        VK_UP => ed.vertical(false, shift),
                        VK_DOWN => ed.vertical(true, shift),
                        VK_HOME => ed.home(shift),
                        VK_END => ed.end(shift),
                        VK_BACK => {
                            ed.backspace(ctrl);
                            changed = true;
                        }
                        VK_DELETE => {
                            ed.delete(ctrl);
                            changed = true;
                        }
                        _ => return Some(0),
                    }
                    if changed {
                        edited(app, id);
                    } else {
                        app.notes.paint(id);
                    }
                }
            }
            app.notes.paint(id);
            Some(0)
        }
        WM_CHAR => {
            let c = wp as u16;
            // Los de control (Ctrl+letra, Backspace, Esc) ya se atendieron.
            if c < 0x20 && c != b'\r' as u16 && c != b'\t' as u16 {
                return Some(0);
            }
            let c = if c == b'\r' as u16 { b'\n' as u16 } else { c };
            if let Some(n) = app.notes.get_mut(id) {
                if n.data.locked {
                    return Some(0);
                }
                n.ed.insert(&[c]);
            }
            edited(app, id);
            Some(0)
        }
        WM_CLOSE => Some(0),
        _ => None,
    }
}

/// `focused`: la nota que tiene el foco (o ninguna, si lo perdió `id`). Una
/// nota Auto que pierde el foco con el mouse afuera se enrolla.
fn set_focus(app: &mut App, focused: Option<u32>, id: u32) {
    let delay = store::ROLL_DELAYS_MS[app.notes.settings.roll_delay as usize % 5];
    let mut repaint = Vec::new();
    for n in &mut app.notes.list {
        let now = match focused {
            Some(f) => n.data.id == f,
            None if n.data.id == id => false,
            None => n.focused,
        };
        if n.focused != now {
            n.focused = now;
            repaint.push(n.data.id);
            if !now && !n.hovered && n.data.roll_mode == 1 && !n.data.rolled {
                n.roll_at = Some(Instant::now() + Duration::from_millis(delay as u64));
            }
        }
    }
    for k in repaint {
        app.notes.paint(k);
    }
    app.wake();
}

fn locked(app: &App, id: u32) -> bool {
    app.notes.get(id).is_some_and(|n| n.data.locked)
}

fn edited(app: &mut App, id: u32) {
    if let Some(n) = app.notes.get_mut(id) {
        n.data.text = n.ed.string();
        n.data.t[0] = store::now_ms();
    }
    app.notes.paint(id);
    app.notes.schedule_save();
}

fn copy(hwnd: HWND, text: &[u16]) {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return;
        }
        EmptyClipboard();
        let bytes = (text.len() + 1) * 2;
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if !mem.is_null() {
            let p = GlobalLock(mem) as *mut u16;
            std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            *p.add(text.len()) = 0;
            GlobalUnlock(mem);
            SetClipboardData(13, mem as HANDLE);
        }
        CloseClipboard();
    }
}

fn paste(hwnd: HWND) -> Vec<u16> {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let h = GetClipboardData(13);
        if !h.is_null() {
            let p = GlobalLock(h as HGLOBAL) as *const u16;
            if !p.is_null() {
                let mut i = 0;
                while *p.add(i) != 0 {
                    let c = *p.add(i);
                    if c != b'\r' as u16 {
                        out.push(c);
                    }
                    i += 1;
                }
                GlobalUnlock(h as HGLOBAL);
            }
        }
        CloseClipboard();
        out
    }
}

unsafe extern "system" fn msg_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        pump::WM_FRAME => {
            with(|app| app.frame());
            0
        }
        WM_TIMER if wp == SAVE_TIMER => {
            KillTimer(hwnd, SAVE_TIMER);
            with(|app| app.notes.save_now());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    store::set_app("SimpckyD2D");
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(std::ptr::null());
        for (name, proc_) in [(NOTE_CLASS, note_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT), (MSG_CLASS, msg_proc)] {
            let class = wide(name);
            let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: inst, lpszClassName: class.as_ptr(), hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW), ..std::mem::zeroed() };
            RegisterClassW(&wc);
        }
        let class = wide(MSG_CLASS);
        let msg_hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), inst, std::ptr::null());
        pump::set_target(msg_hwnd);

        let g = Gfx::new().expect("Direct2D");
        let fonts = Fonts::new(&g);
        let settings = store::load_settings();
        let mut list = store::load_notes();
        if list.is_empty() {
            let mut n = NoteData::new(1, 160, 140, 0);
            n.title = "Simpcky · Direct2D".into();
            n.text = "Arrastrá la barra: con Ctrl aparece la cuadrícula.\nLlevala hasta arriba: el panel de formas y pilas.\n\nCtrl+G vidrio · Ctrl+D oscuro · Ctrl+N nota nueva".into();
            list.push(n);
        }
        let next_id = list.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let mut notes = Notes { g, fonts, list: Vec::new(), settings, next_id, msg_hwnd };
        // Las ocultas también se cargan (si no, el próximo guardado las borraba),
        // pero su ventana no se muestra.
        for data in list {
            notes.create(data);
        }
        APP.with(|c| *c.borrow_mut() = Some(App { notes, snap: Snap::new(), overlay: None, frames: false }));

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with(|app| app.notes.save_now());
    }
}
