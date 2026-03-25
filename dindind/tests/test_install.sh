#!/usr/bin/env bash
# test_install.sh -- Cross-platform install script test runner.
#
# Usage:
#   ./test_install.sh linux     Run tests inside a Ubuntu DinD container
#   ./test_install.sh macos     Run tests inside a Tart macOS VM
#
# Internally dispatched:
#   ./test_install.sh __run_tests   Execute test functions (called inside the target env)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." 2>/dev/null && pwd || echo "")"
INSTALL_SCRIPT="${INSTALL_SCRIPT:-${REPO_ROOT}/external/install.sh}"

GREEN='\033[0;32m'
RED='\033[0;31m'
BOLD='\033[1m'
RESET='\033[0m'

PASS=0
FAIL=0

# =========================================================================
# Shared test helpers
# =========================================================================

check() {
  local name="$1"
  shift
  printf "${BOLD}--- TEST: %s${RESET}\n" "$name"
  if "$@"; then
    printf "${GREEN}PASS${RESET}: %s\n\n" "$name"
    PASS=$((PASS + 1))
  else
    printf "${RED}FAIL${RESET}: %s\n\n" "$name"
    FAIL=$((FAIL + 1))
  fi
}

cleanup_install() {
  rm -rf "${HOME}/.coast"
  for rc in "${HOME}/.bashrc" "${HOME}/.bash_profile" "${HOME}/.zshrc"; do
    if [ -f "$rc" ]; then
      grep -v '.coast/bin' "$rc" > "${rc}.tmp" 2>/dev/null && mv "${rc}.tmp" "$rc" || true
    fi
  done
  hash -r 2>/dev/null || true
}

locate_install_script() {
  if [ -f /coast-repo/external/install.sh ]; then
    echo /coast-repo/external/install.sh
  elif [ -f "${INSTALL_SCRIPT}" ]; then
    echo "${INSTALL_SCRIPT}"
  else
    echo "error: cannot find install.sh" >&2
    return 1
  fi
}

# =========================================================================
# Test functions
# =========================================================================

test_binary_placement() {
  cleanup_install
  local script
  script="$(locate_install_script)"
  eval "$(cat "$script")" 2>&1 || true

  local ok=true
  for bin in coast coastd; do
    if [ -x "${HOME}/.coast/bin/${bin}" ]; then
      echo "${bin} found at ${HOME}/.coast/bin/${bin}"
    else
      echo "${bin} NOT found or not executable at ${HOME}/.coast/bin/${bin}"
      ok=false
    fi
  done
  $ok
}

test_path_in_rc() {
  cleanup_install
  local script
  script="$(locate_install_script)"
  eval "$(cat "$script")" 2>&1 || true

  local os
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  local expected_rc
  case "${SHELL:-/bin/bash}" in
    */zsh)  expected_rc="${HOME}/.zshrc" ;;
    */bash)
      if [ "$os" = "darwin" ]; then
        expected_rc="${HOME}/.bash_profile"
      else
        expected_rc="${HOME}/.bashrc"
      fi
      ;;
    *) expected_rc="${HOME}/.bashrc" ;;
  esac

  if grep -q '.coast/bin' "$expected_rc" 2>/dev/null; then
    echo "PATH entry found in ${expected_rc}:"
    grep '.coast/bin' "$expected_rc"
    return 0
  else
    echo "PATH entry NOT found in ${expected_rc}"
    return 1
  fi
}

test_binaries_colocated() {
  cleanup_install
  local script
  script="$(locate_install_script)"
  eval "$(cat "$script")" 2>&1 || true
  export PATH="${HOME}/.coast/bin:${PATH}"

  local coast_path coastd_path coast_dir coastd_dir
  coast_path="$(command -v coast 2>/dev/null || true)"
  coastd_path="$(command -v coastd 2>/dev/null || true)"

  if [ -z "$coast_path" ] || [ -z "$coastd_path" ]; then
    echo "Could not find both binaries on PATH"
    echo "  coast:  ${coast_path:-NOT FOUND}"
    echo "  coastd: ${coastd_path:-NOT FOUND}"
    return 1
  fi

  coast_dir="$(dirname "$(readlink -f "$coast_path" 2>/dev/null || echo "$coast_path")")"
  coastd_dir="$(dirname "$(readlink -f "$coastd_path" 2>/dev/null || echo "$coastd_path")")"

  echo "coast  at: ${coast_dir}"
  echo "coastd at: ${coastd_dir}"

  if [ "$coast_dir" = "$coastd_dir" ]; then
    echo "Both binaries are co-located"
    return 0
  else
    echo "MISMATCH: coast and coastd are in different directories"
    return 1
  fi
}

