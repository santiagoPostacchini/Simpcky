"""Abre Simpcky o la demo con unas notas de ejemplo (datos aislados en una
carpeta temporal) y saca una captura de la zona de sus ventanas.

Uso: python shot.py EXE salida.png [--dark] [--wait 3] [--keep]
"""
import argparse, ctypes, json, os, shutil, subprocess, tempfile, time
from ctypes import wintypes

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
from PIL import ImageGrab

u32 = ctypes.windll.user32

NOTES = [
    dict(id=1, x=120, y=120, w=300, h=330, color=0, layer=1, rollMode=0, rolled=False, title="Compras",
         text="- [ ] Yerba\n- [x] Pan\n- [ ] Pilas AA\n- [ ] Café"),
    dict(id=2, x=450, y=120, w=300, h=330, color=3, layer=1, rollMode=0, rolled=False, title="Ideas Simpcky",
         text="## Próximo\n\n**Celdas** chicas y *pilas* enrolladas.\n\nProbar Raven 🚀"),
    dict(id=3, x=780, y=120, w=300, h=330, color=1, layer=1, rollMode=1, rolled=True, title="Pass", text="wifi casa: ********"),
    dict(id=4, x=780, y=170, w=300, h=330, color=2, layer=1, rollMode=0, rolled=True, title="Reunión", text="Martes 15 h"),
]


def windows_of(pid):
    found = []
    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def cb(h, _):
        p = wintypes.DWORD()
        u32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid and u32.IsWindowVisible(h):
            r = wintypes.RECT()
            u32.GetWindowRect(h, ctypes.byref(r))
            if r.right - r.left > 20:
                found.append((r.left, r.top, r.right, r.bottom))
        return True
    u32.EnumWindows(cb, 0)
    for name in ("Progman", "WorkerW"):
        top = u32.FindWindowW(name, None)
        if top:
            u32.EnumChildWindows(top, cb, 0)
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("out")
    ap.add_argument("--dark", action="store_true")
    ap.add_argument("--wait", type=float, default=3)
    ap.add_argument("--top", action="store_true", help="notas siempre encima (layer 2)")
    ap.add_argument("--keep", action="store_true", help="no cerrar el programa")
    a = ap.parse_args()
    root = tempfile.mkdtemp(prefix="simpcky-shot-")
    for d in ("Simpcky", "SimpckyRaven"):
        os.makedirs(os.path.join(root, d))
    notes = [dict(n, layer=2) for n in NOTES] if a.top else NOTES
    json.dump(notes, open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"), ensure_ascii=False)
    open(os.path.join(root, "Simpcky", "settings.json"), "w").write(json.dumps({"dark": a.dark, "autoUpdate": False, "desktopMenu": False}))
    open(os.path.join(root, "SimpckyRaven", "settings.json"), "w").write(json.dumps({"dark": a.dark}))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="captura")
    p = subprocess.Popen([os.path.abspath(a.exe)], env=env, cwd=os.path.dirname(os.path.abspath(a.exe)))
    time.sleep(a.wait)
    rects = windows_of(p.pid)
    if rects:
        box = (min(r[0] for r in rects) - 20, min(r[1] for r in rects) - 20, max(r[2] for r in rects) + 20, max(r[3] for r in rects) + 20)
        ImageGrab.grab(bbox=box, all_screens=True).save(a.out)
        print("captura", a.out, box, len(rects), "ventanas")
    else:
        print("sin ventanas visibles; exit:", p.poll())
    if not a.keep:
        p.kill()
        p.wait()
        shutil.rmtree(root, ignore_errors=True)
    else:
        print("pid", p.pid, "datos", root)


if __name__ == "__main__":
    main()
