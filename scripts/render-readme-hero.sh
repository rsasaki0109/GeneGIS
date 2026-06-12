#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ASSETS="$ROOT/docs/assets"
CHROME="${CHROME:-google-chrome}"

mkdir -p "$ASSETS"

echo "Generating Nagoya density map PNG…"
cargo run -q -p genegis-cli -- ask "名古屋市の人口密度を表示" --png -o "$ASSETS/nagoya-density.png" --no-html >/dev/null

capture() {
  local html="$1"
  local png="$2"
  "$CHROME" \
    --headless=new \
    --disable-gpu \
    --hide-scrollbars \
    --window-size=1440,900 \
    --screenshot="$png" \
    "file://$html"
}

echo "Capturing workbench hero PNG…"
capture "$ASSETS/hero-done.html" "$ASSETS/workbench-hero.png"

echo "Capturing animation frames…"
capture "$ASSETS/hero-running.html" "$ASSETS/hero-frame-1.png"
capture "$ASSETS/hero-done.html" "$ASSETS/hero-frame-2.png"

if command -v ffmpeg >/dev/null 2>&1; then
  echo "Building workbench hero GIF…"
  ffmpeg -y -loglevel error \
    -framerate 1 \
    -loop 1 -t 1.8 -i "$ASSETS/hero-frame-1.png" \
    -loop 1 -t 2.8 -i "$ASSETS/hero-frame-2.png" \
    -filter_complex "[0:v][1:v]concat=n=2:v=1:a=0,scale=1280:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3" \
    "$ASSETS/workbench-hero.gif"
else
  echo "ffmpeg not found; skipping GIF generation"
fi

echo "Done:"
ls -lh "$ASSETS/workbench-hero.png" "$ASSETS/workbench-hero.gif" 2>/dev/null || ls -lh "$ASSETS/workbench-hero.png"
