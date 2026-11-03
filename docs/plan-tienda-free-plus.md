# Plan: Simpcky Free y Plus en Microsoft Store

Objetivo: que Simpcky deje una entrada mensual que cubra las suscripciones de IA.
Estado: propuesta (3 de noviembre de 2026). Nada de esto está hecho todavía.

## Lo que sabemos de la tienda (verificado en noviembre de 2026)

- **La cuenta de desarrollador individual es gratis** desde septiembre de 2025: cuenta
  personal de Microsoft, DNI y selfie, sin tarjeta. Se abre en
  <https://storedeveloper.microsoft.com>.
- **Dos formas de publicar una app Win32:**
  - **MSIX** (paquete): la tienda lo firma, lo instala y lo actualiza. Tiene "identidad de
    paquete", que es lo que hace falta para cobrar con la tienda (`StoreContext`, en una app
    de confianza total).
  - **EXE/MSI** (nuestro instalador de Inno Setup): tiene que estar firmado con un
    certificado de una CA reconocida (uno propio no sirve), instalar en silencio y no
    cambiar en la URL. La app se puede actualizar sola. **No puede cobrar con la tienda.**
- **Comisión:** con el cobro de la tienda, Microsoft se queda con el 15 % en apps (12 % en
  juegos). Una app que no es juego puede cobrar con su propio sistema (Stripe, Paddle,
  Lemon Squeezy) y la tienda no se lleva nada; esos sistemas cobran alrededor de 5 % + US$ 0,50
  por venta.
- **Falta confirmar en Partner Center:** cómo se cobra desde Argentina (transferencia,
  moneda, retenciones) y el porcentaje vigente de suscripciones. La documentación pública
  no lo deja claro.

## Recomendación

**Publicar como MSIX y cobrar Plus con la tienda (suscripción por add-on).**

¿Por qué esta y no el EXE con cobro propio?

| | MSIX + cobro de la tienda | EXE + cobro propio |
|---|---|---|
| Firma | La hace la tienda: se acaba el "Windows protegió tu PC" | Hace falta SignPath (pendiente) o pagar un certificado |
| Comisión | 15 % | ~5 % + US$ 0,50 (en US$ 2 es ~30 %) |
| Pagar | Con la cuenta de Microsoft, un clic | Web externa y una clave de licencia |
| Impuestos de cada país | Los resuelve Microsoft | Los resuelve el "merchant of record" |
| Trabajo | Paquete + ajustes de MSIX + llamar a `StoreContext` | Validar licencias (con el WinHTTP que ya tenemos) |

Con precios chicos, el 15 % sale más barato que el 5 % + 50 centavos. Además, pagar con un
clic convierte mucho mejor que mandar a una web y pegar una clave. Y la firma de la tienda
destraba lo de SmartScreen, que hoy frena las instalaciones.

## Qué va en cada versión (propuesta, a decidir)

Criterio: Free tiene que ser la mejor app de notas gratis en una sola PC (es lo que trae
usuarios). Plus cobra por lo que da valor de más: varias compus y contenido rico.

| Free | Plus |
|---|---|
| Notas, enrollar (Manual/Auto), colores, atajos | Todo lo de Free |
| Celdas y pilas (con mosaico) | Sincronización entre compus (Google Drive) |
| Texto oculto, invisible en capturas, cifrado local | Imágenes en las notas |
| Tema claro/oscuro | Modo vidrio y temas extra |
| | Lo nuevo que salga primero (historial de versiones, buscar en todas, exportar) |

Precios sugeridos: **US$ 1,99/mes o US$ 14,99/año**, más un **Plus de por vida a US$ 29,99**
(add-on durable) para los que no quieren suscripciones. En la tienda se puede ajustar el
precio por país (Argentina más barato).

## ¿Cuántos usuarios hacen falta?

Neto por suscriptor, después del 15 %: US$ 1,69/mes (mensual) o US$ 1,06/mes (anual).

| Para cubrir por mes | Suscriptores (mezcla mitad mensual, mitad anual) | Usuarios activos (al 2 % que paga) |
|---|---|---|
| US$ 40 | ~30 | ~1.500 |
| US$ 100 | ~75 | ~3.800 |
| US$ 220 | ~160 | ~8.000 |

El 2 % es un número típico de apps "freemium" y no está medido. Lo difícil no es la parte
técnica: es conseguir usuarios. La ficha de la tienda (capturas, video corto, palabras como
"sticky notes", "notas escritorio") y que la app esté firmada pesan más que cualquier función.

## Decisiones que necesitan al usuario

