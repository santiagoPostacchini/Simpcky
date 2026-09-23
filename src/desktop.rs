//! "Anclar al escritorio": reparenta una nota dentro de la ventana del
//! escritorio (una `WorkerW`, o `Progman` mismo en Windows 11 24H2+),
//! para que quede fija ahí — se ve con "Mostrar escritorio", nunca tapa
//! otras ventanas, y no aparece en Alt+Tab.
//!
//! Esto usa el mismo truco no documentado que Rainmeter, Wallpaper
//! Engine, etc.: pedirle a `Progman` (la ventana del Administrador de
//! programas) que genere una `WorkerW` con el mensaje `0x052C`, y
//! quedarnos con la que aparece como hermana de la que contiene los
//! íconos (`SHELLDLL_DefView`). No es una API pública de Windows, así
//! que puede fallar (o dejar de funcionar tras un reinicio de
//! `explorer.exe`) — por eso todo esto se degrada solo a "Normal" en
//! vez de romper la nota, y [`anchor`] se reintenta solo cada tanto
//! mientras la nota siga en modo escritorio (ver `TIMER_DESKTOP_WATCH`
//! en `note.rs`).

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::wide;

/// El mensaje que hace que `explorer.exe` genere/reutilice la
/// `WorkerW` detrás de los íconos. Sin nombre oficial — así lo
/// identifican todas las implementaciones públicas de este truco.
const SPAWN_WORKER_MSG: u32 = 0x052C;

/// Busca la `WorkerW` que queda detrás de los íconos del escritorio.
/// Devuelve un HWND nulo si el truco no funcionó en esta versión/estado
/// de `explorer.exe`.
fn find_worker() -> HWND {
    unsafe {
        let progman_class = wide("Progman");
        let progman = FindWindowW(progman_class.as_ptr(), std::ptr::null());
        if progman.is_null() {
            return null_mut();
        }

        // Con esto alcanza para que aparezca la WorkerW; llamarlo de
        // más no genera duplicados.
        let mut result: usize = 0;
        SendMessageTimeoutW(progman, SPAWN_WORKER_MSG, 0, 0, SMTO_NORMAL, 1000, &mut result);

        let mut worker: HWND = null_mut();
        EnumWindows(Some(enum_find_worker), &mut worker as *mut HWND as LPARAM);
        if !worker.is_null() {
            return worker;
        }

        // Algunas configuraciones (confirmado: esta sesión, con doce
        // WorkerW sueltas sin íconos y SHELLDLL_DefView colgando
        // directo de Progman) no migran los íconos a una WorkerW
        // aparte ni siquiera después del mensaje — Progman se queda
        // haciendo de contenedor. Ahí reparentar directo a Progman
        // también deja la nota detrás de los íconos.
        let defview_class = wide("SHELLDLL_DefView");
        let has_defview = FindWindowExW(progman, null_mut(), defview_class.as_ptr(), std::ptr::null());
        if !has_defview.is_null() {
            return progman;
        }

        null_mut()
    }
}

unsafe extern "system" fn enum_find_worker(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let defview_class = wide("SHELLDLL_DefView");
    let has_defview = FindWindowExW(hwnd, null_mut(), defview_class.as_ptr(), std::ptr::null());
    if !has_defview.is_null() {
        // La WorkerW que nos sirve es la que aparece JUSTO DESPUÉS de
        // esta (la que sostiene los íconos) en el orden de ventanas de
        // nivel superior.
        let worker_class = wide("WorkerW");
        let sibling = FindWindowExW(null_mut(), hwnd, worker_class.as_ptr(), std::ptr::null());
        if !sibling.is_null() {
            *(lparam as *mut HWND) = sibling;
        }
    }
    1 // seguir enumerando: nos quedamos con la última coincidencia
}

/// `true` si `hwnd` ya es hijo del contenedor de íconos correcto en
/// este momento — para no repetir el reparentado (que corta el foco
/// del control que se esté editando) cuando en realidad no hace
/// falta.
pub fn is_anchored(hwnd: HWND) -> bool {
    let worker = find_worker();
    if worker.is_null() {
        return false;
    }
    unsafe { GetAncestor(hwnd, GA_PARENT) == worker }
}

