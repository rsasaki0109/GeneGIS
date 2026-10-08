#!/usr/bin/env bash
# Build GeneGIS for the browser into public/try/pkg (served at /try/).
#
# Requires: rustup target add wasm32-unknown-unknown
#           cargo install wasm-bindgen-cli --version 0.2.123 --locked
#
# Usage: scripts/build-wasm.sh [out_dir]

set -euo pipefail

OUT="${1:-public/try/pkg}"
cargo build --locked --release -p genegis-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  target/wasm32-unknown-unknown/release/genegis_wasm.wasm
echo "wrote $OUT ($(du -sh "$OUT" | cut -f1))"
