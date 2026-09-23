# Simpcky

Notas adhesivas ultraligeras para el escritorio de Windows 11. Rust +
Win32 puro (`windows-sys`), sin Electron, sin WebView2, sin .NET.

Sitio: <https://santiagopostacchini.github.io/Simpcky/> ·
[Política de privacidad](https://santiagopostacchini.github.io/Simpcky/privacidad.html) ·
Licencia [MIT](LICENSE).

## Instalar

Bajá `Simpcky-Setup-X.Y.Z.exe` del [último release](https://github.com/santiagoPostacchini/Simpcky/releases/latest)
y abrilo. Se instala para tu usuario (sin permisos de administrador) en
`%LOCALAPPDATA%\Programs\Simpcky`, y se actualiza sola: una vez por día
mira si hay una versión nueva, te avisa, y al aceptar baja el
instalador, verifica su huella SHA-256 y se reinstala sin cerrar tus
notas más que un segundo. Para publicar una versión nueva, ver
[`docs/publicar-version.md`](docs/publicar-version.md); la firma digital
(SignPath Foundation), en [`docs/firma-signpath.md`](docs/firma-signpath.md)
y la [política de firma](https://santiagopostacchini.github.io/Simpcky/firma.html).

## Compilar y ejecutar

```bash
cargo build --release
./target/release/simpcky.exe
```

En debug (`cargo run`) el binario abre una consola detrás; en release no
(`#![windows_subsystem = "windows"]`).

## Estado actual (v0.3)

Ya funciona:

- Notas nativas con encabezado de color, arrastre, y cuerpo de texto en
  un control RichEdit real (fuente Segoe UI).
- **Enrollado con el switch pedido**: toda nota nace en modo **Manual**
  (nunca se enrolla sola). Desde el menú "⋯" de cada nota se puede pasar
  a **Auto** (se enrolla al quitar el mouse, se despliega al pasar por
  encima). El valor por defecto para notas nuevas se cambia desde el
  menú de la bandeja.
- Cada nota es, por defecto, un **widget de escritorio**: anclada
  dentro del escritorio vía el truco de la `WorkerW`/`Progman`, hija de
  verdad (`WS_CHILD`, no un `WS_POPUP` "poseído" — si no, se esconde
  sola al hacer clic en el escritorio), **en capas** (`WS_EX_LAYERED`,
  sin eso en Windows 11 24H2+ no se dibuja: ver abajo), arriba de la
  capa de íconos para que se pueda tocar, en coordenadas relativas al
  padre correctamente convertidas, con vigilancia cada 4 s por si
  `explorer.exe` se reinicia (pero solo actúa si de verdad hace falta:
  reanclar sin necesidad corta el foco de lo que estés escribiendo).
  Así sobrevive a "Mostrar escritorio". Desde el menú "⋯" → Capa, o con
  un clic en el pin del encabezado, se puede pasar a **Siempre
  encima**: ahí se convierte en una ventana normal, con botón en la
  barra de tareas y por encima de todo.
- **Nombre propio** para cada nota, siempre visible en el encabezado.
  Sin nombre, muestra la primera línea del texto (y si está vacía,
  "Nota"). **Doble clic en el título**, `F2`, o "⋯" → Cambiar nombre
  abre un cuadro ahí mismo con el nombre seleccionado — como renombrar
  un archivo en el Explorador. Enter confirma, Esc cancela; dejarlo
  vacío vuelve al nombre automático. El chevron enrolla/desenrolla; el
  resto de la barra arrastra la ventana, y un clic sin mover el mouse
  (expandida) la enrolla — pero recién pasado el tiempo de doble clic,
  para que el doble clic de renombrar no la enrolle antes.
- **Redimensionable** desde cualquier borde o esquina (vía
  `WM_NCHITTEST`, sin necesidad de `WS_THICKFRAME`), con un tamaño
  mínimo razonable. Deshabilitado mientras está enrollada.
- **Ícono propio**, simple: un cuadrado amarillo redondeado con la
  franja del encabezado arriba (`assets/simpcky.ico`, 16 a 256 px).
  `build.rs` lo embebe en el `.exe` (arma el `.res` a mano, sin
  `rc.exe` ni dependencias), así que el mismo ícono se ve en el
  Explorador, la bandeja, Alt+Tab y el clic derecho del escritorio.
- Eliminar nota (con confirmación).
- Clic en el **punto de color** del encabezado: abre la paleta de 6
  colores ahí mismo (en el diseño ese punto es el selector de color,
  no un adorno). También está en el menú "⋯" → Color.
- **Duplicar nota** desde el menú "⋯": copia texto, color, modo y
  tamaño en una nota nueva, corrida un poco.
- **Ventana "Todas las notas"** (menú de la bandeja): buscador y
  grilla de tres columnas con una tarjeta por nota, del color de cada
  una, siguiendo la pantalla "Menús y bandeja" del diseño. Todo
  dibujado a mano (GDI+ con doble buffer): Windows no trae ningún
  control que se parezca. Las notas enrolladas se ven como tarjetas
  bajas, las de "siempre encima" llevan el pin en la esquina, y el
  buscador filtra por nombre **y** por cuerpo. Doble clic (o Enter)
  abre la nota: se **asoma al frente** con el cursor en el texto (ver
  abajo). Clic derecho en una tarjeta: abrir, cambiar nombre,
  eliminar. Flechas para moverse, Esc para cerrar.
- **Arrastrar una tarjeta** de "Todas las notas" y soltarla afuera de
  la ventana: la nota queda ahí como widget de escritorio. Mientras se
  arrastra la sigue una mini nota semitransparente (`ghost.rs`); si
  donde cae hay otra ventana tapando el escritorio, la nota se asoma
  un momento para que se vea dónde quedó.
- **Notas que se asoman**: una nota nueva, una duplicada o una abierta
  desde "Todas las notas" se suelta del escritorio y aparece adelante
  de todo, con el cursor listo; al hacer clic en cualquier otra cosa
  vuelve sola a su lugar en el escritorio. Antes una nota nueva
  nacía anclada al escritorio — detrás de todas las ventanas abiertas —
  y parecía que "Nueva nota" no hacía nada (se creaba igual; solo no
  se veía).
- **Modo oscuro / claro**: "Modo oscuro" en el menú de la bandeja o en
  el "⋯" de cualquier nota, o el botón sol/luna de "Todas las notas".
  Cambia la paleta de todas las notas (en oscuro el color vive en el
  encabezado y el cuerpo es casi negro), la ventana "Todas las
  notas", su barra de título, las barras de desplazamiento y los menús
  nativos. La primera vez arranca como tenga Windows elegido para las
  apps; después se recuerda en `settings.json`.
- **"Nueva nota adhesiva" en el clic derecho del escritorio**: un
  verbo en `HKCU\Software\Classes\DesktopBackground\Shell` (sin
  permisos de administrador, sin DLL de shell). La nota aparece donde
  está el cursor. En el menú compacto de Windows 11 queda dentro de
  **"Mostrar más opciones"** (o directo con Mayús + clic derecho):
  el menú nuevo solo acepta entradas de apps empaquetadas como MSIX.
  Se puede apagar desde el menú de la bandeja.
- **Una sola instancia**: abrir el `.exe` con la app ya corriendo le
  pasa el pedido a la que está abierta (una nota nueva con `--new`, o
  "Todas las notas" a secas) en vez de cargar todas las notas dos
  veces.
- **Sobrevive a un reinicio de Explorer**: las notas ancladas son
  hijas de Progman, y cuando `explorer.exe` se reinicia se destruyen
  con él. Antes eso las **borraba** de `notes.json`; ahora se rescata
  el texto, se conservan los datos y se recrean solas.
- **Atajos de teclado** (sección 7 de la especificación), mientras el
  foco esté en una nota: `Ctrl+N` nueva nota, `Ctrl+R` enrollar /
  desenrollar, `Ctrl+Shift+T` siempre encima, `Ctrl+Shift+D` anclar al
  escritorio, `F2` cambiar nombre (`Ctrl+N` también en "Todas las
  notas"). Se resuelven en el bucle de mensajes (`main.rs`), antes
  de que el RichEdit se quede con la tecla. A propósito **no** son
  *hotkeys* globales: registrar Ctrl+N a nivel sistema se lo robaría a
  todas las demás apps de Windows.
- Icono en la bandeja: clic izquierdo crea una nota, clic derecho abre
  el menú (nueva nota, modo de enrollado por defecto, todas las notas,
  modo oscuro, clic derecho del escritorio, iniciar con Windows,
  salir), en el mismo orden que el diseño.
- "Iniciar con Windows" real, vía `HKCU\...\Run`.
- **Sincronización con Google Drive (opcional)**: las mismas notas en
  todas tus compus, en una carpeta oculta de tu Drive que solo ve
  Simpcky. Se conecta desde el menú de la bandeja; la guía para crear el
  cliente de Google (una sola vez) está en
  [`docs/sincronizacion-google.md`](docs/sincronizacion-google.md).
  La fusión es por partes de cada nota (contenido, color, posición,
  estado), con lápidas para lo borrado: lo que se escribe en una compu
  nunca se pierde por algo que pasó en otra. Todo con lo que trae
  Windows (WinHTTP, CNG, DPAPI), sin bibliotecas nuevas. Cubierto por
  tests (`cargo test`; los que usan la red real, con
  `cargo test -- --ignored`), y probado de punta a punta con una cuenta
  real: la subida coincide byte a byte con lo esperado (MD5 de Drive),
  una compu nueva recibe las notas con su identidad y posición, y una
  compu con una versión vieja se corrige sola sin volver a subirla.
- Persistencia en `%APPDATA%\Simpcky\notes.json`, con autoguardado
  (debounce ~600 ms al escribir; inmediato al mover/cambiar ajustes),
  y las preferencias en `settings.json` al lado.
- Primera ejecución: una **pantalla de bienvenida** (iniciar con
  Windows, el clic derecho del escritorio, el tema, y la sincronización
  con Google como opción), y una nota de bienvenida que explica el modo
  Manual/Auto, el renombrado y cómo crear más.
- **Instalador** (Inno Setup, `installer/simpcky.iss`): para el usuario,
  sin administrador; cierra la app de forma ordenada antes de
  reemplazarla (`simpcky.exe --quit`); al desinstalar limpia el registro
  y pregunta si borrar las notas.
- **Actualizaciones** desde los Releases de GitHub (`src/update.rs`),
  con verificación de la huella del instalador. Los releases los arma
  GitHub Actions al subir un tag (`.github/workflows/release.yml`).

Medido en esta máquina: binario de **~256 KB** con el ícono embebido
(release, LTO, strip, `panic=abort`); con ocho notas abiertas, 3,8 MB
de memoria privada (22 MB de *working set*, que incluye las DLL del
sistema compartidas con el resto de Windows). La especificación pedía
≤ 500 KB y 8–12 MB con diez notas.

Diferencias que quedan contra el lienzo de diseño:

- **Checklist con casillas reales** dentro de la nota (badge 3 de la
  pantalla "Anatomía"): hoy el cuerpo es texto libre en RichEdit. Es
  lo único grande que falta. El RichEdit no sabe dibujar casillas:
  habría que insertarlas como objetos OLE, o reemplazar el control por
  uno propio dibujado a mano (que además resolvería el borde de color
  y el padding sin pelear con `EM_SETRECT`).
- **Insignia de capa** dentro del cuerpo (la píldora "Normal" abajo a
  la derecha, badge 4): a propósito no está. El cuerpo entero es el
  RichEdit, así que no hay dónde dibujarla sin taparle texto, y el pin
  del encabezado ya dice en qué capa está la nota.
- **"Ajustes"** en el menú de la bandeja: no hay ventana de ajustes
  porque todo lo configurable (modo de enrollado por defecto, iniciar
  con Windows) ya vive en ese mismo menú. Mejor eso que un ítem que no
  lleva a ninguna parte.
- **`Supr` para eliminar la nota**: tampoco, y a propósito. El foco
  normal de una nota es su cuadro de texto, donde `Supr` tiene que
  borrar caracteres. Eliminar sigue estando en el menú "⋯".
- El JSON guarda `pinned` como parte de `layer` (son la misma cosa en
  este modelo) y todavía no tiene `checklist`.

### Sobre "Anclar al escritorio"

Usa el mismo truco no documentado que Rainmeter, Wallpaper Engine, etc.
(mandarle a `Progman` el mensaje `0x052C` y reparentar en la `WorkerW`
que aparece detrás de los íconos). No es una API pública: en algunas
sesiones de Windows los íconos nunca migran a esa `WorkerW` y se quedan
colgando directo de `Progman` — `desktop.rs` cubre ese caso reparentando
ahí directamente (confirmado en esta máquina). Si `explorer.exe` se
reinicia, cada nota anclada se reintenta anclar sola cada 4 segundos.

**Windows 11 24H2 en adelante** cambió cómo se arma el escritorio:
`Progman` tiene `WS_EX_NOREDIRECTIONBITMAP` (se compone con
DirectComposition) y ya no tiene una superficie donde se dibujen sus
ventanas hijas comunes. Una nota anclada así figuraba visible, en su
lugar y arriba de los íconos, pero no se veía: aparecía un instante y
se borraba en cuanto el escritorio se redibujaba — por ejemplo, al
hacer clic en él. La solución es la misma que usa Windows para su capa
de íconos (`SHELLDLL_DefView` es `WS_EX_LAYERED`): las notas ancladas
son hijas **en capas**, opacas al 100 %, que tienen su propia
superficie. Windows solo acepta `WS_EX_LAYERED` en ventanas hijas si
el `.exe` declara Windows 8+ en su manifiesto, que `build.rs` embebe
junto con el ícono. Verificado en esta máquina (build 26200) con
capturas antes y después de un clic real en el escritorio.

Las notas van arriba de la capa de íconos, no debajo: esa capa ocupa
todo el escritorio y se queda con cada clic, así que una nota debajo
se vería pero no se podría escribir ni arrastrar.

## Estructura

- `build.rs` — embebe en el `.exe` el ícono (`assets/simpcky.ico`), el
  manifiesto (Windows 8+ y Common Controls 6), la ficha de versión y el
  cliente de Google.
- `installer/simpcky.iss` — el instalador.
- `.github/workflows/release.yml` — compila y publica cada versión.
- `docs/` — el sitio (GitHub Pages) y las guías.
- `src/main.rs` — arranque, instancia única (`--new`), ajustes, carga de
  notas, bucle de mensajes y atajos.
- `src/app.rs` — estado global (notas abiertas, ajustes).
- `src/note.rs` — ventana de una nota: dibujo del encabezado, arrastre,
  menú "⋯", enrollado manual/auto, capas.
- `src/desktop.rs` — el truco de la `WorkerW`/`Progman` para anclar una
  nota al escritorio.
- `src/allnotes.rs` — ventana "Todas las notas": buscador y grilla de
  tarjetas, dibujada a mano con GDI+ y doble buffer.
- `src/sync.rs` — sincronización: la fusión (pura, con tests), cuándo
  sincronizar y cómo aplicar lo que llega de otra compu.
- `src/oauth.rs` — inicio de sesión con Google (navegador + 127.0.0.1 +
  PKCE) y la credencial cifrada.
- `src/drive.rs` — el archivo de notas en la carpeta oculta de Drive.
- `src/http.rs` — cliente HTTPS mínimo sobre WinHTTP.
- `src/crypto.rs` — azar, SHA-256, base64url y DPAPI (todo de Windows).
- `src/json.rs` — lector/escritor de JSON a mano.
- `src/update.rs` — buscar, bajar, verificar e instalar versiones nuevas.
- `src/welcome.rs` — la pantalla de bienvenida.
- `src/rename.rs` — el cuadro para cambiarle el nombre a una nota.
- `src/ghost.rs` — la mini nota que sigue al cursor al arrastrar una
  tarjeta al escritorio.
- `src/theme.rs` — tema claro/oscuro: paletas, barra de título, menús.
- `src/shell.rs` — registro: "Iniciar con Windows" y el clic derecho
  del escritorio.
- `src/icon.rs` — carga el ícono embebido.
- `src/tray.rs` — icono de bandeja, menú principal, notas nuevas.
- `src/persist.rs` — modelo de datos (identidad global y hora de cada
  parte de la nota) y los archivos `notes.json`, `settings.json` y
  `sync.json`.
- `src/win.rs` — helpers pequeños (cadenas UTF-16, `COLORREF`).
