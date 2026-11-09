#!/usr/bin/env bash
# Builds the WASM bundle into viz/web/pkg. Requires the wasm32 target and a
# wasm-bindgen-cli matching the wasm-bindgen version in Cargo.lock.
#
# Also copies extracted chip layouts (chip/<NAME>/<run>/layout.bin, from
# `chip build` + `chip extract`) to viz/web/layouts/<NAME>.bin for the 3D view:
# the basic-met5 run if present, else the newest run with a layout.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p logic_circuit_viz --lib --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir viz/web/pkg \
  target/wasm32-unknown-unknown/release/logic_circuit_viz.wasm

rm -rf viz/web/layouts
mkdir -p viz/web/layouts
names=()
for design in chip/*/; do
  name=$(basename "$design")
  [ "$name" = flow ] || [ "$name" = smoke ] && continue
  layout="$design/basic-met5/layout.bin"
  [ -f "$layout" ] || layout=$(ls -t "$design"/*/layout.bin 2>/dev/null | head -1)
  [ -n "$layout" ] && [ -f "$layout" ] || continue
  cp "$layout" "viz/web/layouts/$name.bin"
  names+=("\"$name\"")
done
(IFS=,; echo "[${names[*]}]") > viz/web/layouts/index.json
echo "Layouts: $(cat viz/web/layouts/index.json)"
