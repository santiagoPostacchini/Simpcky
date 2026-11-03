"""Compara consumo y arranque de las dos interfaces de notas en Windows."""

import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time
import uuid
from datetime import datetime, timezone


class MemoryCounters(C.Structure):
    _fields_ = [("cb", W.DWORD), ("PageFaultCount", W.DWORD)] + [
        (name, C.c_size_t) for name in (
            "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
            "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
            "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage",
            "PrivateUsage")]


class ThreadEntry(C.Structure):
    _fields_ = [("dwSize", W.DWORD), ("cntUsage", W.DWORD),
                ("th32ThreadID", W.DWORD), ("th32OwnerProcessID", W.DWORD),
                ("tpBasePri", W.LONG), ("tpDeltaPri", W.LONG),
                ("dwFlags", W.DWORD)]


class CounterValue(C.Union):
    _fields_ = [("doubleValue", C.c_double), ("largeValue", C.c_longlong)]


class FormattedCounter(C.Structure):
    _anonymous_ = ("value",)
    _fields_ = [("CStatus", W.DWORD), ("value", CounterValue)]


class Win32:
    def __init__(self):
        self.kernel = C.WinDLL("kernel32", use_last_error=True)
        self.user = C.WinDLL("user32", use_last_error=True)
        self.psapi = C.WinDLL("psapi", use_last_error=True)
        self.callback = C.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
        self.bind(self.kernel, "OpenProcess", W.HANDLE, W.DWORD, W.BOOL, W.DWORD)
        self.bind(self.kernel, "CloseHandle", W.BOOL, W.HANDLE)
        self.bind(self.kernel, "TerminateProcess", W.BOOL, W.HANDLE, W.UINT)
        self.bind(self.kernel, "GetProcessTimes", W.BOOL, W.HANDLE,
                  *([C.POINTER(W.FILETIME)] * 4))
        self.bind(self.kernel, "GetProcessHandleCount", W.BOOL, W.HANDLE,
                  C.POINTER(W.DWORD))
        self.bind(self.kernel, "CreateToolhelp32Snapshot", W.HANDLE, W.DWORD, W.DWORD)
        for name in ("Thread32First", "Thread32Next"):
            self.bind(self.kernel, name, W.BOOL, W.HANDLE, C.POINTER(ThreadEntry))
        self.bind(self.psapi, "GetProcessMemoryInfo", W.BOOL, W.HANDLE,
                  C.POINTER(MemoryCounters), W.DWORD)
        self.bind(self.user, "GetGuiResources", W.DWORD, W.HANDLE, W.DWORD)
        self.bind(self.user, "EnumWindows", W.BOOL, self.callback, W.LPARAM)
        self.bind(self.user, "EnumChildWindows", W.BOOL, W.HWND, self.callback, W.LPARAM)
        self.bind(self.user, "GetWindowThreadProcessId", W.DWORD, W.HWND,
                  C.POINTER(W.DWORD))
        self.bind(self.user, "IsWindowVisible", W.BOOL, W.HWND)
        self.bind(self.user, "GetClassNameW", C.c_int, W.HWND, W.LPWSTR, C.c_int)
        self.bind(self.user, "GetSystemMetrics", C.c_int, C.c_int)

    @staticmethod
    def bind(dll, name, result, *args):
        fn = getattr(dll, name)
        fn.restype, fn.argtypes = result, list(args)
        return fn

    @staticmethod
    def check(value):
        if not value:
            raise C.WinError(C.get_last_error())
        return value

    def open(self, pid):
        # QUERY_INFORMATION | VM_READ | TERMINATE.
        return self.check(self.kernel.OpenProcess(0x0400 | 0x0010 | 0x0001, False, pid))

    def memory(self, handle):
        info = MemoryCounters()
        info.cb = C.sizeof(info)
        self.check(self.psapi.GetProcessMemoryInfo(handle, C.byref(info), info.cb))
        return info

    def cpu(self, handle):
        creation, exit_time, kernel, user = (W.FILETIME() for _ in range(4))
        self.check(self.kernel.GetProcessTimes(handle, C.byref(creation),
                   C.byref(exit_time), C.byref(kernel), C.byref(user)))
        return sum((t.dwHighDateTime << 32) | t.dwLowDateTime
                   for t in (kernel, user)) / 10_000_000

    def threads(self, pid):
        snapshot = self.kernel.CreateToolhelp32Snapshot(0x00000004, 0)
        if snapshot == C.c_void_p(-1).value:
            raise C.WinError(C.get_last_error())
        try:
            entry = ThreadEntry()
            entry.dwSize = C.sizeof(entry)
            count = 0
            ok = self.kernel.Thread32First(snapshot, C.byref(entry))
            while ok:
                count += entry.th32OwnerProcessID == pid
                ok = self.kernel.Thread32Next(snapshot, C.byref(entry))
            if C.get_last_error() != 18:  # ERROR_NO_MORE_FILES
                raise C.WinError(C.get_last_error())
            return count
        finally:
            self.kernel.CloseHandle(snapshot)

    def visible(self, pid):
        found = False

        @self.callback
        def child(hwnd, _):
            nonlocal found
            owner = W.DWORD()
            self.user.GetWindowThreadProcessId(hwnd, C.byref(owner))
            if owner.value == pid and self.user.IsWindowVisible(hwnd):
                found = True
            return True

        @self.callback
        def top(hwnd, data):
            child(hwnd, data)
            name = C.create_unicode_buffer(256)
            self.user.GetClassNameW(hwnd, name, len(name))
            if name.value in ("Progman", "WorkerW"):
                self.user.EnumChildWindows(hwnd, child, 0)
            return True

        self.user.EnumWindows(top, 0)
        return found

    def sample(self, handle, pid):
        mem = self.memory(handle)
        handles = W.DWORD()
        self.check(self.kernel.GetProcessHandleCount(handle, C.byref(handles)))
        return dict(working_set_bytes=mem.WorkingSetSize,
                    private_bytes=mem.PrivateUsage,
                    peak_working_set_bytes=mem.PeakWorkingSetSize,
                    threads=self.threads(pid),
                    gdi=self.user.GetGuiResources(handle, 0),
                    user=self.user.GetGuiResources(handle, 1),
                    handles=handles.value)


