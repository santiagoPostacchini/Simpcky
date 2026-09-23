//! Actualizaciones desde los Releases de GitHub.
//!
//! - Un rato después de arrancar, y después una vez por día, consulta el
//!   último release del repositorio (la API pública de GitHub: no hace
//!   falta ninguna cuenta). Una compilación de desarrollo (el .exe
//!   corriendo desde `target\`) no busca sola, para no reemplazarse.
//! - Si hay una versión más nueva, avisa (notificación + ítem en el menú
//!   de la bandeja). No se instala nada sin que el usuario lo pida.
//! - Al actualizar: baja el instalador y su huella SHA-256 (publicada en
//!   el mismo release por GitHub Actions), verifica que coincidan, y
//!   recién ahí lo ejecuta en modo silencioso. El instalador cierra esta
//!   instancia y abre la nueva (ver `installer/simpcky.iss`).
//!
//! Todo lo que usa la red corre en un hilo aparte; el resultado vuelve
//! a la ventana controladora con un mensaje.

use std::path::PathBuf;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::app;
use crate::http;
use crate::win::wide;

/// Terminó una búsqueda (`lparam` = `Box<CheckResult>`).
pub const WM_APP_UPDATE_CHECKED: u32 = WM_APP + 12;
/// Terminó la descarga (`lparam` = `Box<Result<PathBuf, String>>`).
pub const WM_APP_UPDATE_DOWNLOADED: u32 = WM_APP + 13;
pub const TIMER_UPDATE_CHECK: usize = 30;
/// La primera búsqueda, un rato después de arrancar (no compite con el
/// arranque ni con la primera sincronización).
const FIRST_CHECK_MS: u32 = 30_000;
const CHECK_EVERY_MS: u32 = 24 * 3600 * 1000;

pub type Version = (u32, u32, u32);

#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    pub setup_name: String,
    setup_url: String,
    sha_url: String,
}

type CheckResult = Result<Option<Release>, String>;

struct Runtime {
    available: Option<Release>,
    busy: bool,
}

static RT: Mutex<Runtime> = Mutex::new(Runtime { available: None, busy: false });

pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

pub fn version_text(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// "v1.2.3", "1.2.3" o "1.2" → (1, 2, 3). Cualquier sufijo ("-beta")
/// se ignora.
pub fn parse_version(s: &str) -> Option<Version> {
    let core = s.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// `dueño/repo` en GitHub, del `repository` de Cargo.toml.
fn repo() -> Option<&'static str> {
    env!("CARGO_PKG_REPOSITORY").strip_prefix("https://github.com/").map(|r| r.trim_end_matches('/'))
}

/// El .exe corre desde la carpeta de compilación (`cargo build`): no
/// buscar actualizaciones solo, que instalarían otra copia aparte.
fn is_dev_build() -> bool {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_lowercase().contains("\\target\\"))
        .unwrap_or(false)
}

/// El release más nuevo, si es más nuevo que esta versión y trae el
/// instalador con su huella.
fn fetch_latest() -> CheckResult {
    let repo = repo().ok_or("esta compilación no sabe de qué repositorio actualizarse")?;
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let r = http::request(
        "GET",
        &url,
        &[("Accept", "application/vnd.github+json"), ("X-GitHub-Api-Version", "2022-11-28")],
        &[],
    )?;
    if r.status == 404 {
        return Ok(None); // todavía no hay ningún release
    }
    if !r.ok() {
        return Err(format!("GitHub respondió {}", r.status));
    }
    let j = r.json().ok_or("respuesta ilegible de GitHub")?;
    let version = parse_version(&j.str_or("tag_name", "")).ok_or("el release no tiene un número de versión")?;
    if version <= current() {
        return Ok(None);
    }
    let assets = j.get("assets").and_then(|a| a.as_array()).unwrap_or(&[]);
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find(|a| pred(&a.str_or("name", ""))).map(|a| (a.str_or("name", ""), a.str_or("browser_download_url", "")))
    };
    let (setup_name, setup_url) =
        find(&|n| n.starts_with("Simpcky-Setup-") && n.ends_with(".exe")).ok_or("el release no trae el instalador")?;
    let sha_name = format!("{setup_name}.sha256");
    let (_, sha_url) = find(&|n| n == sha_name).ok_or("el release no trae la huella del instalador")?;
    Ok(Some(Release { version, setup_name, setup_url, sha_url }))
}

/// Baja el instalador y lo verifica contra su huella. Devuelve dónde
/// quedó guardado.
fn download(rel: &Release) -> Result<PathBuf, String> {
    let sha = http::request("GET", &rel.sha_url, &[], &[])?;
    if !sha.ok() {
        return Err(format!("no se pudo bajar la huella (GitHub respondió {})", sha.status));
    }
    // Formato de sha256sum: "<hex>  <nombre>".
    let expected = sha.text().split_whitespace().next().unwrap_or("").to_lowercase();
    if expected.len() != 64 {
        return Err("la huella publicada no es válida".into());
    }
    let setup = http::request("GET", &rel.setup_url, &[], &[])?;
    if !setup.ok() {
        return Err(format!("no se pudo bajar el instalador (GitHub respondió {})", setup.status));
    }
    if !setup.body.starts_with(b"MZ") {
        return Err("lo que se bajó no es un programa de Windows".into());
    }
    let actual = crate::crypto::hex(&crate::crypto::sha256(&setup.body));
    if actual != expected {
        return Err("el instalador descargado no coincide con su huella: se descartó".into());
    }
    let path = std::env::temp_dir().join(&rel.setup_name);
    std::fs::write(&path, &setup.body).map_err(|e| format!("no se pudo guardar el instalador: {e}"))?;
    Ok(path)
}

