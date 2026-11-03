use crate::{next_adapter, push_action, surface::Surface, Action, Note};
use common::{
    store::{NoteData, Settings},
    theme, win,
};
use slint::ComponentHandle;

pub struct NoteWindow {
    pub data: NoteData,
    pub ui: Note,
    pub surface: Surface,
    pub hovered: bool,
    pub surrogate: Option<u16>,
}
pub fn color(rgb: u32, a: u8) -> slint::Color {
    slint::Color::from_argb_u8(a, (rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}
impl NoteWindow {
    pub fn new(data: NoteData, settings: Settings) -> Self {
        let adapter = next_adapter();
        let ui = Note::new().expect("No se pudo crear la nota");
        let id = data.id;
        ui.on_header_pressed(move |x, y, c, s| {
            push_action(Action::Header(
                id,
                x as i32,
                y as i32,
                c || win::ctrl_down(),
                s || unsafe {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT as i32,
                    ) < 0
                },
            ))
        });
        ui.on_text_changed(move |text| push_action(Action::Text(id, text.to_string())));
        ui.on_new_note(move || push_action(Action::New(id)));
        ui.on_toggle_pin(move || push_action(Action::Pin(id)));
        ui.set_text(data.text.clone().into());
        let surface = Surface::new(
            adapter,
            false,
            data.x,
            data.y,
            data.w,
            if data.rolled { 40 } else { data.h },
        );
        win::round_corners(surface.hwnd, true);
        win::set_layer(surface.hwnd, data.layer != 2);
        ui.show().unwrap();
        surface
            .adapter
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        surface.adapter.set_size(slint::PhysicalSize::new(
            surface.size.0 as u32,
            surface.size.1 as u32,
        ));
        let mut n = Self {
            data,
            ui,
            surface,
            hovered: false,
            surrogate: None,
        };
        n.style(settings);
        n.surface.draw("nota", id, &[]);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                n.surface.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
            );
        }
        n
    }
    pub fn style(&mut self, s: Settings) {
        let (head, body, ink) = theme::note_colors(self.data.color, s.dark);
        let (ha, ba) = theme::glass_alpha(s.glass, s.dark);
        self.ui.set_header_color(color(head, ha));
        self.ui.set_body_color(color(body, ba));
        self.ui.set_ink_color(color(ink, 255));
        self.ui
            .set_title_color(color(ink, theme::title_alpha(s.glass, s.dark)));
        self.ui.set_note_title(self.data.display_title().into());
        self.ui.set_pinned(self.data.layer == 2);
        win::set_blur(self.surface.hwnd, s.glass > 0);
    }
}
