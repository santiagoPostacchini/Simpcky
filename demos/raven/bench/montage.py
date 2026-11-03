"""Junta las capturas de una carpeta en una sola imagen (cuadrícula con el
nombre de cada paso). Uso: python montage.py carpeta salida.png [columnas]"""
import os, sys
from PIL import Image, ImageDraw

folder, out = sys.argv[1], sys.argv[2]
cols = int(sys.argv[3]) if len(sys.argv) > 3 else 3
files = sorted(f for f in os.listdir(folder) if f.endswith(".png"))
imgs = [Image.open(os.path.join(folder, f)) for f in files]
w = max(i.width for i in imgs) // 2
h = max(i.height for i in imgs) // 2 + 22
rows = (len(imgs) + cols - 1) // cols
sheet = Image.new("RGB", (w * cols, h * rows), "white")
d = ImageDraw.Draw(sheet)
for k, (f, im) in enumerate(zip(files, imgs)):
    x, y = (k % cols) * w, (k // cols) * h
    sheet.paste(im.resize((im.width // 2, im.height // 2)), (x, y + 22))
    d.text((x + 6, y + 4), f[:-4], fill="black")
sheet.save(out)
