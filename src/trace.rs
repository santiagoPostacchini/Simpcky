//! Registro opcional; no altera las regiones de dibujo.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::{Mutex, OnceLock},
};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::GetUpdateRect,
    System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    UI::WindowsAndMessaging::GetClientRect,
};
struct Log {
    file: Mutex<File>,
    start: i64,
    frequency: i64,
}
static LOG: OnceLock<Option<Log>> = OnceLock::new();
pub fn init() {
    LOG.get_or_init(|| {
        let path = std::env::var_os("SIMPCKY_PAINT_LOG").filter(|p| !p.is_empty())?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let (mut start, mut frequency) = (0, 0);
        unsafe {
            QueryPerformanceCounter(&mut start);
            QueryPerformanceFrequency(&mut frequency);
        }
        if frequency <= 0 {
            return None;
        }
        let mut file = file;
        // Reloj compartido para correlacionar con la captura de otro proceso.
        let origin = start as i128 * 1_000_000 / frequency as i128;
        let _ = writeln!(file, "0\tQPC_ORIGIN\t0x0\t{origin},0,0,0");
        Some(Log {
            file: Mutex::new(file),
            start,
            frequency,
        })
    });
}
fn write(event: &str, hwnd: HWND, value: impl std::fmt::Display) {
    let Some(log) = LOG.get().and_then(Option::as_ref) else {
        return;
    };
    let mut now = 0;
    unsafe {
        QueryPerformanceCounter(&mut now);
    }
    if let Ok(mut file) = log.file.lock() {
        let _ = writeln!(
            file,
            "{}\t{}\t0x{:x}\t{}",
            (now - log.start) as i128 * 1_000_000 / log.frequency as i128,
            event,
            hwnd as usize,
            value
        );
    }
}
pub fn rect(event: &str, hwnd: HWND, rect: Option<&RECT>) {
    if !matches!(LOG.get(), Some(Some(_))) {
        return;
    }
    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let r = rect.unwrap_or_else(|| {
        unsafe {
            GetClientRect(hwnd, &mut client);
        }
        &client
    });
    write(
        event,
        hwnd,
        format_args!("{},{},{},{}", r.left, r.top, r.right, r.bottom),
    );
}
pub fn update(event: &str, hwnd: HWND) {
    if !matches!(LOG.get(), Some(Some(_))) {
        return;
    }
    let mut r = unsafe { std::mem::zeroed() };
    unsafe {
        GetUpdateRect(hwnd, &mut r, 0);
    }
    rect(event, hwnd, Some(&r));
}
pub fn key(event: &str, hwnd: HWND, code: usize) {
    if !matches!(LOG.get(), Some(Some(_))) {
        return;
    }
    write(event, hwnd, code);
}
