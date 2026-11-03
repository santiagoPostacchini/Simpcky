//! Cifrado de la copia de Drive. La contraseña local y las transiciones
//! pendientes se guardan juntas con DPAPI; las claves derivadas, solo en RAM.
use std::sync::{Mutex, OnceLock};

use crate::{crypto, json, persist};

const ITER: u64 = 600_000;
const MISSING: &str = "La copia de Drive está cifrada: poné la contraseña en Configuración";
const WRONG: &str = "La contraseña no coincide con la de la copia de Drive";
const PLAINTEXT: &str = "La copia de Drive no está cifrada y en esta compu sí: puede haberse desactivado el cifrado en otra compu. Revisalo en Configuración";
const DAMAGED: &str = "el documento cifrado de Drive está dañado o usa otro formato";

#[derive(Clone)]
struct Cached {
    password: String,
    salt: Vec<u8>,
    iter: u64,
    key: Vec<u8>,
}

#[derive(Clone)]
struct Vault {
    password: String,
    // Conservada hasta subir con la nueva (también al quitar el cifrado).
    previous: String,
    salt: Vec<u8>,
    pending: bool,
    accept_plaintext: bool,
    cache: Vec<Cached>,
}

impl Vault {
    fn empty() -> Self {
        Self { password: String::new(), previous: String::new(), salt: crypto::random_bytes(16), pending: false, accept_plaintext: false, cache: Vec::new() }
    }

    fn key(&mut self, password: &str, salt: &[u8], iter: u64) -> Result<Vec<u8>, String> {
        if let Some(c) = self.cache.iter().find(|c| c.password == password && c.salt == salt && c.iter == iter) {
            return Ok(c.key.clone());
        }
        let key = crypto::pbkdf2(password.as_bytes(), salt, iter, 32)?;
        // La copia remota puede cambiar de sal; no acumular claves sin límite.
        if self.cache.len() == 4 {
            self.cache.remove(0);
        }
        self.cache.push(Cached { password: password.into(), salt: salt.to_vec(), iter, key: key.clone() });
        Ok(key)
    }

    fn seal(&mut self, text: &str) -> Result<String, String> {
        if self.password.is_empty() {
            return Ok(text.into());
        }
        let salt = self.salt.clone();
        let key = self.key(&self.password.clone(), &salt, ITER)?;
        let nonce = crypto::random_bytes(12);
        let ct = crypto::gcm_encrypt(&key, &nonce, text.as_bytes())?;
        Ok(format!(
            r#"{{"format":1000,"v":2,"enc":{{"kdf":"pbkdf2-sha256","iter":{ITER},"salt":"{}","nonce":"{}","ct":"{}"}}}}"#,
            crypto::b64url(&salt),
            crypto::b64url(&nonce),
            crypto::b64url(&ct)
        ))
    }

    /// Devuelve también si hace falta reescribir por un cambio de cifrado.
    fn open(&mut self, text: &str) -> Result<(String, bool), String> {
        let j = json::parse(text).ok_or(DAMAGED)?;
        let Some(enc) = j.get("enc") else {
            if !self.password.is_empty() && !self.accept_plaintext {
                return Err(PLAINTEXT.into());
            }
            return Ok((text.into(), self.pending));
        };
        if j.u64_or("v", 0) != 2 || enc.str_or("kdf", "") != "pbkdf2-sha256" || enc.u64_or("iter", 0) != ITER {
            return Err(DAMAGED.into());
        }
        let field = |name| crypto::b64url_decode(&enc.str_or(name, "")).ok_or(DAMAGED);
        let (salt, nonce, ct) = (field("salt")?, field("nonce")?, field("ct")?);
        if salt.len() != 16 || nonce.len() != 12 || ct.len() < 16 {
            return Err(DAMAGED.into());
        }
        if self.password.is_empty() && self.previous.is_empty() {
            return Err(MISSING.into());
        }
        for previous in [false, true] {
            let password = if previous { &self.previous } else { &self.password };
            if password.is_empty() {
                continue;
            }
            let password = password.clone();
            let key = self.key(&password, &salt, ITER)?;
            if let Ok(plain) = crypto::gcm_decrypt(&key, &nonce, &ct) {
                let text = String::from_utf8(plain).map_err(|_| DAMAGED)?;
                if !self.pending {
                    self.salt = salt;
                }
                return Ok((text, self.pending));
            }
        }
        Err(WRONG.into())
    }

