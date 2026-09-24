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

## Estado actual (v0.4.1)

Ya funciona:

- Notas nativas con encabezado de color, arrastre, y cuerpo de texto en
  un control RichEdit real (fuente Segoe UI).
- **Enrollado con el switch pedido**: toda nota nace en modo **Manual**
  (nunca se enrolla sola). Desde el menú "⋯" de cada nota se puede pasar
  a **Auto** (se enrolla al quitar el mouse, se despliega al pasar por
  encima). El valor por defecto para notas nuevas se cambia en
  Configuración y sincronización.
- Cada nota es, por defecto, un **widget de escritorio**: anclada
  dentro de la ventana que contiene los íconos del escritorio, hija de
  verdad (`WS_CHILD`, no un `WS_POPUP` "poseído" — si no, se esconde
  sola al hacer clic en el escritorio), **en capas** (`WS_EX_LAYERED`,
  sin eso en Windows 11 24H2+ no se dibuja: ver abajo), arriba de la
  capa de íconos para que se pueda tocar, en coordenadas relativas al
  padre correctamente convertidas, con vigilancia cada 4 s por si
  `explorer.exe` se reinicia (pero solo actúa si de verdad hace falta:
  reanclar sin necesidad corta el foco de lo que estés escribiendo).
  Así sobrevive a "Mostrar escritorio". Con el pin del encabezado (o
  "⋯" → Siempre encima) se convierte en una ventana normal, con botón en
  la barra de tareas y por encima de todo.
- **Encabezado**: el título, y los botones (nota nueva, siempre encima,
  enrollar, "⋯") solo con el mouse encima o mientras se escribe, con
  ayudas al posar el mouse. Arrastrar la barra mueve la nota; un clic
  suelto ya no la enrolla (para eso está el botón y `Ctrl+R`).
- **Menú "⋯" propio** (`flyout.rs`, al estilo de Windows 11): los seis
  colores como muestras — el único lugar donde se elige el color —,
  siempre encima, enrollado automático, cambiar nombre, duplicar, todas
  las notas y eliminar. No toma el foco: la nota sigue activa.
- **Texto con formato** (`editor.rs`): clic derecho sobre el texto para
  negrita, cursiva, subrayado y tachado (o `Ctrl+B/I/U/T`), cortar,
  copiar, pegar y emojis. Lo pegado toma la letra y los colores de la
  nota. El formato se guarda aparte del texto (`fmt`, ver
  `richtext.rs`), en una forma canónica: no RTF, que cada versión de
  Windows escribe distinto y haría que dos compus se pasaran la misma
  nota "arreglándola" para siempre.
- **Listas de tareas y viñetas**: `[] ` o `- ` al comienzo de una línea
  (o el clic derecho, o `Ctrl+Shift+C` / `Ctrl+Shift+L`). La marca vive
  en el texto (`☐ `, `☑ `, `• `), así que una versión vieja o un copiar y
  pegar la siguen mostrando. Clic en la casilla para tildar (queda en
  gris y tachada), Enter sigue la lista, Enter en un ítem vacío o
  Retroceso junto a la marca la terminan.
- **Emojis en color**: el RichEdit de Windows los dibuja con GDI, en
  blanco y negro; se repintan encima con Direct2D/DirectWrite
  (`d2d.rs`), igual que las casillas (GDI+). El título también va con
  DirectWrite.
- **Escala**: encabezado, botones, márgenes y menús siguen los ppp del
  monitor (a 150 % antes todo quedaba chico).
- **Transparencia con Windhawk** (`glass.rs`): con el mod "Translucent
  Windows" de Windhawk activo, las notas toman su desenfoque, como el
  resto de las aplicaciones. El mod solo toca ventanas de primer nivel con
  estilo de ventana (`WS_POPUPWINDOW`), y al crearlas; una nota anclada es
  hija del escritorio. Así que en este modo las notas del escritorio son
  ventanas de primer nivel **poseídas** por el escritorio: Windows las
  mantiene justo encima de él (detrás de las aplicaciones, visibles con
  "Mostrar escritorio"), y al activarlas no pasan adelante de ninguna
  aplicación (`WM_WINDOWPOSCHANGING`). Se pintan con alfa de verdad: el
  color de la nota premultiplicado en un DIB de 32 bits, porque GDI
  escribe alfa 0 y el compositor mostraría ese color aclarado. Los íconos
  van con una máscara propia; el texto del RichEdit lo recompone el mod.
  Intensidad en Configuración y sincronización → Transparencia
  (requiere Windhawk): suave, media o fuerte (la más transparente). En
  modo claro el color de la nota cubre bastante más (la tinta es
  oscura: sobre un fondo claro casi transparente no se leía). Sin el mod, las notas vuelven solas a ancladas y lisas. El
  `WS_BORDER` que el mod necesita para reconocerlas se les saca apenas
  se crean (si no, el vidrio lleva un marco gris de 1 px).
