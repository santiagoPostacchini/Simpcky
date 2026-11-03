use crate::note::NoteWindow;
use common::{
    snap::{Host, Info},
    store::{self, NoteData, Settings},
    win,
};
use std::time::Instant;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::HMONITOR, UI::WindowsAndMessaging::*};
pub struct Notes {
    pub notes: Vec<NoteWindow>,
    pub settings: Settings,
    pub dirty: Option<Instant>,
    pub message: HWND,
}
impl Notes {
    pub fn get(&self, id: u32) -> Option<&NoteWindow> {
        self.notes.iter().find(|n| n.data.id == id)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut NoteWindow> {
        self.notes.iter_mut().find(|n| n.data.id == id)
    }
    pub fn changed(&mut self) {
        self.dirty = Some(Instant::now());
        unsafe {
            SetTimer(self.message, 1, 800, None);
        }
    }
    pub fn flush(&mut self) {
        if self.dirty.take().is_some() {
            store::save_notes(
                &self
                    .notes
                    .iter()
                    .map(|n| n.data.clone())
                    .collect::<Vec<_>>(),
            );
            store::save_settings(&self.settings);
        }
        unsafe {
            KillTimer(self.message, 1);
        }
    }
    pub fn add(&mut self, from: u32) {
        let id = self.notes.iter().map(|n| n.data.id).max().unwrap_or(0) + 1;
        let (x, y) = self
            .get(from)
            .map(|n| (n.data.x + 32, n.data.y + 48))
            .unwrap_or((100, 100));
        self.notes.push(NoteWindow::new(
            NoteData::new(id, x, y, ((id - 1) % 6) as u8),
            self.settings,
        ));
        self.changed();
    }
    pub fn pin(&mut self, id: u32) {
        let s = self.settings;
        if let Some(n) = self.get_mut(id) {
            n.data.layer = if n.data.layer == 2 { 1 } else { 2 };
            win::set_layer(n.surface.hwnd, n.data.layer != 2);
            n.style(s);
        }
        self.changed();
    }
}
impl Host for Notes {
    fn notes_on(&self, m: HMONITOR) -> Vec<Info> {
        self.notes
            .iter()
            .filter(|n| {
                !n.data.hidden && n.data.layer != 2 && win::monitor_of(n.data.x, n.data.y).0 == m
            })
            .map(|n| {
                let d = &n.data;
                Info {
                    id: d.id,
                    x: d.x,
                    y: d.y,
                    w: d.w,
                    h: d.h,
                    rolled: d.rolled,
                    bar: 40,
                }
            })
            .collect()
    }
    fn window_rect(&self, id: u32) -> RECT {
        self.get(id)
            .map(|n| win::window_rect(n.surface.hwnd))
            .unwrap_or(win::rect(0, 0, 0, 0))
    }
    fn place(&mut self, id: u32, pos: Option<(i32, i32)>, size: Option<(i32, i32)>) {
        if let Some(n) = self.get_mut(id) {
            if let Some((w, h)) = size {
                n.surface.resize(w, h);
            }
            if let Some((x, y)) = pos {
                unsafe {
                    SetWindowPos(
                        n.surface.hwnd,
                        std::ptr::null_mut(),
                        x,
                        y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }
    fn state(&self, id: u32) -> (i32, i32, bool, bool, bool) {
        self.get(id)
            .map(|n| {
                let d = &n.data;
                (d.w, d.h, d.rolled, d.roll_mode == 1, d.locked)
            })
            .unwrap_or((280, 320, false, false, false))
    }
    fn set_shape(&mut self, id: u32, size: Option<(i32, i32)>, rolled: bool) {
        if let Some(n) = self.get_mut(id) {
            if let Some((w, h)) = size {
                n.data.w = w;
                n.data.h = h;
            }
            n.data.rolled = rolled;
        }
        self.changed();
    }
    fn set_auto(&mut self, id: u32, auto: bool) {
        if let Some(n) = self.get_mut(id) {
            n.data.roll_mode = u8::from(auto);
        }
        self.changed();
    }
    fn store(&mut self, id: u32, x: i32, y: i32, size: Option<(i32, i32)>) {
        if let Some(n) = self.get_mut(id) {
            n.data.x = x;
            n.data.y = y;
            if let Some((w, h)) = size {
                n.data.w = w;
                n.data.h = h;
            }
        }
        self.changed();
    }
    fn raise(&mut self, id: u32) {
        if let Some(n) = self.get(id) {
            unsafe {
                SetWindowPos(
                    n.surface.hwnd,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
    fn trash(&mut self, id: u32) {
        self.notes.retain(|n| n.data.id != id);
        self.changed();
    }
    fn save(&mut self) {
        self.changed();
    }
    fn bar(&self) -> i32 {
        40
    }
    fn dark(&self) -> bool {
        self.settings.dark
    }
}
