# Demo Slint

Plataforma Win32 propia, sin winit. Cada nota tiene su componente Slint,
su adaptador de software y un DIB BGRA premultiplicado persistente.
`common::snap` maneja las celdas, las pilas y las transiciones.

Desde `demos/`:

```powershell
cargo build --offline --release -p simpcky-slint
```

Atajos: Ctrl+N crea una nota, Ctrl+R enrolla, Ctrl+G cambia el vidrio,
Ctrl+D cambia el tema y Ctrl+Mayús+T cambia la capa. Ctrl+Mayús+clic en
la barra arma una pila. El texto es plano, editable y con ajuste de línea.
Los datos se guardan en `%APPDATA%/SimpckySlint` tras 800 ms sin cambios.

La cuadrícula usa bordes continuos porque Slint no admite bordes punteados.
El portapapeles del sistema no está conectado.

## Verificación

Desde la raíz del repo:

```powershell
python demos/slint/verification/shot.py demos/target/release/simpcky-slint.exe demos/slint/verification/slint.png --top
python demos/slint/verification/shot.py demos/target/release/simpcky-slint.exe demos/slint/verification/slint-dark.png --top --dark
```

Este adaptador ejecuta `demos/raven/bench/shot.py` sin modificarlo. Si la
sesión no permite capturar la pantalla, compone la imagen con los DIB
presentados por las cuatro ventanas. Esa alternativa permite revisar el
render, pero no verifica la composición de Windows ni el desenfoque nativo.
`SIMPCKY_DUMP_DIR` activa ese volcado opcional; `SIMPCKY_FRAMES` activa el
registro compartido de cuadros.

La prueba `python demos/slint/verification/input.py` comprueba entrada Unicode,
guardado diferido y el botón de nueva nota con mensajes Win32. No sustituye
una prueba de arrastre con el mouse físico: el motor consulta el estado real
del botón y del cursor.
