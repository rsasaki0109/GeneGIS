#!/usr/bin/env bash
# Build the Phase 14 M0 district orbit GIF from `genegis demo frames-3d` output.
#
# Usage:
#   scripts/build-district3d-gif.sh [frames_dir] [out_gif]

set -euo pipefail

FRAMES_DIR="${1:-.genegis/frames-3d}"
OUT="${2:-docs/assets/district3d.gif}"

for index in $(seq -w 0 17); do
  frame="$FRAMES_DIR/district3d-$index.png"
  test -s "$frame" || {
    echo "missing frame: $frame (run: cargo run -p genegis-cli -- demo frames-3d $FRAMES_DIR)" >&2
    exit 1
  }
done

mkdir -p "$(dirname "$OUT")"

ffmpeg -hide_banner -loglevel error -y \
  -framerate 9 -i "$FRAMES_DIR/district3d-%02d.png" \
  -vf "scale=800:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=160:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  -loop 0 "$OUT"

echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
