# Decisiones de arquitectura

## Texto oculto y notas fuera de las capturas (2026-10-30)

**Contexto.** Hay quien guarda claves en las notas: que no se lean por encima del hombro ni al compartir pantalla (Teams, Discord, OBS…), sin depender de la aplicación que captura.

**Decisión.**
- **Texto oculto** como un estilo más (`h` en `fmt`, `richtext::SECRET`). En el RichEdit se marca con `bAnimation` (el RichEdit lo guarda y lo hereda al escribir como la negrita, pero no lo dibuja; probado). Tapado, la tinta es del color del fondo y encima se dibuja la cápsula, en el mismo lienzo que el texto: lo real no llega a la pantalla aunque la cápsula se atrase. Mostrarlo no se guarda ni se puede deshacer, y se vuelve a tapar al enrollar o cerrar la nota.
- **Fuera de las capturas** con `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. Windows lo niega a las ventanas hijas (error 87, probado), y una nota anclada es hija del escritorio. Las privadas (con texto oculto, o todas si se eligió en Configuración) usan lo que ya existía para el modo vidrio: ventana de primer nivel poseída por el escritorio (`desktop::own`), que queda detrás de las aplicaciones y se ve con "Mostrar escritorio". `note::keep_on_desktop` decide anclada o poseída para cada nota.
- **Escala del texto.** El RichEdit dibuja al ppp del sistema, no al de su ventana, y solo se acomoda con el aviso de cambio de ppp, que llega al cambiar de monitor pero no al anclarse, y que además agranda los márgenes (medido). Una nota de primer nivel en un monitor con otra escala tenía encabezado a 150 % y texto a 100 %. Ahora la nota entera se dibuja al ppp del sistema en cualquier monitor (`note::note_dpi`, como siempre hicieron las ancladas), no cambia de tamaño con `WM_DPICHANGED` y el RichEdit no recibe esos avisos: una nota mide y se ve igual en los dos monitores, y la cuadrícula usa la misma escala (pedido del usuario: "mantener el tamaño entre monitores"). `desktop::detach` conserva el tamaño: el aviso de ppp en el medio de `SetParent` la agrandaba cada vez.

**Límites conocidos.** No es un mecanismo de seguridad (Microsoft lo aclara): no cubre el escritorio remoto, la "captura de juego" de OBS ni una foto. El texto oculto viaja tal cual a Drive y queda en `notes.json` sin cifrar (el cifrado es otra tarea). Las ancladas en un monitor con otra escala siguen al ppp del principal (hallazgo anterior).

**Cómo verificar.** Instancia de prueba con dos notas en el monitor 2, una con `fmt` `h…`: en una captura solo aparece la otra, y `GetWindowDisplayAffinity` da `0x11`. Para ver el dibujo hace falta un ejecutable aparte sin la exclusión (`PrintWindow` también devuelve negro).

## Celdas y pilas (2026-10-29)

**Contexto.** Acomodar notas rápido: arrastrarlas a celdas de tamaño de nota, y apilar notas enrolladas una debajo de otra, como ya se hacía a mano. Se diseñó con un boceto interactivo antes de tocar la app.

**Decisión.**
- Todo en `snap.rs`, con las transiciones en `anim.rs`. **El arrastre por la barra es propio** (captura del mouse en `note.rs`, `snap::begin_move`/`drag_move`/`drop`), no el de Windows: con el de Windows la nota no podía cambiar de forma mientras se la llevaba (tomar el tamaño elegido en el panel, volverse barra al ir a una pila), que es lo que hace que se sienta bien.
- El redimensionado sigue siendo el de Windows (ancladas) o el manual de `note.rs` (primer nivel); Ctrl + borde se engancha en `WM_WINDOWPOSCHANGING`, cuyas coordenadas están bien definidas en los dos casos.
- **Transiciones** (`anim.rs`): posición y tamaño por separado (~200 ms, salida suave), así una nota se enrolla mientras el arrastre la lleva. Al enrollarse o abrirse el texto se queda quieto y la nota lo tapa o descubre; si cambia el ancho, el texto lo acompaña. Con los efectos de animación de Windows apagados, todo es inmediato.
- **Capas** (cuadrícula, vista previa, línea de la pila, panel, guía): ventanas en capas de primer nivel, transparentes a los clics, que aparecen con un fundido y se deslizan. La cuadrícula no se dibuja donde hay notas, así se ve "debajo" de ellas como en el boceto. Se probó ponerla de verdad debajo (hija del escritorio, arriba de los íconos), pero en Windows 11 24H2 una hija de `Progman` con `UpdateLayeredWindow` no se compone: no se ve.
- **Las pilas no se guardan**: se reconocen por la posición (enrolladas, alineadas por un borde, una cada barra + 10 px). Sirve para las pilas armadas a mano y viaja por Drive como cualquier posición. Si eso falla en la práctica (monitores con distinta escala), el paso siguiente es guardar la pila en la nota.
- La geometría (cuadrícula, recortar una nota tapada, reconocer pilas) no usa ventanas y tiene tests.
- La capa que dibuja es una ventana en capas transparente a los clics, como `ghost.rs`, y solo se redibuja si cambia lo que muestra.

**Límites conocidos.** Celdas y pilas usan el monitor donde empezó el arrastre. Las notas "siempre encima" quedan afuera.

**Cómo verificar.** `cargo test snap` (geometría). Sin mouse: una instancia de prueba con `SIMPCKY_GUION` corre un guion de arrastres con un cursor falso (mismo código que el arrastre real salvo leer el mouse) y `examples/celdas_guion.py` saca una captura en cada paso `foto`. Lo que el guion no cubre: el despliegue al pasar el mouse y Ctrl sobre una pila (miran el mouse real).


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
