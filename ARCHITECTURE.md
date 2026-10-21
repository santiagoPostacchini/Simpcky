# Decisiones de arquitectura

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
