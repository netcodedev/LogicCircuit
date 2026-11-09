#!/usr/bin/env bash
# Builds the WASM bundle into viz/web/pkg. Requires the wasm32 target and a
# wasm-bindgen-cli matching the wasm-bindgen version in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p logic_circuit_viz --lib --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir viz/web/pkg \
  target/wasm32-unknown-unknown/release/logic_circuit_viz.wasm
