//! Inicio de sesión con Google (OAuth 2.0 para apps de escritorio, tal
//! como lo recomienda Google para apps instaladas):
//!
//! 1. Se abre el navegador del sistema en la página de Google. El
//!    usuario inicia sesión ahí: Simpcky nunca ve su contraseña.
//! 2. Google redirige a `http://127.0.0.1:<puerto>` — un servidorcito
//!    que la app levanta solo durante el inicio de sesión, escuchando
//!    únicamente en esta máquina — con un código de un solo uso.
//! 3. La app cambia ese código por los tokens. PKCE (un secreto al azar
//!    que nunca sale de la app, del que Google solo vio el hash) hace
//!    que el código no le sirva a nadie que lo intercepte.
//!
//! El permiso que se pide es `drive.appdata`: una carpeta oculta del
//! Drive del usuario, que solo ve esta app — Simpcky no puede ver ni
//! tocar ningún otro archivo del Drive. Más `openid email`, para poder
//! mostrar con qué cuenta quedó conectado.
//!
//! El refresh token (lo que permite seguir sincronizando sin volver a
//! iniciar sesión) se guarda cifrado con DPAPI, atado a la cuenta de
//! Windows (ver `crypto.rs`).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::crypto;
use crate::http;
use crate::win::wide;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const SCOPES: &str = "openid email https://www.googleapis.com/auth/drive.appdata";
/// Cuánto se espera a que el usuario termine en el navegador.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// El cliente OAuth de esta compilación, embebido por `build.rs` desde
/// `google_client.json` (o variables de entorno). Una compilación sin
/// él funciona igual, solo sin sincronización.
///
/// Para una app de escritorio el "client secret" no es secreto de
/// verdad (viaja dentro del .exe, y Google lo sabe: por eso existe
/// PKCE); igual no se sube al repositorio.
fn client() -> Option<(&'static str, &'static str)> {
    match (option_env!("SIMPCKY_GOOGLE_CLIENT_ID"), option_env!("SIMPCKY_GOOGLE_CLIENT_SECRET")) {
        (Some(id), Some(secret)) if !id.is_empty() => Some((id, secret)),
        _ => None,
    }
}

pub fn is_configured() -> bool {
    client().is_some()
}

#[derive(Debug)]
pub enum AuthError {
    /// No hay cuenta conectada.
    NotConnected,
    /// Google ya no acepta la credencial guardada (el usuario revocó el
    /// acceso desde su cuenta, o cambió la contraseña): hay que volver
    /// a conectar.
    Revoked,
    /// Sin red, o Google no contestó: se reintenta más tarde.
    Network(String),
}

/// Access token en memoria y cuándo vence (ms desde 1970). Dura una
/// hora; no se guarda en disco.
static ACCESS: Mutex<Option<(String, u64)>> = Mutex::new(None);

fn token_path() -> PathBuf {
    crate::persist::data_dir().join("google.dat")
}

pub fn has_account() -> bool {
    token_path().exists()
}

fn load_refresh_token() -> Option<String> {
    let sealed = std::fs::read(token_path()).ok()?;
    String::from_utf8(crypto::unprotect(&sealed)?).ok()
}