class GPUCounters:
    """PDH es opcional: cualquier fallo produce null para ese contador."""

    def __init__(self, pid):
        self.query = W.HANDLE()
        self.counters = {}
        try:
            self.dll = C.WinDLL("pdh")
            bind = Win32.bind
            bind(self.dll, "PdhOpenQueryW", W.LONG, W.LPCWSTR, C.c_size_t,
                 C.POINTER(W.HANDLE))
            bind(self.dll, "PdhCloseQuery", W.LONG, W.HANDLE)
            bind(self.dll, "PdhExpandWildCardPathW", W.LONG, W.LPCWSTR,
                 W.LPCWSTR, W.LPWSTR, C.POINTER(W.DWORD), W.DWORD)
            bind(self.dll, "PdhAddEnglishCounterW", W.LONG, W.HANDLE,
                 W.LPCWSTR, C.c_size_t, C.POINTER(W.HANDLE))
            bind(self.dll, "PdhCollectQueryData", W.LONG, W.HANDLE)
            bind(self.dll, "PdhGetFormattedCounterValue", W.LONG, W.HANDLE,
                 W.DWORD, C.POINTER(W.DWORD), C.POINTER(FormattedCounter))
            if self.dll.PdhOpenQueryW(None, 0, C.byref(self.query)):
                return
            for key, counter in (("gpu_dedicated_bytes", "Dedicated Usage"),
                                 ("gpu_shared_bytes", "Shared Usage")):
                path = rf"\GPU Process Memory(pid_{pid}*)\{counter}"
                size = W.DWORD()
                self.dll.PdhExpandWildCardPathW(None, path, None, C.byref(size), 0)
                if not size.value:
                    continue
                buffer = C.create_unicode_buffer(size.value)
                if self.dll.PdhExpandWildCardPathW(None, path, buffer,
                                                  C.byref(size), 0):
                    continue
                counters = []
                # Leer MULTI_SZ completo; .value sólo devuelve la primera ruta.
                for expanded in buffer[:].split("\0"):
                    if f"(pid_{pid}_" not in expanded.lower():
                        continue
                    handle = W.HANDLE()
                    if self.dll.PdhAddEnglishCounterW(self.query, expanded, 0,
                                                      C.byref(handle)) == 0:
                        counters.append(handle)
                if counters:
                    self.counters[key] = counters
            self.dll.PdhCollectQueryData(self.query)
        except Exception:
            self.counters = {}

    def sample(self):
        result = dict(gpu_dedicated_bytes=None, gpu_shared_bytes=None)
        try:
            if not self.query or self.dll.PdhCollectQueryData(self.query):
                return result
            for key, counters in self.counters.items():
                values = []
                for counter in counters:
                    value = FormattedCounter()
                    status = self.dll.PdhGetFormattedCounterValue(
                        counter, 0x00000200, None, C.byref(value))  # PDH_FMT_DOUBLE
                    if status or value.CStatus not in (0, 1):
                        break
                    values.append(value.doubleValue)
                if len(values) == len(counters):
                    result[key] = sum(values)
        except Exception:
            pass
        return result

    def close(self):
        try:
            if self.query:
                self.dll.PdhCloseQuery(self.query)
        except Exception:
            pass


