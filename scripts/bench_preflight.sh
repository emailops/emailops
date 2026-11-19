#!/usr/bin/env bash
#
# Refuse a speed benchmark on a machine that would turn its numbers into noise:
#   - on battery or in Low Power Mode, where clocks drop and runs come out
#     8-10% slower;
#   - with an EmailOps app running (the installed app or `make dev`). It shares
#     the GPU, and from outside there is no telling whether it is idle or
#     classifying, embedding or answering in the background.
#
# Exit 0 = fine to measure, 1 = refused (each reason on stderr). Called by the
# speed benches (cli_bench.sh, model_bench.sh, bench_ab.py) before they start.
set -uo pipefail

problems=()
if command -v pmset > /dev/null; then
  pmset -g batt | grep -q "'AC Power'" || problems+=("on battery power: plug in the charger")
  pmset -g | grep -Eq '^[[:space:]]*lowpowermode[[:space:]]+1' \
    && problems+=("Low Power Mode is on: turn it off in System Settings > Battery")
fi
if pids=$(pgrep -x emailops); then
  problems+=("EmailOps is running (pid $(echo $pids)): quit the app and any 'make dev' first")
fi

for p in "${problems[@]+"${problems[@]}"}"; do
  echo "[bench-preflight] $p" >&2
done
[ ${#problems[@]} -eq 0 ]
