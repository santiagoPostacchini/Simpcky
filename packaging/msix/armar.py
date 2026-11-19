"""Arma target/msix/Simpcky-<versión>.msix con target/release/simpcky.exe.

    python packaging/msix/armar.py           identidad de la tienda (para subir a Partner Center)
    python packaging/msix/armar.py --prueba  identidad de prueba, para registrar la carpeta en esta PC
                                             (con el modo de programador de Windows activado):
                                             Add-AppxPackage -Register target/msix/layout/AppxManifest.xml

Antes: cargo build --release (en el árbol principal, que tiene google_client.json).
Necesita makeappx.exe del SDK de Windows.
"""
import glob, re, shutil, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

# Partner Center → la app → Administración de productos → Identidad del producto:
# Package/Identity/Name, Package/Identity/Publisher y Package/Properties/PublisherDisplayName.
STORE = dict(NAME="SantiagoPostacchini.Simpcky", PUBLISHER="CN=1D1DDD2F-106C-41AE-B3B7-52BC035309F0",
             PUBLISHER_DISPLAY="Santiago Postacchini")
# Otro nombre, así no choca con la versión de la tienda. (Sin firmar no se puede instalar el
# .msix de una app: Windows rechaza las "Executable activations" sin firma.)
TEST = dict(NAME="Simpcky.Prueba", PUBLISHER="CN=Simpcky Prueba", PUBLISHER_DISPLAY="Simpcky (prueba)")


def makeappx():
    found = sorted(glob.glob(r"C:\Program Files (x86)\Windows Kits\10\bin\10.*\x64\makeappx.exe"))
    if not found:
        sys.exit("No encuentro makeappx.exe (SDK de Windows 10/11).")
    return found[-1]


def main():
    test = "--prueba" in sys.argv
    version = re.search(r'^version = "(\d+)\.(\d+)\.(\d+)"', (ROOT / "Cargo.toml").read_text(), re.M)
    # La tienda exige el cuarto número en 0.
    if not test and not all(STORE.values()):
        sys.exit("Faltan los datos de Partner Center en STORE (arriba en este archivo).")
    ident = dict(TEST if test else STORE, VERSION=".".join(version.groups()) + ".0")

    exe = ROOT / "target" / "release" / "simpcky.exe"
    if not exe.exists():
        sys.exit("Falta target/release/simpcky.exe: corré cargo build --release.")

    out = ROOT / "target" / "msix"
    layout = out / "layout"
    shutil.rmtree(layout, ignore_errors=True)
    shutil.copytree(HERE / "Assets", layout / "Assets")
    shutil.copy2(exe, layout / "simpcky.exe")
    manifest = (HERE / "AppxManifest.xml").read_text(encoding="utf-8")
    for k, v in ident.items():
        manifest = manifest.replace("{" + k + "}", v)
    assert "{" not in manifest.split("-->", 1)[1], "quedó un {CAMPO} sin reemplazar"
    (layout / "AppxManifest.xml").write_text(manifest, encoding="utf-8")

    msix = out / f"Simpcky-{ident['VERSION']}{'-prueba' if test else ''}.msix"
    subprocess.run([makeappx(), "pack", "/o", "/d", str(layout), "/p", str(msix)], check=True,
                   stdout=subprocess.DEVNULL)
    print(msix)


if __name__ == "__main__":
    main()
