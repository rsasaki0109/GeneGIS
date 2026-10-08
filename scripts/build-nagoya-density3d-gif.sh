#!/usr/bin/env bash
# Build the verified 3D population-density GIF from
# `genegis demo frames-nagoya3d` output.
#
# Usage:
#   scripts/build-nagoya-density3d-gif.sh [frames_dir] [out_gif]
#
# Real data: export GENEGIS_POPULATION_MESH_PATH / GENEGIS_POPULATION_MESH_SHA
# before rendering the frames; the render refuses when any check fails.

set -euo pipefail

FRAMES_DIR="${1:-.genegis/frames-nagoya3d}"
OUT="${2:-docs/assets/nagoya-density3d.gif}"

for index in $(seq -w 0 41); do
  frame="$FRAMES_DIR/nagoya-density3d-$index.png"
  test -s "$frame" || {
    echo "missing frame: $frame (run: cargo run -p genegis-cli -- demo frames-nagoya3d $FRAMES_DIR)" >&2
    exit 1
  }
done

mkdir -p "$(dirname "$OUT")"

# Hold the last frame so the verified state reads before the loop restarts.
ffmpeg -hide_banner -loglevel error -y \
  -framerate 12 -i "$FRAMES_DIR/nagoya-density3d-%02d.png" \
  -vf "tpad=stop_mode=clone:stop_duration=1.2,scale=960:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=192:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
  -loop 0 "$OUT"

echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