fn save_refresh_token(token: &str) -> Result<(), String> {
    let sealed = crypto::protect(token.as_bytes()).ok_or("no se pudo cifrar la credencial")?;
    std::fs::create_dir_all(crate::persist::data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(token_path(), sealed).map_err(|e| e.to_string())
}

/// Olvida la cuenta en esta compu (sin avisarle a Google).
pub fn forget() {
    let _ = std::fs::remove_file(token_path());
    *ACCESS.lock().unwrap() = None;
}

/// Tras un 401 de la API: el access token en memoria ya no sirve.
pub fn invalidate_access() {
    *ACCESS.lock().unwrap() = None;
}

/// Un access token válido, renovándolo si hace falta.
pub fn access_token() -> Result<String, AuthError> {
    let now = crate::persist::now_ms();
    if let Some((token, expires)) = ACCESS.lock().unwrap().clone() {
        if expires > now + 60_000 {
            return Ok(token);
        }
    }
    let (id, secret) = client().ok_or(AuthError::NotConnected)?;
    let refresh = load_refresh_token().ok_or(AuthError::NotConnected)?;
    let body = http::form(&[
        ("client_id", id),
        ("client_secret", secret),
        ("refresh_token", &refresh),
        ("grant_type", "refresh_token"),
    ]);
    let r = http::request(
        "POST",
        TOKEN_URL,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.as_bytes(),
    )
    .map_err(AuthError::Network)?;
    let json = r.json();
    if !r.ok() {
        let err = json.as_ref().map(|j| j.str_or("error", "")).unwrap_or_default();
        return Err(if err == "invalid_grant" || err == "unauthorized_client" {
            AuthError::Revoked
        } else {
            AuthError::Network(format!("Google respondió {} ({err})", r.status))
        });
    }
    let json = json.ok_or_else(|| AuthError::Network("respuesta ilegible de Google".into()))?;
    let token = json.str_or("access_token", "");
    if token.is_empty() {
        return Err(AuthError::Network("Google no devolvió un token".into()));
    }
    let expires = now + json.u64_or("expires_in", 3600) * 1000;
    *ACCESS.lock().unwrap() = Some((token.clone(), expires));
    Ok(token)
}

/// Inicio de sesión completo (bloqueante: correrlo en un hilo aparte).
/// Devuelve el email de la cuenta conectada. `cancel` corta la espera.
pub fn sign_in(cancel: &AtomicBool) -> Result<String, String> {
    let (id, secret) = client().ok_or("esta compilación no tiene la sincronización configurada")?;

    let verifier = crypto::b64url(&crypto::random_bytes(32));
    let challenge = crypto::b64url(&crypto::sha256(verifier.as_bytes()));
    let state = crypto::b64url(&crypto::random_bytes(16));

    // Puerto elegido por el sistema, solo en la interfaz local.
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("no se pudo abrir el puerto local: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");

    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        http::url_encode(id),
        http::url_encode(&redirect),
        http::url_encode(SCOPES),
        challenge,
        http::url_encode(&state),
    );
    open_browser(&url)?;

    let code = wait_for_code(&listener, &state, cancel)?;

    let body = http::form(&[
        ("code", &code),
        ("client_id", id),
        ("client_secret", secret),
        ("redirect_uri", &redirect),
        ("grant_type", "authorization_code"),
        ("code_verifier", &verifier),
    ]);
    let r = http::request(
        "POST",
        TOKEN_URL,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.as_bytes(),
    )?;
    let json = r.json().ok_or("respuesta ilegible de Google")?;
    if !r.ok() {
        return Err(format!("Google rechazó el inicio de sesión: {}", json.str_or("error_description", &json.str_or("error", "?"))));
    }
    let refresh = json.str_or("refresh_token", "");
    if refresh.is_empty() {
        return Err("Google no devolvió una credencial duradera; probá de nuevo".into());
    }
    save_refresh_token(&refresh)?;
    let access = json.str_or("access_token", "");
    let expires = crate::persist::now_ms() + json.u64_or("expires_in", 3600) * 1000;
    *ACCESS.lock().unwrap() = Some((access, expires));

    Ok(email_from_id_token(&json.str_or("id_token", "")).unwrap_or_else(|| "tu cuenta de Google".into()))
}

/// El id_token llega directo de Google por HTTPS (no de un tercero), así
/// que alcanza con leer su contenido para saber el email; no hace falta
/// verificarle la firma.
fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = crypto::b64url_decode(payload)?;
    let json = crate::json::parse(std::str::from_utf8(&bytes).ok()?)?;
    let email = json.str_or("email", "");
    (!email.is_empty()).then_some(email)
}

/// Desconecta: le pide a Google que invalide la credencial y la borra de
/// esta compu. Si no hay red, igual se borra localmente.
pub fn sign_out() {
    if let Some(refresh) = load_refresh_token() {
        let body = http::form(&[("token", &refresh)]);
        let _ = http::request(
            "POST",
            REVOKE_URL,
            &[("Content-Type", "application/x-www-form-urlencoded")],
            body.as_bytes(),
        );
    }
    forget();
}

fn open_browser(url: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        // ShellExecute pide COM inicializado en el hilo que lo llama.
        CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32);
        let verb = wide("open");
        let target = wide(url);
        let r = ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        if (r as isize) <= 32 {
            return Err("no se pudo abrir el navegador".into());
        }
    }
    Ok(())
}