test_stale_binary_detected() {
  cleanup_install
  mkdir -p "${HOME}/.local/bin"
  echo '#!/bin/false' > "${HOME}/.local/bin/coast"
  chmod +x "${HOME}/.local/bin/coast"
  export PATH="${HOME}/.local/bin:${PATH}"

  local script
  script="$(locate_install_script)"
  local output
  output="$(eval "$(cat "$script")" 2>&1 || true)"

  rm -f "${HOME}/.local/bin/coast"

  if echo "$output" | grep -qi "stale coast binary"; then
    echo "Stale binary warning detected in output"
    return 0
  else
    echo "Stale binary warning NOT found in output"
    echo "--- output ---"
    echo "$output"
    return 1
  fi
}

test_eval_vs_pipe() {
  cleanup_install
  local script
  script="$(locate_install_script)"

  # Pipe method: PATH should NOT be in the current shell
  cat "$script" | sh 2>&1 || true

  if echo "$PATH" | grep -q '.coast/bin'; then
    echo "UNEXPECTED: .coast/bin is on PATH after pipe install"
    # Not necessarily a failure -- could be from .bashrc sourcing
  else
    echo "Confirmed: .coast/bin is NOT on PATH after pipe install (expected)"
  fi

  local pipe_has_binaries=false
  if [ -x "${HOME}/.coast/bin/coast" ] && [ -x "${HOME}/.coast/bin/coastd" ]; then
    pipe_has_binaries=true
    echo "Pipe install deposited binaries correctly"
  fi

  cleanup_install

  # Eval method: PATH SHOULD be in the current shell
  eval "$(cat "$script")" 2>&1 || true

  local eval_has_path=false
  if echo "$PATH" | grep -q '.coast/bin'; then
    eval_has_path=true
    echo "Confirmed: .coast/bin IS on PATH after eval install (expected)"
  else
    echo "UNEXPECTED: .coast/bin is NOT on PATH after eval install"
  fi

  $pipe_has_binaries && $eval_has_path
}

test_docker_warning() {
  cleanup_install
  local script
  script="$(locate_install_script)"

  # Hide docker by temporarily renaming it (requires write access)
  local docker_path
  docker_path="$(command -v docker 2>/dev/null || true)"

  if [ -z "$docker_path" ]; then
    echo "docker not found on PATH -- warning should fire by default"
    local output
    output="$(eval "$(cat "$script")" 2>&1 || true)"
  else
    sudo mv "$docker_path" "${docker_path}.__hidden" 2>/dev/null || {
      echo "Cannot rename docker binary -- skipping test (no sudo)"
      return 0
    }
    local output
    output="$(eval "$(cat "$script")" 2>&1 || true)"
    sudo mv "${docker_path}.__hidden" "$docker_path"
  fi

  if echo "$output" | grep -qi "docker is not installed"; then
    echo "Docker warning detected in output"
    return 0
  else
    echo "Docker warning NOT found in output"
    echo "--- output ---"
    echo "$output"
    return 1
  fi
}

# =========================================================================
# run_tests -- executed inside the target environment
# =========================================================================

run_tests() {
  echo "=============================="
  echo "  test_install -- $(uname -s) $(uname -m)"
  echo "=============================="
  echo ""

  check "binary placement"       test_binary_placement
  check "PATH in shell rc"       test_path_in_rc
  check "binaries co-located"    test_binaries_colocated
  check "stale binary detected"  test_stale_binary_detected
  check "eval vs pipe behavior"  test_eval_vs_pipe
  check "docker warning"         test_docker_warning

  echo ""
  echo "=============================="
  echo "  Results: ${PASS} passed, ${FAIL} failed"
  echo "=============================="

  [ "$FAIL" -eq 0 ]
}

# =========================================================================
# Target: linux (DinD container)
# =========================================================================

run_linux() {
  echo "==> Building base image..."
  docker build \
    -t coast-dindind-base \
    -f "${REPO_ROOT}/dindind/lib/base.Dockerfile" \
    "${REPO_ROOT}/dindind" 2>&1

  echo ""
  echo "==> Running install tests in Ubuntu DinD container..."
  docker run --rm \
    --privileged \
    -v "${REPO_ROOT}:/coast-repo:ro" \
    -e SHELL=/bin/bash \
    coast-dindind-base \
    bash -l -c "/coast-repo/dindind/tests/test_install.sh __run_tests"
}

# =========================================================================
# Target: macos (Tart VM)
# =========================================================================

