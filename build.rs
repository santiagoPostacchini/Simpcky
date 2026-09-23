//! Embebe en el ejecutable `assets/simpcky.ico`, para que el `.exe`
//! tenga su ícono en el Explorador, en el menú Inicio, en "Iniciar con
//! Windows" y en el clic derecho del escritorio, y el manifiesto de la
//! app (ver `MANIFEST`).
//!
//! Sin dependencias ni `rc.exe`: el formato `.res` es simple, así que
//! se arma acá a mano y se le pasa directo al linker de MSVC (que
//! acepta archivos `.res` como entrada y los convierte solo).
//!
//! Recursos que se generan:
//! - un `RT_ICON` por cada tamaño del `.ico` (ids 1..n);
//! - un `RT_GROUP_ICON` con id 1 que los agrupa. Es el que usa el
//!   Explorador (el primero del ejecutable) y el que carga `icon.rs`.

use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_MANIFEST: u16 = 24;
const ICON_GROUP_ID: u16 = 1;
/// `CREATEPROCESS_MANIFEST_RESOURCE_ID`: el manifiesto que Windows lee
/// al arrancar el proceso.
const MANIFEST_ID: u16 = 1;

/// Manifiesto de la app.
///
/// - `supportedOS` (Windows 7 a 11): sin esto Windows trata al proceso
///   como una app vieja, y entre otras cosas **ignora `WS_EX_LAYERED`
///   en ventanas hijas** — que es justo lo que necesitan las notas
///   ancladas al escritorio en Windows 11 24H2+ para dibujarse (ver
///   `desktop.rs`).
/// - Common Controls 6: controles y cuadros de diálogo con el estilo
///   actual de Windows (y el tema oscuro de las barras de
///   desplazamiento, que `theme.rs` pide con `SetWindowTheme`).
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Simpcky" version="0.1.0.0" processorArchitecture="*"/>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/>
    </application>
  </compatibility>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=assets/simpcky.ico");
    println!("cargo:rerun-if-changed=build.rs");
    embed_google_client();

    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows-msvc") {
        // Otro linker (GNU) no acepta .res: el binario queda sin ícono
        // de archivo, pero la app funciona igual.
        return;
    }

    let ico = std::fs::read("assets/simpcky.ico").expect("no se pudo leer assets/simpcky.ico");
    let res = build_res(&ico);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("simpcky.res");
    std::fs::write(&out, res).expect("no se pudo escribir el .res");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// El cliente OAuth de Google para la sincronización (ver
/// `docs/sincronizacion-google.md`). Sale de, en orden:
/// 1. las variables de entorno `SIMPCKY_GOOGLE_CLIENT_ID` y
///    `SIMPCKY_GOOGLE_CLIENT_SECRET` (así compila GitHub Actions, con
///    secretos del repositorio);
/// 2. `google_client.json` en la raíz del proyecto: el archivo tal cual
///    lo descarga la consola de Google Cloud. Está en `.gitignore`.
///
/// Sin ninguno de los dos, la app compila igual, solo que sin
/// sincronización (el menú lo dice).
fn embed_google_client() {
    println!("cargo:rerun-if-changed=google_client.json");
    println!("cargo:rerun-if-env-changed=SIMPCKY_GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=SIMPCKY_GOOGLE_CLIENT_SECRET");

    let from_env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (id, secret) = match (from_env("SIMPCKY_GOOGLE_CLIENT_ID"), from_env("SIMPCKY_GOOGLE_CLIENT_SECRET")) {
        (Some(id), Some(secret)) => (id, secret),
        _ => match std::fs::read_to_string("google_client.json") {
            Ok(text) => match (json_string(&text, "client_id"), json_string(&text, "client_secret")) {
                (Some(id), Some(secret)) => (id, secret),
                _ => {
                    println!("cargo:warning=google_client.json no tiene client_id/client_secret: sin sincronización");
                    return;
                }
            },
            Err(_) => return,
        },
    };
    println!("cargo:rustc-env=SIMPCKY_GOOGLE_CLIENT_ID={id}");
    println!("cargo:rustc-env=SIMPCKY_GOOGLE_CLIENT_SECRET={secret}");
}

/// El valor de texto de `"clave": "…"`, donde sea que esté en el JSON
/// (alcanza para el archivo de Google, que no tiene nada anidado raro).
fn json_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let after = &text[text.find(&pattern)? + pattern.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(after[..after.find('"')?].to_string())
}

fn u16le(v: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([v[at], v[at + 1]])
}

fn u32le(v: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([v[at], v[at + 1], v[at + 2], v[at + 3]])
}

/// Una entrada del `.res`: encabezado (con tipo y nombre numéricos) +
/// datos, alineados a 4 bytes.
fn push_resource(res: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    res.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    res.extend_from_slice(&0xFFFFu16.to_le_bytes()); // TYPE: ordinal…
    res.extend_from_slice(&kind.to_le_bytes());
    res.extend_from_slice(&0xFFFFu16.to_le_bytes()); // NAME: ordinal…
    res.extend_from_slice(&id.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    res.extend_from_slice(&0x1010u16.to_le_bytes()); // MemoryFlags (MOVEABLE | DISCARDABLE)
    res.extend_from_slice(&0u16.to_le_bytes()); // LanguageId: neutral
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    res.extend_from_slice(data);
    while res.len() % 4 != 0 {
        res.push(0);
    }
}

fn build_res(ico: &[u8]) -> Vec<u8> {
    assert!(ico.len() >= 6 && u16le(ico, 2) == 1, "assets/simpcky.ico no es un .ico válido");
    let count = u16le(ico, 4) as usize;

    // Todo .res arranca con una entrada vacía de 32 bytes.
    let mut res = vec![0u8; 32];
    res[4] = 32; // HeaderSize
    res[8..12].copy_from_slice(&[0xFF, 0xFF, 0x00, 0x00]);
    res[12..16].copy_from_slice(&[0xFF, 0xFF, 0x00, 0x00]);

    // GRPICONDIR: igual que el ICONDIR del .ico, pero cada entrada
    // apunta a un id de recurso en vez de a un offset en el archivo.
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());

    for i in 0..count {
        let e = 6 + i * 16;
        let size = u32le(ico, e + 8) as usize;
        let offset = u32le(ico, e + 12) as usize;
        let id = (i + 1) as u16;
        push_resource(&mut res, RT_ICON, id, &ico[offset..offset + size]);

        group.extend_from_slice(&ico[e..e + 12]); // ancho, alto, colores, reservado, planos, bpp, tamaño
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut res, RT_GROUP_ICON, ICON_GROUP_ID, &group);
    push_resource(&mut res, RT_MANIFEST, MANIFEST_ID, MANIFEST.as_bytes());
    res
}
