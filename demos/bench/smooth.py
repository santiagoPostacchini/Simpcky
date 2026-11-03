"""Fluidez del arrastre y de las animaciones, medida desde afuera e igual
para Simpcky y las demos: mueve el mouse de verdad y lee la posición y el
tamaño de la ventana de la nota ~cada milisegundo en otro hilo.

Escenarios (cada uno en un proceso nuevo, con datos aislados):
  arrastre  la nota sigue al cursor a velocidad constante (1000 px/s)
  encaje    Ctrl + arrastrar y soltar: la nota se acomoda en su celda (200 ms)
  enrollar  clic en la barra: se enrolla (170 ms); otro clic: se abre

Muestra el escritorio (Win+D) al empezar y al terminar. No tocar nada.

Uso: python smooth.py --exe EXE --label NOMBRE [--runs 3] [--out smooth.json] [--glass N]
     python smooth.py --compare smooth.json
"""
import argparse, ctypes, json, os, shutil, statistics, subprocess, tempfile, threading, time
from ctypes import wintypes as w
from pathlib import Path

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
u32 = ctypes.windll.user32
u32.WindowFromPoint.argtypes = [w.POINT]
u32.WindowFromPoint.restype = w.HWND
u32.GetParent.argtypes = [w.HWND]
u32.GetParent.restype = w.HWND
u32.GetWindowRect.argtypes = [w.HWND, ctypes.POINTER(w.RECT)]
u32.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
ctypes.windll.winmm.timeBeginPeriod(1)

NOTE = dict(id=1, uid="%032x" % 1, x=300, y=300, w=300, h=330, color=0, layer=1, rollMode=0, rolled=False,
            title="Medir", text="Una nota para medir la fluidez.\nSegunda línea.", t=[0, 0, 0, 0])
OTHER = dict(NOTE, id=2, uid="%032x" % 2, x=1300, y=600, color=3, title="Otra")
GRAB = (380, 318)


def pid_of(h):
    p = w.DWORD()
    u32.GetWindowThreadProcessId(h, ctypes.byref(p))
    return p.value


def note_window(pid, pt):
    """La ventana de la nota bajo `pt` (subiendo hasta la última del proceso)."""
    h = u32.WindowFromPoint(w.POINT(*pt))
    if not h or pid_of(h) != pid:
        return None
    while True:
        p = u32.GetParent(h)
        if not p or pid_of(p) != pid:
            return h
        h = p


class Sampler(threading.Thread):
    def __init__(self, hwnd):
        super().__init__(daemon=True)
        self.hwnd, self.go, self.samples = hwnd, True, []

    def run(self):
        r = w.RECT()
        last = None
        while self.go:
            u32.GetWindowRect(self.hwnd, ctypes.byref(r))
            v = (r.left, r.top, r.right - r.left, r.bottom - r.top)
            if v != last:
                self.samples.append((time.perf_counter(), v))
                last = v
            time.sleep(0.0005)


