#!/usr/bin/env bash
# Top-level runner for DinDinD integration tests.
#
# Usage:
#   ./run.sh                        Run full matrix
#   ./run.sh wsl-ubuntu             Run single scenario, all depths
#   ./run.sh wsl-ubuntu install     Run single scenario at specific depth
#   ./run.sh wsl-ubuntu shell       Interactive shell in the scenario container
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/matrix.sh"

build_base() {
  echo "==> Building shared base image..."
  docker build \
    -t coast-dindind-base \
    -f "${SCRIPT_DIR}/lib/base.Dockerfile" \
    "$SCRIPT_DIR"
}

SCENARIO="${1:-}"
DEPTH="${2:-}"

build_base

if [ "$DEPTH" = "shell" ]; then
  build_scenario "$SCENARIO"
  run_interactive "$SCENARIO"
elif [ -n "$SCENARIO" ]; then
  matrix_run "$SCENARIO" "$DEPTH"
else
  matrix_run
fi