    fn save(&self) -> Result<(), String> {
        let text = format!(
            r#"{{"password":"{}","previous":"{}","salt":"{}","pending":{},"accept_plaintext":{}}}"#,
            json::escape(&self.password),
            json::escape(&self.previous),
            crypto::b64url(&self.salt),
            self.pending,
            self.accept_plaintext
        );
        let sealed = crypto::protect(text.as_bytes()).ok_or("no se pudo proteger la contraseña con Windows")?;
        std::fs::create_dir_all(persist::data_dir()).map_err(|e| e.to_string())?;
        if !persist::write_atomic(&path(), &crypto::b64url(&sealed)) {
            return Err("no se pudo guardar la contraseña".into());
        }
        Ok(())
    }
}

fn path() -> std::path::PathBuf {
    persist::data_dir().join("drive-password.dat")
}

fn load() -> Result<Vault, String> {
    let encoded = match std::fs::read_to_string(path()) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vault::empty()),
        Err(_) => return Err("no se pudo leer la contraseña local de Drive".into()),
    };
    let text = crypto::b64url_decode(&encoded)
        .and_then(|b| crypto::unprotect(&b))
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or("no se pudo descifrar la contraseña local de Drive")?;
    let j = json::parse(&text).ok_or("contraseña local de Drive dañada")?;
    let salt = crypto::b64url_decode(&j.str_or("salt", "")).filter(|s| s.len() == 16).ok_or("sal local de Drive dañada")?;
    Ok(Vault { password: j.str_or("password", ""), previous: j.str_or("previous", ""), salt, pending: j.bool_or("pending", false), accept_plaintext: j.bool_or("accept_plaintext", false), cache: Vec::new() })
}

static VAULT: OnceLock<Mutex<Result<Vault, String>>> = OnceLock::new();
fn vault() -> &'static Mutex<Result<Vault, String>> {
    VAULT.get_or_init(|| Mutex::new(load()))
}

pub fn enabled() -> bool {
    // El dibujo nunca espera la derivación PBKDF2 del hilo de red.
    vault().try_lock().ok().and_then(|v| v.as_ref().ok().map(|v| !v.password.is_empty())).unwrap_or(true)
}

pub fn pending() -> bool {
    vault().lock().unwrap().as_ref().map(|v| v.pending).unwrap_or(true)
}

pub fn set_password(password: &str, accept_plaintext: bool) -> Result<(), String> {
    let mut guard = vault().lock().unwrap();
    // Ante archivo ilegible se puede reemplazar la configuración explícitamente.
    let mut next = guard.as_ref().cloned().unwrap_or_else(|_| Vault::empty());
    // Si el cifrado estaba apagado, la copia de Drive está en claro: es la que se va a cifrar.
    let accept_plaintext = accept_plaintext || next.password.is_empty();
    if next.previous.is_empty() {
        next.previous = next.password.clone();
    }
    next.password = password.into();
    next.salt = crypto::random_bytes(16);
    next.pending = true;
    next.accept_plaintext = accept_plaintext;
    next.cache.clear();
    next.save()?;
    *guard = Ok(next);
    Ok(())
}

pub enum OpenError {
    Retry(String),
    Paused(String),
}

pub fn open(text: &str) -> Result<(String, bool), OpenError> {
    let mut guard = vault().lock().unwrap();
    // No conservar para siempre un fallo pasajero de lectura.
    if guard.is_err() { *guard = load(); }
    guard.as_mut().map_err(|e| OpenError::Retry(e.clone()))?.open(text).map_err(OpenError::Paused)
}

pub fn seal(text: &str) -> Result<String, String> {
    let mut guard = vault().lock().unwrap();
    if guard.is_err() { *guard = load(); }
    guard.as_mut().map_err(|e| e.clone())?.seal(text)
}

