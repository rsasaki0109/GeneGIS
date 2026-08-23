#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ASSETS="$ROOT/docs/assets"
CHROME="${CHROME:-google-chrome}"
FRAME_DIR="$(mktemp -d)"

cleanup() {
  rm -rf -- "$FRAME_DIR"
}
trap cleanup EXIT

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

echo "Capturing rich Intent → Workflow → Execute → Verify → Trust frames…"
states=(intent plan execute verify trust)
for index in "${!states[@]}"; do
  frame=$((index + 1))
  capture "$ASSETS/hero.html?state=${states[$index]}" "$FRAME_DIR/hero-frame-$frame.png"
done
cp "$FRAME_DIR/hero-frame-5.png" "$ASSETS/workbench-hero.png"

if command -v ffmpeg >/dev/null 2>&1; then
  echo "Building workbench hero GIF…"
  ffmpeg -y -loglevel error \
    -loop 1 -t 1.4 -i "$FRAME_DIR/hero-frame-1.png" \
    -loop 1 -t 1.8 -i "$FRAME_DIR/hero-frame-2.png" \
    -loop 1 -t 1.8 -i "$FRAME_DIR/hero-frame-3.png" \
    -loop 1 -t 2.4 -i "$FRAME_DIR/hero-frame-4.png" \
    -loop 1 -t 3.0 -i "$FRAME_DIR/hero-frame-5.png" \
    -filter_complex "[0:v][1:v][2:v][3:v][4:v]concat=n=5:v=1:a=0,scale=1280:-1:flags=lanczos,fps=12,split[s0][s1];[s0]palettegen=stats_mode=diff:max_colors=192[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3" \
    -loop 0 \
    "$ASSETS/workbench-hero.gif"
else
  echo "ffmpeg not found; skipping GIF generation"
fi

echo "Done:"
ls -lh "$ASSETS/workbench-hero.png" "$ASSETS/workbench-hero.gif" 2>/dev/null || ls -lh "$ASSETS/workbench-hero.png"
