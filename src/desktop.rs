//! "Anclar al escritorio": reparenta una nota dentro de la ventana que
//! contiene los íconos del escritorio (`Progman` en Windows 11 24H2+, o
//! la `WorkerW` a la que Explorer los haya mudado), encima de ellos,
//! para que quede fija ahí — se ve con "Mostrar escritorio", nunca tapa
//! otras ventanas, y no aparece en Alt+Tab.
//!
//! No es una API pública de Windows: puede fallar (o dejar de valer
//! tras un reinicio de `explorer.exe`), así que [`anchor`] se reintenta
//! solo cada tanto mientras la nota siga en modo escritorio (ver
//! `TIMER_DESKTOP_WATCH` en `note.rs`) y, mientras tanto, la nota queda
//! como ventana suelta en vez de perderse.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::win::wide;

/// La ventana que contiene los íconos del escritorio: el padre de
/// `SHELLDLL_DefView`. Las notas van ahí, encima de los íconos. Nulo si
/// Explorer no está (reiniciándose, por ejemplo).
///
/// Antes se buscaba "la WorkerW que viene después de la de los íconos"
/// en el orden de apilado de las ventanas, con el mensaje no documentado
/// `0x052C` de por medio (el truco de los fondos de pantalla animados).
/// En Windows 11 24H2 los íconos viven directo en `Progman`, y hay una
/// docena de `WorkerW` ocultas dando vueltas: cuando "Mostrar
/// escritorio" subía a `Progman` en esa pila, la "siguiente WorkerW"
/// pasaba a ser una oculta de 198×56, el vigilante mudaba las notas ahí
/// y desaparecían hasta volver a ponerlas. El padre de los íconos no
/// depende de ningún orden.
fn find_host() -> HWND {
    unsafe {
        let defview_class = wide("SHELLDLL_DefView");
        let progman_class = wide("Progman");
        let progman = FindWindowW(progman_class.as_ptr(), std::ptr::null());
        if !progman.is_null() && !FindWindowExW(progman, null_mut(), defview_class.as_ptr(), std::ptr::null()).is_null() {
            return progman;
        }
        // Explorer mudó los íconos a una WorkerW (pasa en Windows 10 y
        // en Windows 11 anteriores a 24H2 si algún programa de fondos
        // animados se lo pidió).
        let mut host: HWND = null_mut();
        EnumWindows(Some(enum_find_host), &mut host as *mut HWND as LPARAM);
        host
    }
}

unsafe extern "system" fn enum_find_host(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let defview_class = wide("SHELLDLL_DefView");
    if !FindWindowExW(hwnd, null_mut(), defview_class.as_ptr(), std::ptr::null()).is_null() {
        *(lparam as *mut HWND) = hwnd;
        return 0; // hay uno solo
    }
    1
}

/// `true` si `hwnd` ya es hijo del contenedor de íconos correcto en
/// este momento — para no repetir el reparentado (que corta el foco
/// del control que se esté editando) cuando en realidad no hace
/// falta.
pub fn is_anchored(hwnd: HWND) -> bool {
    let host = find_host();
    !host.is_null() && unsafe { GetAncestor(hwnd, GA_PARENT) } == host && unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32 & WS_VISIBLE != 0
}

/// Reparenta `hwnd` en el escritorio, encima de los íconos. `true` si
/// se encontró el contenedor (ver `find_host`) y `SetParent` confirmó
/// el cambio.
pub fn anchor(hwnd: HWND) -> bool {
    let worker = find_host();
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
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
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
