"""Recorrido con mouse y teclado reales sobre la demo (o Simpcky), con una
captura en cada paso. Mueve el mouse de verdad unos segundos.

Uso: python interact.py EXE carpeta_capturas [--dark]
"""
import argparse, ctypes, json, os, shutil, subprocess, tempfile, time
from ctypes import wintypes

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
from PIL import ImageGrab

u32 = ctypes.windll.user32
NOTES = [
    dict(id=1, x=120, y=120, w=300, h=330, color=0, layer=2, rollMode=0, rolled=False, title="Compras",
         text="- [ ] Yerba\n- [x] Pan"),
    dict(id=2, x=450, y=120, w=300, h=330, color=3, layer=2, rollMode=1, rolled=True, title="Auto", text="Se despliega sola"),
]
BOX = (100, 100, 1000, 640)


def click(x, y, double=False):
    u32.SetCursorPos(x, y)
    time.sleep(0.15)
    for _ in range(2 if double else 1):
        u32.mouse_event(0x0002, 0, 0, 0, 0)
        u32.mouse_event(0x0004, 0, 0, 0, 0)
        time.sleep(0.05)


def key(vk, up=False):
    u32.keybd_event(vk, 0, 2 if up else 0, 0)


def combo(*vks):
    for v in vks:
        key(v)
    for v in reversed(vks):
        key(v, True)


def type_text(text):
    # WM_CHAR por SendInput con KEYEVENTF_UNICODE
    class KI(ctypes.Structure):
        _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_size_t)]
    class INP(ctypes.Structure):
        _fields_ = [("type", wintypes.DWORD), ("ki", KI), ("pad", ctypes.c_ubyte * 8)]
    for ch in text:
        for flags in (4, 4 | 2):
            i = INP(1, KI(0, ord(ch), flags, 0, 0))
            u32.SendInput(1, ctypes.byref(i), ctypes.sizeof(i))
        time.sleep(0.01)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("out")
    ap.add_argument("--dark", action="store_true")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    root = tempfile.mkdtemp(prefix="simpcky-int-")
    for d in ("Simpcky", "SimpckyRaven"):
        os.makedirs(os.path.join(root, d))
    json.dump(NOTES, open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"), ensure_ascii=False)
    open(os.path.join(root, "Simpcky", "settings.json"), "w").write(json.dumps({"dark": a.dark, "autoUpdate": False, "desktopMenu": False}))
    open(os.path.join(root, "SimpckyRaven", "settings.json"), "w").write(json.dumps({"dark": a.dark}))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="recorrido")
    p = subprocess.Popen([os.path.abspath(a.exe)], env=env, cwd=os.path.dirname(os.path.abspath(a.exe)))
    n = [0]
    def shot(name):
        time.sleep(0.6)
        n[0] += 1
        ImageGrab.grab(bbox=BOX, all_screens=True).save(os.path.join(a.out, f"{n[0]:02d}-{name}.png"))
    try:
        time.sleep(3)
        u32.SetCursorPos(250, 135); shot("hover-encabezado")
        click(200, 300); type_text("Hola desde el recorrido"); shot("escribir")
        click(390, 137); time.sleep(0.4); shot("menu")
        key(0x1B); key(0x1B, True)
        click(200, 135); shot("clic-enrolla")
        click(200, 135); shot("clic-despliega")
        u32.SetCursorPos(600, 137); time.sleep(0.8); shot("auto-se-despliega")
        u32.SetCursorPos(900, 600); time.sleep(1.2); shot("auto-se-enrolla")
        click(200, 135, double=True); time.sleep(0.3); type_text("Renombrada"); key(0x0D); key(0x0D, True); shot("renombrar")
        click(200, 300); combo(0x11, 0x4E); shot("ctrl-n")
        time.sleep(1.5)
        saved = os.path.join(root, "SimpckyRaven", "notes.json")
        if not os.path.exists(saved):
            saved = os.path.join(root, "Simpcky", "notes.json")
        print(open(saved, encoding="utf-8").read()[:600])
    finally:
        p.kill(); p.wait()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
