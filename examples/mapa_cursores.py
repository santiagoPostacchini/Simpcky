"""Mapa del cursor sobre una nota: mueve el mouse de verdad por una grilla
que cubre la nota y un margen alrededor, y anota qué cursor muestra
Windows en cada punto. Notas: anclada abierta, siempre encima abierta y
anclada enrollada (en Manual); con vidrio, --glass N.

Uso: python examples/mapa_cursores.py EXE [--glass N] [--step 3]
"""
import argparse, ctypes, json, os, shutil, subprocess, tempfile, time
from ctypes import wintypes as w

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
u32 = ctypes.windll.user32
u32.LoadCursorW.restype = w.HANDLE
u32.LoadCursorW.argtypes = [w.HINSTANCE, ctypes.c_void_p]


class CURSORINFO(ctypes.Structure):
    _fields_ = [("cbSize", w.DWORD), ("flags", w.DWORD), ("hCursor", w.HANDLE), ("pt", w.POINT)]


NAMES = {32512: ".", 32644: "-", 32645: "|", 32642: "\\", 32643: "/", 32646: "+", 32649: "h", 32513: "I"}
HANDLES = {u32.LoadCursorW(None, ctypes.c_void_p(k)): v for k, v in NAMES.items()}


def cur():
    ci = CURSORINFO()
    ci.cbSize = ctypes.sizeof(ci)
    u32.GetCursorInfo(ctypes.byref(ci))
    return HANDLES.get(ci.hCursor, "?")


def note(i, x, y, title, layer=1, rolled=False, mode=0, w=300, h=260):
    return dict(id=i, uid="%032x" % i, x=x, y=y, w=w, h=h, color=i % 5, layer=layer, rollMode=mode, rolled=rolled,
                title=title, text="Texto de prueba\nsegunda línea\n" * 6, t=[0, 0, 0, 0])


NOTES = [
    note(1, 200, 200, "Anclada"),
    note(2, 700, 200, "Encima", layer=2),
    note(3, 1200, 200, "Enrollada", rolled=True, mode=0),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("--glass", type=int, default=0)
    ap.add_argument("--step", type=int, default=3)
    ap.add_argument("--out")
    a = ap.parse_args()
    root = tempfile.mkdtemp(prefix="simpcky-cursor-")
    os.makedirs(os.path.join(root, "Simpcky"))
    json.dump(NOTES, open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"), ensure_ascii=False)
    open(os.path.join(root, "Simpcky", "settings.json"), "w").write(json.dumps({"autoUpdate": False, "desktopMenu": False, "translucency": a.glass}))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="cursores")
    u32.keybd_event(0x5B, 0, 0, 0); u32.keybd_event(0x4D, 0, 0, 0); u32.keybd_event(0x4D, 0, 2, 0); u32.keybd_event(0x5B, 0, 2, 0)  # Win+M
    time.sleep(0.8)
    p = subprocess.Popen([os.path.abspath(a.exe)], env=env)
    out = []
    try:
        time.sleep(3)
        for n in NOTES:
            h = 40 if n["rolled"] else n["h"]
            m = 14
            out.append(f"== {n['title']} ({n['w']}x{h}), margen {m}px, paso {a.step}px")
            for y in range(n["y"] - m, n["y"] + h + m, a.step):
                row = ""
                for x in range(n["x"] - m, n["x"] + n["w"] + m, a.step):
                    u32.SetCursorPos(x, y)
                    time.sleep(0.004)
                    row += cur()
                out.append(row)
            u32.SetCursorPos(1000, 900)
            time.sleep(0.5)
    finally:
        p.kill()
        p.wait()
        shutil.rmtree(root, ignore_errors=True)
    text = "\n".join(out)
    if a.out:
        open(a.out, "w", encoding="utf-8").write(text)
    print(text)


if __name__ == "__main__":
    main()
