# Plan: lavado de cara, ronda de depuración y Free/Plus

Estado: propuesta (3 de noviembre de 2026). Base: `master` en `0ed72df`.
Orden: **0 → 1 → 2 → 3**. Free/Plus (4) va después, cuando esté decidida la licencia.

## Decisiones tomadas

- **Plus** = imágenes (16), mosaico en pilas (21), mover y juntar pilas (22), texto oculto (25),
  invisible al compartir pantalla (26), cifrado local (27, en duda: ver 4), modo vidrio (30),
  sincronización (31) y cifrado en Drive (32). El resto es Free.
- **La página del Free** es la portada de GitHub Pages (`docs/index.html`). Se reemplaza por la de
  Plus cuando exista la ficha en la tienda. Se quedan `privacidad.html` y `firma.html`, porque
  Google, la tienda y SignPath piden esos enlaces.

## Rumbo (auditoría con la skill unslop, 4/11)

La skill está en `.claude/skills/unslop/` (de Damir Sokolovsky, CC BY 4.0). Su lista de señales da
**slop liviano**: no hay emojis en la interfaz, ni degradés, ni brillos, ni movimiento en bucle.
Hay rótulos en MAYÚSCULAS (Configuración y Bienvenida), varios acentos, radios y escalas
desparejos, y hovers y menús que cambian de golpe. Para ese caso la skill pide **pulir sin
rediseñar**: se quedan la paleta de las notas, la letra de Windows y el orden de cada pantalla.

- **Producto.** Notas adhesivas sobre el escritorio de Windows, para tener a mano lo que no
  querés perder (claves incluidas).
- **Carácter.** A mano, callada, rápida.
- **Mundo.** Papel sobre el escritorio. El color lo pone el papel de las notas, y todo lo demás
  (menús, ventanas, marcos) es gris de Windows.
- **Momentos.** Las notas mismas, con su papel, su título y sus pilas. La ventana "Todas las
  notas" es el otro momento, porque es donde se ven todas juntas.
- **Firma.** El encabezado de la nota (40 px, título semibold con emoji en color), dibujado igual
  en todos los lugares donde aparece una nota: la nota, el ghost, el selector y "Todas las notas".
- **Acento.** Uno solo: el color de acento de Windows (decidido el 4/11). Se usa en las zonas de
  arrastre, el marco del borde, el foco, la selección, el botón principal y el interruptor
  encendido.
  - Se lee con `DwmGetColorizationColor` y se vuelve a leer con
    `WM_DWMCOLORIZATIONCOLORCHANGED`.
  - Si no se puede leer, queda el azul `3B82F6` de hoy.
  - El texto encima va blanco o negro, el que dé 4,5 a 1.
  - El amarillo y el ámbar quedan solo en el papel de las notas, que son el contenido, no el marco.

## Movimiento (paquete H, pedido el 4/11)

Las reglas de movimiento de la skill, aplicadas a Win32:

- **Una sola curva:** la salida suave que ya usa `anim.rs`.
- **Cuatro duraciones:**
  - `FAST` 120 ms: hover, presionado e interruptores;
  - `MENU` 160 ms: menús y barrita;
  - `ENTER` 200 ms: ventanas, y es el `MOVE` actual;
  - `ROLL` 170 ms.
- **Hover y presionado:** el fondo pasa de `surface` a `surface_hover` y a `surface_pressed` con un
  fundido de color, no de golpe. Nada se agranda.
- **Menús** ("⋯", barrita, tarjeta, bandeja y selector): aparecen con un fundido y suben 6 px desde
  el lado por donde abren; al cerrarse se funden en 120 ms.
- **Ventanas propias** (Configuración, Todas las notas, Bienvenida y diálogos): fundido más 8 px
  de subida, solo donde DWM no anima ya la ventana, para no animar dos veces.
- **Interruptor:** la perilla se desliza.
- **Nota nueva:** aparece con un fundido; arrastrar, enrollar y apilar quedan como están.
- **Accesibilidad:** si Windows tiene apagadas las animaciones, todo pasa al instante, como ya hace
  `anim.rs`.
- **Lo que no se hace:**
  - rebotes, bucles, pulsos ni parallax;
  - agrandar al pasar el mouse;
  - animar el ancho o el alto de lo que no sea una nota;
  - fundidos sobre las notas en vidrio, que ya usan su propia transparencia.

Lo técnico va en el paso 1:
- en `anim.rs`, un fundido con subida (alfa con `SetLayeredWindowAttributes` más un `to_pos`
  corto) y un tween de color;
- en `frames.rs`, un `Who::Ui` para que el reloj de cuadros mueva los hovers.

Cada paquete del paso 2 lo aplica en sus superficies.

## Qué está mal hoy (del inventario, 3/11)

La app tiene una paleta de notas y unos pocos colores de ventana (`theme::Chrome`). Fuera de eso,
cada ventana se resuelve sola:

- **Escala:** "Todas las notas", Configuración, Bienvenida y la mini nota que se arrastra (ghost)
  usan píxeles fijos y a 150 % se ven chicas. Además cada superficie toma el ppp de un lado
  distinto (el del monitor, el del sistema o el del sistema por la escala de la nota).
