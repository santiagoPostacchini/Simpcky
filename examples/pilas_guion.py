"""Uso real de una pila en mosaico, sin mouse y sin capturas: corre `pilas_guion.txt`
en una instancia de prueba (cursor falso, ver `snap.rs` → "Guion de prueba"), anota
dónde están las ventanas cada vez que el registro dice FOTO y revisa que la pila
quede bien armada:
  - la columna mide lo que la nota más ancha (si no, dos notas comparten una fila
    que ya no entra y los huecos de la cuadrícula quedan corridos);
  - cada nota pegada a un borde de la columna, sin pisarse, una fila cada `step`.

Las notas van en un segundo monitor a la derecha del principal (x de 1920 a 3840).

Uso: cargo build --release
     python examples/pilas_guion.py target/release/simpcky.exe examples/pilas_guion.txt
"""
import base64, ctypes, json, os, subprocess, sys, time
from ctypes import wintypes as W

u = ctypes.windll.user32
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
EXE, GUION = sys.argv[1], sys.argv[2]
PROFILE = os.path.join(os.environ["TEMP"], "simpcky-pilas-prueba")
DATA = os.path.join(PROFILE, "Simpcky")
os.makedirs(DATA, exist_ok=True)
for f in os.listdir(DATA):
    os.remove(os.path.join(DATA, f))

E, T, BAR, STEP = 3822, 18, 40, 50
def note(i, title, color, x, y, w, h, rolled, mode, pile=0):
    return {"id": i, "x": x, "y": y, "w": w, "h": h, "color": color, "layer": 1, "rollMode": mode, "rolled": rolled, "title": title, "text": "prueba", "pile": pile}
# Pila a la derecha en mosaico: Pass (izquierda) y mafernandez (derecha) comparten fila
# porque entran en el ancho de Arriba (300 + 10 + 300 <= 645).
PILE = ["Arriba", "Pass", "mafernandez", "Remoto"]
notes = [
    note(1, "Arriba", 4, E - 645, T, 645, 300, True, 1, 7),
    note(2, "Pass", 0, E - 645, T + STEP, 300, 300, True, 1, 7),
    note(3, "mafernandez", 3, E - 300, T + STEP, 300, 300, True, 1, 7),
    note(4, "Remoto", 2, E - 400, T + 2 * STEP, 400, 300, True, 1, 7),
    note(5, "Compras", 1, 2050, 300, 420, 480, False, 0),
]
json.dump(notes, open(os.path.join(DATA, "notes.json"), "w", encoding="utf-8"), ensure_ascii=False)
open(os.path.join(DATA, "settings.json"), "w").write('{"dark":true,"defaultRollMode":0,"desktopMenu":false,"autoUpdate":false,"translucency":0,"scale":0,"hotkey":0,"pileHover":true}')
log = os.path.join(PROFILE, "trace.log")
if os.path.exists(log):
    os.remove(log)

EnumProc = ctypes.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
def windows(pid):
    """(ventana, x, y, w, h) de las notas visibles de la instancia de prueba."""
    out = []
    def each(h, _):
        p = W.DWORD()
        u.GetWindowThreadProcessId(h, ctypes.byref(p))
        c = ctypes.create_unicode_buffer(64)
        u.GetClassNameW(h, c, 64)
        if p.value == pid and c.value == "SimpckyNote" and u.IsWindowVisible(h):
            r = W.RECT()
            u.GetWindowRect(h, ctypes.byref(r))
            out.append((h, r.left, r.top, r.right - r.left, r.bottom - r.top))
        return True
    def top(h, l):
        each(h, l)
        u.EnumChildWindows(h, EnumProc(each), 0)
        return True
    u.EnumWindows(EnumProc(top), 0)
    return out

env = dict(os.environ, APPDATA=PROFILE, SIMPCKY_INSTANCIA="pilasprueba", SIMPCKY_PAINT_LOG=log, SIMPCKY_GUION=GUION)
p = subprocess.Popen([EXE], env=env)
shots, seen, t0 = {}, 0, time.time()
try:
    while time.time() - t0 < 90:
        time.sleep(0.03)
        if not os.path.exists(log):
            continue
        lines = open(log, encoding="utf-8", errors="replace").read().splitlines()
        for line in lines[seen:]:
            parts = line.split("\t")
            if len(parts) > 1 and parts[1].startswith("FOTO "):
                time.sleep(0.3)
                shots[parts[1][5:]] = windows(p.pid)
            if len(parts) > 1 and parts[1] == "GUION FIN":
                raise StopIteration
        seen = len(lines)
except StopIteration:
    pass