fn controller() -> HWND {
    app().lock().unwrap().controller_hwnd as HWND
}

/// Corre `work` en otro hilo y le manda el resultado a la controladora.
fn in_background<T: Send + 'static>(msg: u32, wparam: usize, work: impl FnOnce() -> T + Send + 'static) {
    let hwnd = controller() as isize;
    std::thread::spawn(move || {
        let ptr = Box::into_raw(Box::new(work()));
        if unsafe { PostMessageW(hwnd as HWND, msg, wparam, ptr as isize) } == 0 {
            drop(unsafe { Box::from_raw(ptr) });
        }
    });
}

// -----------------------------------------------------------------
// Lo que ve el resto de la app
// -----------------------------------------------------------------

pub fn on_startup() {
    if !is_dev_build() {
        unsafe { SetTimer(controller(), TIMER_UPDATE_CHECK, FIRST_CHECK_MS, None) };
    }
}

pub fn on_timer() {
    // La primera vez dispara a los 30 s; de ahí en más, una por día.
    unsafe { SetTimer(controller(), TIMER_UPDATE_CHECK, CHECK_EVERY_MS, None) };
    check(false);
}

/// "Buscar actualizaciones" del menú: avisa también si no hay nada.
pub fn check_now() {
    check(true);
}

fn check(manual: bool) {
    {
        let mut rt = RT.lock().unwrap();
        if rt.busy {
            return;
        }
        rt.busy = true;
    }
    in_background(WM_APP_UPDATE_CHECKED, manual as usize, fetch_latest);
}

pub fn on_checked(wparam: usize, lparam: isize) {
    let manual = wparam != 0;
    let result = *unsafe { Box::from_raw(lparam as *mut CheckResult) };
    let mut rt = RT.lock().unwrap();
    rt.busy = false;
    match result {
        Ok(Some(rel)) => {
            let v = version_text(rel.version);
            let already_known = rt.available.as_ref().is_some_and(|r| r.version == rel.version);
            rt.available = Some(rel);
            drop(rt);
            // Una sola notificación por versión (salvo que la pidan).
            if manual || !already_known {
                crate::tray::notify_action(
                    &format!("Hay una versión nueva de Simpcky: {v}"),
                    "Hacé clic acá (o usá el menú de la bandeja) para actualizar. Tus notas no se tocan.",
                    install,
                );
            }
        }
        Ok(None) => {
            drop(rt);
            if manual {
                crate::tray::notify("Simpcky está al día", &format!("Tenés la versión más nueva ({}).", version_text(current())));
            }
        }
        Err(e) => {
            drop(rt);
            if manual {
                crate::tray::notify("No se pudo buscar actualizaciones", &e);
            }
        }
    }
}

/// La versión nueva disponible, para el menú de la bandeja.
pub fn available() -> Option<Version> {
    RT.lock().unwrap().available.as_ref().map(|r| r.version)
}

/// "Actualizar a la versión X…".
pub fn install() {
    let rel = {
        let mut rt = RT.lock().unwrap();
        if rt.busy {
            return;
        }
        let Some(rel) = rt.available.clone() else { return };
        rt.busy = true;
        rel
    };
    crate::tray::notify(
        &format!("Actualizando Simpcky a la versión {}", version_text(rel.version)),
        "Descargando… Simpcky se va a cerrar y volver a abrir solo.",
    );
    in_background(WM_APP_UPDATE_DOWNLOADED, 0, move || download(&rel));
}

pub fn on_downloaded(lparam: isize) {
    let result = *unsafe { Box::from_raw(lparam as *mut Result<PathBuf, String>) };
    RT.lock().unwrap().busy = false;
    match result {
        Ok(path) => {
            if launch_installer(&path) {
                // El instalador reemplaza el .exe y vuelve a abrir la app
                // (/RELAUNCH): esta instancia se va, guardando todo.
                crate::tray::quit_app();
            } else {
                crate::tray::notify("No se pudo actualizar", "No se pudo ejecutar el instalador descargado.");
            }
        }
        Err(e) => crate::tray::notify("No se pudo actualizar", &e),
    }
}

fn launch_installer(path: &std::path::Path) -> bool {
    use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let file = wide(&path.to_string_lossy());
    let args = wide("/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH");
    let verb = wide("open");
    unsafe {
        CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32);
        let r = ShellExecuteW(controller(), verb.as_ptr(), file.as_ptr(), args.as_ptr(), std::ptr::null(), SW_SHOWNORMAL);
        (r as isize) > 32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v0.3.0"), Some((0, 3, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v2.0.1-beta.1"), Some((2, 0, 1)));
        assert_eq!(parse_version("nada"), None);
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
    }

    #[test]
    fn knows_its_repository() {
        assert_eq!(repo(), Some("santiagoPostacchini/Simpcky"));
    }

    /// Red de verdad: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn github_api_answers() {
        // Sin releases todavía da Ok(None); con releases, Ok(algo). Lo
        // que no puede pasar es un error.
        assert!(fetch_latest().is_ok(), "{:?}", fetch_latest());
    }
}
