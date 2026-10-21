# Tareas

- Hecho — Codex: casillas en cualquier lugar del texto (`codex/casillas-en-linea`).
  Atajos con límite de espacio/tab, posiciones UTF-16, dibujo normal/vidrio,
  clic deshacible y cursor de mano; el prefijo conserva el formato de lista.
- Verificación: `cargo build` sin warnings; `cargo test`: 36 pasan, 1 falla,
  2 ignorados. Los 6 tests de `richtext` pasan, incluidos los 2 nuevos.
- Hallazgo: `crypto::tests::dpapi_roundtrip` falla en `src/crypto.rs:143`;
  también falla compilando ese archivo sin cambios como test independiente.
  No se modificó criptografía. TMP/TEMP apuntaron a `target/tmp` para evitar
  un error de acceso del enlazador al temporal predeterminado.
- Pendiente: validación visual en Windows (clic, deshacer, bloqueo y vidrio)
  y repetir la suite en un entorno donde DPAPI funcione.
- No existían `CONTEXT.md` ni `TASKS.md`; se limitó la edición a los dos
  archivos solicitados y este registro, sin inicializar documentación extra.