def write_notes(folder, count, api):
    width = api.user.GetSystemMetrics(0)
    height = api.user.GetSystemMetrics(1)
    cols = max(1, (width - 80) // 300)
    rows = max(1, (height - 80) // 300)
    lines = [
        "En un lugar tranquilo, la luz entra por la ventana.",
        "Las ideas pequeñas encuentran su sitio en el papel.",
        "Recordar lo importante también deja espacio para soñar.",
        "Hoy caminamos despacio entre palabras y recuerdos.",
        "El viento mueve las hojas mientras pasa la tarde.",
        "Cada detalle cuenta una historia sencilla y cercana.",
        "Mañana habrá tiempo para nuevos caminos y preguntas.",
        "Una taza de café acompaña los planes pendientes.",
        "La memoria guarda colores, sonidos y momentos.",
        "Seguimos escribiendo con paciencia y curiosidad.",
    ]
    notes = [dict(id=i + 1, uid=uuid.uuid4().hex,
                  x=80 + (i % cols) * 300,
                  y=80 + ((i // cols) % rows) * 300,
                  w=280, h=320, color=i % 6, layer=1, rollMode=0,
                  rolled=False, title=f"Nota {i + 1}", text="\n".join(lines),
                  t=[0, 0, 0, 0]) for i in range(count)]
    destination = Path(folder) / "Simpcky"
    destination.mkdir()
    (destination / "notes.json").write_text(
        json.dumps(notes, ensure_ascii=False), encoding="utf-8")


def alive(process):
    if process.poll() is not None:
        raise RuntimeError(f"El proceso terminó antes de completar la prueba ({process.returncode})")


def startup(api, process, handle, started):
    first = settled = None
    stable_since = None
    anchor = None
    while time.perf_counter() - started < 20:
        alive(process)
        visible = api.visible(process.pid)
        ws = api.memory(handle).WorkingSetSize
        now = time.perf_counter()
        if visible and first is None:
            first = (now - started) * 1000
        if anchor is None or abs(ws - anchor) > max(1, anchor * 0.01):
            anchor, stable_since = ws, now
        if visible and now - stable_since >= 0.5:
            settled = (now - started) * 1000
            break
        time.sleep(0.01)
    return dict(first_window_ms=first, settled_ms=settled,
                startup_timed_out=settled is None)


def run(api, exe, count, idle):
    with tempfile.TemporaryDirectory(prefix="simpcky-bench-") as folder:
        write_notes(folder, count, api)
        env = dict(os.environ, APPDATA=folder, SIMPCKY_INSTANCIA="bench")
        process = None
        handle = None
        gpu = None
        try:
            started = time.perf_counter()
            process = subprocess.Popen([str(exe)], cwd=exe.parent, env=env)
            handle = api.open(process.pid)
            result = startup(api, process, handle, started)
            gpu = GPUCounters(process.pid)
            cpu_start = api.cpu(handle)
            wall_start = time.perf_counter()
            samples = []
            # Plazos absolutos evitan acumular el coste de cada muestreo.
            for second in range(1, math.ceil(idle) + 1):
                deadline = wall_start + min(second, idle)
                time.sleep(max(0, deadline - time.perf_counter()))
                alive(process)
                sample = api.sample(handle, process.pid)
                sample.update(gpu.sample())
                samples.append(sample)
            cpu_end = api.cpu(handle)
            wall_end = time.perf_counter()
            for key in samples[0]:
                values = [s[key] for s in samples if s[key] is not None]
                result[key] = (max(values) if key == "peak_working_set_bytes"
                               else statistics.median(values)) if values else None
            result["idle_cpu_percent"] = (cpu_end - cpu_start) / (wall_end - wall_start) * 100
            result["idle_wall_seconds"] = wall_end - wall_start
            return result
        finally:
            if gpu:
                gpu.close()
            try:
                if process is not None:
                    if process.poll() is None:
                        if handle:
                            api.check(api.kernel.TerminateProcess(handle, 1))
                        else:
                            process.kill()
                    process.wait(timeout=10)
            finally:
                if handle:
                    api.kernel.CloseHandle(handle)


def load_records(path):
    records = json.loads(path.read_text(encoding="utf-8")) if path.exists() else []
    if not isinstance(records, list):
        raise ValueError("El archivo de resultados debe contener una lista JSON")
    return records


def compare(path):
    records = load_records(path)
    metrics = [
        ("working_set_bytes", "WS MB", "mb"),
        ("private_bytes", "Privados MB", "mb"),
        ("peak_working_set_bytes", "Pico WS MB", "mb"),
        ("idle_cpu_percent", "CPU %", "cpu"),
        ("threads", "Hilos", "count"), ("gdi", "GDI", "count"),
        ("user", "USER", "count"), ("handles", "Handles", "count"),
        ("first_window_ms", "Ventana ms", "ms"),
        ("settled_ms", "Estable ms", "ms"),
        ("gpu_dedicated_bytes", "GPU dedicada MB", "mb"),
        ("gpu_shared_bytes", "GPU compartida MB", "mb"),
        ("exe_size_bytes", "EXE MB", "mb"),
        ("dll_size_bytes", "DLL MB", "mb")]
    for count in sorted({r["notes"] for r in records}):
        group = [r for r in records if r["notes"] == count]
        labels = list(dict.fromkeys(r["label"] for r in group))
        means = {}
        for label in labels:
            rows = [r for r in group if r["label"] == label]
            means[label] = {}
            for key, _, _ in metrics:
                values = [r[key] for r in rows if r.get(key) is not None]
                means[label][key] = statistics.mean(values) if values else None
        print(f"\nNotas: {count}\n")
        headers = ["Etiqueta", "Runs"] + [m[1] for m in metrics]
        if len(labels) == 2:
            a, b = labels
            print(f"A = {a}, B = {b}. MB = 2²⁰ bytes.\n")
            headers += ["B/A WS", "B/A privados"]
        print("| " + " | ".join(headers) + " |")
        print("| " + " | ".join("---" for _ in headers) + " |")
        for label in labels:
            cells = [str(label).replace("|", "\\|").replace("\n", " "),
                     str(sum(r["label"] == label for r in group))]
            for key, _, kind in metrics:
                value = means[label][key]
                if value is None:
                    cells.append("—")
                elif kind == "mb":
                    cells.append(f"{value / 2**20:.1f}")
                elif kind == "cpu":
                    cells.append(f"{value:.2f}")
                elif kind == "ms":
                    cells.append(f"{value:.0f}")
                else:
                    cells.append(f"{value:.1f}")
            if len(labels) == 2:
                for key in ("working_set_bytes", "private_bytes"):
                    numerator, denominator = means[b][key], means[a][key]
                    cells.append(f"{numerator / denominator:.2f}×"
                                 if label == b and numerator is not None and denominator
                                 else "—")
            print("| " + " | ".join(cells) + " |")


def positive(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("Debe ser un número positivo")
    return number


def positive_int(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("Debe ser un entero positivo")
    return number


def main():
    # La consola de Windows no es UTF-8 por defecto (la tabla usa ², ×, —).
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, help="Ejecutable a medir")
    parser.add_argument("--label", help="Nombre de la implementación")
    parser.add_argument("--notes", default="1,5,20", help="Cantidades separadas por comas")
    parser.add_argument("--runs", type=positive_int, default=3)
    parser.add_argument("--idle", type=positive, default=20, help="Segundos de muestreo")
    parser.add_argument("--out", type=Path, default=Path("results.json"))
    parser.add_argument("--compare", type=Path, help="Imprime una tabla Markdown")
    parser.add_argument("--selftest", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.compare:
        compare(args.compare)
        return
    if sys.platform != "win32":
        parser.error("Las mediciones requieren Windows")
    api = Win32()
    if args.selftest:
        handle = api.open(os.getpid())
        gpu = GPUCounters(os.getpid())
        try:
            result = api.sample(handle, os.getpid())
            result.update(gpu.sample())
            result.update(pid=os.getpid(), cpu_seconds=api.cpu(handle),
                          visible_window=api.visible(os.getpid()),
                          memory_struct_size=C.sizeof(MemoryCounters),
                          thread_struct_size=C.sizeof(ThreadEntry))
            assert result["working_set_bytes"] > 0 and result["threads"] > 0
            print(json.dumps(result, indent=2))
        finally:
            gpu.close()
            api.kernel.CloseHandle(handle)
        return
    if not args.exe or not args.label:
        parser.error("Se requieren --exe y --label")
    exe = args.exe.resolve(strict=True)
    if not exe.is_file():
        parser.error("--exe debe ser un archivo")
    try:
        counts = [positive_int(n.strip()) for n in args.notes.split(",")]
    except (ValueError, argparse.ArgumentTypeError):
        parser.error("--notes debe contener enteros positivos separados por comas")
    records = load_records(args.out)
    metadata = dict(exe=str(exe), label=args.label,
                    exe_size_bytes=exe.stat().st_size,
                    dll_size_bytes=sum(p.stat().st_size for p in exe.parent.iterdir()
                                       if p.is_file() and p.suffix.lower() == ".dll"))
    for count in counts:
        for repetition in range(1, args.runs + 1):
            print(f"{args.label}: {count} notas, run {repetition}/{args.runs}", flush=True)
            result = dict(metadata, notes=count, run=repetition, idle_seconds=args.idle,
                          timestamp=datetime.now(timezone.utc).isoformat())
            result.update(run(api, exe, count, args.idle))
            records.append(result)
            args.out.write_text(json.dumps(records, ensure_ascii=False, indent=2) + "\n",
                                encoding="utf-8")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)