/// Espera la redirección del navegador y devuelve el código. Ignora
/// cualquier otra cosa que pida el navegador (el favicon, por ejemplo).
fn wait_for_code(listener: &TcpListener, state: &str, cancel: &AtomicBool) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("inicio de sesión cancelado".into());
        }
        if Instant::now() > deadline {
            return Err("se agotó el tiempo esperando al navegador".into());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = handle_callback(stream, state) {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(150)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `None` si la petición no era la redirección (se sigue esperando).
fn handle_callback(mut stream: TcpStream, state: &str) -> Option<Result<String, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let target = request.lines().next()?.split_whitespace().nth(1)?.to_string();
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let param = |name: &str| {
        query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == name).then(|| url_decode(v))
        })
    };

    if let Some(error) = param("error") {
        respond(&mut stream, false);
        return Some(Err(if error == "access_denied" {
            "no se dio permiso en la página de Google".into()
        } else {
            format!("Google devolvió un error: {error}")
        }));
    }
    match (param("code"), param("state")) {
        (Some(code), Some(s)) if s == state => {
            respond(&mut stream, true);
            Some(Ok(code))
        }
        (Some(_), _) => {
            // Un código con otro `state`: no lo pidió esta app.
            respond(&mut stream, false);
            Some(Err("respuesta inesperada del navegador".into()))
        }
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            None
        }
    }
}

fn respond(stream: &mut TcpStream, ok: bool) {
    let (title, text) = if ok {
        ("Listo", "Simpcky quedó conectado con tu cuenta de Google. Ya podés cerrar esta pestaña y volver a tus notas.")
    } else {
        ("No se pudo conectar", "Simpcky no quedó conectado. Podés cerrar esta pestaña y volver a intentarlo desde Configuración y sincronización (ícono de la bandeja).")
    };
    let html = format!(
        "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\"><title>Simpcky — {title}</title>\
<meta name=\"viewport\" content=\"width=device-width\"><style>\
:root{{color-scheme:light dark}}body{{margin:0;min-height:100vh;display:grid;place-items:center;\
font:16px/1.5 'Segoe UI',system-ui,sans-serif;background:#F8FAFC;color:#1F2937}}\
@media (prefers-color-scheme:dark){{body{{background:#1F2023;color:#E5E7EB}}}}\
.card{{max-width:420px;margin:24px;padding:28px 32px;border-radius:12px;background:#FEF9C3;color:#1F2937;\
border-top:14px solid #FDE68A}}h1{{font-size:20px;margin:0 0 8px}}p{{margin:0}}</style></head>\
<body><div class=\"card\"><h1>{title}</h1><p>{text}</p></div></body></html>"
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        html.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(html.as_bytes());
    let _ = stream.flush();
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = if bytes[i] == b'%' {
            bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        } else {
            None
        };
        match (bytes[i], escaped) {
            (_, Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (b, None) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn decodes_query_values() {
        assert_eq!(url_decode("4%2F0Ab_x-y"), "4/0Ab_x-y");
        assert_eq!(url_decode("a+b%C3%B1"), "a bñ");
        assert_eq!(url_decode("100%"), "100%");
    }

    #[test]
    fn email_from_token_payload() {
        let payload = crypto::b64url(br#"{"email":"ana@example.com","sub":"1"}"#);
        assert_eq!(email_from_id_token(&format!("x.{payload}.y")).as_deref(), Some("ana@example.com"));
        assert_eq!(email_from_id_token("basura"), None);
    }

    /// Simula al navegador: primero pide el favicon, después llega la
    /// redirección con el código.
    #[test]
    fn loopback_receives_code_and_ignores_favicon() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = thread::spawn(move || {
            let get = |path: &str| {
                let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).unwrap();
                let mut resp = String::new();
                let _ = s.read_to_string(&mut resp);
                resp
            };
            let fav = get("/favicon.ico");
            let page = get("/?state=abc123&code=4%2F0Ab-codigo&scope=email");
            (fav, page)
        });
        let cancel = AtomicBool::new(false);
        let code = wait_for_code(&listener, "abc123", &cancel).unwrap();
        let (fav, page) = browser.join().unwrap();
        assert_eq!(code, "4/0Ab-codigo");
        assert!(fav.starts_with("HTTP/1.1 404"));
        assert!(page.contains("quedó conectado"));
    }

    #[test]
    fn loopback_rejects_wrong_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /?state=otro&code=x HTTP/1.1\r\n\r\n").unwrap();
            let mut resp = String::new();
            let _ = s.read_to_string(&mut resp);
        });
        let cancel = AtomicBool::new(false);
        assert!(wait_for_code(&listener, "abc123", &cancel).is_err());
    }
}
