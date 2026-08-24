#!/usr/bin/env bash
# Build docs/assets/usecase-showcase.gif from the RFC 0005 showcase frames.
#
# Frames are rendered deterministically by `genegis demo frames`, then
# assembled with ffmpeg's two-pass palette for crisp flat-color maps.
# Usage: scripts/build-readme-showcase.sh [frames_dir] [out_gif]

set -euo pipefail

FRAMES_DIR="${1:-.genegis/frames}"
OUT="${2:-docs/assets/usecase-showcase.gif}"
SEQ="$(mktemp -d)"
trap 'rm -rf "$SEQ"' EXIT

ORDER=(
  density
  flood
  evacuation
  xmin-city
  ndvi
  uc5-epoch-a
  uc5-epoch-b
)

for index in "${!ORDER[@]}"; do
  name="${ORDER[$index]}"
  src="$FRAMES_DIR/$name.png"
  test -s "$src" || { echo "missing frame: $src (run: cargo run -p genegis-cli -- demo frames $FRAMES_DIR)" >&2; exit 1; }
  cp "$src" "$SEQ/$(printf '%02d' "$((index + 1))").png"
done

ffmpeg -hide_banner -loglevel error -y \
  -framerate 1 -i "$SEQ/%02d.png" \
  -vf "scale=960:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=160:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  -loop 0 "$OUT"

echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
