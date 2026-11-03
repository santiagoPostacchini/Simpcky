#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod host;
mod note;
mod overlay;
mod surface;

use common::{pump, snap::Snap, store, win};
use slint::platform::{
    self,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    Key, PointerEventButton, WindowEvent,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    ptr::{null, null_mut},
    rc::Rc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Controls::WM_MOUSELEAVE, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
slint::include_modules!();
thread_local! {
    static NEXT:RefCell<Option<Rc<MinimalSoftwareWindow>>>=const{RefCell::new(None)};
    static ACTIONS:RefCell<VecDeque<Action>>=const{RefCell::new(VecDeque::new())};
    static EVENTS:RefCell<VecDeque<(HWND,u32,WPARAM,LPARAM)>>=const{RefCell::new(VecDeque::new())};
}
fn next_adapter() -> Rc<MinimalSoftwareWindow> {
    let a = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    NEXT.with(|n| {
        assert!(n.borrow().is_none());
        *n.borrow_mut() = Some(a.clone());
    });
    a
}
struct Platform {
    start: Instant,
}
impl platform::Platform for Platform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn platform::WindowAdapter>, platform::PlatformError> {
        NEXT.with(|n| {
            n.borrow_mut()
                .take()
                .map(|a| a as Rc<dyn platform::WindowAdapter>)
                .ok_or_else(|| platform::PlatformError::Other("Falta el adaptador".into()))
        })
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}
enum Action {
    Header(u32, i32, i32, bool, bool),
    Text(u32, String),
    New(u32),
    Pin(u32),
}
fn push_action(a: Action) {
    ACTIONS.with(|q| q.borrow_mut().push_back(a));
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE
        | WM_MOUSELEAVE
        | WM_LBUTTONDOWN
        | WM_LBUTTONUP
        | WM_RBUTTONDOWN
        | WM_RBUTTONUP
        | WM_MBUTTONDOWN
        | WM_MBUTTONUP
        | WM_MOUSEWHEEL
        | WM_MOUSEHWHEEL
        | WM_KEYDOWN
        | WM_KEYUP
        | WM_CHAR
        | WM_ACTIVATE
        | WM_CAPTURECHANGED
        | WM_CLOSE
        | WM_TIMER
        | pump::WM_FRAME => {
            EVENTS.with(|q| q.borrow_mut().push_back((hwnd, msg, w, l)));
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut p: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut p);
            EndPaint(hwnd, &p);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
fn special(w: usize) -> Option<Key> {
    Some(match w as u16 {
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_LEFT => Key::LeftArrow,
        VK_RIGHT => Key::RightArrow,
        VK_UP => Key::UpArrow,
        VK_DOWN => Key::DownArrow,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_RETURN => Key::Return,
        VK_TAB => Key::Tab,
        VK_ESCAPE => Key::Escape,
        VK_SHIFT => Key::Shift,
        VK_CONTROL => Key::Control,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        _ => return None,
    })
}
struct App {
    host: host::Notes,
    snap: Snap,
    overlay: Option<overlay::CellOverlay>,
    animating: bool,
}
impl App {
    fn actions(&mut self) {
        loop {
            let a = ACTIONS.with(|q| q.borrow_mut().pop_front());
            let Some(a) = a else { break };
            match a {
                Action::Header(id, x, y, ctrl, shift) => {
                    if self.host.get(id).is_some_and(|n| n.data.locked) {
                        continue;
                    }
                    if ctrl && shift {
                        self.snap.make_pile(&mut self.host, id);
                        self.animating = true;
                    } else {
                        if let Some(n) = self.host.get(id) {
                            unsafe {
                                SetCapture(n.surface.hwnd);
                            }
                        }
                        self.snap.begin_move(&mut self.host, id, POINT { x, y });
                    }
                }
                Action::Text(id, text) => {
                    if let Some(n) = self.host.get_mut(id) {
                        n.data.text = text;
                        n.ui.set_note_title(n.data.display_title().into());
                    }
                    self.host.changed();
                }
                Action::New(id) => self.host.add(id),
                Action::Pin(id) => self.host.pin(id),
            }
        }
    }
    fn roll(&mut self, id: u32) {
        if let Some(n) = self.host.get(id) {
            if n.data.locked {
                return;
            }
            let rolled = !n.data.rolled;
            self.snap.reshape(&mut self.host, id, None, rolled);
            self.animating = true;
        }
    }
    fn event(&mut self, hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) {
        if msg == pump::WM_FRAME {
            pump::frame_taken();
            platform::update_timers_and_animations();
            self.frame();
            return;
        }
        if msg == WM_TIMER {
            unsafe {
                KillTimer(hwnd, w);
            }
            if w == 1 {
                self.host.flush();
            }
            return;
        }
        let Some(index) = self.host.notes.iter().position(|n| n.surface.hwnd == hwnd) else {
            return;
        };
        let id = self.host.notes[index].data.id;
        let adapter = self.host.notes[index].surface.adapter.clone();
        let position =
            slint::LogicalPosition::new((l as u16 as i16) as f32, ((l >> 16) as u16 as i16) as f32);
        match msg {
            WM_MOUSEMOVE => {
                unsafe {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut track);
                }
                self.host.notes[index].hovered = true;
                self.host.notes[index].ui.set_hovered(true);
                adapter.dispatch_event(WindowEvent::PointerMoved { position });
                if self.snap.dragging().is_some() {
                    self.snap.step(&mut self.host);
                }
            }
            WM_MOUSELEAVE => {
                self.host.notes[index].hovered = false;
                self.host.notes[index].ui.set_hovered(false);
                adapter.dispatch_event(WindowEvent::PointerExited);
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                unsafe {
                    SetFocus(hwnd);
                    SetCapture(hwnd);
                }
                let button = if msg == WM_LBUTTONDOWN {
                    PointerEventButton::Left
                } else if msg == WM_RBUTTONDOWN {
                    PointerEventButton::Right
                } else {
                    PointerEventButton::Middle
                };
                adapter.dispatch_event(WindowEvent::PointerPressed { position, button });
            }
            WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
                if msg == WM_LBUTTONUP && self.snap.dragging().is_some() {
                    let click = !self.snap.moved();
                    let moving = self.snap.dragging();
                    self.snap.end(&mut self.host, false);
                    if click {
                        if let Some(id) = moving {
                            if self.host.get(id).is_some_and(|n| n.data.roll_mode == 0) {
                                self.roll(id);
                            }
                        }
                    }
                }
                let button = if msg == WM_LBUTTONUP {
                    PointerEventButton::Left
                } else if msg == WM_RBUTTONUP {
                    PointerEventButton::Right
                } else {
                    PointerEventButton::Middle
                };
                adapter.dispatch_event(WindowEvent::PointerReleased { position, button });
                unsafe {
                    ReleaseCapture();
                }
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let mut p = POINT {
                    x: position.x as i32,
                    y: position.y as i32,
                };
                unsafe {
                    ScreenToClient(hwnd, &mut p);
                }
                let delta = ((w >> 16) as u16 as i16) as f32 / 120. * 45.;
                adapter.dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(p.x as f32, p.y as f32),
                    delta_x: if msg == WM_MOUSEHWHEEL { delta } else { 0. },
                    delta_y: if msg == WM_MOUSEWHEEL { delta } else { 0. },
                });
            }
            WM_ACTIVATE => {
                adapter.dispatch_event(WindowEvent::WindowActiveChanged(
                    w & 0xffff != WA_INACTIVE as usize,
                ));
            }
            WM_KEYDOWN | WM_KEYUP => {
                if msg == WM_KEYDOWN && win::ctrl_down() {
                    let shift = unsafe { GetKeyState(VK_SHIFT as i32) < 0 };
                    match w as u8 {
                        b'N' => {
                            self.host.add(id);
                            return;
                        }
                        b'R' => {
                            self.roll(id);
                            return;
                        }
                        b'G' | b'D' => {
                            if w as u8 == b'G' {
                                self.host.settings.glass = (self.host.settings.glass + 1) % 4;
                            } else {
                                self.host.settings.dark = !self.host.settings.dark;
                            }
                            let s = self.host.settings;
                            for n in &mut self.host.notes {
                                n.style(s);
                            }
                            self.host.changed();
                            return;
                        }
                        b'T' if shift => {
                            self.host.pin(id);
                            return;
                        }
                        _ => {}
                    }
                }
                if let Some(key) = special(w) {
                    let text = key.into();
                    adapter.dispatch_event(if msg == WM_KEYUP {
                        WindowEvent::KeyReleased { text }
                    } else if l & (1 << 30) != 0 {
                        WindowEvent::KeyPressRepeated { text }
                    } else {
                        WindowEvent::KeyPressed { text }
                    });
                } else if win::ctrl_down() {
                    if let Some(c) = char::from_u32(w as u32) {
                        let text = c.to_lowercase().to_string().into();
                        adapter.dispatch_event(if msg == WM_KEYUP {
                            WindowEvent::KeyReleased { text }
                        } else {
                            WindowEvent::KeyPressed { text }
                        });
                    }
                }
            }
            WM_CHAR => {
                let ch = w as u16;
                if (0xd800..=0xdbff).contains(&ch) {
                    self.host.notes[index].surrogate = Some(ch);
                    return;
                }
                let c = if (0xdc00..=0xdfff).contains(&ch) {
                    self.host.notes[index].surrogate.take().and_then(|hi| {
                        char::from_u32(
                            0x10000 + (((hi as u32 - 0xd800) << 10) | (ch as u32 - 0xdc00)),
                        )
                    })
                } else {
                    self.host.notes[index].surrogate = None;
                    char::from_u32(ch as u32)
                };
                if let Some(c) = c.filter(|c| !c.is_control()) {
                    let text: slint::SharedString = c.to_string().into();
                    adapter.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                    adapter.dispatch_event(WindowEvent::KeyReleased { text });
                }
            }
            // Perder la captura no corta el arrastre (como en Simpcky): `snap.frame`
            // mira el botón del mouse y suelta la nota cuando se lo suelta. (Antes
            // se cancelaba, y volver a tomar la captura en la misma ventana ya
            // manda este aviso: ningún arrastre pasaba del primer movimiento.)
            WM_CAPTURECHANGED => {}
            WM_CLOSE => {
                self.host.flush();
                unsafe {
                    PostQuitMessage(0);
                }
            }
            _ => {}
        }
        self.actions();
        if msg == WM_MOUSELEAVE {
            self.frame();
        }
    }
    fn frame(&mut self) {
        let was = self.snap.dragging().is_some();
        self.animating = self.snap.frame(&mut self.host);
        if was && self.snap.dragging().is_none() {
            unsafe {
                ReleaseCapture();
            }
        }
        let pt = win::cursor();
        let mut changes = Vec::new();
        for n in &mut self.host.notes {
            let hovered = !n.data.hidden && win::contains(&win::window_rect(n.surface.hwnd), pt);
            if n.hovered != hovered {
                n.hovered = hovered;
                n.ui.set_hovered(hovered);
            }
            if n.data.roll_mode == 1
                && !self.snap.hush(n.data.id, hovered)
                && self.snap.dragging().is_none()
                && n.data.rolled == hovered
            {
                changes.push((n.data.id, !hovered));
            }
        }
        for (id, rolled) in changes {
            self.snap.reshape(&mut self.host, id, None, rolled);
            self.animating = true;
        }
    }
    fn render(&mut self) {
        for n in &mut self.host.notes {
            n.surface.draw("nota", n.data.id, &[]);
        }
        let layers = self.snap.layers();
        if layers.visible() && self.overlay.is_none() {
            self.overlay = Some(overlay::CellOverlay::new(layers));
        }
        if let Some(o) = &mut self.overlay {
            o.update(layers);
        }
        pump::run_frames(
            self.animating
                || self.snap.busy()
                || self
                    .host
                    .notes
                    .iter()
                    .any(|n| n.hovered && n.data.roll_mode == 1)
                || self.snap.layers().visible()
                || self
                    .host
                    .notes
                    .iter()
                    .any(|n| n.surface.adapter.has_active_animations()),
        );
        unsafe {
            KillTimer(self.host.message, 2);
            if let Some(d) = platform::duration_until_next_timer_update() {
                SetTimer(
                    self.host.message,
                    2,
                    d.as_millis().clamp(1, u32::MAX as u128) as u32,
                    None,
                );
            }
        }
    }
}
fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        platform::set_platform(Box::new(Platform {
            start: Instant::now(),
        }))
        .expect("No se pudo iniciar Slint");
        let name = win::wide("SimpckySlint");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
        assert!(RegisterClassW(&class) != 0);
        let message = CreateWindowExW(
            0,
            name.as_ptr(),
            null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            null_mut(),
            null(),
        );
        pump::set_target(message);
        store::set_app("SimpckySlint");
        let mut data = store::load_notes();
        let mut settings = store::load_settings();
        // El capturador compartido deja el tema en la carpeta de la app original.
        if std::env::var_os("SIMPCKY_INSTANCIA").is_some() {
            if let Ok(s) = std::fs::read_to_string(
                std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
                    .join("Simpcky/settings.json"),
            ) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    settings.dark = v["dark"].as_bool().unwrap_or(settings.dark);
                }
            }
        }
        let welcome = data.is_empty();
        if welcome {
            let mut n = store::NoteData::new(1, 100, 100, 0);
            n.title = "Simpcky · Slint".into();
            n.text = "Arrastrá la barra: con Ctrl aparece la cuadrícula.\nLlevala hasta arriba: el panel de formas y pilas.\n\nCtrl+G vidrio · Ctrl+D oscuro · Ctrl+N nota nueva".into();
            data.push(n);
        }
        let notes = data
            .into_iter()
            .map(|mut d| {
                d.w = d.w.max(120);
                d.h = d.h.max(80);
                let n = note::NoteWindow::new(d, settings);
                if n.data.hidden {
                    ShowWindow(n.surface.hwnd, SW_HIDE);
                }
                n
            })
            .collect();
        let mut app = App {
            host: host::Notes {
                notes,
                settings,
                dirty: None,
                message,
            },
            snap: Snap::new(),
            overlay: None,
            animating: false,
        };
        if welcome {
            app.host.changed();
        }
        app.render();
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            platform::update_timers_and_animations();
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            loop {
                let e = EVENTS.with(|q| q.borrow_mut().pop_front());
                if let Some((h, m, w, l)) = e {
                    app.event(h, m, w, l);
                } else {
                    break;
                }
            }
            app.actions();
            app.render();
        }
        app.host.flush();
        pump::run_frames(false);
        pump::set_target(null_mut());
        DestroyWindow(message);
    }
}

