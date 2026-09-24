# Publicar una versión

Cada versión de Simpcky es un release de GitHub con dos archivos:

- `Simpcky-Setup-X.Y.Z.exe` — el instalador.
- `Simpcky-Setup-X.Y.Z.exe.sha256` — su huella SHA-256.

Los arma GitHub Actions (`.github/workflows/release.yml`) cuando se sube
un tag `vX.Y.Z`. Las copias instaladas buscan una vez por día el último
release, y si es más nuevo avisan; al actualizar, bajan el instalador,
**verifican la huella** y lo ejecutan en silencio (ver `src/update.rs`).

## Una sola vez: los secretos de Google

La versión publicada lleva adentro el cliente de Google de la
sincronización. En el repositorio de GitHub → **Settings → Secrets and
variables → Actions → New repository secret**, crear dos secretos con los
valores de `google_client.json` (el que está en tu compu, que no se sube
al repositorio):

| Nombre | Valor |
|---|---|
| `SIMPCKY_GOOGLE_CLIENT_ID` | el `client_id` (termina en `.apps.googleusercontent.com`) |
| `SIMPCKY_GOOGLE_CLIENT_SECRET` | el `client_secret` (empieza con `GOCSPX-`) |

Sin ellos el workflow se detiene con un error, en vez de publicar una
versión sin sincronización.

## Cada versión

1. Subir el número en `Cargo.toml` (`version = "X.Y.Z"`, [semver](https://semver.org/lang/es/):
   el último número para arreglos, el del medio para funciones nuevas).
2. Commit y push a `master`.
3. Crear y subir el tag (tiene que coincidir con `Cargo.toml`, si no el
   workflow falla):

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

4. En unos minutos aparece en la pestaña **Releases**, con las notas
   generadas a partir de los commits. Las copias instaladas se enteran
   en el día (o al tocar "Buscar ahora" en Configuración y sincronización).

## Armar el instalador en la compu

Con [Inno Setup 6](https://jrsoftware.org/isinfo.php) instalado:

```bash
cargo build --release
"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" /DAppVersion=X.Y.Z installer\simpcky.iss
```

Queda en `dist\Simpcky-Setup-X.Y.Z.exe`. Para probar una actualización
como la hace la app: `Simpcky-Setup-X.Y.Z.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH`.

## Firma digital

Las versiones se firman con SignPath Foundation (gratis para código
abierto) apenas el proyecto quede aceptado: ver
[`firma-signpath.md`](firma-signpath.md). Mientras tanto salen sin firmar,
y Windows SmartScreen puede mostrar "Windows protegió tu PC" →
*Más información* → *Ejecutar de todas formas*. Firmar no borra ese aviso
de un día para el otro (desde 2024 ni los certificados EV lo hacen): la
reputación se gana con descargas, pero con firma se acumula de una versión
a la otra.
