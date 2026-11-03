# Tareas

## En curso (rama `feat/fluidez-y-limites`, sale de master)

- Claude — Cuadros al ritmo de la pantalla (`frames.rs`, `DwmFlush`) para transiciones, arrastre y
  capas, en vez de `SetTimer`; Auto se despliega apenas entra el mouse. Medido con mouse real
  (400 Hz): enrollar 27–30 pasos (antes 10), sin tirones; encaje 18 (antes 10).
- Claude — Borde para cambiar el tamaño (`edges.rs`): marco que se agarra desde 6 px afuera, con
  contorno y el borde elegido resaltado. Probado con mouse real en anclada y siempre encima
  (`examples/borde_prueba.py`, `examples/mapa_cursores.py`).
- Claude — "Siempre encima" entra en celdas y pilas. Pilas guardadas en la nota (`pile`,
  `find_piles_keyed`, con test). El guion de prueba cubre el despliegue al pasar el mouse y Ctrl
  sobre una pila (`examples/despliegue_guion.py`, verificado).
- Codex + Claude — Cifrado de `notes.json` (DPAPI) y de Drive con contraseña; auditado
  (security-auditor) y corregido (sobre con `format:1000` para versiones viejas, copia en claro
  rechazada con contraseña puesta, mínimo 10 caracteres, primera activación con Drive en claro).
  Verificado: `notes.json` queda cifrado y se vuelve a leer.
- Falta probar a mano: el diálogo de contraseña y Drive real entre dos compus; el marco del borde
  en modo vidrio y en notas privadas; una pila con notas "siempre encima"; la franja oscura de una
  nota creada enrollada en un monitor con otra escala (acá los dos monitores están a 100 %).
- OBS ("captura de juego"): queda para después, como pidió el usuario.

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

## En curso (rama `feat/privacidad`, sale de `feat/celdas`)

- Claude — Texto oculto (`Ctrl+H`, barrita de formato, clic derecho): cápsula con puntitos, clic
  para mostrar/tapar, se tapa al enrollar o cerrar; fuera del título, vistas previas y búsqueda
  (`visible_text`, con test). Notas con texto oculto fuera de las capturas de pantalla; ajuste
  "Ocultar las notas al compartir pantalla" (`hideCapture`) para todas, el selector y "Todas las
  notas". Las privadas son poseídas por el escritorio (como en vidrio).
- Arreglos que salieron de esto: una sola escala para las notas en todos los monitores
  (`note_dpi` = ppp del sistema; sin resize por `WM_DPICHANGED`), y `detach` que conserva el tamaño.
- Verificado (instancia de prueba en el monitor 2 a 150 %): la nota con texto oculto no sale en la
  captura y la otra sí; ocultar texto en una nota anclada la saca de la captura y mostrarlo la
  devuelve, tres veces sin cambiar de tamaño; clic que muestra y tapa una sola clave; lo guardado
  (`h16+5`) y nada al mostrarlo; el ajuste para todas.
- Falta probar a mano: OBS, Teams y Discord de verdad; `Ctrl+H` y arrastrar sobre lo oculto con el
  mouse real; enrollar con una clave a la vista; modo vidrio; Win+D con una nota privada.
- Clic derecho sobre lo oculto: lo copia directo, sin menú (la cápsula se llena con un tilde); copiar algo oculto (menú o Ctrl+C) va como
  texto plano fuera del historial del portapapeles y de la nube. Verificado con el menú en la
  instancia de prueba (formatos `ExcludeClipboardContentFromMonitorProcessing` y compañía).
  Ctrl+X sigue siendo el corte común.
- Clic derecho sobre lo oculto: se atiende al apretar (antes el RichEdit movía el cursor ahí).
  Verificado: copia y el cursor queda donde estaba.
- Arrastrar entre monitores: al cruzar, el arrastre toma la cuadrícula y las pilas del monitor
  nuevo (`Drag::rehome`), con la nota o la pila entera (Ctrl); la pila de la que salió se cierra
  al soltar, o vuelve con Esc. Verificado con guion (cursor falso): suelta del monitor 1 a una
  pila del 2, pila con Ctrl del 1 al 2, y sacar una de la pila del 2 al 1.
- Nota asomada (Abrir bajo el mouse): lo que se le cambia de tamaño mientras está asomada no
  queda; al volver recupera lugar y tamaño (vuelve a su pila). Sin probar (no hay guion para
  asomar).
- Prueba: sin el botón de enrollar; en Manual, un clic en la barra enrolla/desenrolla (si llega
  el doble clic para renombrar, se deshace). Sin probar con mouse real (DragDetect).
- Auto: una nota abierta no se enrolla con el mouse a menos de 40 px (no sobre otra nota),
  mientras se selecciona texto con el botón apretado, ni con su menú abierto.
- Hecho: cifrado de `notes.json` con DPAPI y de Drive con contraseña (CNG, PBKDF2-SHA256 + AES-256-GCM).
  Migración desde JSON en claro, respaldo de blobs ilegibles, pausa sin fusionar ante contraseña
  ausente o incorrecta, y cambio o eliminación desde Configuración. Build release sin warnings y
  tests sin red pasando, incluidos DPAPI y vectores RFC/NIST. Pendiente: prueba visual del diálogo
  y sincronización real entre dos compus (ver ARCHITECTURE.md).
- Pendiente: el usuario vio una vez las notas "trabadas" (no se movían ni respondían) después de
  instalar esta versión; se destrabaron reiniciando la app. Sin reproducir: sin captura del mouse
  ni ventanas tapando después, y un registro de 3 minutos de uso (`GetGUIThreadInfo`, ventana bajo
  el mouse) no mostró nada raro. Si vuelve: antes de reiniciar, mirar `GetGUIThreadInfo` (captura,
  move/size) del hilo de Simpcky.
- Raro visto en los datos: "Copypastes" quedó con `h=120` (el mínimo) y `x` fuera de pantalla con
  su ancho; revisar cuándo se guarda el alto de una nota enrollada.

## Hecho (rama `feat/celdas`, commiteada en `6e6b168`)

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