TART_VM_NAME="coast-test-macos"
TART_IMAGE="ghcr.io/cirruslabs/macos-sequoia-base:latest"
TART_USER="admin"
TART_PASS="admin"

tart_ssh() {
  local ip="$1"
  shift
  sshpass -p "${TART_PASS}" ssh \
    -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR \
    "${TART_USER}@${ip}" "$@"
}

tart_scp() {
  local ip="$1"
  shift
  sshpass -p "${TART_PASS}" scp \
    -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null \
    -o LogLevel=ERROR \
    "$@"
}

run_macos() {
  if ! command -v tart >/dev/null 2>&1; then
    echo "error: tart is not installed. Install with: brew install cirruslabs/cli/tart" >&2
    exit 1
  fi
  if ! command -v sshpass >/dev/null 2>&1; then
    echo "error: sshpass is not installed. Install with: brew install sshpass" >&2
    exit 1
  fi

  # Clone VM if it doesn't exist
  if ! tart list | grep -q "${TART_VM_NAME}"; then
    echo "==> Cloning Tart macOS VM (this may take a while on first run)..."
    tart clone "${TART_IMAGE}" "${TART_VM_NAME}"
  fi

  echo "==> Starting Tart VM..."
  tart run "${TART_VM_NAME}" --no-graphics &
  local tart_pid=$!
  trap "echo '==> Stopping Tart VM...'; tart stop ${TART_VM_NAME} 2>/dev/null; wait ${tart_pid} 2>/dev/null" EXIT

  # Wait for VM to get an IP
  echo "==> Waiting for VM to boot..."
  local ip=""
  local elapsed=0
  while [ -z "$ip" ] && [ "$elapsed" -lt 120 ]; do
    ip="$(tart ip "${TART_VM_NAME}" 2>/dev/null || true)"
    if [ -z "$ip" ]; then
      sleep 2
      elapsed=$((elapsed + 2))
    fi
  done
  if [ -z "$ip" ]; then
    echo "error: Tart VM did not get an IP within 120s" >&2
    exit 1
  fi
  echo "==> VM IP: ${ip}"

  # Wait for SSH
  echo "==> Waiting for SSH..."
  elapsed=0
  while ! tart_ssh "$ip" "true" 2>/dev/null; do
    if [ "$elapsed" -ge 60 ]; then
      echo "error: SSH not ready within 60s" >&2
      exit 1
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  echo "==> SSH ready"

  # Install Homebrew if not present
  echo "==> Ensuring Homebrew is available in VM..."
  tart_ssh "$ip" 'command -v brew >/dev/null 2>&1 || /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" </dev/null'
  tart_ssh "$ip" 'eval "$(/opt/homebrew/bin/brew shellenv)" && echo "eval \"\$(/opt/homebrew/bin/brew shellenv)\"" >> ~/.zshrc'

  # Install socat (required by install.sh). Docker is intentionally NOT installed --
  # the docker_warning test validates that the warning fires when Docker is absent.
  echo "==> Ensuring socat is available in VM..."
  tart_ssh "$ip" 'eval "$(/opt/homebrew/bin/brew shellenv)" && command -v socat >/dev/null 2>&1 || brew install socat'

  # Copy test files into the VM
  echo "==> Copying test files to VM..."
  tart_ssh "$ip" "mkdir -p ~/coast-test"
  tart_scp "$ip" "${INSTALL_SCRIPT}" "${TART_USER}@${ip}:~/coast-test/install.sh"
  tart_scp "$ip" "${SCRIPT_DIR}/test_install.sh" "${TART_USER}@${ip}:~/coast-test/test_install.sh"

  # Run tests -- set INSTALL_SCRIPT so locate_install_script finds the copied file
  echo ""
  echo "==> Running install tests in macOS VM..."
  tart_ssh "$ip" 'eval "$(/opt/homebrew/bin/brew shellenv)" && chmod +x ~/coast-test/test_install.sh && INSTALL_SCRIPT=~/coast-test/install.sh ~/coast-test/test_install.sh __run_tests'
  local exit_code=$?

  return $exit_code
}

# =========================================================================
# Dispatch
# =========================================================================

case "${1:-}" in
  linux)
    run_linux
    ;;
  macos)
    run_macos
    ;;
  __run_tests)
    run_tests
    ;;
  *)
    echo "Usage: $0 {linux|macos}"
    echo ""
    echo "  linux   Run install tests in a Ubuntu DinD container"
    echo "  macos   Run install tests in a Tart macOS VM"
    exit 1
    ;;
esac