- **Barra de desplazamiento fina**, del color de la nota: la de Windows
  queda recortada fuera de la vista y se dibuja una rayita que se
  ensancha con el mouse y se arrastra.
- **Si no se puede guardar**, avisa: una notificación, un ícono en la
  barra de cada nota y un ítem en el menú de la bandeja, y reintenta
  solo cada 30 s. (Un antivirus corporativo llegó a bloquearle la
  escritura a Simpcky durante horas sin que nada se enterara.)
- **Nombre propio** para cada nota, siempre visible en el encabezado.
  Sin nombre, muestra la primera línea del texto (y si está vacía,
  "Nota"). **Doble clic en el título**, `F2`, o "⋯" → Cambiar nombre
  abre un cuadro ahí mismo con el nombre seleccionado — como renombrar
  un archivo en el Explorador. Enter confirma, Esc cancela; dejarlo
  vacío vuelve al nombre automático.
- **Redimensionable** desde cualquier borde o esquina (vía
  `WM_NCHITTEST`, sin necesidad de `WS_THICKFRAME`), con un tamaño
  mínimo razonable. Deshabilitado mientras está enrollada. Windows solo
  hace ese cambio de tamaño en ventanas hijas (las ancladas) o con marco
  grueso, así que las de primer nivel (siempre encima, asomadas, o
  cualquiera en modo vidrio) lo hacen a mano: se toma el mouse y el
  borde lo sigue hasta soltar. Los bordes agarran 8 px y las esquinas
  18 px de cada lado (el texto le deja pasar el mouse a la nota ahí),
  y con el mouse encima aparece una agarradera de tres puntitos abajo a
  la derecha.
- **Sacarle el "siempre encima"** a la nota que se está usando no la
  manda al escritorio de golpe: se queda adelante, asomada, hasta que
  el foco pase a otra cosa.
- **Escala de las notas** (100, 125 o 150 %, en Configuración): letra,
  encabezado, márgenes y la ventana entera, en proporción, sobre el
  ppp del monitor.
- **Traer una nota al frente con un atajo** (`picker.rs`): Ctrl+Alt+N
  (o Ctrl+Alt+Espacio, o ninguno, en Configuración) muestra la lista de notas,
  la última usada primero; se escribe para filtrar, flechas y Enter (o
  clic), y la elegida se asoma adelante sin quedar "siempre encima".
  También trae las ocultas.
- **Ícono propio**, simple: un cuadrado amarillo redondeado con la
  franja del encabezado arriba (`assets/simpcky.ico`, 16 a 256 px).
  `build.rs` lo embebe en el `.exe` (arma el `.res` a mano, sin
  `rc.exe` ni dependencias), así que el mismo ícono se ve en el
  Explorador, la bandeja, Alt+Tab y el clic derecho del escritorio.
- Eliminar nota (con confirmación).
- **Ocultar una nota** ("⋯" → Ocultar, `Ctrl+W`, o cerrarla con
  Alt+F4): deja el escritorio y queda guardada en "Todas las notas", con
  la tarjeta apagada y el ojo tachado. Vuelve con doble clic o
  arrastrándola afuera. Cerrar una nota ya no la borra.
- **El menú de la nota** se abre solo con "⋯" (antes también con clic
  derecho en la barra). El clic derecho en el texto sigue siendo el del
  formato.
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
  abajo). Clic derecho en una tarjeta: abrir, ocultar, cambiar nombre,
  eliminar. Flechas para moverse, Esc para cerrar. Con el mod de
  Windhawk la ventana es de vidrio: la píldora del buscador se compone
  igual que el cuadro de texto que tiene adentro (alfa 0, como GDI),
  así no se ve un rectángulo de otro tono.
- **Configuración y sincronización**: la otra pestaña de "Todas las
  notas" (el engranaje, o el menú de la bandeja; `settings.rs`). La
  cuenta de Google, el enrollado de las notas nuevas, el tema, la
  transparencia con Windhawk, iniciar con Windows, el clic derecho del
  escritorio y las actualizaciones. El menú de la bandeja quedó en lo
  de todos los días: nueva nota, todas las notas, configuración, salir.
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
- **Modo oscuro / claro**: en Configuración y sincronización.
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
  Se puede apagar desde Configuración y sincronización.
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
  escritorio, `F2` cambiar nombre, `Ctrl+W` ocultar (`Ctrl+N` también
  en "Todas las notas"). Se resuelven en el bucle de mensajes (`main.rs`), antes
  de que el RichEdit se quede con la tecla. A propósito **no** son
  *hotkeys* globales: registrar Ctrl+N a nivel sistema se lo robaría a
  todas las demás apps de Windows.
