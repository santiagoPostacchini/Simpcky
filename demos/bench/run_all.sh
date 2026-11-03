#!/usr/bin/env bash
# Todas las mediciones, una detrás de la otra (no tocar el mouse mientras corre).
# Desde `demos/`:  bash bench/run_all.sh
set -e
OUT=bench/results
mkdir -p "$OUT"
rm -f "$OUT"/smooth.json "$OUT"/smooth_glass.json "$OUT"/memory.json
SIMPCKY=../target/release/simpcky.exe
D2D=target/release/simpcky-d2d.exe
SLINT=target/release/simpcky-slint.exe

python bench/smooth.py --exe $SIMPCKY --label "Simpcky (GDI)" --runs 3 --out $OUT/smooth.json
python bench/smooth.py --exe $D2D --label "Direct2D" --runs 3 --out $OUT/smooth.json
python bench/smooth.py --exe $SLINT --label "Slint (software)" --runs 3 --out $OUT/smooth.json

python bench/smooth.py --exe $D2D --label "Direct2D + vidrio" --runs 3 --glass 2 --out $OUT/smooth_glass.json
python bench/smooth.py --exe $SLINT --label "Slint + vidrio" --runs 3 --glass 2 --out $OUT/smooth_glass.json

python raven/bench/bench.py --exe $SIMPCKY --label "Simpcky (GDI)" --notes 1,5,20 --runs 3 --idle 10 --out $OUT/memory.json
python raven/bench/bench.py --exe $D2D --label "Direct2D" --notes 1,5,20 --runs 3 --idle 10 --out $OUT/memory.json
python raven/bench/bench.py --exe $SLINT --label "Slint (software)" --notes 1,5,20 --runs 3 --idle 10 --out $OUT/memory.json

echo LISTO