1. **Código abierto.** Hoy el repo es MIT y público: cualquiera puede compilar Plus gratis.
   - (a) Seguir MIT y cobrar por la comodidad (tienda, actualizaciones). Funciona en apps
     chicas: casi nadie compila.
   - (b) Lo de Plus en un módulo privado y el resto abierto ("open core").
   - (c) Cerrar el repo.

   Recomendación: **(b)**. El núcleo sigue abierto, eso mantiene a SignPath para el EXE de
   GitHub, y la parte que cobra queda aparte.
2. **El EXE de GitHub.** ¿Sigue existiendo? Recomendación: sí, como **Free** (mismas
   funciones que Free en la tienda), sin Plus. Quien ya tiene sincronización o imágenes
   en la versión de GitHub conserva lo que tiene hasta la versión en que se separa, y se le
   avisa.
3. **Qué entra en Plus.** La tabla de arriba es una propuesta.

## Trabajo técnico (cada punto en su rama, con su criterio)

Fase 1: Free en la tienda (sin cobrar todavía)

1. `feat/msix`: armar el paquete MSIX (manifiesto, íconos, `makeappx`) desde el workflow de
   release, además del instalador de Inno.
   *Listo cuando:* el `.msix` se instala en una VM limpia y la app abre.
2. `feat/msix-ajustes`: lo que cambia dentro de un paquete.
   - Iniciar con Windows: con `StartupTask` en vez de la clave `Run` (`src/shell.rs`).
   - Actualizador: apagado si la app corre empaquetada (`src/update.rs`); actualiza la tienda.
   - Datos: confirmar dónde quedan `notes.json` y `settings.json`, porque MSIX virtualiza
     `%APPDATA%`, y que las notas de una instalación EXE anterior se vean.
   - Anclar al escritorio (hija de WorkerW), ocultar de capturas y la sincronización con
     Google (el navegador más 127.0.0.1): probarlos empaquetada.

   *Listo cuando:* lo anterior anda en una VM con el `.msix`, y el test de pilas
   (`examples/pilas_guion.py`) pasa.
3. Ficha en Partner Center: nombre reservado, política de privacidad (ya está en
   `docs/privacidad.html`), clasificación por edad, capturas y descripción. Enviar a
   certificación.

Fase 2: Plus

4. `feat/plus-tienda`: consultar y comprar el add-on con `StoreContext`
   (`GetAppLicenseAsync`, `RequestPurchaseAsync`, con `IInitializeWithWindow` y la ventana
   de configuración). Hay dos caminos para llamar a WinRT desde Rust:
   - el crate `windows` (es una dependencia nueva, hay que consultarla);
   - las tablas COM a mano, como ya hacemos con `IStream` en `images.rs`.

   *Listo cuando:* con una cuenta de prueba, comprar activa Plus sin reiniciar, y cancelar
   lo apaga al vencer.
5. `feat/plus-limites`: las funciones de Plus se apagan sin Plus, sin perder datos.
   - Las imágenes ya guardadas se siguen viendo; lo que no se puede es pegar nuevas.
   - La sincronización se pausa y no borra nada.

   *Listo cuando:* hay tests de las dos cosas.
6. Configuración: una sección "Simpcky Plus" que muestre el estado, el precio y el botón
   de compra.

Fase 3: crecer

7. Ficha en inglés y en español, video de 20 segundos y pedir reseñas dentro de la app
   (una vez, después de una semana de uso).

## Lo que hay que hacer fuera del código

- Abrir la cuenta individual (gratis) y completar los perfiles de cobro e impuestos en
  Partner Center; confirmar ahí cómo se cobra desde Argentina.
- Hablar con un contador: ingresos en dólares desde el exterior (exportación de servicios,
  monotributo o el régimen que corresponda).

## Fuentes

- [Registro gratis para desarrolladores individuales (blog de Windows, 2025)](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)
- [Empaquetado e identidad de paquete (Microsoft Learn)](https://learn.microsoft.com/windows/apps/package-and-deploy/packaging/)
- [APIs de WinRT no admitidas en apps de escritorio (Microsoft Learn)](https://learn.microsoft.com/windows/uwp/porting/desktop-to-uwp-supported-api)
- [Cambios del acuerdo de desarrolladores desde julio de 2021 (Neowin)](https://www.neowin.net/news/here-are-all-the-changes-microsoft-is-making-to-its-app-developer-agreement-from-july-28/)
- [Win32 en la tienda: actualizaciones propias (The Register, 2021)](https://www.theregister.com/2021/07/27/microsoft_new_store_updates/)
- [Lemon Squeezy: países admitidos](https://docs.lemonsqueezy.com/help/getting-started/supported-countries)
- [Perfiles de cobro e impuestos (Partner Center)](https://learn.microsoft.com/en-us/partner-center/set-up-your-payout-account)
