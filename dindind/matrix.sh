#!/usr/bin/env bash
# Matrix definition and runner for DinDinD integration tests.
#
# Scenarios define the platform/environment.
# Depths define how far into the Docker nesting we test:
#   install  -- just the install script (no inner Docker needed)
#   daemon   -- install + coast daemon install (needs inner Docker)
#   run      -- install + daemon + coast run (DinDinD)
#   e2e      -- full compose stack inside Coast containers (DinDinDinD, future)
#
# Usage (sourced by run.sh):
#   source matrix.sh
#   matrix_run [scenario] [depth]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# Matrix axes -- extend these arrays to add new scenarios or depth levels
# ---------------------------------------------------------------------------
SCENARIOS=(wsl-ubuntu)
DEPTHS=(install daemon)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

IMAGE_PREFIX="coast-dindind"

scenario_dir() {
  echo "${SCRIPT_DIR}/scenarios/$1"
}

image_tag() {
  echo "${IMAGE_PREFIX}-$1"
}

build_scenario() {
  local scenario="$1"
  local tag
  tag="$(image_tag "$scenario")"
  local sdir
  sdir="$(scenario_dir "$scenario")"

  if [ ! -f "${sdir}/Dockerfile" ]; then
    echo "error: no Dockerfile for scenario '${scenario}'" >&2
    return 1
  fi

  echo "==> Building image ${tag} for scenario ${scenario}..."
  docker build \
    -t "$tag" \
    -f "${sdir}/Dockerfile" \
    "$SCRIPT_DIR"
}

run_scenario() {
  local scenario="$1"
  local depth="$2"
  local tag
  tag="$(image_tag "$scenario")"
  local sdir
  sdir="$(scenario_dir "$scenario")"

  local test_script="${sdir}/test-install.sh"
  if [ ! -f "$test_script" ]; then
    echo "error: missing ${test_script}" >&2
    return 1
  fi

  local container_name="${IMAGE_PREFIX}-${scenario}-${depth}-$$"

  echo "==> Running scenario=${scenario} depth=${depth} ..."
  docker run --rm \
    --privileged \
    --name "$container_name" \
    -e "DINDIND_DEPTH=${depth}" \
    -v "${SCRIPT_DIR}/..:/coast-repo:ro" \
    "$tag" \
    bash -l -c "/test-scripts/test-install.sh"
}

run_interactive() {
  local scenario="$1"
  local tag
  tag="$(image_tag "$scenario")"

  echo "==> Interactive shell for scenario=${scenario}"
  docker run --rm -it \
    --privileged \
    -v "${SCRIPT_DIR}/..:/coast-repo:ro" \
    "$tag"
}

# ---------------------------------------------------------------------------
# Matrix runner
# ---------------------------------------------------------------------------

matrix_run() {
  local filter_scenario="${1:-}"
  local filter_depth="${2:-}"

  local pass=0
  local fail=0
  local skip=0
  local results=()

  for scenario in "${SCENARIOS[@]}"; do
    if [ -n "$filter_scenario" ] && [ "$filter_scenario" != "$scenario" ]; then
      continue
    fi

    build_scenario "$scenario"

    for depth in "${DEPTHS[@]}"; do
      if [ -n "$filter_depth" ] && [ "$filter_depth" != "$depth" ]; then
        continue
      fi

      if run_scenario "$scenario" "$depth"; then
        results+=("PASS  ${scenario}  ${depth}")
        pass=$((pass + 1))
      else
        results+=("FAIL  ${scenario}  ${depth}")
        fail=$((fail + 1))
      fi
    done
  done

  echo ""
  echo "=============================="
  echo "  DinDinD Matrix Results"
  echo "=============================="
  printf "%-6s %-20s %s\n" "RESULT" "SCENARIO" "DEPTH"
  printf "%-6s %-20s %s\n" "------" "--------" "-----"
  for r in "${results[@]}"; do
    echo "$r"
  done
  echo ""
  echo "Pass: ${pass}  Fail: ${fail}  Skip: ${skip}"

  [ "$fail" -eq 0 ]
}