- **Menús y diálogos de dos mundos:** el menú "⋯" es propio, redondeado y con íconos. En cambio,
  el menú de las tarjetas y el de la bandeja son nativos, y hay cuatro `MessageBox` y un diálogo de
  contraseña clásicos sin tema oscuro. Los tooltips tampoco siguen el tema.
- **Colores sin token:**
  - tres acentos: azul `3B82F6` (snap y edges, repetido), ámbar `F59E0B` y el amarillo de la nota;
  - cuatro grises de borde o separador;
  - dos rojos de peligro;
  - el fondo de los paneles distinto del de las ventanas;
  - tres mezclas de "tinta apagada";
  - un texto oscuro fijo que también se usa en modo oscuro.
- **Formas:**
  - Radios: 4, 5, 6, 8, 10 y 12 para cosas equivalentes.
  - Sombras: hay tres tipos.
  - Barrita de formato y menú: los botones B/I/U/S tienen otra medida y otro radio.
- **Títulos:** el de la nota se dibuja de tres maneras.
  - En la nota y en el selector: DirectWrite, con emojis en color.
  - En "Todas las notas": GDI, con emojis grises.
  - En el ghost: negrita y con un encabezado de 30 en vez de 40.
- **Íconos:** la lupa y el pin están dibujados a mano en un lugar y son glifos en otro. Además, un
  mismo glifo se usa para dos cosas: Duplicar y Copiar, y también Ocultar nota y texto oculto.
- **Detalles:** el ícono de la bienvenida sale borroso, los trazos de 1,6 px no escalan, el cursor
  es la mano en unos botones y la flecha en otros, y nada tiene estado "presionado".
- **Textos:**
  - "Sincronizando…." (con un punto de más), "icono" sin tilde y "computadora" en vez de "compu".
  - Los mismos ajustes se llaman distinto en la Bienvenida y en Configuración.
  - Los puntos finales son irregulares, las secciones van en MAYÚSCULAS y se mezclan "al frente"
    y "adelante".

## 1. El sistema (lo primero, lo hace Claude)

Rama `ui/tokens`. Todo lo visual sale de `theme.rs`, con estas piezas:

- **Colores** (claro y oscuro, más vidrio donde aplique):
  - de ventana: `window`, `surface`, `surface_hover`, `surface_pressed`, `border`, `separator`;
  - de texto: `text`, `muted`, `ink_faded` (una sola mezcla);
  - acento y estados: `accent` (el de Windows: zonas, marco, foco, selección, botón principal,
    interruptor encendido), `on_accent` (blanco o negro según contraste) y `danger`.
- **Radios:**
  - `R_WINDOW` 8: ventanas, menús, tarjetas y notas (el mismo que pone DWM);
  - `R_CONTROL` 4: filas, botones y segmentos;
  - `R_PILL`: interruptor y buscador.
- **Espacios:** de a 4 px (4, 8, 12, 16, 24).
- **Letra:** Segoe UI Variable (si no está, Segoe UI).
  - `caption` 12, `body` 14 y `body_strong` 14 semibold;
  - `title` 15 semibold (el de la nota);
  - `subtitle` 20 semibold (la bienvenida).
- **Movimiento:** `FAST` 120 ms (aparecer y desaparecer, estados) y `MOVE` 200 / `ROLL` 170 ms
  (los que ya hay).
- **Escala, una sola regla:**
  - las ventanas propias de la app usan el ppp del monitor donde aparecen;
  - lo que acompaña a una nota (su menú, la barrita, renombrar, el marco) usa además la escala
    de la nota.

  Una función `ui::px(hwnd, v)` para cada caso.
- **Piezas compartidas** (`ui.rs`, un módulo nuevo sin dependencias), sacadas de lo que ya está
  en `flyout.rs` y `settings.rs`:
  - fila de menú, botón de herramienta, botón principal y secundario;
  - interruptor, segmentos, separador y anillo de foco;
  - título con DirectWrite (emojis en color) y una sombra (la de DWM, o una hecha a mano para las
    capas).

*Listo cuando:* compila, los tests pasan, y `theme.rs` y `ui.rs` tienen cada token y cada pieza
documentados. Todavía no cambia nada visible.

## 0. La galería (en paralelo con 1)

Rama `ui/galeria` (para Codex).

- `examples/galeria.py` abre una instancia de prueba con notas inventadas.
- Muestra cada superficie: nota abierta, enrollada, con texto oculto, imagen y casillas; los menús
  "⋯" y de texto; la barrita; el selector; "Todas las notas"; Configuración; Bienvenida; el panel
  de arrastre; la zona de pila; el marco; el ghost; el diálogo de eliminar.
- Lo hace en claro, oscuro y vidrio, y al 100 y 150 % de escala.
- Guarda cada una con `PrintWindow` sobre nuestras propias ventanas, así nunca se captura la
  pantalla del usuario.
- Para las capas (snap, edges) se pide el mismo dibujo en un bitmap en memoria.
- Hace falta un paso nuevo del guion, `mostrar <superficie>`.

