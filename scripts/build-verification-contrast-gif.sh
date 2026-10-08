#!/usr/bin/env bash
# Build the unverified-vs-verified contrast GIF from
# `genegis demo frames-contrast` output.
#
# Usage:
#   scripts/build-verification-contrast-gif.sh [frames_dir] [out_gif]

set -euo pipefail

FRAMES_DIR="${1:-.genegis/frames-contrast}"
OUT="${2:-docs/assets/verification-contrast.gif}"

for index in $(seq -w 0 36); do
  frame="$FRAMES_DIR/contrast-$index.png"
  test -s "$frame" || {
    echo "missing frame: $frame (run: cargo run -p genegis-cli -- demo frames-contrast $FRAMES_DIR)" >&2
    exit 1
  }
done

mkdir -p "$(dirname "$OUT")"

# Hold the verified state before the loop restarts.
ffmpeg -hide_banner -loglevel error -y \
  -framerate 6 -i "$FRAMES_DIR/contrast-%02d.png" \
  -vf "tpad=stop_mode=clone:stop_duration=1.5,scale=960:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
  -loop 0 "$OUT"

echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
