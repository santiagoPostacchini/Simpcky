//! Medición visual sobre un perfil descartable y un fondo quieto.
use std::{
    path::PathBuf,
    process::{Child, Command},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Performance::*},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn counter() -> i64 {
    let mut n = 0;
    unsafe {
        QueryPerformanceCounter(&mut n);
    }
    n
}
fn micros(n: i64, freq: i64) -> u64 {
    (n as i128 * 1_000_000 / freq as i128) as u64
}
#[derive(Debug, PartialEq)]
struct Blink {
    start: u64,
    duration: u64,
}
// Cada muestra conserva la hora; el último tramo termina en `end`.
fn analyze(samples: &[(u64, u64)], end: u64, threshold: u64) -> Vec<Blink> {
    let mut runs: Vec<(u64, u64)> = Vec::new();
    for &(time, hash) in samples {
        if runs.last().is_none_or(|r| r.1 != hash) {
            runs.push((time, hash));
        }
    }
    let durations: Vec<_> = (0..runs.len())
        .map(|i| {
            if i + 1 < runs.len() {
                runs[i + 1].0 - runs[i].0
            } else {
                end - runs[i].0
            }
        })
        .collect();
    (1..runs.len().saturating_sub(1))
        .filter(|&i| {
            durations[i] < threshold
                && durations[i - 1] >= threshold
                && durations[i + 1] >= threshold
        })
        .map(|i| Blink {
            start: runs[i].0,
            duration: durations[i],
        })
        .collect()
}
struct Options {
    glass: bool,
    keys: usize,
    interval: u64,
    threshold: u64,
    exe: PathBuf,
    name: String,
}
fn options() -> Result<Options, String> {
    let mut o = Options {
        glass: true,
        keys: 40,
        interval: 150,
        threshold: 50,
        exe: "target/release/simpcky.exe".into(),
        name: String::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(k) = args.next() {
        let v = args.next().ok_or(format!("Falta valor para {k}"))?;
        match k.as_str() {
            "--vidrio" => {
                o.glass = match v.as_str() {
                    "si" => true,
                    "no" => false,
                    _ => return Err("--vidrio: si|no".into()),
                }
            }
            "--teclas" => o.keys = v.parse().map_err(|_| "--teclas inválido")?,
            "--intervalo" => o.interval = v.parse().map_err(|_| "--intervalo inválido")?,
            "--umbral" => o.threshold = v.parse().map_err(|_| "--umbral inválido")?,
            "--exe" => o.exe = v.into(),
            "--nombre" => o.name = v,
            _ => return Err(format!("Opción desconocida: {k}")),
        }
    }
    if o.name.is_empty() {
        o.name = if o.glass { "vidrio" } else { "liso" }.into();
    }
    if o.name == "."
        || o.name == ".."
        || o.name
            .chars()
            .any(|c| !c.is_alphanumeric() && c != '-' && c != '_')
    {
        return Err("Nombre de perfil inválido".into());
    }
    if o.keys == 0 || o.threshold == 0 {
        return Err("Teclas y umbral deben ser mayores que cero".into());
    }
    Ok(o)
}
struct Session {
    child: Option<Child>,
    background: HWND,
    brush: HBRUSH,
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if let Some(child) = &mut self.child {
                let controller = FindWindowW(wide("SimpckyController.medicion").as_ptr(), null());
                let mut pid = 0;
                GetWindowThreadProcessId(controller, &mut pid);
                if pid == child.id() {
                    PostMessageW(controller, WM_CLOSE, 0, 0);
                }
                let until = Instant::now() + Duration::from_secs(5);
                while Instant::now() < until && child.try_wait().ok().flatten().is_none() {
                    pause(Duration::from_millis(50));
                }
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            if !self.background.is_null() {
                DestroyWindow(self.background);
            }
            DeleteObject(self.brush);
        }
    }
}
struct Search {
    pid: u32,
    note: HWND,
}
unsafe extern "system" fn find_note(hwnd: HWND, data: LPARAM) -> BOOL {
    let search = &mut *(data as *mut Search);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let mut class = [0u16; 128];
    let n = GetClassNameW(hwnd, class.as_mut_ptr(), 128);
    if pid == search.pid && String::from_utf16_lossy(&class[..n as usize]) == "SimpckyNote" {
        search.note = hwnd;
    }
    1
}
unsafe extern "system" fn find_top(hwnd: HWND, data: LPARAM) -> BOOL {
    find_note(hwnd, data);
    EnumChildWindows(hwnd, Some(find_note), data);
    1
}
/// Las teclas y clics simulados van a la ventana de adelante: si no es la
/// instancia de prueba (el usuario hizo clic en otra cosa), se corta antes
/// de escribir en otro programa.
unsafe fn ensure_foreground(pid: u32) -> Result<(), String> {
    let mut fg_pid = 0;
    GetWindowThreadProcessId(GetForegroundWindow(), &mut fg_pid);
    if fg_pid != pid {
        return Err("La ventana de adelante no es la nota de prueba: se cortó la medición para no escribir en otro programa".into());
    }
    Ok(())
}
unsafe fn key(code: u16, flags: u32) -> Result<(), String> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: if flags & KEYEVENTF_UNICODE != 0 {
                    0
                } else {
                    code
                },
                wScan: if flags & KEYEVENTF_UNICODE != 0 {
                    code
                } else {
                    0
                },
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    if SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) != 1 {
        return Err("SendInput de teclado falló".into());
    }
    Ok(())
}
unsafe fn click(x: i32, y: i32) -> Result<(), String> {
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    for flags in [
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP,
    ] {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: ((x - vx) as i64 * 65535 / (w - 1) as i64) as i32,
                    dy: ((y - vy) as i64 * 65535 / (h - 1) as i64) as i32,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        if SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) != 1 {
            return Err("SendInput de mouse falló".into());
        }
    }
    Ok(())
}
// Atiende el fondo para que Windows no lo marque como colgado.
fn pause(duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        unsafe {
            let mut msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        thread::sleep(
            (until.saturating_duration_since(Instant::now())).min(Duration::from_millis(10)),
        );
    }
}
struct Zone {
    name: String,
    rect: RECT,
}
struct Capture {
    samples: Vec<Vec<(u64, u64)>>,
    end: u64,
    frames: usize,
    start: u64,
}
unsafe fn capture(
    rect: RECT,
    zones: Vec<Zone>,
    stop: Arc<AtomicBool>,
    freq: i64,
) -> Result<Capture, String> {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    let screen = GetDC(null_mut());
    let dc = CreateCompatibleDC(screen);
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = w;
    info.bmiHeader.biHeight = -h;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if screen.is_null() || dc.is_null() || bitmap.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !dc.is_null() {
            DeleteDC(dc);
        }
        if !screen.is_null() {
            ReleaseDC(null_mut(), screen);
        }
        return Err("No se pudo crear la captura GDI".into());
    }
    let old = SelectObject(dc, bitmap);
    let mut result = Capture {
        samples: vec![Vec::new(); zones.len()],
        end: 0,
        frames: 0,
        start: micros(counter(), freq),
    };
    let mut failed = false;
    while !stop.load(Ordering::Relaxed) {
        let t = micros(counter(), freq);
        if BitBlt(
            dc,
            0,
            0,
            w,
            h,
            screen,
            rect.left,
            rect.top,
            SRCCOPY | CAPTUREBLT,
        ) == 0
        {
            failed = true;
            break;
        }
        GdiFlush();
        let bytes = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
        for (i, z) in zones.iter().enumerate() {
            let mut hash = 0xcbf29ce484222325u64;
            for y in z.rect.top.max(0)..z.rect.bottom.min(h) {
                let a = ((y * w + z.rect.left.clamp(0, w)) * 4) as usize;
                let b = ((y * w + z.rect.right.clamp(0, w)) * 4) as usize;
                for &v in &bytes[a..b] {
                    hash = (hash ^ v as u64).wrapping_mul(0x100000001b3);
                }
            }
            result.samples[i].push((t, hash));
        }
        result.frames += 1;
    }
    result.end = micros(counter(), freq);
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
    ReleaseDC(null_mut(), screen);
    if failed {
        Err("BitBlt falló durante la captura".into())
    } else {
        Ok(result)
    }
}
fn run() -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let o = options()?;
        if !FindWindowW(wide("SimpckyController.medicion").as_ptr(), null()).is_null() {
            return Err("Ya hay una instancia de medicion abierta".into());
        }
        let root = std::env::temp_dir().join("simpcky-medicion");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let profile = root.join(&o.name);
        // No seguir enlaces al borrar un perfil anterior.
        if let Ok(meta) = std::fs::symlink_metadata(&profile) {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("El perfil es un enlace o junction".into());
            }
            std::fs::remove_dir_all(&profile).map_err(|e| e.to_string())?;
        }
        let data = profile.join("Simpcky");
        std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
        std::fs::write(data.join("settings.json"),format!(r#"{{"dark":false,"defaultRollMode":0,"desktopMenu":false,"autoUpdate":false,"translucency":{},"scale":0,"hotkey":0}}"#,if o.glass {2} else {0})).map_err(|e|e.to_string())?;
        let text=(1..=12).map(|n| format!("Línea {n}: texto de muestra suficientemente largo para ocupar más de un renglón.\\n")).collect::<String>();
        let note = format!(
            r#"[{{"id":1,"x":200,"y":200,"w":420,"h":360,"color":0,"layer":2,"rollMode":0,"rolled":false,"title":"Medición","text":"☐ tarea 😀\n{text}"}}]"#
        );
        std::fs::write(data.join("notes.json"), note).map_err(|e| e.to_string())?;
        let class = wide("SimpckyMedicionFondo");
        let brush = CreateSolidBrush(0x605040);
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hbrBackground: brush,
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let background = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Fondo de medición").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            100,
            100,
            700,
            650,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if background.is_null() {
            DeleteObject(brush);
            return Err("No se pudo crear el fondo".into());
        }
        let mut session = Session {
            child: None,
            background,
            brush,
        };
        SetForegroundWindow(background);
        UpdateWindow(background);
        let mut freq = 0;
        QueryPerformanceFrequency(&mut freq);
        let log = data.join("paint.log");
        session.child = Some(
            Command::new(&o.exe)
                .env("APPDATA", &profile)
                .env("SIMPCKY_INSTANCIA", "medicion")
                .env("SIMPCKY_PAINT_LOG", &log)
                .spawn()
                .map_err(|e| e.to_string())?,
        );
        let pid = session.child.as_ref().unwrap().id();
        let mut search = Search {
            pid,
            note: null_mut(),
        };
        let until = Instant::now() + Duration::from_secs(10);
        while search.note.is_null() && Instant::now() < until {
            EnumWindows(Some(find_top), &mut search as *mut _ as isize);
            let mut msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            pause(Duration::from_millis(50));
        }
        if search.note.is_null() {
            return Err("No apareció SimpckyNote en 10 s".into());
        }
        let edit = FindWindowExW(
            search.note,
            null_mut(),
            wide("RICHEDIT50W").as_ptr(),
            null(),
        );
        if edit.is_null() {
            return Err("No apareció RICHEDIT50W".into());
        }
        pause(Duration::from_millis(1500));
        SetForegroundWindow(search.note);
        let mut er: RECT = std::mem::zeroed();
        GetClientRect(edit, &mut er);
        let mut edit_origin = POINT { x: 0, y: 0 };
        ClientToScreen(edit, &mut edit_origin);
        er.left += edit_origin.x;
        er.right += edit_origin.x;
        er.top += edit_origin.y;
        er.bottom += edit_origin.y;
        // El punto del clic tiene que ser de la nota de prueba.
        let mut target_pid = 0;
        GetWindowThreadProcessId(WindowFromPoint(POINT { x: er.left + 20, y: er.top + 20 }), &mut target_pid);
        if target_pid != pid {
            return Err("Hay otra ventana tapando la nota de prueba: no se hace clic".into());
        }
        click(er.left + 20, er.top + 20)?;
        pause(Duration::from_millis(100));
        ensure_foreground(pid)?;
        key(VK_CONTROL, 0)?;
        key(VK_END, 0)?;
        key(VK_END, KEYEVENTF_KEYUP)?;
        key(VK_CONTROL, KEYEVENTF_KEYUP)?;
        pause(Duration::from_millis(200));
        let mut nr: RECT = std::mem::zeroed();
        GetWindowRect(search.note, &mut nr);
        // El borde nativo del RichEdit queda fuera de la nota.
        er.left = er.left.max(nr.left);
        er.right = er.right.min(nr.right);
        er.top = er.top.max(nr.top);
        er.bottom = er.bottom.min(nr.bottom);
        if nr.right <= nr.left || nr.bottom <= nr.top || er.right <= er.left || er.bottom <= er.top
        {
            return Err("La nota no tiene una region visible para capturar".into());
        }
        let dpi = GetDpiForWindow(search.note) as i32;
        let mut zones = vec![Zone {
            name: "encabezado".into(),
            rect: RECT {
                left: 0,
                top: 0,
                right: nr.right - nr.left,
                bottom: 40 * dpi / 96,
            },
        }];
        let line = (21 * dpi / 96).max(1);
        let bar = (14 * dpi / 96).max(1);
        let mut y = er.top;
        let mut row = 1;
        while y < er.bottom {
            zones.push(Zone {
                name: format!("fila {row}"),
                rect: RECT {
                    left: er.left - nr.left,
                    top: y - nr.top,
                    right: er.right - nr.left - bar,
                    bottom: (y + line).min(er.bottom) - nr.top,
                },
            });
            y += line;
            row += 1;
        }
        zones.push(Zone {
            name: "barra".into(),
            rect: RECT {
                left: er.right - nr.left - bar,
                top: er.top - nr.top,
                right: er.right - nr.left,
                bottom: er.bottom - nr.top,
            },
        });
        let names: Vec<_> = zones.iter().map(|z| z.name.clone()).collect();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = thread::spawn(move || capture(nr, zones, signal, freq));
        // Márgenes largos para clasificar también la primera y última tecla.
        pause(Duration::from_millis(o.threshold.max(100)));
        let typing = (|| -> Result<(), String> {
            for i in 0..o.keys {
                let c = b'a' as u16 + (i % 26) as u16;
                ensure_foreground(pid)?;
                key(c, KEYEVENTF_UNICODE)?;
                key(c, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)?;
                pause(Duration::from_millis(o.interval));
            }
            Ok(())
        })();
        pause(Duration::from_millis(o.threshold.max(100)));
        stop.store(true, Ordering::Relaxed);
        let captured = worker.join().map_err(|_| "Falló el hilo de captura")??;
        typing?;
        println!(
            "{} cuadros; {:.1} cuadros/s",
            captured.frames,
            captured.frames as f64 * 1_000_000.0 / (captured.end - captured.start).max(1) as f64
        );
        let mut events = Vec::new();
        for (i, samples) in captured.samples.iter().enumerate() {
            let blinks = analyze(samples, captured.end, o.threshold * 1000);
            println!(
                "{}: {} parpadeos; {:.3}/tecla",
                names[i],
                blinks.len(),
                blinks.len() as f64 / o.keys as f64
            );
            for b in blinks {
                events.push((b.start, i, b.duration));
            }
        }
        events.sort_unstable();
        println!(
            "Total: {} parpadeos de zona; {:.3}/tecla",
            events.len(),
            events.len() as f64 / o.keys as f64
        );
        drop(session);
        let contents =
            std::fs::read_to_string(&log).map_err(|e| format!("No se pudo leer paint.log: {e}"))?;
        // QPC comparte reloj entre procesos; el registro fija el origen.
        let origin = contents
            .lines()
            .find_map(|l| {
                let fields: Vec<_> = l.split('\t').collect();
                if fields.get(1) == Some(&"QPC_ORIGIN") {
                    fields.get(3)?.split(',').next()?.parse::<u64>().ok()
                } else {
                    None
                }
            })
            .ok_or("Falta QPC_ORIGIN en paint.log; recompilar el ejecutable medido")?;
        for (time, i, duration) in events.into_iter().take(30) {
            let relative = time.saturating_sub(origin);
            println!(
                "{} a {:.3} ms (duración {:.3} ms)",
                names[i],
                relative as f64 / 1000.0,
                duration as f64 / 1000.0
            );
            for l in contents.lines() {
                if let Some(t) = l.split('\t').next().and_then(|s| s.parse::<u64>().ok()) {
                    if t.abs_diff(relative) <= 30_000 {
                        println!("  {l}");
                    }
                }
            }
        }
        println!("Registro: {}", log.display());
        Ok(())
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cambio_largo() {
        assert!(analyze(&[(0, 1), (100_000, 2)], 200_000, 50_000).is_empty());
    }
    #[test]
    fn vuelve_al_anterior() {
        assert_eq!(
            analyze(&[(0, 1), (100_000, 2), (110_000, 1)], 210_000, 50_000),
            vec![Blink {
                start: 100_000,
                duration: 10_000
            }]
        );
    }
    #[test]
    fn cambia_a_otro() {
        assert_eq!(
            analyze(&[(0, 1), (100_000, 2), (110_000, 3)], 210_000, 50_000).len(),
            1
        );
    }
    #[test]
    fn cursor() {
        assert!(analyze(
            &[(0, 1), (530_000, 2), (1_060_000, 1), (1_590_000, 2)],
            2_120_000,
            50_000
        )
        .is_empty());
    }
}
