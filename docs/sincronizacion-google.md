# Sincronización con Google Drive

Simpcky puede mantener las mismas notas en varias compus a través de tu
cuenta de Google. Es **opcional**: si nunca la conectás, la app funciona
exactamente igual, sin tocar la red.

## Qué hace, y qué no

- Las notas se guardan en una **carpeta oculta de tu Drive que solo ve
  Simpcky** (`appDataFolder`). No aparece en tu Drive, y Simpcky no puede
  ver ni tocar ningún otro archivo tuyo: el permiso que pide
  (`drive.appdata`) no lo permite. Google lo clasifica como permiso **no
  sensible**.
- Viajan el nombre, el texto, el color, la posición, el tamaño y el
  estado de cada nota. Las preferencias (tema, iniciar con Windows) son
  de cada compu.
- Si en una compu la nota cae fuera de la pantalla (monitores distintos),
  se muestra ajustada al borde sin cambiar el dato: la otra compu la
  sigue viendo donde la dejaste.
- Nunca se pierde lo que escribiste por un cambio en otra compu: cada
  parte de la nota (contenido, color, posición, estado) se resuelve por
  separado, y gana el cambio más nuevo de esa parte. Una nota borrada en
  una compu se borra en las demás; si alguien la editó *después* de que
  la borraran, vuelve.
- Iniciás sesión en **tu navegador**, en la página de Google: Simpcky
  nunca ve tu contraseña. La credencial que queda se guarda cifrada con
  tu cuenta de Windows (DPAPI).

Se sincroniza unos segundos después de cada cambio, cada minuto y medio
para traer lo de las otras compus, al arrancar, y cuando elegís
"Sincronizar ahora" en el menú de la bandeja.

## Configuración (una sola vez, ~10 minutos)

Google exige que cada app que usa su API tenga un "cliente OAuth" propio.
Es gratis y se crea una sola vez; después sirve para todas tus compus
(y para cualquiera que use tu compilación de Simpcky).

### 1. Crear el proyecto

1. Entrá a <https://console.cloud.google.com/> con tu cuenta de Google.
2. Arriba, en el selector de proyectos → **Proyecto nuevo** → nombre
   `Simpcky` → **Crear**. Asegurate de que quede seleccionado.

### 2. Activar la API de Google Drive

1. Entrá a <https://console.cloud.google.com/apis/library/drive.googleapis.com>.
2. **Habilitar**.

### 3. Pantalla de consentimiento (Google Auth Platform)

1. Entrá a <https://console.cloud.google.com/auth/branding> y tocá
   **Comenzar** (Get started).
2. **Información de la app**: nombre `Simpcky`, y tu email como correo
   de asistencia. **Siguiente**.
3. **Público**: **Externo**. **Siguiente**.
4. **Información de contacto**: tu email. **Siguiente**.
5. Aceptá la política de datos del usuario de Google → **Continuar** →
   **Crear**.
6. **Acceso a los datos** (Data Access) → **Agregar o quitar permisos**,
   y marcá estos tres:
   - `.../auth/drive.appdata`
   - `openid`
   - `.../auth/userinfo.email`

   → **Actualizar** → **Guardar**.
7. **Desarrollo de la marca** (Branding): para publicar la app, Google
   pide además una página principal y una política de privacidad
   publicadas. Son las de la carpeta `docs/` de este repo, servidas por
   GitHub Pages (ver "Publicar el sitio" más abajo):
   - Página principal: `https://TU-USUARIO.github.io/simpcky/`
   - Política de privacidad: `https://TU-USUARIO.github.io/simpcky/privacidad.html`
   - Dominios autorizados: `TU-USUARIO.github.io`

   → **Guardar**.
8. **Público** (Audience) → **Publicar la app** → confirmar.

   Esto importa: mientras la app está "en prueba", Google hace vencer la
   conexión **cada 7 días** y habría que volver a conectarla. Como
   Simpcky solo pide permisos no sensibles, publicarla no requiere la
   revisión de seguridad de Google.

   **Para probar antes de tener el sitio publicado**: dejala en prueba,
   y en **Público → Usuarios de prueba → Agregar usuarios** sumá tu
   email. Funciona igual; solo que a los 7 días hay que volver a
   conectar.

### 4. Crear el cliente para la app

1. Entrá a <https://console.cloud.google.com/auth/clients> →
   **Crear cliente**.
2. Tipo de aplicación: **App de escritorio**. Nombre: `Simpcky Windows`.
   **Crear**.
3. En el cuadro que aparece, **Descargar JSON**.

### 5. Dárselo a Simpcky

1. Guardá el archivo descargado como **`google_client.json`** en la
   carpeta del proyecto (al lado de `Cargo.toml`).

   No se sube a git (está en `.gitignore`). Para una app de escritorio,
   Google aclara que el "client secret" no se considera secreto de
   verdad: viaja dentro del programa, y la seguridad del inicio de
   sesión la da PKCE. Igual no tiene por qué estar en un repositorio
   público.
2. Compilá: `cargo build --release`. El `build.rs` lo lee y lo embebe.
3. En el menú de la bandeja aparece **Sincronización con Google Drive →
   Conectar con mi cuenta de Google…**.

Para las compilaciones de GitHub Actions (más adelante), en lugar del
archivo se usan dos secretos del repositorio:
`SIMPCKY_GOOGLE_CLIENT_ID` y `SIMPCKY_GOOGLE_CLIENT_SECRET`.

## Publicar el sitio (GitHub Pages)

La página principal (`docs/index.html`) y la política de privacidad
(`docs/privacidad.html`) se publican gratis con GitHub Pages desde este
mismo repositorio (tiene que ser público):

1. Repositorio en GitHub → **Settings → Pages**.
2. **Source: Deploy from a branch** → rama `master`, carpeta **`/docs`**
   → **Save**.
3. Al minuto quedan en `https://TU-USUARIO.github.io/simpcky/`. Los
   enlaces al repositorio dentro de las páginas se arman solos a partir
   de esa dirección.

## Conectar cada compu

1. Clic derecho en el ícono de Simpcky en la bandeja → **Conectar con mi
   cuenta de Google…**
2. Se abre el navegador: elegí tu cuenta y aceptá. La página termina
   diciendo "Simpcky quedó conectado".
3. Simpcky avisa con una notificación y sincroniza. Las notas que ya
   tenías en esa compu se suman a las de Drive (no se reemplazan).

Para dejar de sincronizar: menú de la bandeja → **Desconectar…**. Las
notas quedan donde están (en la compu y en Drive); solo dejan de viajar.
Para revocar el acceso desde Google: <https://myaccount.google.com/connections>.

## Dónde queda cada cosa

En `%APPDATA%\Simpcky\`:

| Archivo | Qué es |
|---|---|
| `notes.json` | Tus notas en esta compu. |
| `sync.json` | Con qué cuenta está conectada, el estado de la última sincronización y las "lápidas" de notas borradas. |
| `google.dat` | La credencial de Google, cifrada con tu cuenta de Windows. Borrarlo equivale a desconectar. |
