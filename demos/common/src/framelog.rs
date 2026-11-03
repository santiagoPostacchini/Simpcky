//! Registro de cuadros para medir la fluidez: con `SIMPCKY_FRAMES=<archivo>`
//! cada demo anota cuándo presentó cada cosa (`nota`, `capa`, `paso`) y
//! cuánto tardó en dibujarla. `bench/smooth.py` lo lee.

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

struct Log {
    start: Instant,
    file: Option<std::fs::File>,
}

fn log() -> &'static Mutex<Log> {
    static L: OnceLock<Mutex<Log>> = OnceLock::new();
    L.get_or_init(|| {
        let file = std::env::var_os("SIMPCKY_FRAMES").and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
        Mutex::new(Log { start: Instant::now(), file })
    })
}

pub fn enabled() -> bool {
    log().lock().map(|l| l.file.is_some()).unwrap_or(false)
}

/// `kind`: qué se presentó; `id`: la nota (0 si no es una); `cost`: cuánto
/// llevó dibujarlo.
pub fn mark(kind: &str, id: u32, cost: std::time::Duration) {
    if let Ok(mut l) = log().lock() {
        let t = l.start.elapsed().as_micros();
        if let Some(f) = l.file.as_mut() {
            let _ = writeln!(f, "{t} {kind} {id} {}", cost.as_micros());
        }
    }
}
