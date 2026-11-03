"""Copia la geometría de celdas y pilas de `src/snap.rs` (con sus tests) a
`demos/common/src/geom.rs`, cambiando solo la visibilidad. Correr desde la
raíz del repo cada vez que cambie `snap.rs`:

    python demos/tools/extract_geom.py
"""
import re

src = open('src/snap.rs', encoding='utf-8').read().split('\n')
start = next(i for i, l in enumerate(src) if l.startswith('/// Celda mínima y separación'))
end = next(i for i, l in enumerate(src) if l.startswith('// Las notas de un monitor')) - 1
tests = next(i for i, l in enumerate(src) if l.startswith('#[cfg(test)]') and src[i + 1].startswith('mod tests'))
out = []
in_grid = False
for line in src[start:end]:
    if line.startswith('// ====='):
        continue
    if line.startswith('pub struct Grid {'):
        in_grid = True
    elif in_grid and line.startswith('}'):
        in_grid = False
    line = re.sub(r'^(\s*)fn ', r'\1pub fn ', line)
    if in_grid:
        line = re.sub(r'^(\s+)(left|top|cols|rows|cw|ch|gap): ', r'\1pub \2: ', line)
    line = re.sub(r'^const ', 'pub const ', line)
    out.append(line)
# Los tests usan `q` (de más abajo en snap.rs).
out.append('''
/// Un rectángulo como arreglo (izquierda, arriba, derecha, abajo).
pub fn q(r: RECT) -> [i32; 4] {
    [r.left, r.top, r.right, r.bottom]
}''')
head = '''//! Geometría de celdas y pilas, copiada tal cual de `src/snap.rs` de Simpcky
//! (con sus tests) por `demos/tools/extract_geom.py`, para que las demos se
//! comporten igual que la app. Solo cambia la visibilidad (`pub`).

#![allow(clippy::all)]

pub use windows_sys::Win32::Foundation::RECT;

'''
open('demos/common/src/geom.rs', 'w', encoding='utf-8', newline='\n').write(head + '\n'.join(out) + '\n\n' + '\n'.join(src[tests:]))
print(start + 1, end, tests + 1)
