#!/usr/bin/env bash
# Build a README showcase GIF from `genegis demo frames` output.
#
# Usage:
#   scripts/build-readme-showcase.sh [frames_dir] [out_gif] [order_csv]
#
# Defaults assemble all seven synthetic-fixture frames. Pass a comma
# separated frame list to build a subset, e.g. the real-data set:
#   scripts/build-readme-showcase.sh .genegis/frames-real \
#     docs/assets/usecase-showcase-real.gif density,flood,evacuation,xmin-city

set -euo pipefail

FRAMES_DIR="${1:-.genegis/frames}"
OUT="${2:-docs/assets/usecase-showcase.gif}"
ORDER_CSV="${3:-density,flood,evacuation,xmin-city,ndvi,uc5-epoch-a,uc5-epoch-b}"

IFS=',' read -r -a ORDER <<< "$ORDER_CSV"
SEQ="$(mktemp -d)"
trap 'rm -rf "$SEQ"' EXIT

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
