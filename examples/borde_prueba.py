"""Prueba del marco del borde con mouse real: para cada nota (anclada,
siempre encima) entra a la nota, va al borde derecho por afuera,
saca una captura ampliada, arrastra 60 px y mide el ancho nuevo.

Uso: python examples/borde_prueba.py EXE carpeta [--glass N]
"""
import argparse, ctypes, json, os, shutil, subprocess, tempfile, time
from ctypes import wintypes as w

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
from PIL import ImageGrab

u32 = ctypes.windll.user32
u32.WindowFromPoint.argtypes = [w.POINT]
u32.WindowFromPoint.restype = w.HWND
u32.GetParent.argtypes = [w.HWND]
u32.GetParent.restype = w.HWND
u32.GetWindowRect.argtypes = [w.HWND, ctypes.POINTER(w.RECT)]
u32.LoadCursorW.restype = w.HANDLE
u32.LoadCursorW.argtypes = [w.HINSTANCE, ctypes.c_void_p]


class CURSORINFO(ctypes.Structure):
    _fields_ = [("cbSize", w.DWORD), ("flags", w.DWORD), ("hCursor", w.HANDLE), ("pt", w.POINT)]


NAMES = {32512: "flecha", 32644: "↔", 32645: "↕", 32642: "↘", 32643: "↗", 32513: "texto"}
HANDLES = {u32.LoadCursorW(None, ctypes.c_void_p(k)): v for k, v in NAMES.items()}


def cur():
    ci = CURSORINFO()
    ci.cbSize = ctypes.sizeof(ci)
    u32.GetCursorInfo(ctypes.byref(ci))
    return HANDLES.get(ci.hCursor, "?")


def note_rect(x, y):
    h = u32.WindowFromPoint(w.POINT(x, y))
    buf = ctypes.create_unicode_buffer(64)
    while h:
        u32.GetClassNameW(h, buf, 64)
        if buf.value == "SimpckyNote":
            r = w.RECT()
            u32.GetWindowRect(h, ctypes.byref(r))
            return (r.left, r.top, r.right, r.bottom)
        h = u32.GetParent(h)
    return None


def move(x0, y0, x1, y1, n=25, dt=0.008):
    for i in range(1, n + 1):
        u32.SetCursorPos(int(x0 + (x1 - x0) * i / n), int(y0 + (y1 - y0) * i / n))
        time.sleep(dt)


def note(i, x, y, title, layer=1):
    return dict(id=i, uid="%032x" % i, x=x, y=y, w=300, h=260, color=i % 5, layer=layer, rollMode=0, rolled=False,
                title=title, text="Texto de prueba", t=[0, 0, 0, 0])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("out")
    ap.add_argument("--glass", type=int, default=0)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    notes = [note(1, 200, 300, "Anclada"), note(2, 800, 300, "Encima", layer=2)]
    root = tempfile.mkdtemp(prefix="simpcky-bordes-")
    os.makedirs(os.path.join(root, "Simpcky"))
    json.dump(notes, open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"))
    open(os.path.join(root, "Simpcky", "settings.json"), "w").write(json.dumps({"autoUpdate": False, "desktopMenu": False, "translucency": a.glass}))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="bordes")
    u32.keybd_event(0x5B, 0, 0, 0); u32.keybd_event(0x4D, 0, 0, 0); u32.keybd_event(0x4D, 0, 2, 0); u32.keybd_event(0x5B, 0, 2, 0)  # Win+M
    time.sleep(0.8)
    p = subprocess.Popen([os.path.abspath(a.exe)], env=env)
    try:
        time.sleep(3)
        for n in notes:
            cx, cy = n["x"] + 150, n["y"] + 130
            before = note_rect(cx, n["y"] + 15); print(n["title"], before)
            right = before[2]
            u32.SetCursorPos(cx, cy)
            time.sleep(0.3)
            move(cx, cy, right - 20, cy)
            res = {"nota": n["title"], "antes": before}
            for dx in (-4, 0, 3, 5):
                u32.SetCursorPos(right + dx, cy)
                time.sleep(0.15)
                res[f"cursor@{dx:+d}"] = cur()
            u32.SetCursorPos(right + 3, cy)
            time.sleep(0.25)
            ImageGrab.grab(bbox=(right - 60, cy - 160, right + 40, cy + 160), all_screens=True).resize((300, 960)).save(os.path.join(a.out, f"{n['title']}-borde.png"))
            u32.SetCursorPos(before[2] + 3, before[3] + 3)
            time.sleep(0.25)
            res["cursor@esquina+3"] = cur()
            ImageGrab.grab(bbox=(before[2] - 60, before[3] - 60, before[2] + 30, before[3] + 30), all_screens=True).resize((360, 360)).save(os.path.join(a.out, f"{n['title']}-esquina.png"))
            # Arrastrar desde afuera del borde derecho.
            u32.SetCursorPos(right + 3, cy)
            time.sleep(0.15)
            u32.mouse_event(0x0002, 0, 0, 0, 0)
            move(right + 3, cy, right + 63, cy, n=30)
            time.sleep(0.1)
            u32.mouse_event(0x0004, 0, 0, 0, 0)
            time.sleep(0.4)
            res["despues"] = note_rect(cx, n["y"] + 15)
            print(json.dumps(res, ensure_ascii=False))
            u32.SetCursorPos(1000, 950)
            time.sleep(0.6)
    finally:
        p.kill()
        p.wait()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
