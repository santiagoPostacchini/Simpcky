//! Cambiar el nombre de una nota "en el lugar": un cuadro de texto sin
//! borde que aparece encima del título del encabezado, con el nombre
//! actual seleccionado — como renombrar un archivo en el Explorador.
//! Enter confirma, Esc cancela, y hacer clic en cualquier otro lado
//! también confirma.
//!
//! Se abre con doble clic en la barra, con F2, o desde el menú "⋯" (y
//! desde el clic derecho de una tarjeta en "Todas las notas").

use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::EM_SETSEL;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::note;
use crate::win::{from_wide, wide};

/// Lo manda el cuadro de texto a su nota (en cola, nunca en el acto:
/// así el EDIT no se destruye a sí mismo en medio de su propio
/// wndproc). `wparam` = 1 guardar, 0 descartar.
pub const WM_RENAME_DONE: u32 = WM_APP + 20;

struct Active {
    note: isize,
    edit: isize,
    /// Lo que decía el cuadro al abrirse. Si al confirmar sigue igual,
    /// no se toca nada: sin esto, abrir y cerrar el cuadro de una nota
    /// sin nombre propio le dejaba fijo el nombre automático ("Nota",
    /// o su primera línea) y dejaba de seguir al texto.
    original: String,
    brush: isize,
    ink: u32,
    header: u32,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

pub fn is_renaming(note: HWND) -> bool {
    ACTIVE.lock().unwrap().as_ref().map(|a| a.note == note as isize).unwrap_or(false)
}

/// Abre el cuadro de renombrar sobre el título de `note`.
pub fn begin(note: HWND) {
    // Si ya había otro abierto (en otra nota), se confirma primero.
    let other = ACTIVE.lock().unwrap().as_ref().map(|a| a.note);
    if let Some(other) = other {
        if other == note as isize {
            return;
        }
        finish(other as HWND, true);
    }

    let Some((current, header, ink, rc)) = note::rename_info(note) else { return };
    let hinstance = { crate::app::app().lock().unwrap().hinstance } as HINSTANCE;

    unsafe {
        let class = wide("EDIT");
        let text = wide(&current);
        let edit = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | (ES_AUTOHSCROLL as u32),
            rc.left,
            rc.top,
            rc.right - rc.left,
            rc.bottom - rc.top,
            note,
            null_mut(),
            hinstance,
            null(),
        );
        if edit.is_null() {
            return;
        }
        SendMessageW(edit, WM_SETFONT, note::header_font(note) as usize, 1);
        // El wndproc original va en GWLP_USERDATA del propio EDIT: así
        // sigue a mano aunque el estado de acá ya se haya limpiado
        // (los últimos mensajes de un EDIT, WM_DESTROY/WM_NCDESTROY,
        // llegan después).
        let prev = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        SetWindowLongPtrW(edit, GWLP_USERDATA, prev);

        let brush = CreateSolidBrush(header);
        *ACTIVE.lock().unwrap() = Some(Active {
            note: note as isize,
            edit: edit as isize,
            original: current.clone(),
            brush: brush as isize,
            ink,
            header,
        });

        SetFocus(edit);
        SendMessageW(edit, EM_SETSEL, 0, -1);
        InvalidateRect(note, null(), 1);
    }
}

/// Cierra el cuadro de `note`, guardando (o no) lo escrito.
pub fn finish(note: HWND, save: bool) {
    let active = {
        let mut guard = ACTIVE.lock().unwrap();
        match guard.as_ref() {
            Some(a) if a.note == note as isize => guard.take(),
            _ => None,
        }
    };
    let Some(active) = active else { return };
    let edit = active.edit as HWND;
    let text = unsafe {
        let len = GetWindowTextLengthW(edit).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32);
        from_wide(&buf)
    };
    unsafe {
        DestroyWindow(edit);
        DeleteObject(active.brush as HGDIOBJ);
    }
    if save && text.trim() != active.original.trim() {
        note::set_title(note, text.trim());
    }
    unsafe { InvalidateRect(note, null(), 1) };
}

/// La nota se está destruyendo (el EDIT se va con ella): solo soltar
/// el estado, sin tocar ventanas.
pub fn abort_for(note: HWND) {
    let active = {
        let mut guard = ACTIVE.lock().unwrap();
        match guard.as_ref() {
            Some(a) if a.note == note as isize => guard.take(),
            _ => None,
        }
    };
    if let Some(a) = active {
        unsafe { DeleteObject(a.brush as HGDIOBJ) };
    }
}

/// Para el WM_CTLCOLOREDIT de la nota: el cuadro va con el color del
/// encabezado y la tinta de la nota, no blanco sobre el pastel.
pub fn ctl_color(hdc: HDC, edit: HWND) -> Option<LRESULT> {
    let guard = ACTIVE.lock().unwrap();
    let a = guard.as_ref()?;
    if a.edit != edit as isize {
        return None;
    }
    unsafe {
        SetTextColor(hdc, a.ink);
        SetBkColor(hdc, a.header);
    }
    Some(a.brush as LRESULT)
}

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let prev: WNDPROC = std::mem::transmute(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    let parent = GetParent(hwnd);
    match msg {
        WM_KEYDOWN if wparam as u16 == VK_RETURN => {
            PostMessageW(parent, WM_RENAME_DONE, 1, 0);
            return 0;
        }
        WM_KEYDOWN if wparam as u16 == VK_ESCAPE => {
            PostMessageW(parent, WM_RENAME_DONE, 0, 0);
            return 0;
        }
        // Sin esto el EDIT hace "beep" con Enter y con Esc.
        WM_CHAR if wparam == 13 || wparam == 27 => return 0,
        WM_KILLFOCUS => {
            PostMessageW(parent, WM_RENAME_DONE, 1, 0);
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        }
        _ => {}
    }
    CallWindowProcW(prev, hwnd, msg, wparam, lparam)
}
