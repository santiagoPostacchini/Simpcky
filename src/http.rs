//! Cliente HTTPS mínimo sobre WinHTTP, la pila HTTP que trae Windows:
//! TLS, proxies del sistema y certificados salen del propio sistema
//! operativo, sin sumar ni un kilobyte de biblioteca al binario.
//!
//! Solo síncrono, y pensado para correr en un hilo aparte (ver
//! `sync.rs`): nunca desde el hilo de las ventanas, que se congelaría
//! mientras espera la red.

use std::ptr::{null, null_mut};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Networking::WinHttp::*;

use crate::json::Json;
use crate::win::wide;

pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Option<Json> {
        crate::json::parse(&self.text())
    }
}

/// La sesión de WinHTTP se abre una vez y se reusa (sus handles son
/// seguros entre hilos).
fn session() -> Result<*mut core::ffi::c_void, String> {
    static SESSION: OnceLock<usize> = OnceLock::new();
    let h = *SESSION.get_or_init(|| unsafe {
        let agent = wide(concat!("Simpcky/", env!("CARGO_PKG_VERSION")));
        // Proxy automático (el mismo que usa el resto de Windows);
        // en versiones viejas sin esa opción, el configurado a mano.
        let mut h = WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0);
        if h.is_null() {
            h = WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0);
        }
        if !h.is_null() {
            WinHttpSetTimeouts(h, 10_000, 15_000, 30_000, 30_000);
        }
        h as usize
    });
    if h == 0 {
        Err("no se pudo iniciar WinHTTP".into())
    } else {
        Ok(h as *mut _)
    }
}

/// Cierra un handle de WinHTTP al salir de alcance.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// `https://host[:puerto]/ruta?consulta` → (host, puerto, ruta+consulta).
fn split_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (authority, INTERNET_DEFAULT_HTTPS_PORT),
    };
    Some((host.to_string(), port, path.to_string()))
}

/// Un error de red en palabras que se le pueden mostrar al usuario.
fn describe(stage: &str) -> String {
    let code = unsafe { GetLastError() };
    let what = match code {
        12007 => "sin conexión a internet",
        12002 => "la conexión tardó demasiado",
        12029 | 12030 => "no se pudo conectar",
        12175 | 12045 | 12038 => "el certificado del servidor no es válido",
        _ => "error de red",
    };
    format!("{what} ({stage}, {code})")
}

/// Hace una petición HTTPS y devuelve el estado y el cuerpo. Cualquier
/// código HTTP (también 4xx/5xx) es un `Ok`: el que llama decide qué
/// hacer con él. `Err` es solo cuando no hubo respuesta.
pub fn request(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response, String> {
    let (host, port, path) = split_url(url).ok_or_else(|| format!("URL no soportada: {url}"))?;
    let session = session()?;
    unsafe {
        let host_w = wide(&host);
        let connect = Handle(WinHttpConnect(session, host_w.as_ptr(), port, 0));
        if connect.0.is_null() {
            return Err(describe("conectar"));
        }
        let verb = wide(method);
        let path_w = wide(&path);
        let req = Handle(WinHttpOpenRequest(connect.0, verb.as_ptr(), path_w.as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return Err(describe("abrir"));
        }

        let header_text: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        let header_w = wide(&header_text);
        let sent = WinHttpSendRequest(
            req.0,
            if header_text.is_empty() { null() } else { header_w.as_ptr() },
            if header_text.is_empty() { 0 } else { u32::MAX }, // -1: terminada en NUL
            if body.is_empty() { null() } else { body.as_ptr() as *const _ },
            body.len() as u32,
            body.len() as u32,
            0,
        );
        if sent == 0 {
            return Err(describe("enviar"));
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(describe("recibir"));
        }

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut _,
            &mut size,
            null_mut(),
        );

        let mut out = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(req.0, &mut available) == 0 {
                return Err(describe("leer"));
            }
            if available == 0 {
                break;
            }
            let start = out.len();
            out.resize(start + available as usize, 0);
            let mut read: u32 = 0;
            if WinHttpReadData(req.0, out[start..].as_mut_ptr() as *mut _, available, &mut read) == 0 {
                return Err(describe("leer"));
            }
            out.truncate(start + read as usize);
        }
        Ok(Response { status, body: out })
    }
}

/// Codifica para una URL o un formulario (RFC 3986: todo lo que no sea
/// letra, número o `-._~` va como `%XX`).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Cuerpo `application/x-www-form-urlencoded`.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect::<Vec<_>>().join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_urls() {
        assert_eq!(
            split_url("https://oauth2.googleapis.com/token").unwrap(),
            ("oauth2.googleapis.com".into(), 443, "/token".into())
        );
        assert_eq!(split_url("https://h:8443").unwrap(), ("h".into(), 8443, "/".into()));
        assert!(split_url("http://inseguro.com/").is_none());
    }

    #[test]
    fn encodes() {
        assert_eq!(url_encode("a b/ñ"), "a%20b%2F%C3%B1");
        assert_eq!(form(&[("q", "name='x'")]), "q=name%3D%27x%27");
    }

    /// Red de verdad: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_https_request() {
        // Sin credenciales, la API de Drive contesta 401 con un JSON de
        // error: alcanza para probar TLS, estado y lectura del cuerpo.
        let r = request("GET", "https://www.googleapis.com/drive/v3/about?fields=user", &[], &[]).unwrap();
        assert_eq!(r.status, 401);
        assert!(r.json().and_then(|j| j.get("error").cloned()).is_some());
    }
}
