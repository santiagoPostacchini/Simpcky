"""Costo "en uso": escribir texto y crear notas nuevas, con teclado real.

Para cada programa: abre 5 notas (siempre encima, datos aislados), hace clic
en el cuerpo de la primera, escribe N caracteres a ritmo de persona y después
crea 10 notas con Ctrl+N. Mide el tiempo de CPU gastado y la memoria antes y
después. Mueve el mouse y usa el teclado de verdad: no tocar nada mientras corre.

Uso: python active.py --exe EXE --label NOMBRE [--chars 300] [--runs 3] [--out active.json]
     python active.py --compare active.json
"""
import argparse, ctypes, json, os, shutil, statistics, subprocess, tempfile, time
from ctypes import wintypes
from pathlib import Path

ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
u32, k32, psapi = ctypes.windll.user32, ctypes.windll.kernel32, ctypes.windll.psapi


class PMC(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD), ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t), ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t), ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t), ("PrivateUsage", ctypes.c_size_t)]


class KI(ctypes.Structure):
    _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class INP(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("ki", KI), ("pad", ctypes.c_ubyte * 8)]


def mem(h):
    m = PMC(); m.cb = ctypes.sizeof(m)
    psapi.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb)
    return m.WorkingSetSize, m.PrivateUsage


def cpu(h):
    c, e, k, u = (wintypes.FILETIME() for _ in range(4))
    k32.GetProcessTimes(h, ctypes.byref(c), ctypes.byref(e), ctypes.byref(k), ctypes.byref(u))
    f = lambda t: (t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7
    return f(k) + f(u)


def send_char(ch):
    for flags in (4, 6):
        i = INP(1, KI(0, ord(ch), flags, 0, 0))
        u32.SendInput(1, ctypes.byref(i), ctypes.sizeof(i))


def ctrl_n():
    for vk, up in ((0x11, 0), (0x4E, 0), (0x4E, 2), (0x11, 2)):
        u32.keybd_event(vk, 0, up, 0)


def run(exe, chars):
    root = tempfile.mkdtemp(prefix="simpcky-act-")
    for d in ("Simpcky", "SimpckyRaven"):
        os.makedirs(os.path.join(root, d))
    notes = [dict(id=i + 1, x=100 + i * 60, y=100 + i * 40, w=300, h=330, color=i % 6, layer=2, rollMode=0, rolled=False,
                  title=f"Nota {i + 1}", text="Texto de prueba.") for i in range(5)]
    notes.reverse()  # la nota 1 queda arriba de todo
    json.dump(notes, open(os.path.join(root, "Simpcky", "notes.json"), "w", encoding="utf-8"))
    env = dict(os.environ, APPDATA=root, SIMPCKY_INSTANCIA="activo")
    p = subprocess.Popen([str(exe)], env=env, cwd=str(exe.parent))
    h = k32.OpenProcess(0x1000 | 0x0010, False, p.pid)  # QUERY_LIMITED_INFORMATION | VM_READ
    try:
        time.sleep(4)
        ws0, pv0 = mem(h)
        u32.SetCursorPos(250, 300)
        u32.mouse_event(2, 0, 0, 0, 0); u32.mouse_event(4, 0, 0, 0, 0)
        time.sleep(0.5)
        text = ("Escribo una nota de prueba para medir. " * 20)[:chars]
        c0, t0 = cpu(h), time.perf_counter()
        for ch in text:
            send_char(ch)
            time.sleep(0.02)  # ~50 caracteres por segundo
        time.sleep(1.0)
        c1, t1 = cpu(h), time.perf_counter()
        ws1, pv1 = mem(h)
        c2 = cpu(h)
        for _ in range(10):
            ctrl_n()
            time.sleep(0.4)
        time.sleep(2.0)
        c3 = cpu(h)
        ws2, pv2 = mem(h)
        mb = 1 / 1048576
        return dict(
            typing_cpu_ms=(c1 - c0) * 1000, typing_cpu_ms_per_char=(c1 - c0) * 1000 / chars,
            typing_cpu_percent=(c1 - c0) / (t1 - t0) * 100,
            ws_before_mb=ws0 * mb, ws_after_typing_mb=ws1 * mb, private_after_typing_mb=pv1 * mb,
            new_notes_cpu_ms=(c3 - c2) * 1000, ws_after_10_new_mb=ws2 * mb, private_after_10_new_mb=pv2 * mb,
            ws_per_new_note_mb=(ws2 - ws1) * mb / 10, private_per_new_note_mb=(pv2 - pv1) * mb / 10,
        )
    finally:
        p.kill(); p.wait()
        k32.CloseHandle(h)
        shutil.rmtree(root, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", type=Path)
    ap.add_argument("--label")
    ap.add_argument("--chars", type=int, default=300)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--out", type=Path, default=Path("active.json"))
    ap.add_argument("--compare", type=Path)
    a = ap.parse_args()
    if a.compare:
        recs = json.loads(a.compare.read_text(encoding="utf-8"))
        labels = list(dict.fromkeys(r["label"] for r in recs))
        keys = [k for k in recs[0] if k not in ("label",)]
        print("| métrica | " + " | ".join(labels) + " |")
        print("|---|" + "---|" * len(labels))
        for k in keys:
            vals = [statistics.mean(r[k] for r in recs if r["label"] == l) for l in labels]
            print(f"| {k} | " + " | ".join(f"{v:.2f}" for v in vals) + " |")
        return
    recs = json.loads(a.out.read_text(encoding="utf-8")) if a.out.exists() else []
    for i in range(a.runs):
        r = run(a.exe.resolve(), a.chars)
        r["label"] = a.label
        recs.append(r)
        print(a.label, i + 1, {k: round(v, 2) for k, v in r.items() if isinstance(v, float)})
        time.sleep(1)
    a.out.write_text(json.dumps(recs, indent=1, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    main()
