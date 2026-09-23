# Firmar las versiones con SignPath Foundation

[SignPath Foundation](https://signpath.org/) firma gratis los programas de
proyectos de código abierto. El certificado es de SignPath Foundation (el
aviso de Windows dice "SignPath Foundation"), y cada versión se firma en
GitHub Actions después de que la aprobás con un clic.

El workflow (`.github/workflows/release.yml`) ya tiene los pasos de firma:
se activan solos cuando el repositorio tiene la configuración de SignPath.
Hasta entonces, las versiones salen sin firmar.

La política que exige SignPath ya está publicada:
<https://santiagopostacchini.github.io/Simpcky/firma.html>.

## 1. Antes de pedirlo

- **Una versión publicada.** SignPath exige que el proyecto ya esté
  publicado en la forma que se va a firmar: la 0.3.0 (sin firmar) cumple.
- **Verificación en dos pasos en GitHub**:
  <https://github.com/settings/security> → *Two-factor authentication*.
  SignPath la exige para todo el equipo.

## 2. Pedirlo

En <https://signpath.org/> → *Apply*. Datos para el formulario (en
inglés, que es como los revisan):

| Campo | Valor |
|---|---|
| Project name | Simpcky |
| Repository | https://github.com/santiagoPostacchini/Simpcky |
| Homepage | https://santiagopostacchini.github.io/Simpcky/ |
| License | MIT |
| Code signing policy | https://santiagopostacchini.github.io/Simpcky/firma.html |
| Privacy policy | https://santiagopostacchini.github.io/Simpcky/privacidad.html |
| Build system | GitHub Actions (GitHub-hosted `windows-latest` runners) |
| Artifacts to sign | `simpcky.exe` and the Inno Setup installer `Simpcky-Setup-X.Y.Z.exe` |

Descripción sugerida:

> Simpcky is a lightweight sticky-notes app for the Windows desktop,
> written in Rust on top of the plain Win32 API (no Electron, no .NET).
> Notes live on the desktop as widgets. Optional sync across the user's
> own computers through their Google Drive (app-data folder only, opt-in).
> Releases are built by GitHub Actions from the public repository and
> published as GitHub Releases; the app offers in-app updates from those
> releases, verifying the installer's SHA-256 before running it. The app
> has an uninstaller, discloses every network connection in its privacy
> policy, and both network features (sync, update check) can be turned
> off.

La revisión es manual (suele tardar una o dos semanas) y pueden hacer
preguntas.

## 3. Cuando te acepten

En SignPath (<https://app.signpath.io/>), con la organización que te crean:

1. **Proyecto** `simpcky`, vinculado al repositorio de GitHub como
   *trusted build system*.
2. **Artifact configuration** (la predeterminada del proyecto): cada
   pedido trae un ZIP con un único `.exe` adentro:

   ```xml
   <?xml version="1.0" encoding="utf-8"?>
   <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
     <zip-file>
       <pe-file path="*.exe">
         <authenticode-sign />
       </pe-file>
     </zip-file>
   </artifact-configuration>
   ```

3. **Signing policy** `release-signing`, con vos como aprobador.
4. Un usuario de CI con permiso de *submitter*, y su **API token**.

Y en GitHub → *Settings → Secrets and variables → Actions*:

| Tipo | Nombre | Valor |
|---|---|---|
| Variable | `SIGNPATH_ORGANIZATION_ID` | el id de la organización (un GUID) |
| Variable | `SIGNPATH_PROJECT_SLUG` | `simpcky` |
| Variable | `SIGNPATH_POLICY_SLUG` | `release-signing` |
| Secreto | `SIGNPATH_API_TOKEN` | el token del usuario de CI |

## 4. Cada versión firmada

Igual que siempre (ver `publicar-version.md`): tag `vX.Y.Z` y push. El
workflow pide **dos firmas** —primero `simpcky.exe`, que va adentro del
instalador, y después el instalador— y espera hasta una hora a que las
apruebes en SignPath (llega un mail). Antes de publicar comprueba que las
dos firmas sean válidas.

Cuando las versiones estén firmadas, conviene que el actualizador exija
la firma: si la versión instalada está firmada, rechazar cualquier
instalador que no lo esté (hoy verifica la huella SHA-256, que viaja en
el mismo release que el instalador).
