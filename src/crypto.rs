//! Lo poco de criptografía que necesita la sincronización, todo con lo
//! que ya trae Windows (CNG y DPAPI): cero dependencias, cero peso.
//!
//! - Números aleatorios seguros: identificadores de nota, y el
//!   verificador y el `state` del inicio de sesión con Google.
//! - SHA-256: el `code_challenge` de PKCE.
//! - Base64url: el formato que piden PKCE y los tokens de Google.
//! - DPAPI: guardar la credencial de Google cifrada con la cuenta de
//!   Windows del usuario — ni otro usuario de la compu, ni alguien que
//!   se lleve el archivo a otra máquina, puede leerla.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::*;

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let status = unsafe {
        BCryptGenRandom(null_mut(), buf.as_mut_ptr(), n as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG)
    };
    assert!(status >= 0, "BCryptGenRandom falló: {status:#x}");
    buf
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let status = unsafe {
        BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32)
    };
    assert!(status >= 0, "BCryptHash falló: {status:#x}");
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url sin relleno (RFC 4648 §5), como piden PKCE y los JWT.
pub fn b64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let chars = chunk.len() + 1;
        for k in 0..chars {
            out.push(B64URL[((n >> (18 - 6 * k)) & 63) as usize] as char);
        }
    }
    out
}

/// Decodifica base64url (con o sin relleno; también acepta `+` y `/`).
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let mut n = 0u32;
        for (k, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * k);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// Cifra con DPAPI, atado a la cuenta de Windows actual.
pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, true)
}

pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, false)
}

fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        let ok = if encrypt {
            CryptProtectData(&input, null(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, null_mut(), null(), null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Some(bytes)
    }
}

/// PBKDF2-HMAC-SHA256 de CNG. El largo variable permite verificar RFC 7914.
pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u64, len: usize) -> Result<Vec<u8>, String> {
    if iterations == 0 || password.len() > u32::MAX as usize || salt.len() > u32::MAX as usize || len > u32::MAX as usize {
        return Err("parámetros de PBKDF2 inválidos".into());
    }
    let mut out = vec![0; len];
    cng(unsafe {
        BCryptDeriveKeyPBKDF2(BCRYPT_HMAC_SHA256_ALG_HANDLE, password.as_ptr(), password.len() as u32,
            salt.as_ptr(), salt.len() as u32, iterations, out.as_mut_ptr(), len as u32, 0)
    })?;
    Ok(out)
}

fn cng(status: i32) -> Result<(), String> {
    if status >= 0 { Ok(()) } else { Err(format!("falló el cifrado de Windows ({status:#x})")) }
}

struct Algorithm(BCRYPT_ALG_HANDLE);
impl Drop for Algorithm {
    fn drop(&mut self) { unsafe { BCryptCloseAlgorithmProvider(self.0, 0); } }
}
struct Key(BCRYPT_KEY_HANDLE);
impl Drop for Key {
    fn drop(&mut self) { unsafe { BCryptDestroyKey(self.0); } }
}

/// AES-256-GCM; el tag de 16 bytes va al final del texto cifrado.
pub fn gcm_encrypt(key: &[u8], nonce: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    gcm(key, nonce, plain, true)
}

pub fn gcm_decrypt(key: &[u8], nonce: &[u8], sealed: &[u8]) -> Result<Vec<u8>, String> {
    gcm(key, nonce, sealed, false)
}

