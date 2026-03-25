#!/usr/bin/env bash
# WSL Ubuntu install test -- reproduces issue #130.
# Runs inside the DinDinD container as testuser.
#
# Controlled by DINDIND_DEPTH env var:
#   install  -- test the install script only
#   daemon   -- install + coast daemon install
#   run      -- install + daemon + coast run (future)
#   e2e      -- full stack (future)
set -euo pipefail

DEPTH="${DINDIND_DEPTH:-install}"
PASS=0
FAIL=0

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

source /test-scripts/env.sh

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
  # Remove PATH line from shell rc files
  for rc in "${HOME}/.bashrc" "${HOME}/.bash_profile" "${HOME}/.zshrc"; do
    if [ -f "$rc" ]; then
      grep -v '.coast/bin' "$rc" > "${rc}.tmp" 2>/dev/null && mv "${rc}.tmp" "$rc" || true
    fi
  done
  hash -r 2>/dev/null || true
}

# =========================================================================
# Depth: install
# =========================================================================

test_eval_install() {
  cleanup_install
  echo "Running: eval \"\$(curl -fsSL https://coasts.dev/install)\""
  eval "$(curl -fsSL https://coasts.dev/install)" 2>&1 || true

  if command -v coast >/dev/null 2>&1; then
    echo "coast found at: $(command -v coast)"
    echo "coastd location: $(dirname "$(command -v coast)")/coastd"
    [ -x "$(dirname "$(command -v coast)")/coastd" ]
  else
    echo "coast not found on PATH after eval install"
    echo "PATH=${PATH}"
    ls -la "${HOME}/.coast/bin/" 2>/dev/null || echo "~/.coast/bin/ does not exist"
    return 1
  fi
}

test_pipe_install() {
  cleanup_install
  echo "Running: curl -fsSL https://coasts.dev/install | sh"
  curl -fsSL https://coasts.dev/install | sh 2>&1 || true

  echo ""
  echo "Checking if coast is on PATH in current shell..."
  if command -v coast >/dev/null 2>&1; then
    echo "coast found at: $(command -v coast) (unexpected -- pipe install usually loses PATH)"
  else
    echo "coast NOT on PATH (expected with pipe install)"
  fi

  echo ""
  echo "Checking if binaries exist at expected location..."
  if [ -x "${HOME}/.coast/bin/coast" ] && [ -x "${HOME}/.coast/bin/coastd" ]; then
    echo "Binaries present at ~/.coast/bin/ -- install succeeded, PATH just didn't stick"
    echo "  coast:  $(ls -la "${HOME}/.coast/bin/coast")"
    echo "  coastd: $(ls -la "${HOME}/.coast/bin/coastd")"
    return 0
  else
    echo "Binaries NOT found at ~/.coast/bin/"
    ls -la "${HOME}/.coast/bin/" 2>/dev/null || echo "~/.coast/bin/ does not exist"
    return 1
  fi
}

test_path_in_bashrc() {
  cleanup_install
  eval "$(curl -fsSL https://coasts.dev/install)" 2>&1 || true

  if grep -q '.coast/bin' "${HOME}/.bashrc" 2>/dev/null; then
    echo "PATH entry found in ~/.bashrc"
    grep '.coast/bin' "${HOME}/.bashrc"
    return 0
  else
    echo "PATH entry NOT found in ~/.bashrc"
    echo "Contents of ~/.bashrc:"
    cat "${HOME}/.bashrc" 2>/dev/null || echo "(file does not exist)"
    return 1
  fi
}

test_binary_paths_match() {
  cleanup_install
  eval "$(curl -fsSL https://coasts.dev/install)" 2>&1 || true
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

  coast_dir="$(dirname "$(readlink -f "$coast_path")")"
  coastd_dir="$(dirname "$(readlink -f "$coastd_path")")"

  echo "coast  resolved to: ${coast_dir}/coast"
  echo "coastd resolved to: ${coastd_dir}/coastd"

  if [ "$coast_dir" = "$coastd_dir" ]; then
    echo "Both binaries are in the same directory"
    return 0
  else
    echo "MISMATCH: coast and coastd are in different directories"
    return 1
  fi
}

check "eval install puts coast on PATH"        test_eval_install
check "pipe install deposits binaries"          test_pipe_install
check "install adds PATH to .bashrc"            test_path_in_bashrc
check "coast and coastd are co-located"         test_binary_paths_match

# =========================================================================
# Depth: daemon
# =========================================================================

if [ "$DEPTH" = "daemon" ] || [ "$DEPTH" = "run" ] || [ "$DEPTH" = "e2e" ]; then

  test_daemon_install() {
    cleanup_install
    eval "$(curl -fsSL https://coasts.dev/install)" 2>&1 || true
    export PATH="${HOME}/.coast/bin:${PATH}"

    echo "Running: coast daemon install"
    if coast daemon install 2>&1; then
      echo "coast daemon install succeeded"
    else
      local ec=$?
      echo "coast daemon install failed (exit ${ec})"
      echo ""
      echo "Checking systemd availability..."
      if command -v systemctl >/dev/null 2>&1; then
        echo "systemctl is available"
        systemctl --user status coastd 2>&1 || true
      else
        echo "systemctl NOT available (common in WSL without systemd enabled)"
      fi
      return 1
    fi
  }

  test_systemd_unit_path() {
    local unit_path="${HOME}/.config/systemd/user/coastd.service"
    if [ ! -f "$unit_path" ]; then
      echo "systemd unit not found at ${unit_path}"
      return 1
    fi

    echo "systemd unit contents:"
    cat "$unit_path"
    echo ""

    local exec_start
    exec_start="$(grep '^ExecStart=' "$unit_path" | cut -d= -f2 | awk '{print $1}')"
    echo "ExecStart binary: ${exec_start}"

    if [ -x "$exec_start" ]; then
      echo "Binary at ExecStart path exists and is executable"
      return 0
    else
      echo "Binary at ExecStart path does NOT exist or is not executable"
      ls -la "$exec_start" 2>/dev/null || echo "(file not found)"
      return 1
    fi
  }

  check "coast daemon install"                  test_daemon_install
  check "systemd unit points to valid binary"   test_systemd_unit_path
fi

# =========================================================================
# Depth: run (future)
# =========================================================================

if [ "$DEPTH" = "run" ] || [ "$DEPTH" = "e2e" ]; then
  echo "${YELLOW}==> Depth 'run' tests not yet implemented${RESET}"
fi

# =========================================================================
# Depth: e2e (future)
# =========================================================================

if [ "$DEPTH" = "e2e" ]; then
  echo "${YELLOW}==> Depth 'e2e' tests not yet implemented${RESET}"
fi

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "=============================="
echo "  Scenario: wsl-ubuntu"
echo "  Depth:    ${DEPTH}"
echo "=============================="
echo "  Pass: ${PASS}"
echo "  Fail: ${FAIL}"
echo ""

[ "$FAIL" -eq 0 ]