*Listo cuando:* genera unas 40 imágenes en `target/galeria/<fecha>/` y una página `index.html` local
para verlas lado a lado (antes y después).

## 2. Aplicarlo, superficie por superficie (en paralelo)

Cada paquete toca **sus archivos y nada más**, así no se pisan. Todos salen de `ui/tokens` ya
mergeado. En cada uno:

- Ningún color, radio ni tamaño de letra literal en sus archivos (se revisa con un grep, salvo la
  paleta de las notas).
- Escala bien al 150 %.
- La galería muestra el antes y el después.
- `cargo test --release -- --include-ignored` y `examples/pilas_guion.py` pasan.

| Paquete | Archivos | Qué | Quién |
|---|---|---|---|
| A. Menús | `flyout.rs`, `seltool.rs` | Barrita y menú con los mismos botones; borde y sombra iguales; estado presionado; mano en los botones; glifos sin repetir (Duplicar ≠ Copiar, Ocultar nota ≠ texto oculto) | Sonnet |
| B. Buscar | `picker.rs`, `allnotes.rs` | Escala con el ppp; títulos con DirectWrite como en la nota; lupa y pin con glifos; el menú de la tarjeta pasa al flyout; las filas reaccionan al mouse | Sonnet |
| C. Ajustes | `settings.rs`, `welcome.rs`, `icon.rs` | Escala; las mismas piezas en las dos (rótulos y medidas iguales); interruptor y botón principal con `accent`; ícono nítido a 48 px | Codex |
| D. Arrastre | `snap.rs` (solo dibujo), `edges.rs`, `ghost.rs` | La paleta propia pasa a tokens; una sola sombra; el ghost igual a una nota (encabezado 40, radio 8, semibold) y escalado | Codex |
| E. Diálogos | nuevo `dialog.rs`, `tray.rs` y los 5 llamados a diálogos | Un diálogo propio (confirmar y contraseña) con tema; el menú de la bandeja con el flyout; tooltips con tema oscuro | Sonnet |
| F. La nota | `note.rs` (encabezado y botones), `editor.rs` (dibujo), `rename.rs` | Hover igual en liso y vidrio; una sola tinta apagada; trazos que escalan; selección con `accent`; renombrar con la letra del título | Codex |
| G. Textos | todos (solo cadenas) | Al final, aparte: rótulos iguales, puntos finales, mayúsculas, "compu", "ícono", "al frente", "Bienvenida/o", la ayuda de la bandeja | Codex |

- **Codex** va por `delegate-codex` (rama `codex/<slug>`).
- **Sonnet** va como subagente en su propio worktree.
- **Claude** revisa cada diff, corre los tests y la galería, y propone el merge. El merge lo decide
  el usuario.
- **G** va último porque toca cadenas de todos los archivos.

**Lo que no se hace en esta ronda:** cambiar cómo funciona algo y traducir. Es solo cohesión y el
movimiento descrito arriba.

**Al cerrar cada paquete:** se repasa la lista de verificación de la skill unslop sobre sus
superficies, usando la galería en lugar de capturas de pantalla.

## 3. Ronda de depuración multiagente (después del lavado)

1. **Juntar.** El usuario anota lo que encuentra en la sección "Ronda de pulido" de `TASKS.md`: qué
   hizo, qué esperaba y qué pasó (una línea basta, más una captura si quiere).
2. **Triar.** Claude agrupa por archivo, decide qué se puede reproducir con el guion (cursor falso
   e instancia de prueba) y arma tandas de hasta 4 arreglos que no compartan archivos.
3. **Arreglar.** Cada arreglo va en su rama, con su reproducción primero: un test, un paso de
   guion, o la galería si es visual. Sin reproducción no se cierra.
4. **Verificar.** Claude corre todo y el usuario prueba con el mouse real, que es lo que el guion
   no cubre. El merge lo decide el usuario.

## 4. Free y Plus (cuando se decida la licencia)

- **Antes:** decidir la licencia. Con el repo MIT y público, cualquiera compila Plus; la
  recomendación es un módulo privado para lo de Plus (ver `plan-tienda-free-plus.md`).
- **Un solo lugar que decide:** `plus.rs` con `plus::has(Función)`. Al principio dice "sí" en las
  compilaciones de desarrollo; después lo decide la licencia de la tienda.
- **Sin Plus no se pierde nada:**
  - El texto oculto que ya existe sigue tapado: nunca se destapa una clave por dejar de pagar.
    Solo no se puede ocultar texto nuevo.
  - Las imágenes se siguen viendo; no se pueden pegar nuevas.
  - La sincronización se pausa sin borrar nada.
  - Las pilas en mosaico se acomodan de a una por fila.
  - Vidrio pasa a liso.
- **Cifrado local (27):** la recomendación es dejarlo siempre activo. Pasarlo a Plus deja las
  notas de Free (con sus claves) en texto plano en el disco, y descifrar lo que ya está cifrado
  sería bajarle la seguridad a quien ya la tenía.
- **La página:** la nueva portada con lo que trae Plus, el precio y el botón de la tienda; se
  publica junto con la ficha.