fn gcm(secret: &[u8], nonce: &[u8], input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    if secret.len() != 32 || nonce.len() != 12 || input.len() > u32::MAX as usize || (!encrypt && input.len() < 16) {
        return Err("parámetros de AES-GCM inválidos".into());
    }
    unsafe {
        let mut alg = Algorithm(null_mut());
        cng(BCryptOpenAlgorithmProvider(&mut alg.0, BCRYPT_AES_ALGORITHM, null(), 0))?;
        let mode = crate::win::wide("ChainingModeGCM");
        cng(BCryptSetProperty(alg.0, BCRYPT_CHAINING_MODE, mode.as_ptr() as *const u8, (mode.len() * 2) as u32, 0))?;
        let mut key = Key(null_mut());
        // Windows administra el objeto de clave cuando pbKeyObject es NULL.
        cng(BCryptGenerateSymmetricKey(alg.0, &mut key.0, null_mut(), 0, secret.as_ptr(), 32, 0))?;
        let (data, mut tag) = if encrypt {
            (input, [0u8; 16])
        } else {
            let (data, tag) = input.split_at(input.len() - 16);
            (data, tag.try_into().unwrap())
        };
        let mut info: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        info.dwInfoVersion = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION;
        info.pbNonce = nonce.as_ptr() as *mut u8;
        info.cbNonce = 12;
        info.pbTag = tag.as_mut_ptr();
        info.cbTag = 16;
        // Un puntero de salida válido incluso para texto vacío: NULL consulta el tamaño.
        let mut out = vec![0; data.len().max(1)];
        let mut written = 0;
        let operation = if encrypt { BCryptEncrypt } else { BCryptDecrypt };
        cng(operation(key.0, data.as_ptr(), data.len() as u32, &mut info as *mut _ as _,
            null_mut(), 0, out.as_mut_ptr(), out.len() as u32, &mut written, 0))?;
        out.truncate(written as usize);
        if encrypt { out.extend_from_slice(&tag); }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i+2], 16).unwrap()).collect()
    }

    #[test]
    fn pbkdf2_rfc7914_section11() {
        // https://www.rfc-editor.org/rfc/rfc7914.html#section-11
        assert_eq!(hex(&pbkdf2(b"passwd", b"salt", 1, 64).unwrap()),
            "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783");
    }

    #[test]
    fn aes256_gcm_nist_empty_and_authentication() {
        // NIST, AES_GCM.pdf, GCM-AES256, ejemplo 1 (sin AAD ni texto).
        // https://csrc.nist.gov/CSRC/media/Projects/Cryptographic-Standards-and-Guidelines/documents/examples/AES_GCM.pdf
        let key = unhex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
        let nonce = unhex("cafebabefacedbaddecaf888");
        let mut sealed = gcm_encrypt(&key, &nonce, b"").unwrap();
        assert_eq!(hex(&sealed), "fd2caa16a5832e76aa132c1453eeda7e");
        assert_eq!(gcm_decrypt(&key, &nonce, &sealed).unwrap(), b"");
        sealed[0] ^= 1;
        assert!(gcm_decrypt(&key, &nonce, &sealed).is_err());
        assert!(gcm_decrypt(&key, &nonce, &[0; 15]).is_err());
    }

    #[test]
    fn aes256_gcm_nist_nonempty() {
        // Mismo documento de NIST, GCM-AES256, ejemplo 2 (512 bits).
        let key = unhex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
        let nonce = unhex("cafebabefacedbaddecaf888");
        let plain = unhex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255");
        let mut sealed = gcm_encrypt(&key, &nonce, &plain).unwrap();
        assert_eq!(hex(&sealed), "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015adb094dac5d93471bdec1a502270e3cc6c");
        assert_eq!(gcm_decrypt(&key, &nonce, &sealed).unwrap(), plain);
        sealed[0] ^= 1;
        assert!(gcm_decrypt(&key, &nonce, &sealed).is_err());
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn pkce_rfc7636_example() {
        // RFC 7636, apéndice B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64url(&sha256(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn b64url_roundtrip() {
        for len in 0..40 {
            let data = random_bytes(len);
            assert_eq!(b64url_decode(&b64url(&data)).unwrap(), data);
        }
        assert_eq!(b64url(b"hola"), "aG9sYQ");
    }

    #[test]
    fn dpapi_roundtrip() {
        let secret = b"1//refresh-token-de-prueba";
        let sealed = protect(secret).unwrap();
        assert_ne!(&sealed[..], &secret[..]);
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }
}
