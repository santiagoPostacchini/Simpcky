//! Utilidades Win32 pequeñas y compartidas por el resto de módulos.

/// Convierte una cadena Rust a UTF-16 terminada en NUL, lista para pasar
/// a cualquier API `...W` de Win32.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Construye un `COLORREF` (0x00BBGGRR) a partir de componentes R,G,B de 8 bits.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// Convierte un `&str` en un buffer UTF-16 recibido de vuelta como `String`,
/// recortando en el primer NUL (para leer resultados de `GetWindowTextW`, etc.).
pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
