"""Corre examples/despliegue_guion.txt (cursor falso) en una instancia de prueba en el monitor
principal 1920 x 1080 a 100 %, con una pila de tres notas Auto, y anota la
altura de cada nota en cada FOTO (enrollada = 40).

Uso: cargo build --release
     python examples/despliegue_guion.py target/release/simpcky.exe examples/despliegue_guion.txt
"""
import ctypes, json, os, shutil, subprocess, sys, tempfile, time
from ctypes import wintypes as w

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
u32 = ctypes.windll.user32
EXE, GUION = sys.argv[1], sys.argv[2]


def note(i, title, x, y, rolled, mode):
    return {"id": i, "uid": "%032x" % i, "x": x, "y": y, "w": 300, "h": 300, "color": i, "layer": 1, "rollMode": mode, "rolled": rolled, "title": title, "text": "texto", "t": [0, 0, 0, 0]}


notes = [note(1, "Pass", 1580, 100, True, 1), note(2, "Reunion", 1580, 150, True, 1), note(3, "Llamar", 1580, 200, True, 1), note(4, "Fuera", 300, 600, False, 0)]
root = tempfile.mkdtemp(prefix="simpcky-despliegue-")
data = os.path.join(root, "Simpcky")
os.makedirs(data)
json.dump(notes, open(os.path.join(data, "notes.json"), "w", encoding="utf-8"))
open(os.path.join(data, "settings.json"), "w").write('{"autoUpdate":false,"desktopMenu":false,"rollDelay":0}')
log = os.path.join(root, "trace.log")
env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="despliegue", SIMPCKY_PAINT_LOG=log, SIMPCKY_GUION=os.path.abspath(GUION))
u32.SetCursorPos(1000, 1000)
# Que ninguna ventana tape la pila: minimizar todo (Win+M; Win+D alterna).
for vk, up in ((0x5B, 0), (0x4D, 0), (0x4D, 2), (0x5B, 2)):
    u32.keybd_event(vk, 0, up, 0)
time.sleep(0.8)
p = subprocess.Popen([os.path.abspath(EXE)], env=env)


def heights():
    out = {}
    def each(h, _):
        buf = ctypes.create_unicode_buffer(64)
        u32.GetClassNameW(h, buf, 64)
        if buf.value == "SimpckyNote":
            r = w.RECT(); u32.GetWindowRect(h, ctypes.byref(r))
            out[(r.left, r.top)] = r.bottom - r.top
        return True
    cb = ctypes.WINFUNCTYPE(ctypes.c_bool, w.HWND, w.LPARAM)(each)
    u32.EnumWindows(cb, 0)
    for top in ("Progman", "WorkerW"):
        h = u32.FindWindowW(top, None)
        while h:
            u32.EnumChildWindows(h, cb, 0)
            h = u32.FindWindowExW(None, h, top, None) if top == "WorkerW" else 0
    return dict(sorted(out.items()))


seen, t0 = 0, time.time()
try:
    while time.time() - t0 < 30:
        time.sleep(0.03)
        if not os.path.exists(log):
            continue
        lines = open(log, encoding="utf-8", errors="replace").read().splitlines()
        for line in lines[seen:]:
            parts = line.split("\t")
            if len(parts) > 1 and parts[1].startswith("FOTO "):
                time.sleep(0.25)
                print(parts[1][5:], heights())
            if len(parts) > 1 and parts[1] == "GUION FIN":
                raise SystemExit
        seen = len(lines)
finally:
    p.kill()
    shutil.rmtree(root, ignore_errors=True)
