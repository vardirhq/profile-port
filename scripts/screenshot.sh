#!/usr/bin/env bash
# Render the desktop UI headlessly (Linux + Xvfb, ImageMagick, xdotool) and save PNG screenshots.
# Usage: scripts/screenshot.sh [scale]      (default scale 2 = HiDPI)
set -euo pipefail
cd "$(dirname "$0")/.."
SCALE="${1:-2}"
W=1200; H=760
OUT=docs/screenshots
mkdir -p "$OUT"
cargo build -q -p profileport-desktop
BIN=target/debug/ProfilePort
PW=$(python3 -c "print(int($W*$SCALE))"); PH=$(python3 -c "print(int($H*$SCALE))")

for theme in light dark; do
  xvfb-run -a -s "-screen 0 ${PW}x${PH}x24" bash -c "
    PROFILEPORT_THEME=$theme SLINT_SCALE_FACTOR=$SCALE SLINT_BACKEND=winit-femtovg $BIN &
    pid=\$!
    sleep 3
    xdotool mousemove 1 $((PH - 1)) 2>/dev/null || true
    sleep 1
    import -window root '$OUT/dashboard-$theme.png'
    kill \$pid
  "
  echo "wrote $OUT/dashboard-$theme.png"
done
