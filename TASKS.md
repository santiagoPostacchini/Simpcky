# Tareas

## Hecho (rama `fix/bugs-notas`, pendiente de merge a master)

- Claude — Esquinas redondeadas: se vuelven a pedir a DWM cada vez que la nota
  deja de ser hija del escritorio (asomada, siempre encima). (`fix/escritorio-bordes-render`)
- Claude — Notas que se iban del escritorio: una nota asomada ya no queda suelta
  si el foco pasa por un menú/diálogo propio y de ahí a otra app; una nota que
  quedó fuera de toda pantalla vuelve al monitor más cercano.
- Claude — Texto que dejaba de dibujarse: repintado completo al poner o sacar
  `WS_EX_LAYERED` (anclar/soltar).
- Codex — Casillas ☐/☑ en cualquier lugar del texto: `[] ` tras un espacio,
  se dibujan y se tildan con clic. (`codex/casillas-en-linea`)
- Codex + Claude — Traer una nota al frente (selector, "Todas las notas") la
  asoma bajo el mouse y vuelve a su lugar al perder el foco; redimensionarla
  asomada no guarda la posición temporal. (`codex/asomar-en-cursor`)
- Claude — `notes.json` ilegible: se respalda como `notes.json.ilegible-<hora>`
  en vez de pisarse con una lista vacía. Modo vidrio: una falla momentánea al
  listar módulos ya no rehace todas las notas. (`fix/datos-y-vidrio`)

- Claude — Candado en la barra de cada nota (sale del menú "⋯"): muestra el estado
  (abierto entre los botones; bloqueada, cerrado y pegado a la derecha); bloqueada,
  el texto se atenúa un poco y
  se puede seguir seleccionando y copiando. Ancho mínimo de nota: 180 px, para que
  entren los cinco botones. (`feat/candado-en-barra`)

- Claude — Parpadeo al escribir: el fondo ya no se repinta sobre la franja de la
  barra de desplazamiento, se pinta solo el rectángulo que cambió, y la barra y
  las casillas se dibujan en un lienzo aparte y se copian de una vez.
  (`fix/candado-y-parpadeo`)
- Claude — Parpadeo de la línea que se escribe: el texto se dibuja con
  `ITextServices::TxDraw` en un lienzo junto con fondo, casillas, emojis y barra,
  y se copia a la pantalla de una vez. Si no hay `ITextServices`, se pinta como
  antes. Prueba: `cargo test -- --ignored richedit_se_dibuja`. (`fix/parpadeo-linea`)

- Claude — Ajuste "Esperar antes de enrollar" (al instante, 0,5, 1, 2 o 3 s; `rollDelay` en
  `settings.json`): el enrollado automático espera con el mouse afuera todo ese tiempo seguido;
  si vuelve antes, la cuenta se reinicia. (`feat/demora-enrollar`)

- Claude — Sombra que quedaba en pantalla al asomar una nota (atajo, selector) y volver
  al escritorio: un rectángulo oscuro translúcido, encima de todo, en el lugar donde se
  vuelve a anclar. Era la imagen de ventana flotante que DWM no borra si la nota pasa a
  ser hija del escritorio estando visible: `desktop::anchor` ahora la oculta antes.
  Reproducido y verificado sin vidrio (nota enrollada, clic en el escritorio). También
  se quitó `CS_DROPSHADOW` (su `SysShadow` tampoco seguía a la nota). Con vidrio no se
  pudo probar: Windhawk no inyecta el mod en procesos lanzados desde la terminal.

## En curso (rama `feat/celdas`, sale de `fix/bugs-notas`)

- Claude — Celdas y pilas (`snap.rs`, ganchos en `note.rs`, `desktop::set_screen_rect`): panel al
  borde de arriba (4 tamaños + pila), cuadrícula por monitor, Ctrl + arrastrar (cuadrícula o pila
  entera), Ctrl + borde por celdas, notas tapadas que se achican sin quedar en L o se corren, pilas
  reconocidas por posición, salir de una pila la abre y la pasa a Manual, huecos que se cierran al
  ocultar o borrar, Ctrl no despliega las notas de una pila. 6 tests nuevos (`cargo test snap`).
- Segunda vuelta (no se sentía como el boceto): arrastre propio (la nota toma la forma elegida o
  se vuelve barra mientras se la lleva), transiciones (`anim.rs`) al encajar, achicar, cerrar huecos,
  enrollar y desplegar, capas con fundido y deslizamiento, guía arriba al centro, cuadrícula que no
  pisa las notas, notas sin alinear que se redondean a la celda más cercana (nunca crecen), la pila
  movida con Ctrl corre a las notas que pisa, cambiar de opción en el panel pide ~120 ms encima,
  ninguna nota se despliega mientras se arrastra, pilas armadas a mano reconocidas con tolerancia.
- Verificado con el guion (`examples/celdas_guion.py`, capturas paso a paso en el monitor 2 a 150 %):
  panel y formas, vista previa, encajar, achicar con Ctrl, sacar y volver a meter en una pila, mover
  la pila entera, Ctrl + borde, pila nueva y Esc.
- Falta probar a mano: el mouse real (desde la sesión de Claude Windows no deja mover el cursor),
  el despliegue al pasar por una pila y Ctrl, modo vidrio, siempre encima.
