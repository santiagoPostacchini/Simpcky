# Decisiones de arquitectura

## Texto oculto y notas fuera de las capturas (2026-10-30)

**Contexto.** Hay quien guarda claves en las notas: que no se lean por encima del hombro ni al compartir pantalla (Teams, Discord, OBS…), sin depender de la aplicación que captura.

**Decisión.**
- **Texto oculto** como un estilo más (`h` en `fmt`, `richtext::SECRET`). En el RichEdit se marca con `bAnimation` (el RichEdit lo guarda y lo hereda al escribir como la negrita, pero no lo dibuja; probado). Tapado, la tinta es del color del fondo y encima se dibuja la cápsula, en el mismo lienzo que el texto: lo real no llega a la pantalla aunque la cápsula se atrase. Mostrarlo no se guarda ni se puede deshacer, y se vuelve a tapar al enrollar o cerrar la nota.
- **Fuera de las capturas** con `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. Windows lo niega a las ventanas hijas (error 87, probado), y una nota anclada es hija del escritorio. Las privadas (con texto oculto, o todas si se eligió en Configuración) usan lo que ya existía para el modo vidrio: ventana de primer nivel poseída por el escritorio (`desktop::own`), que queda detrás de las aplicaciones y se ve con "Mostrar escritorio". `note::keep_on_desktop` decide anclada o poseída para cada nota.
- **Escala del texto.** El RichEdit dibuja al ppp del sistema, no al de su ventana, y solo se acomoda con el aviso de cambio de ppp, que llega al cambiar de monitor pero no al anclarse, y que además agranda los márgenes (medido). Una nota de primer nivel en un monitor con otra escala tenía encabezado a 150 % y texto a 100 %. Ahora la nota entera se dibuja al ppp del sistema en cualquier monitor (`note::note_dpi`, como siempre hicieron las ancladas), no cambia de tamaño con `WM_DPICHANGED` y el RichEdit no recibe esos avisos: una nota mide y se ve igual en los dos monitores, y la cuadrícula usa la misma escala (pedido del usuario: "mantener el tamaño entre monitores"). `desktop::detach` conserva el tamaño: el aviso de ppp en el medio de `SetParent` la agrandaba cada vez.

**Límites conocidos.** No es un mecanismo de seguridad (Microsoft lo aclara): no cubre el escritorio remoto, la "captura de juego" de OBS ni una foto. El texto oculto viaja tal cual dentro de las notas, que ahora van cifradas (ver "Cifrado local y de la copia de Drive"). Todas las notas, ancladas o no, se dibujan al ppp del sistema en cualquier monitor (decisión de arriba); falta probar a mano el caso viejo de una nota creada enrollada en un monitor con otra escala (franja oscura bajo la barra).

**Cómo verificar.** Instancia de prueba con dos notas en el monitor 2, una con `fmt` `h…`: en una captura solo aparece la otra, y `GetWindowDisplayAffinity` da `0x11`. Para ver el dibujo hace falta un ejecutable aparte sin la exclusión (`PrintWindow` también devuelve negro).

## Fluidez y borde para cambiar el tamaño (2026-11-03)

**Contexto.** Las transiciones avanzaban con `SetTimer(10 ms)`, que Windows redondea a ~15,6 ms: 10 pasos por animación, saltos de ~80 px al enrollar y algún tirón. El despliegue en Auto esperaba al próximo vistazo (hasta 150 ms). Y el borde que cambia el tamaño era una franja invisible de 8 px hacia adentro: no se veía dónde empezaba ni dónde terminaba. Se compararon dibujos con demos en Direct2D y Slint (rama `demo/raven`, `demos/README.md`): la fluidez la decidía el reloj, no el dibujo.

**Decisión.**
- **Cuadros al ritmo de la pantalla** (`frames.rs`): un hilo espera cada cuadro del compositor (`DwmFlush`) y avisa a una ventana de mensajes; solo mientras alguien los pide (transiciones, arrastre, el marco del borde), y sin acumular avisos. Llegan también dentro de los bucles modales de Windows. Los fundidos de las capas dependen del tiempo, no de la cantidad de cuadros.
- **Auto** se despliega apenas entra el mouse (`on_hover`); el vistazo para enrollar pasó a 60 ms.
- **Marco del borde** (`edges.rs`): con el mouse sobre una nota, una ventana en capas la rodea, 6 px afuera. Sus píxeles de afuera tienen alfa 1 (no se ven, pero toman el mouse) y los de adentro alfa 0 (el mouse llega a la nota). Dibuja el contorno de la nota y resalta el borde o la esquina que se va a mover; el clic se lo pasa a la nota, que redimensiona como siempre. Poseído por la nota (o por el escritorio si está anclada), queda justo encima de ella y debajo de las aplicaciones. El cursor sigue siendo la flecha en la barra (elegido por el usuario).

**Medido** (monitor de 400 Hz, `demos/bench/smooth.py` de la rama `demo/raven`, 3 corridas): enrollar 27–30 pasos (antes 10), salto máximo 20–33 px (antes 80), 0 tirones (antes 1,7 por animación); encaje en celda 18 pasos (antes 10); arrastre igual (1,6 ms detrás del cursor). No llega a los 56 pasos de las demos: con GDI cada paso cuesta ~5 ms entre redibujar la nota y el escritorio de abajo. CPU al enrollar: 300 ms en vez de 130 (más cuadros).

**Límites conocidos.** El marco aparece después de que el mouse pasa por la nota: llegando desde afuera, el borde se agarra recién al entrar en ella (la franja de 8 px de adentro sigue estando). Ganar más pasos en las transiciones pide dibujar distinto (ver "Alternativa descartada por ahora" del parpadeo).

**Cómo verificar.** Medir: `python smooth.py --exe target/release/simpcky.exe --label X --runs 3` (de `demos/bench` en `demo/raven`). Borde, con el mouse real (mueven el cursor: no tocar nada): `python examples/mapa_cursores.py target/release/simpcky.exe` (qué cursor hay en cada punto alrededor de una nota) y `python examples/borde_prueba.py target/release/simpcky.exe capturas` (cursor a 3 y 5 px afuera del borde, capturas del resaltado y arrastre desde afuera: la nota se agranda 60 px).

## Celdas y pilas (2026-10-29)

**Contexto.** Acomodar notas rápido: arrastrarlas a celdas de tamaño de nota, y apilar notas enrolladas una debajo de otra, como ya se hacía a mano. Se diseñó con un boceto interactivo antes de tocar la app.

**Decisión.**
- Todo en `snap.rs`, con las transiciones en `anim.rs`. **El arrastre por la barra es propio** (captura del mouse en `note.rs`, `snap::begin_move`/`drag_move`/`drop`), no el de Windows: con el de Windows la nota no podía cambiar de forma mientras se la llevaba (tomar el tamaño elegido en el panel, volverse barra al ir a una pila), que es lo que hace que se sienta bien.
- El redimensionado sigue siendo el de Windows (ancladas) o el manual de `note.rs` (primer nivel); Ctrl + borde se engancha en `WM_WINDOWPOSCHANGING`, cuyas coordenadas están bien definidas en los dos casos.
- **Transiciones** (`anim.rs`): posición y tamaño por separado (~200 ms, salida suave), así una nota se enrolla mientras el arrastre la lleva. Al enrollarse o abrirse el texto se queda quieto y la nota lo tapa o descubre; si cambia el ancho, el texto lo acompaña. Con los efectos de animación de Windows apagados, todo es inmediato.
- **Capas** (cuadrícula, vista previa, línea de la pila, panel, guía): ventanas en capas de primer nivel, transparentes a los clics, que aparecen con un fundido y se deslizan. La cuadrícula no se dibuja donde hay notas, así se ve "debajo" de ellas como en el boceto. Se probó ponerla de verdad debajo (hija del escritorio, arriba de los íconos), pero en Windows 11 24H2 una hija de `Progman` con `UpdateLayeredWindow` no se compone: no se ve.
- **Pilas**: se reconocen por la posición (enrolladas, alineadas por un borde, una cada barra + 10 px), lo que sirve para las armadas a mano. Además cada nota guarda el número de su pila (`pile`, de esta compu como la posición, no va a Drive): `settle_pile` se lo pone a todas las de una pila y se borra al sacarla (a una celda o suelta) o al ocultarla. Las que comparten número forman su pila aunque se desacomoden (`find_piles_keyed`, con test); las demás, por la posición.
- La geometría (cuadrícula, recortar una nota tapada, reconocer pilas) no usa ventanas y tiene tests.
- La capa que dibuja es una ventana en capas transparente a los clics, como `ghost.rs`, y solo se redibuja si cambia lo que muestra.

- Las notas "siempre encima" entran en celdas y pilas como las demás y siguen encima (2026-11-03).

**Límites conocidos.** Al cruzar de monitor durante el arrastre se toman la cuadrícula y las pilas del nuevo (`Drag::rehome`). En una pila que mezcla notas "siempre encima" con otras, las de encima quedan sobre las demás aunque se despliegue una de abajo.

**Cómo verificar.** `cargo test snap` (geometría). Sin mouse: una instancia de prueba con `SIMPCKY_GUION` corre un guion de arrastres con un cursor falso (mismo código que el arrastre real salvo leer el mouse) y `examples/celdas_guion.py` saca una captura en cada paso `foto`. El despliegue al pasar el mouse y Ctrl sobre una pila también leen el cursor falso: `examples/despliegue_guion.py` (paso `sobre <título> <dx> <dy>`) anota la altura de cada nota en cada foto.


## Dibujo del texto de una nota sin parpadeo (2026-10-21)

**Contexto.** Al escribir, la línea escrita y un pedazo de la barra de desplazamiento parpadeaban, sobre todo en modo vidrio. Dos arreglos a ciegas no alcanzaron; se midió con `examples/medir_parpadeo.rs` (instancia de prueba aparte, captura de pantalla por zonas y registro de dibujo con `SIMPCKY_PAINT_LOG`).

**Causa.** El RichEdit es `WS_EX_TRANSPARENT`: ese estilo es lo que lo deja con fondo transparente por dentro, y sin él pinta un fondo opaco que en vidrio queda sin alfa. Pero en la ventana hace que Windows repinte primero la nota debajo de cada línea que cambia. La nota pintaba ahí su color liso y el texto llegaba unos 2 ms después: un cuadro con la línea vacía.

**Decisión.** Un solo dueño por píxel:
- la nota no pinta debajo del texto (`note::exclude_text`);
- el RichEdit pinta todo su rectángulo en un lienzo (fondo, texto con `ITextServices::TxDraw`, selección, casillas, emojis, barra) y lo copia de una vez (`editor::paint_buffered`);
- fuera del `WM_PAINT` solo se redibuja la barra, que no toca el cursor.

**Alternativa descartada por ahora.** RichEdit sin ventana (`CreateTextServices` + un `ITextHost` propio), o dibujo con Direct2D/DirectComposition. Daría control total, pero es el cambio más grande del proyecto y no hizo falta: 0 parpadeos medidos. Si algún día vuelve a haber parpadeo que no se pueda evitar, es el próximo paso.

**Cómo verificar.**
```
cargo build --release
cargo run --release --example medir_parpadeo -- --vidrio si
cargo run --release --example medir_parpadeo -- --vidrio no
cargo test -- --ignored richedit_se_dibuja
```

## Cifrado local y de la copia de Drive (2026-11-03)

**Contexto.** Las notas y la copia de Drive se guardaban en JSON legible. La protección local no debe pedir contraseña; la copia remota debe poder abrirse en otra compu con una contraseña elegida por el usuario, sin sumar dependencias de cripto.

**Decisión.**
- `notes.json` se escribe siempre como `simpcky-dpapi:1:` seguido del blob DPAPI en base64url sin relleno. Se usa la cuenta de Windows, sin entropía adicional. Los JSON viejos se siguen leyendo y se cifran al guardar. Un blob inválido toma el mismo camino de respaldo `notes.json.ilegible-<hora>` que un JSON ilegible.
- `crypto.rs` usa CNG para PBKDF2-HMAC-SHA256 (600 000 iteraciones, sal aleatoria de 16 bytes) y AES-256-GCM (nonce aleatorio de 12 bytes y tag de 16). No hay primitivas criptográficas propias.
- `drive_crypto.rs` envuelve el documento de `sync::doc_json` en `{"format":1000,"v":2,"enc":{"kdf":"pbkdf2-sha256","iter":600000,"salt":"…","nonce":"…","ct":"…"}}`. Los campos binarios son base64url; `ct` concatena el texto cifrado y el tag. El marcador `format:1000` hace que las versiones anteriores rechacen el sobre como una versión más nueva. El JSON sin `enc` sigue siendo el formato viejo; con contraseña local se rechaza antes de fusionar, salvo aceptación explícita pendiente desde Configuración.
- La contraseña vive en `drive-password.dat`, protegida con DPAPI, separado de `google.dat`. La sal se conserva y las claves se cachean en RAM por contraseña, sal e iteraciones (hasta cuatro entradas); PBKDF2 corre en el hilo de sincronización; el dibujo consulta la bóveda sin esperar su mutex. Los fallos de carga se reintentan y los de guardado tras subir se tratan como errores comunes, sin pausa permanente. Al cambiar o quitar la contraseña, se conserva la anterior con DPAPI hasta que la nueva subida termine, incluso si se reinicia la app. El cambio pendiente fuerza la subida aunque las notas no hayan cambiado.
- Configuración ofrece un diálogo con contraseña de al menos 10 caracteres, repetición y aviso de recuperación; los campos se vacían antes de cerrarlo. Una casilla separada permite aceptar una copia remota en claro y volver a cifrarla; la autorización se conserva con DPAPI hasta completar la subida. Desmarcar la opción quita el cifrado de Drive. Mientras hay una sincronización en curso se pide esperar antes de cambiar la contraseña. Si falta o no coincide, se pausa antes de fusionar o subir y se muestra el motivo en Configuración y en el menú de bandeja; guardar la contraseña reanuda. Al arrancar se valida el documento remoto aunque su MD5 no haya cambiado.

**Límites conocidos.** DPAPI depende del perfil de Windows: copiar los archivos locales a otra cuenta no alcanza para recuperarlos. El cifrado de Drive no tiene recuperación de contraseña; las notas locales siguen disponibles. Los respaldos de versiones anteriores pueden conservar texto plano. Un tag inválido también puede indicar daño del archivo. Las compus con versiones anteriores al cifrado rechazan el sobre por su formato y deben actualizarse antes de usar la copia cifrada. No se verificó todavía el diálogo con interacción visual ni la sincronización con una cuenta real de Drive.

**Cómo verificar.** `cargo build --release` sin warnings y `cargo test`: ida y vuelta y migración del archivo local, respaldo de blobs inválidos, documento real de sincronización cifrado, contraseña ausente e incorrecta, cambio y eliminación, caché y sobres inválidos. Vectores: [RFC 7914 §11](https://www.rfc-editor.org/rfc/rfc7914.html#section-11) para PBKDF2-SHA256 y [NIST GCM-AES256, ejemplos 1 y 2](https://csrc.nist.gov/CSRC/media/Projects/Cryptographic-Standards-and-Guidelines/documents/examples/AES_GCM.pdf). Los tests de DPAPI siguen habilitados aunque pueden fallar en un sandbox sin acceso al perfil. Prueba manual pendiente: conectar dos perfiles, cifrar, ingresar una contraseña incorrecta en el segundo, comprobar la pausa sin cambios, corregirla, editar notas y luego cambiar y quitar el cifrado desde el primero.
