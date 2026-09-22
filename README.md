# Simpcky

Notas adhesivas ultraligeras para el escritorio de Windows 11. Rust +
Win32 puro (`windows-sys`), sin Electron, sin WebView2, sin .NET.

Diseño: ver el lienzo publicado — cinco pantallas (escritorio, prototipo
interactivo, anatomía, menús, especificación técnica).

## Compilar y ejecutar

```bash
cargo build --release
./target/release/simpcky.exe
```

En debug (`cargo run`) el binario abre una consola detrás; en release no
(`#![windows_subsystem = "windows"]`).

## Estado actual (v0.1 — primer corte funcional)

Ya funciona:

- Notas nativas con encabezado de color, arrastre, y cuerpo de texto en
  un control RichEdit real (fuente Segoe UI).
- **Enrollado con el switch pedido**: toda nota nace en modo **Manual**
  (nunca se enrolla sola). Desde el menú "⋯" de cada nota se puede pasar
  a **Auto** (se enrolla al quitar el mouse, se despliega al pasar por
  encima). El valor por defecto para notas nuevas se cambia desde el
  menú de la bandeja.
- Color (6 opciones), "siempre encima" (`HWND_TOPMOST`), eliminar nota
  (con confirmación).
- Icono en la bandeja: clic izquierdo crea una nota, clic derecho abre
  el menú (nueva nota, modo de enrollado por defecto, iniciar con
  Windows, salir).
- "Iniciar con Windows" real, vía `HKCU\...\Run`.
- Persistencia en `%APPDATA%\Simpcky\notes.json`, con autoguardado
  (debounce ~600 ms al escribir; inmediato al mover/cambiar ajustes).
- Primera ejecución: crea una nota de bienvenida que explica el modo
  Manual/Auto.

Medido en esta máquina: binario de **~165 KB** (release, LTO, strip,
`panic=abort`) y **~5 MB** de memoria privada con una nota abierta.

Falta (queda para la próxima iteración, según el diseño):

- **Anclar al escritorio** (capa `Layer::Desktop`, detrás de los
  iconos vía `WorkerW`): el campo ya existe en los datos pero la
  ventana todavía no se reparenta ahí; hoy se trata como "Normal".
- Ventana "Todas las notas".
- Checklist con casillas reales dentro de la nota (hoy el cuerpo es
  texto libre en RichEdit; el diseño la muestra con checkboxes).
- Atajos de teclado globales (Ctrl+N, Ctrl+R, etc.).
- Icono propio (hoy usa el icono genérico de Windows) — falta un
  `.ico` y enlazarlo como recurso.

## Estructura

- `src/main.rs` — arranque, carga de notas guardadas, bucle de mensajes.
- `src/app.rs` — estado global (notas abiertas, modo por defecto).
- `src/note.rs` — ventana de una nota: dibujo del encabezado, arrastre,
  menú "⋯", enrollado manual/auto.
- `src/tray.rs` — icono de bandeja, menú principal, "iniciar con
  Windows".
- `src/persist.rs` — modelo de datos y lectura/escritura de
  `notes.json` (JSON escrito a mano, sin serde, para mantener el
  binario chico).
- `src/win.rs` — helpers pequeños (cadenas UTF-16, `COLORREF`).
