# Plan: Simpcky gratis en Microsoft Store

Estado: decidido el 4 de noviembre de 2026. Simpcky sale **gratis y completo** en la tienda,
sin versión paga. El repo sigue MIT y el EXE de GitHub sigue igual (con todo). Si algún día
vuelve la idea de cobrar, el análisis está en la historia de git de este archivo
(`plan-tienda-free-plus.md`, 3/11).

## Lo que sabemos de la tienda (verificado en noviembre de 2026)

- **La cuenta de desarrollador individual es gratis** desde septiembre de 2025: cuenta
  personal de Microsoft, DNI y selfie, sin tarjeta. Se abre en
  <https://storedeveloper.microsoft.com>.
- **Dos formas de publicar una app Win32:**
  - **MSIX** (paquete): la tienda lo firma, lo instala y lo actualiza.
  - **EXE/MSI** (nuestro instalador de Inno Setup): tiene que estar firmado con un
    certificado de una CA reconocida, instalar en silencio y no cambiar en la URL.

**Elegido: MSIX.** La tienda lo firma gratis, así que quien instala desde la tienda no ve
"Windows protegió tu PC". El EXE necesita firma propia, que todavía no tenemos (SignPath está
pendiente, ver `firma-signpath.md`).

## Qué cambia adentro de un paquete MSIX

Un paquete redirige a una copia privada las escrituras en `HKCU\Software` y los archivos
nuevos en `%APPDATA%`. Eso pega en cuatro lugares:

| Qué | Hoy | En el paquete |
|---|---|---|
| Iniciar con Windows (`shell.rs`) | Clave `Run` | La clave no se ve. Va `StartupTask` en el manifiesto; el interruptor abre Configuración → Aplicaciones → Inicio (`ms-settings:startupapps`) |
| "Nueva nota adhesiva" en el clic derecho del escritorio (`shell.rs`) | Verbo en `Software\Classes` | El verbo no se ve. Se esconde la opción en el paquete; Ctrl+N y la bandeja siguen |
| Actualizador (`update.rs`) | Baja el instalador de GitHub | Apagado: actualiza la tienda (la tienda no deja que una MSIX se actualice sola) |
| Datos (`persist.rs`) | `%APPDATA%\Simpcky\` | Las notas que ya existen se leen y se escriben en su lugar. En una PC nueva quedan en la carpeta privada del paquete y **se borran al desinstalar**. Hay que avisarlo en la ficha, o pedir `unvirtualizedResources` (necesita justificarlo ante la tienda) |

Cómo sabe la app que corre empaquetada: `GetCurrentPackageFullName` (Win32, ya está en
`windows-sys`; sin dependencias nuevas).

Hay que probar empaquetadas, porque son las partes más raras:
- anclar al escritorio (como hija de WorkerW);
- ocultar de capturas;
- la sincronización con Google (abre el navegador y escucha en 127.0.0.1);
- el atajo global.

## Trabajo (cada punto en su rama, con su criterio)

1. `feat/msix`: armar el paquete.
   - El manifiesto (`AppxManifest.xml`, con `runFullTrust` y `StartupTask`).
   - Los íconos que pide la tienda, sacados de `assets/simpcky.ico`.
   - `makeappx`, desde el workflow de release, además del instalador de Inno. El SDK de
     Windows ya está instalado en esta PC.

   *Listo cuando:* el `.msix`, firmado con un certificado de prueba, se instala en una PC
   limpia (o en Windows Sandbox) y la app abre.
2. `feat/msix-ajustes`: los cuatro cambios de la tabla, detrás de un `app::packaged()`.

   *Listo cuando:* con el `.msix` andan iniciar con Windows, las notas, la sincronización, el
   anclado y ocultar de capturas; y pasan los tests y `examples/pilas_guion.py`.
3. La ficha en Partner Center (la hace el usuario; Claude prepara los textos):
   - reservar el nombre;
   - la política de privacidad, que ya está en `docs/privacidad.html`;
   - la clasificación por edad;
   - las capturas (salen de la galería del lavado de cara);
   - la descripción en español y en inglés, con palabras como "sticky notes" y "notas
     escritorio".

   Después se envía a certificación.

El lavado de cara no bloquea esto, pero conviene que las capturas de la ficha sean las de
después.

## Lo que hay que hacer fuera del código

- Abrir la cuenta individual (gratis). Sin cobro no hacen falta los perfiles de pago ni de
  impuestos.

## Fuentes

- [Registro gratis para desarrolladores individuales (blog de Windows, 2025)](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)
- [Empaquetado e identidad de paquete (Microsoft Learn)](https://learn.microsoft.com/windows/apps/package-and-deploy/packaging/)
- [Cómo corren las apps de escritorio empaquetadas: registro y AppData (Microsoft Learn)](https://learn.microsoft.com/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)
- [Win32 en la tienda: actualizaciones propias (The Register, 2021)](https://www.theregister.com/2021/07/27/microsoft_new_store_updates/)