def cpu_seconds(h):
    c, e, k, us = (w.FILETIME() for _ in range(4))
    ctypes.windll.kernel32.GetProcessTimes(h, ctypes.byref(c), ctypes.byref(e), ctypes.byref(k), ctypes.byref(us))
    f = lambda t: (t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7
    return f(k) + f(us)


def down():
    u32.mouse_event(0x0002, 0, 0, 0, 0)


def up():
    u32.mouse_event(0x0004, 0, 0, 0, 0)


def glide(x0, y0, x1, y1, ms, log):
    """Cursor a velocidad constante, anotando dónde estaba en cada momento."""
    t0 = time.perf_counter()
    while True:
        t = (time.perf_counter() - t0) * 1000 / ms
        if t >= 1:
            break
        x, y = int(x0 + (x1 - x0) * t), int(y0 + (y1 - y0) * t)
        u32.SetCursorPos(x, y)
        log.append((time.perf_counter(), x, y))
        time.sleep(0.002)
    u32.SetCursorPos(x1, y1)
    log.append((time.perf_counter(), x1, y1))


def gaps(times):
    return [(b - a) * 1000 for a, b in zip(times, times[1:])]


def anim_stats(samples, t_start, key):
    """Pasos de una animación desde `t_start` hasta que se queda quieta."""
    pts = [(t, v) for t, v in samples if t >= t_start]
    if len(pts) < 2:
        return dict(steps=len(pts), ms=None, max_gap_ms=None, mean_gap_ms=None, jank=None, max_jump_px=None)
    times = [t for t, _ in pts]
    vals = [key(v) for _, v in pts]
    g = gaps(times)
    jumps = [abs(b - a) for a, b in zip(vals, vals[1:])]
    return dict(steps=len(pts), ms=(times[-1] - t_start) * 1000, max_gap_ms=max(g), mean_gap_ms=statistics.mean(g),
                jank=sum(1 for x in g if x > 25), max_jump_px=max(jumps))


def scenario(exe, kind, glass):
    root = tempfile.mkdtemp(prefix="simpcky-smooth-")
    for d in ("Simpcky", "SimpckyD2D", "SimpckySlint"):
        os.makedirs(os.path.join(root, d))
    json.dump([NOTE, OTHER], open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"))
    open(os.path.join(root, "Simpcky", "settings.json"), "w").write(json.dumps({"autoUpdate": False, "desktopMenu": False, "translucency": glass}))
    for d in ("SimpckyD2D", "SimpckySlint"):
        open(os.path.join(root, d, "settings.json"), "w").write(json.dumps({"glass": glass}))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="fluidez")
    p = subprocess.Popen([str(exe)], env=env, cwd=str(exe.parent))
    try:
        time.sleep(3.0)
        hwnd = note_window(p.pid, GRAB)
        if not hwnd:
            return dict(error="no encontré la nota")
        handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, p.pid)
        cpu0 = cpu_seconds(handle)
        s = Sampler(hwnd)
        s.start()
        out = {}
        if kind == "arrastre":
            u32.SetCursorPos(*GRAB)
            time.sleep(0.3)
            down()
            log = []
            glide(GRAB[0], GRAB[1], GRAB[0] + 20, GRAB[1] + 10, 100, [])
            t_from = time.perf_counter()
            glide(GRAB[0] + 20, GRAB[1] + 10, GRAB[0] + 820, GRAB[1] + 210, 800, log)
            glide(GRAB[0] + 820, GRAB[1] + 210, GRAB[0] + 20, GRAB[1] + 10, 800, log)
            time.sleep(0.15)
            up()
            time.sleep(0.4)
            s.go = False
            s.join()
            pts = [(t, v) for t, v in s.samples if t >= t_from and t <= log[-1][0]]
            # Atraso: dónde debería estar la nota (cursor - agarre) vs dónde está.
            gx, gy = GRAB[0] - NOTE["x"], GRAB[1] - NOTE["y"]
            lag_px, lag_ms = [], []
            for t, v in pts:
                cur = max((c for c in log if c[0] <= t), key=lambda c: c[0], default=None)
                if cur:
                    lag_px.append(((cur[1] - gx - v[0]) ** 2 + (cur[2] - gy - v[1]) ** 2) ** 0.5)
                # hace cuánto el cursor estuvo donde está la nota
                past = [c for c in log if c[0] <= t and abs(c[1] - gx - v[0]) <= 2 and abs(c[2] - gy - v[1]) <= 2]
                if past:
                    lag_ms.append((t - past[-1][0]) * 1000)
            g = gaps([t for t, _ in pts])
            dur = pts[-1][0] - pts[0][0] if len(pts) > 1 else 1
            out = dict(updates_per_s=len(pts) / dur, lag_px_mean=statistics.mean(lag_px) if lag_px else None,
                       lag_px_p95=sorted(lag_px)[int(len(lag_px) * 0.95)] if lag_px else None,
                       lag_ms_mean=statistics.mean(lag_ms) if lag_ms else None,
                       max_gap_ms=max(g) if g else None, jank_gt25ms=sum(1 for x in g if x > 25))
        elif kind == "encaje":
            u32.SetCursorPos(*GRAB)
            time.sleep(0.3)
            down()
            glide(GRAB[0], GRAB[1], GRAB[0] + 30, GRAB[1] + 10, 100, [])
            u32.keybd_event(0x11, 0, 0, 0)
            glide(GRAB[0] + 30, GRAB[1] + 10, GRAB[0] + 433, GRAB[1] + 157, 600, [])
            time.sleep(0.3)
            t_up = time.perf_counter()
            up()
            u32.keybd_event(0x11, 0, 2, 0)
            time.sleep(0.8)
            s.go = False
            s.join()
            out = anim_stats(s.samples, t_up, lambda v: (v[0] ** 2 + v[1] ** 2) ** 0.5)
        else:  # enrollar
            u32.SetCursorPos(*GRAB)
            time.sleep(0.4)
            t0 = time.perf_counter()
            down(); up()
            time.sleep(0.6)
            t1 = time.perf_counter()
            down(); up()
            time.sleep(0.6)
            s.go = False
            s.join()
            a = anim_stats([x for x in s.samples if x[0] < t1], t0, lambda v: v[3])
            b = anim_stats(s.samples, t1, lambda v: v[3])
            out = {f"cerrar_{k}": v for k, v in a.items()}
            out.update({f"abrir_{k}": v for k, v in b.items()})
        out["cpu_ms"] = (cpu_seconds(handle) - cpu0) * 1000
        ctypes.windll.kernel32.CloseHandle(handle)
        return out
    finally:
        p.kill()
        p.wait()
        shutil.rmtree(root, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", type=Path)
    ap.add_argument("--label")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--glass", type=int, default=0)
    ap.add_argument("--out", type=Path, default=Path("smooth.json"))
    ap.add_argument("--compare", type=Path)
    a = ap.parse_args()
    if a.compare:
        recs = json.loads(a.compare.read_text(encoding="utf-8"))
        labels = list(dict.fromkeys(r["label"] for r in recs))
        for kind in ("arrastre", "encaje", "enrollar"):
            rows = [r for r in recs if r["kind"] == kind and "error" not in r]
            if not rows:
                continue
            keys = [k for k in rows[0] if k not in ("label", "kind")]
            print(f"\n### {kind}\n")
            print("| métrica | " + " | ".join(labels) + " |")
            print("|---|" + "---|" * len(labels))
            for k in keys:
                cells = []
                for l in labels:
                    vals = [r[k] for r in rows if r["label"] == l and r.get(k) is not None]
                    cells.append(f"{statistics.mean(vals):.1f}" if vals else "—")
                print(f"| {k} | " + " | ".join(cells) + " |")
        return
    recs = json.loads(a.out.read_text(encoding="utf-8")) if a.out.exists() else []
    u32.keybd_event(0x5B, 0, 0, 0); u32.keybd_event(0x44, 0, 0, 0); u32.keybd_event(0x44, 0, 2, 0); u32.keybd_event(0x5B, 0, 2, 0)
    time.sleep(0.8)
    try:
        for kind in ("arrastre", "encaje", "enrollar"):
            for i in range(a.runs):
                r = scenario(a.exe.resolve(), kind, a.glass)
                r.update(label=a.label, kind=kind)
                recs.append(r)
                print(a.label, kind, i + 1, {k: (round(v, 1) if isinstance(v, float) else v) for k, v in r.items() if k not in ("label", "kind")})
    finally:
        a.out.write_text(json.dumps(recs, indent=1, ensure_ascii=False), encoding="utf-8")
        u32.keybd_event(0x5B, 0, 0, 0); u32.keybd_event(0x44, 0, 0, 0); u32.keybd_event(0x44, 0, 2, 0); u32.keybd_event(0x5B, 0, 2, 0)


if __name__ == "__main__":
    main()