- Icono en la bandeja: clic izquierdo crea una nota, clic derecho abre
  el menú (nueva nota, todas las notas, configuración y
  sincronización, salir).
- "Iniciar con Windows" real, vía `HKCU\...\Run`.
- **Sincronización con Google Drive (opcional)**: las mismas notas en
  todas tus compus, en una carpeta oculta de tu Drive que solo ve
  Simpcky. Se conecta desde Configuración y sincronización; la guía para crear el
  cliente de Google (una sola vez) está en
  [`docs/sincronizacion-google.md`](docs/sincronizacion-google.md).
  Viajan el contenido (nombre, texto y formato) y el color, cada parte
  con su hora, con lápidas para lo borrado: lo que se escribe en una
  compu nunca se pierde por algo que pasó en otra. Dónde está cada nota
  en el escritorio (posición, tamaño, siempre encima, enrollada,
  oculta) es de cada compu: una nota nueva que llega de otra toma de
  allá solo su lugar inicial. Todo con lo que trae
  Windows (WinHTTP, CNG, DPAPI), sin bibliotecas nuevas. Cubierto por
  tests (`cargo test`; los que usan la red real, con
  `cargo test -- --ignored`), y probado de punta a punta con una cuenta
  real: la subida coincide byte a byte con lo esperado (MD5 de Drive),
  una compu nueva recibe las notas con su identidad y posición, y una
  compu con una versión vieja se corrige sola sin volver a subirla.
- Persistencia en `%APPDATA%\Simpcky\notes.json`, con autoguardado
  (debounce ~600 ms al escribir; inmediato al mover/cambiar ajustes),
  y las preferencias en `settings.json` al lado. (Hasta la 0.3 el
  RichEdit no avisaba los cambios —faltaba `ENM_CHANGE`— y lo escrito
  solo se guardaba al sincronizar o al salir.)
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

Medido en esta máquina: binario de **~440 KB** con el ícono embebido
(release, LTO, strip, `panic=abort`); con tres notas abiertas, 19 MB de
memoria privada, estable (sin fugas de objetos GDI ni de memoria tras
cientos de repintados). En la 0.3 eran 256 KB y 3,8 MB con ocho notas:
la diferencia es Direct2D/DirectWrite y la fuente de emojis en color.
La especificación pedía ≤ 500 KB y 8–12 MB con diez notas.

Diferencias que quedan contra el lienzo de diseño:

- **Insignia de capa** dentro del cuerpo (la píldora "Normal" abajo a
  la derecha, badge 4): a propósito no está. El cuerpo entero es el
  RichEdit, así que no hay dónde dibujarla sin taparle texto, y el pin
  del encabezado ya dice en qué capa está la nota.
- **`Supr` para eliminar la nota**: tampoco, y a propósito. El foco
  normal de una nota es su cuadro de texto, donde `Supr` tiene que
  borrar caracteres. Eliminar sigue estando en el menú "⋯".
- El JSON guarda `pinned` como parte de `layer` (son la misma cosa en
  este modelo), y las tareas no son un campo aparte: viven en el texto.

### Sobre "Anclar al escritorio"

Las notas se reparentan en la ventana que contiene los íconos
(`SHELLDLL_DefView`): `Progman` en Windows 11 24H2+, o la `WorkerW` a la
que Explorer los haya mudado. No es una API pública. Hasta la 0.3 se
usaba el truco de los fondos animados (el mensaje `0x052C` y "la
`WorkerW` que sigue" en el orden de apilado), y con "Mostrar escritorio"
esa "siguiente" pasaba a ser una `WorkerW` oculta: las notas
desaparecían. Si `explorer.exe` se reinicia, cada nota anclada se
reintenta anclar sola cada 4 segundos.

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
- `src/note.rs` — ventana de una nota: encabezado y sus botones,
  arrastre, menú "⋯", enrollado manual/auto, capas, escala.
- `src/editor.rs` — el cuerpo de la nota: RichEdit con formato, listas,
  pegado limpio, emojis y casillas encima, barra fina y menú del clic
  derecho.
- `src/richtext.rs` — el formato como dato (tramos, listas, emojis),
  sin ventanas y con tests.
- `src/flyout.rs` — el menú propio (muestras de color, botones de
  formato, íconos de Segoe Fluent Icons).
- `src/d2d.rs` — Direct2D/DirectWrite a mano (emojis y títulos en
  color).
- `src/glass.rs` — modo vidrio para el mod de Windhawk: detección del
  mod y lienzo con alfa premultiplicado.
- `src/desktop.rs` — anclar una nota al escritorio (el padre de los
  íconos).
- `src/allnotes.rs` — ventana "Todas las notas": buscador y grilla de
  tarjetas, dibujada a mano con GDI+ y doble buffer.
- `src/settings.rs` — la pestaña "Configuración y sincronización".
- `src/picker.rs` — el selector de notas del atajo global.
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
