"""Recorrido de celdas y pilas con mouse y teclado reales, igual para
Simpcky y las demos, con una captura en cada paso.

Mueve el mouse de verdad (~25 s) y muestra el escritorio (Win+D) al empezar y
al terminar: no tocar nada mientras corre. Datos aislados en una carpeta temporal.

Uso: python tour.py EXE carpeta [--dark] [--glass N] [--frames archivo]
"""
import argparse, ctypes, json, os, shutil, subprocess, tempfile, time
from ctypes import wintypes as w

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
from PIL import ImageGrab

u32 = ctypes.windll.user32


def note(i, x, y, color, title, text, rolled=False, mode=0, w=300, h=330):
    return dict(id=i, uid="%032x" % i, x=x, y=y, w=w, h=h, color=color, layer=1, rollMode=mode, rolled=rolled,
                title=title, text=text, t=[0, 0, 0, 0])


NOTES = [
    note(1, 140, 140, 0, "Compras", "Yerba\nPan\nPilas AA\nCafé"),
    note(2, 480, 140, 3, "Ideas", "Celdas chicas y pilas enrolladas.\nProbar vidrio nativo."),
    note(3, 820, 140, 2, "Llamar", "Martes 15 h: contador\nJueves: dentista"),
    note(4, 1500, 120, 1, "Pass", "wifi: ********", rolled=True, mode=1),
    note(5, 1500, 170, 4, "Reunión", "Lunes 10 h", rolled=True, mode=1),
]


def move(x0, y0, x1, y1, ms=500, steps=None):
    steps = steps or max(10, ms // 8)
    for i in range(1, steps + 1):
        t = i / steps
        e = t * t * (3 - 2 * t)
        u32.SetCursorPos(int(x0 + (x1 - x0) * e), int(y0 + (y1 - y0) * e))
        time.sleep(ms / 1000 / steps)


def down():
    u32.mouse_event(0x0002, 0, 0, 0, 0)


def up():
    u32.mouse_event(0x0004, 0, 0, 0, 0)


def key(vk, release=False):
    u32.keybd_event(vk, 0, 2 if release else 0, 0)


def combo(*vks):
    for v in vks:
        key(v)
    for v in reversed(vks):
        key(v, True)


def show_desktop():
    combo(0x5B, 0x44)  # Win+D
    time.sleep(0.8)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("out")
    ap.add_argument("--dark", action="store_true")
    ap.add_argument("--glass", type=int, default=0)
    ap.add_argument("--frames")
    ap.add_argument("--only-start", action="store_true", help="solo la captura inicial")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    root = tempfile.mkdtemp(prefix="simpcky-tour-")
    for d in ("Simpcky", "SimpckyD2D", "SimpckySlint"):
        os.makedirs(os.path.join(root, d))
    json.dump(NOTES, open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"), ensure_ascii=False)
    sim = {"dark": a.dark, "autoUpdate": False, "desktopMenu": False, "translucency": a.glass}
    open(os.path.join(root, "Simpcky", "settings.json"), "w").write(json.dumps(sim))
    for d in ("SimpckyD2D", "SimpckySlint"):
        open(os.path.join(root, d, "settings.json"), "w").write(json.dumps({"dark": a.dark, "glass": a.glass}))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="recorrido")
    if a.frames:
        env["SIMPCKY_FRAMES"] = os.path.abspath(a.frames)
        if os.path.exists(a.frames):
            os.remove(a.frames)
    show_desktop()
    exe = os.path.abspath(a.exe)
    p = subprocess.Popen([exe], env=env, cwd=os.path.dirname(exe))
    n = [0]

    def shot(name, wait=0.0):
        time.sleep(wait)
        n[0] += 1
        ImageGrab.grab(bbox=(0, 0, 1920, 1080), all_screens=True).save(os.path.join(a.out, f"{n[0]:02d}-{name}.png"))

    try:
        time.sleep(3.5)
        shot("inicio")
        if a.only_start:
            return
        # 1. "Ideas" a la pila de la derecha (antes de que las otras se corran).
        u32.SetCursorPos(560, 158)
        time.sleep(0.2)
        down()
        move(560, 158, 1580, 205, 800)
        shot("hacia-la-pila", 0.3)
        up()
        u32.SetCursorPos(1100, 700)  # que no se despliegue por tener el mouse encima
        shot("en-la-pila", 0.5)
        # 2. Ctrl + arrastrar "Compras": cuadrícula, vista previa.
        u32.SetCursorPos(220, 158)
        time.sleep(0.2)
        down()
        move(220, 158, 260, 200, 150)
        key(0x11)
        move(260, 200, 560, 230, 700)
        shot("ctrl-arrastre", 0.25)
        up()
        key(0x11, True)
        shot("soltada-en-celda", 0.45)
        # 3. "Llamar" al borde de arriba: panel; elegir 2×1; bajar y soltar.
        u32.SetCursorPos(900, 158)
        time.sleep(0.2)
        down()
        move(900, 158, 960, 2, 600)
        shot("panel", 0.3)
        move(960, 2, 873, 38, 300)
        time.sleep(0.25)
        shot("forma-2x1", 0.2)
        move(873, 38, 1000, 600, 700)
        shot("forma-con-cuadricula", 0.25)
        up()
        shot("soltada-2x1", 0.45)
        # 4. Esc a mitad de un arrastre: vuelve.
        u32.SetCursorPos(1000, 640)
        time.sleep(0.2)
        down()
        move(1000, 640, 700, 800, 400)
        key(0x1B)
        time.sleep(0.15)  # que el cuadro vea Esc antes de que se suelte el botón
        key(0x1B, True)
        up()
        shot("esc-vuelve", 0.5)
        time.sleep(1.0)
    finally:
        p.kill()
        p.wait()
        shutil.rmtree(root, ignore_errors=True)
        show_desktop()


if __name__ == "__main__":
    main()