finally:
    time.sleep(0.3)
    p.kill()

class Blob(ctypes.Structure):
    _fields_ = [("n", W.DWORD), ("p", ctypes.POINTER(ctypes.c_byte))]
def unprotect(data):
    i, o = Blob(len(data), ctypes.cast(ctypes.create_string_buffer(data, len(data)), ctypes.POINTER(ctypes.c_byte))), Blob()
    assert ctypes.windll.crypt32.CryptUnprotectData(ctypes.byref(i), None, None, None, None, 0, ctypes.byref(o))
    return ctypes.string_at(o.p, o.n)
raw = open(os.path.join(DATA, "notes.json"), encoding="utf-8").read()
if raw.startswith("simpcky-dpapi:1:"):
    b = raw[len("simpcky-dpapi:1:"):]
    raw = unprotect(base64.urlsafe_b64decode(b + "=" * (-len(b) % 4))).decode("utf-8")
saved = {n["title"]: n for n in json.loads(raw)}

fails = []
def check(name, bars, step):
    """bars: título -> (x, y, w). La pila bien armada."""
    l, r = min(b[0] for b in bars.values()), max(b[0] + b[2] for b in bars.values())
    widest = max(b[2] for b in bars.values())
    if abs((r - l) - widest) > 1:
        fails.append(f"{name}: columna de {r - l} y la más ancha mide {widest}")
    for t, (x, y, w) in bars.items():
        if abs(x - l) > 1 and abs(x + w - r) > 1:
            fails.append(f"{name}: {t} no está pegada a un borde de la columna")
    ys = sorted({b[1] for b in bars.values()})
    if any(abs((b - a) - step) > 1 for a, b in zip(ys, ys[1:])):
        fails.append(f"{name}: filas {ys} (paso {step})")
    for y in ys:
        row = sorted((x, x + w, t) for t, (x, yy, w) in bars.items() if yy == y)
        if any(a[1] > b[0] for a, b in zip(row, row[1:])):
            fails.append(f"{name}: se pisan en la fila {y}: {[t for *_, t in row]}")

# Las ventanas no tienen título: cada una se reconoce por dónde arrancó.
start = {(n["x"], n["y"]): n["title"] for n in notes}
names = {h: start.get((x, y), "?") for h, x, y, w, hh in shots.get("inicio", [])}
shots = {k: [(names.get(h, "?"), x, y, w, hh) for h, x, y, w, hh in v] for k, v in shots.items()}
for name, ws in shots.items():
    print(name, sorted(ws))
    bars = {t: (x, y, w) for t, x, y, w, h in ws if t in PILE}
    if name not in ("remoto_afuera",) and len(bars) >= 2:
        check(name, bars, STEP)
    elif len(bars) >= 2:
        check(name, {t: b for t, b in bars.items() if t != "Remoto"}, STEP)

# La de arriba achicada: la fila doble ya no entra y se separa.
a = {t: (x, y, w, h) for t, x, y, w, h in shots.get("arriba_achicada", [])}
if a and a["Pass"][1] == a["mafernandez"][1] and a["Pass"][2] + a["mafernandez"][2] + 10 > a["Arriba"][2]:
    fails.append("arriba_achicada: Pass y mafernandez siguen en una fila que no entra")
# Pass abierta y estirada con Ctrl: sigue en su lugar de la pila y más alta.
o, s = {t: v for t, *v in shots.get("pass_abierta", [])}, {t: v for t, *v in shots.get("pass_estirada", [])}
if o and s:
    if o["Pass"][3] <= BAR:
        fails.append("pass_abierta: no se abrió al pasar el mouse")
    if (s["Pass"][0], s["Pass"][1]) != (o["Pass"][0], o["Pass"][1]):
        fails.append(f"pass_estirada: se movió de {o['Pass'][:2]} a {s['Pass'][:2]}")
    if s["Pass"][3] <= o["Pass"][3]:
        fails.append("pass_estirada: no creció")
    print("Pass: alto", o["Pass"][3], "->", s["Pass"][3], "(pedido", o["Pass"][3] + 137, ")")
# Lo guardado: las cuatro en la misma pila, otra vez.
keys = {saved[t].get("pile", 0) for t in PILE}
if len(keys) != 1 or 0 in keys:
    fails.append(f"guardado: números de pila {[(t, saved[t].get('pile', 0)) for t in PILE]}")
check("guardado", {t: (saved[t]["x"], saved[t]["y"], saved[t]["w"]) for t in PILE}, STEP)
print("\n".join(fails) if fails else "OK: la pila quedó bien armada en todos los pasos")
sys.exit(1 if fails else 0)