- Tercera vuelta: el arrastre ya no depende de quedarse con el mouse (una nota anclada es hija de
  Explorer y podía perderlo: no se movía); entrar a una pila pasa la nota a Auto; dos pilas no se
  pisan, se juntan (con Ctrl, mientras se lleva la pila se ve cómo quedarían); el candado deja
  cambiar el tamaño; Ctrl + Mayús + clic en la barra convierte la nota en pila. Juntar pilas y
  Ctrl + Mayús + clic verificados con el guion (`hacer_pila`).
- Con Ctrl, una nota de pila se alarga o achica de a una celda (el lado alineado queda fijo;
  verificado con el guion). "+" en el selector de notas para crear una nueva (sin probar a mano).
- Cuadrícula y aplicaciones a la vista (`app_windows`: bordes reales de DWM, sin minimizadas, ocultas
  en otro escritorio, auxiliares ni las propias): columnas y filas elegidas para que los bordes de las
  ventanas caigan entre celdas (`fit_count`, con test), celdas tapadas bloqueadas (ni se dibujan ni
  se usan; el destino que cae encima va al lugar libre más cercano), pilas tapadas sin zona. Panel y
  guía centrados en la parte libre más grande (`free_area`). Probado con Spotify en el monitor 2.
- Pilas con lados: cada nota angosta va a la izquierda o a la derecha de la columna según dónde
  se la suelta (la línea lo anticipa); se reconocen también las armadas a mano con lados mezclados.
  Verificado con el guion.
- Tacho en el panel de arriba: soltar la nota encima la borra (vacía, enseguida; con algo, con la
  confirmación de siempre y, si se dice que no, vuelve a su lugar; bloqueada, el tacho no se activa).
  Visto con el guion (`opcion 5`); el borrado en sí, sin probar a mano.
- Hallazgo (de antes, no de esto): en un monitor con otra escala que el principal, una nota anclada
  usa el ppp del principal (`GetDpiForWindow` de una hija de `Progman`); una nota creada enrollada
  ahí queda con una franja oscura bajo la barra hasta que se enrolla de nuevo.
- Fuera de esta versión: interruptor en Configuración, "Apilar todas", abrir hacia arriba al pie de
  la pantalla, celdas en otro monitor que el del comienzo del arrastre.

## Pendiente

- Usuario — Probar en Windows: esquinas al asomar/siempre encima, desconectar un
  monitor, casillas en el medio del texto (clic, Ctrl+Z, bloqueada, vidrio),
  asomar desde el selector y que vuelva a su lugar, candado en la barra (bloquear,
  desbloquear, seleccionar y copiar bloqueada, tema claro/oscuro y vidrio).
- Bugs 1, 2 y 4 no se pudieron reproducir: los arreglos atacan las causas más
  probables encontradas leyendo el código. Si vuelven a pasar, anotar qué se
  estaba haciendo (asomar, cambiar de monitor, suspender, reiniciar Explorer).
- `wip/menu-nuevo`: trabajo sin terminar que estaba sin commitear en master.

## Hallazgos

- `crypto::tests::dpapi_roundtrip` falla dentro del sandbox de Codex; fuera de
  él pasa.
- Faltan `CONTEXT.md` y `ARCHITECTURE.md` (correr `ai-init`).

## Medir parpadeo (Codex, hecho en `codex/medir-parpadeo`)
- Instancia aislada por `SIMPCKY_INSTANCIA`; sin shell, arranque de sync/update, bienvenida ni atajo global.
- `SIMPCKY_PAINT_LOG` registra dibujo y teclas con QPC; no se modificaron las operaciones de dibujo.
- Ejemplo con perfil temporal, fondo fijo, DPI v2, SendInput, captura por zonas y correlacion de eventos.
- Uso: `cargo build --release`; `cargo run --release --example medir_parpadeo -- --vidrio si` (o `no`).
- Opciones: `--teclas N --intervalo MS --umbral MS --exe RUTA --nombre X`; registro en el perfil temporal.
- Verificado: `cargo build`, `cargo build --examples`, cuatro tests con `cargo test --example medir_parpadeo`; sin warnings.
- `cargo test`: 37 pasan, 3 ignorados; solo falla `crypto::tests::dpapi_roundtrip` (sandbox conocido).
- Claude: el chequeo de que la ventana de adelante sea la de prueba antes de cada clic/tecla.

## Parpadeo al escribir (Claude, `refactor/dibujo`) — resuelto, medido
- Base (vidrio): 0,70 parpadeos/tecla (fila escrita y barra). Causa, por el registro: el RichEdit es
  `WS_EX_TRANSPARENT`, así que Windows repinta la nota debajo de cada línea que cambia; la nota pintaba
  ahí su color liso y el texto llegaba ~2 ms después.
- Arreglo: la nota no pinta debajo del texto (`note::exclude_text`), el RichEdit arma fondo + texto
  (`TxDraw`) + casillas + emojis en un lienzo (`editor::paint_buffered`), y fuera del WM_PAINT solo se
  redibuja la barra (esconder el cursor en cada tecla lo hacía titilar).
- Resultado: 0 parpadeos en 3 corridas con vidrio y 2 lisas (40 teclas cada una, nota siempre encima).
- No hizo falta la Fase 2 del plan (RichEdit sin ventana). Ver `ARCHITECTURE.md`.
- Pendiente: probar a mano anclada al escritorio sin vidrio (la medición usa siempre encima).
