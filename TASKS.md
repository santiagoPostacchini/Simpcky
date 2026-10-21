# Tareas

- Implementado — Codex: asomar widgets bajo el cursor desde `open_by_id`, restaurar su posición guardada al anclarlos y conservar la posición visible al pasar a siempre encima. Llamadores actualizados; sin dependencias nuevas.
- Verificación: `cargo build` sin warnings; `cargo test`: 34 aprobados, 1 fallido (`crypto::tests::dpapi_roundtrip`, `src/crypto.rs:143`), 2 ignorados. Se usó TEMP/TMP en `target/tmp` por permisos del entorno.
- Pendiente: comprobar visualmente apertura desde selector/Todas las notas, retorno al perder foco, arrastre, cambio de capa y conservación de coordenadas en `notes.json`, incluidos vidrio y varios monitores.
- Hallazgo: faltaban CONTEXT.md y TASKS.md. `ai-init` falló por permisos sobre la configuración Git del repositorio principal; no se generaron convenciones ni zonas protegidas. No se cambió de rama ni se hizo commit.
