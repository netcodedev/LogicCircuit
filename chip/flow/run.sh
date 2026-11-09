#!/usr/bin/env bash
# Runs pnr.tcl inside the pinned ORFS image. Usage:
#   chip/flow/run.sh NETLIST TOP OUT_DIR [VAR=value ...]
# e.g. chip/flow/run.sh chip/smoke/smoke.v smoke chip/smoke/out CLOCK_PORT=CLK DIE_SIZE=60
# (Interim helper until the Rust CLI drives the flow in Phase 2.)
set -euo pipefail
IMAGE=openroad/orfs:26Q4-374-g2ccdcd3c2
ORFS=/OpenROAD-flow-scripts

cd "$(dirname "$0")/../.."
netlist=$1 top=$2 out=$3
shift 3
envs=()
for kv in "$@"; do envs+=(-e "$kv"); done

docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
    -v "$PWD:/work" -w /work \
    -e PLATFORM_DIR=$ORFS/flow/platforms/sky130hd \
    -e NETLIST="$netlist" -e TOP="$top" -e OUT_DIR="$out" \
    "${envs[@]}" \
    "$IMAGE" $ORFS/tools/install/OpenROAD/bin/openroad -no_init -exit chip/flow/pnr.tcl
