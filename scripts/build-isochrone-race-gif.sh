#!/usr/bin/env bash
# Build the walk vs walk+rail reach-race GIF from
# `genegis demo frames-isochrone` output.
#
# Usage:
#   scripts/build-isochrone-race-gif.sh [frames_dir] [out_gif]

set -euo pipefail

FRAMES_DIR="${1:-.genegis/frames-isochrone}"
OUT="${2:-docs/assets/isochrone-race.gif}"

for index in $(seq -w 0 30); do
  frame="$FRAMES_DIR/isochrone-race-$index.png"
  test -s "$frame" || {
    echo "missing frame: $frame (run: cargo run -p genegis-cli -- demo frames-isochrone $FRAMES_DIR)" >&2
    exit 1
  }
done

mkdir -p "$(dirname "$OUT")"

# One frame per 2 simulated minutes; hold the 60-minute state before looping.
ffmpeg -hide_banner -loglevel error -y \
  -framerate 6 -i "$FRAMES_DIR/isochrone-race-%02d.png" \
  -vf "tpad=stop_mode=clone:stop_duration=2,scale=960:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=160:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  -loop 0 "$OUT"

echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