pub fn finish() -> Result<(), String> {
    let mut guard = vault().lock().unwrap();
    let v = guard.as_mut().map_err(|e| e.clone())?;
    if v.pending {
        let mut next = v.clone();
        next.previous.clear();
        next.pending = false;
        next.accept_plaintext = false;
        next.cache.retain(|c| c.password == next.password);
        next.save()?;
        *v = next;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_sync_document_roundtrip_preserves_notes_and_tombs() {
        let mut note = persist::NoteData::new(1, 10, 20, 2, persist::RollMode::Manual);
        note.text = "Texto secreto".into();
        note.fmt = "b0+5".into();
        note.t = [42; persist::PARTS];
        let tomb = persist::Tomb { uid: "borrada".into(), at: 99 };
        let doc = crate::sync::doc_json(&[note.clone()], &[tomb.clone()]);
        let mut v = Vault::empty();
        v.password = "prueba".into();
        let sealed = v.seal(&doc).unwrap();
        let (opened, _) = v.open(&sealed).unwrap();
        assert_eq!(crate::sync::doc_parse(&opened).unwrap(), crate::sync::doc_parse(&doc).unwrap());
        assert_eq!(crate::sync::doc_parse(&doc).unwrap().0[0].text, note.text);
        assert_eq!(crate::sync::doc_parse(&doc).unwrap().1, [tomb]);
        // La lectura vieja sigue usando el mismo parser.
        assert_eq!(Vault::empty().open(&doc).unwrap().0, doc);
    }

    #[test]
    fn document_roundtrip_wrong_password_and_missing() {
        let mut v = Vault::empty();
        v.password = "contraseña 🦀".into();
        let doc = r#"{"format":1,"notes":[],"deleted":[]}"#;
        let sealed = v.seal(doc).unwrap();
        assert!(!sealed.contains("notes"));
        assert!(crate::sync::doc_parse(&sealed).is_err());
        assert_eq!(v.open(&sealed).unwrap(), (doc.into(), false));
        let count = v.cache.len();
        assert_eq!(v.open(&sealed).unwrap().0, doc);
        assert_eq!(v.cache.len(), count);
        let mut other = Vault::empty();
        assert_eq!(other.open(&sealed).unwrap_err(), MISSING);
        other.password = "otra".into();
        assert_eq!(other.open(&sealed).unwrap_err(), WRONG);
        assert_eq!(v.open(doc).unwrap_err(), PLAINTEXT);
        v.pending = true;
        assert_eq!(v.open(doc).unwrap_err(), PLAINTEXT);
        v.accept_plaintext = true;
        assert_eq!(v.open(doc).unwrap(), (doc.into(), true));
        v.accept_plaintext = false;
        v.previous = std::mem::take(&mut v.password);
        assert_eq!(v.open(doc).unwrap(), (doc.into(), true));
        assert_eq!(Vault::empty().open(doc).unwrap(), (doc.into(), false));
    }

    #[test]
    fn password_rotation_and_removal_read_previous_copy() {
        let mut v = Vault::empty();
        v.password = "anterior".into();
        let doc = r#"{"format":1,"notes":[],"deleted":[]}"#;
        let sealed = v.seal(doc).unwrap();
        v.previous = v.password.clone();
        v.password = "nueva".into();
        v.pending = true;
        assert_eq!(v.open(&sealed).unwrap(), (doc.into(), true));
        let rotated = v.seal(doc).unwrap();
        v.previous.clear();
        assert_eq!(v.open(&rotated).unwrap().0, doc);
        assert_eq!(v.open(&sealed).unwrap_err(), WRONG);
        v.previous = v.password.clone();
        v.password.clear();
        assert_eq!(v.open(&rotated).unwrap(), (doc.into(), true));
        assert_eq!(v.seal(doc).unwrap(), doc);
    }

    #[test]
    fn malformed_envelopes_fail_closed() {
        for doc in [r#"{"v":2,"enc":null}"#, r#"{"v":3,"enc":{}}"#, r#"{"v":2,"enc":{"kdf":"pbkdf2-sha256","iter":600000,"salt":"!"}}"#] {
            assert!(Vault::empty().open(doc).is_err());
        }
    }
}
