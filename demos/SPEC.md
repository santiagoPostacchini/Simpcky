# Lo que hace cada demo (Direct2D propio y Slint en modo software)

Las dos demos hacen exactamente lo mismo; solo cambia **cómo se dibuja**.
Todo lo que no es dibujo está en `common/` y se usa igual en las dos:

| Módulo | Qué es |
|---|---|
| `common::geom` | Geometría de celdas y pilas, copiada de `src/snap.rs` (con sus tests) |
| `common::snap` | El arrastre con celdas, panel y pilas, portado de `src/snap.rs`. La demo implementa `snap::Host` |
| `common::anim::Motion` | Transiciones (200 ms posición, 170 ms enrollar, cúbica) |
| `common::pump` | Cuadros al ritmo de la pantalla (`DwmFlush` en un hilo → `WM_FRAME`) |
| `common::win` | Monitor, cursor, teclas, ventanas de apps, vidrio nativo (`set_blur`), capa (`set_layer`) |
| `common::theme` | Paleta de notas, opacidades de vidrio y colores de las capas de celdas |
| `common::store` | `notes.json` (lee el de Simpcky la primera vez; escribe en su carpeta: `set_app`) |
| `common::framelog` | Con `SIMPCKY_FRAMES=<archivo>`: una línea por cuadro presentado |

## Ventanas

- **Una ventana de primer nivel por nota** (`WS_POPUP`), sin borde, esquinas
  redondeadas (`win::round_corners`). En el escritorio: `win::set_layer(hwnd, true)`
  (poseída por `Progman`, como el modo vidrio de Simpcky); "siempre encima":
  `set_layer(hwnd, false)`. No aparece en la barra de tareas (`WS_EX_TOOLWINDOW`).
- Tamaño: `w × h` abierta, `w × 40` enrollada (barra de 40 px a 96 ppp).
- **Una capa para las celdas**: ventana de primer nivel del tamaño del área de
  trabajo del monitor, siempre encima, transparente a los clics
  (`WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`),
  que se muestra mientras `snap.layers().visible()` y dibuja `snap.layers()`.
- **Una ventana de mensajes oculta** que recibe `pump::WM_FRAME`.

## Dibujo de una nota (claro/oscuro con `theme::note_colors`)

- Encabezado de 40 px con el color de encabezado; título (`display_title`) en
  Segoe UI Semibold 14 px con la tinta; con el mouse encima, botones a la
  derecha: **+** (nota nueva), **pin** (siempre encima).
- Cuerpo con el color de cuerpo y el texto editable (Segoe UI 15 px, tinta).
- **Vidrio** (`settings.glass` 1..3): `win::set_blur(hwnd, true)` y los colores
  de encabezado y cuerpo con la opacidad de `theme::glass_alpha(nivel, oscuro)`,
  premultiplicados; el título con `theme::title_alpha`. Con 0, todo opaco y
  `set_blur(hwnd, false)`.

## Interacción

- **Clic** en la barra (sin mover): enrolla/despliega (en Manual), con
  `snap.reshape(...)`.
- **Arrastrar** la barra: `SetCapture` + `snap.begin_move(host, id, grab)`;
  cada `WM_MOUSEMOVE` → `snap.step(host)`; `WM_LBUTTONUP` → si `!snap.moved()`
  es un clic (`snap.end(host, false)` y enrollar), si no `snap.end(host, false)`.
  Esc y soltar el botón fuera de la ventana los atiende `snap.frame()`.
- **Ctrl + Mayús + clic** en la barra: `snap.make_pile(host, id)`.
- **Auto** (las de pila): con el mouse encima se despliega (salvo
  `snap.hush(id, true)`), al salir se enrolla. La salida se detecta mirando
  el cursor en cada cuadro mientras está encima.
- Atajos con una nota enfocada: **Ctrl+N** nota nueva, **Ctrl+R** enrollar,
  **Ctrl+G** siguiente nivel de vidrio (todas), **Ctrl+D** claro/oscuro
  (todas), **Ctrl+Mayús+T** siempre encima.
- Cambios de texto, posición y forma se guardan con `store::save_notes`
  (800 ms después del último cambio).

## Cuadros

Mientras `snap.frame(host)` devuelva `true` (o se esté enrollando algo, o
el panel se desvanece), `pump::run_frames(true)`; en cada `WM_FRAME`:
`pump::frame_taken()`, `snap.frame(host)`, y si `snap.layers().serial`
cambió, redibujar la capa. Cuando nada se mueve, `run_frames(false)`: en
reposo no se gasta nada.

`framelog::mark("nota", id, costo)` después de presentar una nota y
`framelog::mark("capa", 0, costo)` después de presentar la capa.

## Capa de celdas (lo que dibuja `snap.layers()`, como `snap::Fx::frame`)

Coordenadas relativas a `layers.work`. Todo multiplicado por su opacidad (`*_a`):

- **Cuadrícula** (`grid_a`): cada celda, rectángulo redondeado (radio 6)
  con borde punteado `grid_line` de 1,5 px; las **zonas** de pila, relleno
  `zone_fill` y borde punteado `accent` al 55 %, con "Soltá acá para sumar a
  la pila" (13 px, `ink`) abajo al centro. Nada se dibuja dentro de los
  `holes` (radio 8): la cuadrícula queda "debajo" de las notas.
- **Vista previa** (`preview`, `preview_a`): redondeado radio 8, relleno
  `zone_fill`, borde `accent` de 2 px.
- **Línea de pila** (`line`, `line_a`): redondeado radio 4 relleno
  `zone_fill` y adentro (2 px menos por lado) radio 2 relleno `accent`.
- **Panel** (`panel`, `hot`, `panel_a`): fondo redondeado (radio 10) `panel`
  con borde `panel_line` y sombra suave hacia abajo, que baja 12 px mientras
  aparece (`(1 - panel_a) * -12`). Seis opciones (46 px): las cuatro formas
  (1×1, 2×1, 1×2, 2×2 como bloques), la pila (tres barras alineadas a la
  derecha) y el tacho (rojo `danger` cuando está elegido); separadores antes
  de la pila y antes del tacho. La elegida con relleno `zone_fill` y tinta
  `accent`; las otras con tinta `option`.
- **Guía** (`hint`, `hint_a`): barra `accent` arriba al centro.
