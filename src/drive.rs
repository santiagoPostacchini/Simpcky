//! Google Drive API v3, limitada a la carpeta oculta de la app
//! (`appDataFolder`): el usuario no la ve en su Drive y ninguna otra app
//! puede leerla. Adentro hay un solo archivo, `simpcky-notes.json`, con
//! todas las notas y las "lápidas" de las borradas (ver `sync.rs`).

use crate::http::{self, url_encode, Response};
use crate::oauth::{self, AuthError};

const FILE_NAME: &str = "simpcky-notes.json";
const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";

/// El archivo remoto: su id en Drive y el MD5 de su contenido (cambia
/// cada vez que otra compu lo actualiza; así se sabe si hay novedades
/// sin bajarlo entero).
#[derive(Clone, Debug)]
pub struct Remote {
    pub id: String,
    pub md5: String,
}

#[derive(Debug)]
pub enum DriveError {
    Auth(AuthError),
    Http(String),
}

impl From<AuthError> for DriveError {
    fn from(e: AuthError) -> Self {
        DriveError::Auth(e)
    }
}

/// Petición autenticada. Si el token venció entre medio (401), se
/// renueva y se reintenta una vez.
fn authed(method: &str, url: &str, content_type: Option<&str>, body: &[u8]) -> Result<Response, DriveError> {
    for attempt in 0..2 {
        let token = oauth::access_token()?;
        let auth = format!("Bearer {token}");
        let mut headers: Vec<(&str, &str)> = vec![("Authorization", &auth)];
        if let Some(ct) = content_type {
            headers.push(("Content-Type", ct));
        }
        let r = http::request(method, url, &headers, body).map_err(DriveError::Http)?;
        if r.status == 401 && attempt == 0 {
            oauth::invalidate_access();
            continue;
        }
        return Ok(r);
    }
    unreachable!()
}

fn api_error(what: &str, r: &Response) -> DriveError {
    let detail = r
        .json()
        .and_then(|j| j.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str().map(str::to_string)))
        .unwrap_or_default();
    DriveError::Http(format!("{what}: Drive respondió {} {detail}", r.status))
}

fn remote_from(r: &Response) -> Option<Remote> {
    let j = r.json()?;
    Some(Remote { id: j.str_or("id", ""), md5: j.str_or("md5Checksum", "") }).filter(|x| !x.id.is_empty())
}

/// Busca el archivo de notas. Si por alguna carrera hubiera más de uno,
/// se queda con el modificado más recientemente.
pub fn find() -> Result<Option<Remote>, DriveError> {
    let query = format!("name='{FILE_NAME}' and trashed=false");
    let url = format!(
        "{FILES_URL}?spaces=appDataFolder&q={}&fields={}&orderBy={}&pageSize=10",
        url_encode(&query),
        url_encode("files(id,md5Checksum)"),
        url_encode("modifiedTime desc"),
    );
    let r = authed("GET", &url, None, &[])?;
    if !r.ok() {
        return Err(api_error("buscar", &r));
    }
    let json = r.json().ok_or_else(|| DriveError::Http("lista ilegible".into()))?;
    Ok(json.get("files").and_then(|f| f.as_array()).and_then(|files| files.first()).map(|f| Remote {
        id: f.str_or("id", ""),
        md5: f.str_or("md5Checksum", ""),
    }))
}

pub fn download(id: &str) -> Result<String, DriveError> {
    let r = authed("GET", &format!("{FILES_URL}/{}?alt=media", url_encode(id)), None, &[])?;
    if !r.ok() {
        return Err(api_error("bajar", &r));
    }
    Ok(r.text())
}

/// Crea el archivo (primera vez que se sincroniza con esta cuenta).
pub fn create(content: &str) -> Result<Remote, DriveError> {
    // Subida "multipart": metadatos (nombre y carpeta) y contenido en
    // una sola petición.
    let boundary = format!("simpcky-{}", crate::crypto::hex(&crate::crypto::random_bytes(8)));
    let body = format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n\
{{\"name\":\"{FILE_NAME}\",\"parents\":[\"appDataFolder\"]}}\r\n\
--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{content}\r\n--{boundary}--\r\n"
    );
    let url = format!("{UPLOAD_URL}?uploadType=multipart&fields={}", url_encode("id,md5Checksum"));
    let ct = format!("multipart/related; boundary={boundary}");
    let r = authed("POST", &url, Some(&ct), body.as_bytes())?;
    if !r.ok() {
        return Err(api_error("crear", &r));
    }
    remote_from(&r).ok_or_else(|| DriveError::Http("Drive no devolvió el archivo creado".into()))
}

/// Reemplaza el contenido. `Ok(None)` si el archivo ya no existe (lo
/// borraron): el que llama lo vuelve a crear.
pub fn update(id: &str, content: &str) -> Result<Option<Remote>, DriveError> {
    let url = format!("{UPLOAD_URL}/{}?uploadType=media&fields={}", url_encode(id), url_encode("id,md5Checksum"));
    let r = authed("PATCH", &url, Some("application/json; charset=UTF-8"), content.as_bytes())?;
    if r.status == 404 {
        return Ok(None);
    }
    if !r.ok() {
        return Err(api_error("actualizar", &r));
    }
    Ok(remote_from(&r))
}
