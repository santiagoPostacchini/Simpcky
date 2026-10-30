"""Prueba de celdas y pilas sin mouse: corre `celdas_guion.txt` en una instancia
de prueba (cursor falso, ver `snap.rs` → "Guion de prueba") y saca una captura
cada vez que el registro dice FOTO.

Las coordenadas son de un segundo monitor de 1920 × 1200 a 150 % ubicado a la
derecha del principal (MON y las notas de abajo): ajustarlas a otra pantalla.

Uso: cargo build --release
     python examples/celdas_guion.py target/release/simpcky.exe examples/celdas_guion.txt <carpeta de capturas>
"""
import json, os, subprocess, sys, time
import ctypes
ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
from PIL import ImageGrab
EXE, GUION, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
MON = (1920, 0, 3840, 1200)
PROFILE = os.path.join(os.environ["TEMP"], "simpcky-celdas-prueba")
DATA = os.path.join(PROFILE, "Simpcky")
os.makedirs(DATA, exist_ok=True); os.makedirs(OUT, exist_ok=True)
for f in os.listdir(DATA): os.remove(os.path.join(DATA, f))
for f in os.listdir(OUT): os.remove(os.path.join(OUT, f))
def note(i, title, color, x, y, w, h, rolled, mode, text):
    return {"id": i, "x": x, "y": y, "w": w, "h": h, "color": color, "layer": 1, "rollMode": mode, "rolled": rolled, "title": title, "text": text}
# Pila a la derecha (barra de 40 + 15 de separación a 150 %), Auto: se enrollan solas.
notes = [
    note(1, "Copypastes", 4, 3822 - 645, 18, 645, 450, False, 1, "git log --oneline -20\ncargo build --release"),
    note(2, "Pass", 0, 3822 - 495, 73, 495, 360, False, 1, "wifi casa: ********"),
    note(3, "mafernandez", 3, 3822 - 405, 128, 405, 390, False, 1, "Martes 15 h\nRevisar contrato"),
    note(4, "Compras", 2, 2050, 200, 420, 480, False, 0, "☐ Yerba\n☑ Pan\n☐ Pilas AA\n☐ Café"),
    note(5, "Ideas Simpcky", 1, 2600, 600, 420, 480, False, 0, "• Celdas chicas\n• Pilas enrolladas"),
]
json.dump(notes, open(os.path.join(DATA, "notes.json"), "w", encoding="utf-8"), ensure_ascii=False)
open(os.path.join(DATA, "settings.json"), "w").write('{"dark":true,"defaultRollMode":0,"desktopMenu":false,"autoUpdate":false,"translucency":0,"scale":0,"hotkey":0}')
log = os.path.join(PROFILE, "trace.log")
if os.path.exists(log): os.remove(log)
env = dict(os.environ, APPDATA=PROFILE, SIMPCKY_INSTANCIA="celdasprueba", SIMPCKY_PAINT_LOG=log, SIMPCKY_GUION=GUION)
p = subprocess.Popen([EXE], env=env)
seen, n, t0 = 0, 0, time.time()
try:
    while time.time() - t0 < 90:
        time.sleep(0.03)
        if not os.path.exists(log): continue
        lines = open(log, encoding="utf-8", errors="replace").read().splitlines()
        for line in lines[seen:]:
            parts = line.split("\t")
            if len(parts) > 1 and parts[1].startswith("FOTO "):
                time.sleep(0.0 if parts[1][5:].startswith("ya_") else 0.25)
                n += 1
                ImageGrab.grab(bbox=MON, all_screens=True).resize((960, 600)).save(os.path.join(OUT, f"{n:02d}_{parts[1][5:]}.png"))
            if len(parts) > 1 and parts[1] == "GUION FIN":
                raise StopIteration
        seen = len(lines)
except StopIteration:
    pass
finally:
    time.sleep(0.3)
    p.kill()
for k, v in sorted({x["title"]: x for x in json.load(open(os.path.join(DATA, "notes.json"), encoding="utf-8"))}.items()):
    print(f'{k:14} x={v["x"]} y={v["y"]} w={v["w"]} h={v["h"]} rolled={v["rolled"]} mode={v["rollMode"]}')
