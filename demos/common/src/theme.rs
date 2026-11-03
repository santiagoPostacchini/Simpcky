//! Colores de Simpcky: la paleta de las notas (`theme.rs`), las opacidades
//! del modo vidrio (`glass.rs`) y los colores de las capas de celdas (`snap.rs`).

/// (encabezado, cuerpo, tinta) en 0xRRGGBB.
pub const LIGHT: [(u32, u32, u32); 6] = [
    (0xFDE68A, 0xFEF9C3, 0x78350F),
    (0xFBCFE8, 0xFCE7F3, 0x831843),
    (0xBBF7D0, 0xDCFCE7, 0x065F46),
    (0xBFDBFE, 0xDBEAFE, 0x1E3A8A),
    (0xE9D5FF, 0xF3E8FF, 0x581C87),
    (0xE2E8F0, 0xF1F5F9, 0x334155),
];
pub const DARK: [(u32, u32, u32); 6] = [
    (0x6B5514, 0x2A2515, 0xFDE68A),
    (0x74284F, 0x2B1822, 0xFBCFE8),
    (0x1D5E3E, 0x15261C, 0xBBF7D0),
    (0x234A80, 0x151F30, 0xBFDBFE),
    (0x52307F, 0x201830, 0xE9D5FF),
    (0x3E4652, 0x1E2227, 0xE2E8F0),
];

pub fn note_colors(color: u8, dark: bool) -> (u32, u32, u32) {
    let i = color as usize % 6;
    if dark { DARK[i] } else { LIGHT[i] }
}

/// Opacidad (encabezado, cuerpo) sobre el desenfoque según el nivel de
/// vidrio (0 apagado, 1 suave, 2 media, 3 fuerte): la tabla de `glass.rs`.
pub fn glass_alpha(level: u8, dark: bool) -> (u8, u8) {
    match (dark, level) {
        (_, 0) => (0xFF, 0xFF),
        (true, 1) => (0xB8, 0xA0),
        (true, 3) => (0x50, 0x30),
        (true, _) => (0x80, 0x60),
        (false, 1) => (0xE8, 0xE0),
        (false, 3) => (0xC0, 0xB0),
        (false, _) => (0xD4, 0xC8),
    }
}

/// Opacidad del título en vidrio.
pub fn title_alpha(level: u8, dark: bool) -> u8 {
    if !dark || level == 0 {
        return 0xFF;
    }
    match level {
        1 => 0xEE,
        3 => 0xC4,
        _ => 0xDA,
    }
}

/// Colores de las capas de celdas, en 0xAARRGGBB (los de `snap::colors`).
#[derive(Clone, Copy, Debug)]
pub struct SnapColors {
    pub accent: u32,
    pub zone_fill: u32,
    pub grid_line: u32,
    pub ink: u32,
    pub panel: u32,
    pub panel_line: u32,
    pub option: u32,
    pub danger: u32,
}

pub fn snap_colors(dark: bool) -> SnapColors {
    if dark {
        SnapColors {
            accent: 0xff60_a5fa,
            zone_fill: 0x4760_a5fa,
            grid_line: 0x66e2_e8f0,
            ink: 0xffe5_e7eb,
            panel: 0xf528_2a2e,
            panel_line: 0xff30_343a,
            option: 0xff4b_5058,
            danger: 0xfff8_7171,
        }
    } else {
        SnapColors {
            accent: 0xff3b_82f6,
            zone_fill: 0x423b_82f6,
            grid_line: 0x661e_293b,
            ink: 0xff1f_2937,
            panel: 0xf2fa_fafa,
            panel_line: 0xffdf_e3dc,
            option: 0xffd4_d7dc,
            danger: 0xffdc_2626,
        }
    }
}

/// 0xAARRGGBB con la opacidad multiplicada por `a`.
pub fn with_alpha(c: u32, a: f32) -> u32 {
    ((((c >> 24) as f32 * a.clamp(0.0, 1.0)) as u32) << 24) | (c & 0x00ff_ffff)
}

/// (r, g, b, a) en 0..1 de 0xAARRGGBB.
pub fn rgba(c: u32) -> (f32, f32, f32, f32) {
    (((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0, (c >> 24) as f32 / 255.0)
}

/// (r, g, b) en 0..1 de 0xRRGGBB.
pub fn rgb(c: u32) -> (f32, f32, f32) {
    (((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0)
}
