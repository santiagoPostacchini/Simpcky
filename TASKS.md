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

- Claude — Candado en la barra de cada nota (sale del menú "⋯"): candado cerrado
  para bloquear, abierto para desbloquear; bloqueada, el texto se atenúa un poco y
  se puede seguir seleccionando y copiando. Ancho mínimo de nota: 180 px, para que
  entren los cinco botones. (`feat/candado-en-barra`)

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