/// Reparenta `hwnd` detrás de los íconos del escritorio.
/// `true` si se encontró una `WorkerW` (o, en algunas sesiones, el
/// propio Progman — ver `find_worker`) y `SetParent` confirmó el
/// cambio.
pub fn anchor(hwnd: HWND) -> bool {
    let worker = find_worker();
    if worker.is_null() {
        return false;
    }
    unsafe {
        // GetWindowRect siempre da coordenadas de PANTALLA, tenga o no
        // padre la ventana — hay que guardarlas antes de tocar nada,
        // para poder reconvertirlas después.
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(hwnd, &mut rect);

        // SetParent devuelve NULL si falla de verdad (GetParent, en
        // cambio, no es confiable para una ventana WS_POPUP sin
        // WS_CHILD: puede devolver NULL igual aunque el reparentado
        // haya funcionado — por eso se confía en este valor de
        // retorno y no en una relectura posterior).
        if SetParent(hwnd, worker).is_null() {
            return false;
        }
        // SetParent por sí solo no alcanza: sin el bit WS_CHILD, Windows
        // sigue tratando la ventana como un WS_POPUP "poseído" por el
        // padre nuevo en vez de un hijo de verdad — y eso hace que
        // desaparezca sola al perder el foco (por ejemplo, con un clic
        // en el escritorio). Con WS_CHILD puesto se comporta como
        // corresponde: clip y z-order normales, sin esconderse.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_STYLE, (style | WS_CHILD) as isize);

        // Y además, en capas (opaca al 100 %). Desde Windows 11 24H2,
        // Progman tiene WS_EX_NOREDIRECTIONBITMAP: el escritorio se
        // compone con DirectComposition y NO tiene una superficie donde
        // se dibujen sus ventanas hijas "comunes". Una nota sin esto
        // figuraba visible, en su lugar y encima de los íconos… pero no
        // se veía: aparecía un instante y se borraba apenas el
        // escritorio se redibujaba (por ejemplo, al hacer clic en él).
        // Una hija en capas tiene su propia superficie, y se compone
        // bien — por eso la capa de íconos de Windows (SHELLDLL_DefView)
        // también es WS_EX_LAYERED. Requiere que el .exe declare
        // Windows 8+ en su manifiesto (ver `build.rs`).
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | WS_EX_LAYERED) as isize);
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);

        // Pero WS_CHILD trae su propia trampa: a partir de acá, x/y
        // se interpretan relativos al CLIENTE del padre, no a la
        // pantalla. Sin reconvertir, la nota termina en cualquier
        // lado (o recortada fuera del área de Progman) — que es
        // justamente por qué "desaparecía" al anclarla.
        let mut top_left = POINT { x: rect.left, y: rect.top };
        ScreenToClient(worker, &mut top_left);

        // SWP_FRAMECHANGED fuerza a que el cambio de estilo de arriba
        // surta efecto.
        SetWindowPos(
            hwnd,
            stack_slot(worker),
            top_left.x,
            top_left.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    true
}

/// Dónde meter una nota en el orden de apilado del contenedor: arriba
/// de todo, también de la capa de íconos.
///
/// Debajo de los íconos (lo que se hacía antes) la nota se ve a
/// través de la capa de íconos, que es transparente… pero no se puede
/// tocar: esa capa ocupa todo el escritorio y se queda con cada clic
/// para su selección de íconos. Arriba, la nota se comporta como
/// cualquier widget: se escribe, se arrastra, se enrolla.
fn stack_slot(_container: HWND) -> HWND {
    HWND_TOP
}

/// Sube una nota anclada al tope de la pila de notas. Las ventanas
/// hijas no se reordenan solas al hacer clic, así que sin esto una
/// nota tapada por otra no había forma de traerla adelante.
pub fn raise(hwnd: HWND) {
    unsafe {
        let parent = GetAncestor(hwnd, GA_PARENT);
        if parent.is_null() || GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD == 0 {
            return;
        }
        let slot = stack_slot(parent);
        // ¿Ya está arriba de todo? Entonces no tocar nada (esto corre
        // en cada clic, también mientras se escribe).
        if GetWindow(parent, GW_CHILD) != hwnd {
            SetWindowPos(hwnd, slot, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

/// Mueve la nota a una posición de PANTALLA, esté anclada (hija de
/// Progman/WorkerW, coordenadas relativas al padre) o no.
pub fn move_to_screen(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let mut pt = POINT { x, y };
        if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_CHILD != 0 {
            let parent = GetAncestor(hwnd, GA_PARENT);
            if !parent.is_null() {
                ScreenToClient(parent, &mut pt);
            }
        }
        SetWindowPos(hwnd, null_mut(), pt.x, pt.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// `true` si en el punto (`x`, `y`) de la pantalla se ve el escritorio
/// (o una nota anclada a él), y no otra ventana tapándolo.
pub fn is_desktop_visible_at(x: i32, y: i32) -> bool {
    unsafe {
        let under = WindowFromPoint(POINT { x, y });
        if under.is_null() {
            return false;
        }
        let root = GetAncestor(under, GA_ROOT);
        let mut buf = [0u16; 32];
        let len = GetClassNameW(root, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&buf[..len]);
        class == "Progman" || class == "WorkerW"
    }
}

/// Deshace el anclaje: la nota vuelve a ser una ventana normal del
/// escritorio (sin padre ni WS_CHILD), en la misma posición de
/// pantalla en la que estaba, traída al frente para que no quede
/// perdida detrás de otras ventanas.
pub fn detach(hwnd: HWND) {
    unsafe {
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(hwnd, &mut rect); // coordenadas de pantalla, aun siendo WS_CHILD

        SetParent(hwnd, null_mut());
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !WS_CHILD) as isize);
        // Suelta, no hace falta estar en capas (ver `anchor`), y sin eso
        // recupera las esquinas redondeadas y la sombra de Windows 11.
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex & !WS_EX_LAYERED) as isize);
        SetWindowPos(hwnd, HWND_TOP, rect.left, rect.top, 0, 0, SWP_NOSIZE | SWP_FRAMECHANGED);
    }
}
