#!/usr/bin/env bash
# Test Coast install from locally-built binaries.
# Expects the Coast repo mounted read-only at /coast-repo with
# pre-built release binaries in target/release/.
#
# Usage (inside a DinDinD container):
#   /test-scripts/test-local-build.sh
set -euo pipefail

COAST_REPO="${COAST_REPO:-/coast-repo}"
INSTALL_DIR="${HOME}/.coast/bin"

GREEN='\033[0;32m'
RED='\033[0;31m'
BOLD='\033[1m'
RESET='\033[0m'

die() { printf "${RED}error:${RESET} %s\n" "$1" >&2; exit 1; }

for bin in coast coastd; do
  src="${COAST_REPO}/target/release/${bin}"
  [ -f "$src" ] || die "Binary not found: ${src} -- build the project first (cargo build --release)"
done

echo "==> Installing local binaries to ${INSTALL_DIR}..."
mkdir -p "$INSTALL_DIR"
cp "${COAST_REPO}/target/release/coast"  "${INSTALL_DIR}/coast"
cp "${COAST_REPO}/target/release/coastd" "${INSTALL_DIR}/coastd"
chmod 755 "${INSTALL_DIR}/coast" "${INSTALL_DIR}/coastd"

export PATH="${INSTALL_DIR}:${PATH}"

echo "==> Verifying..."
echo "  coast:  $(command -v coast)"
echo "  coastd: $(command -v coastd)"
coast --version 2>/dev/null || true

echo ""
echo "==> Testing coast daemon install..."
if coast daemon install 2>&1; then
  printf "${GREEN}coast daemon install succeeded${RESET}\n"
else
  printf "${RED}coast daemon install failed${RESET}\n"
fi

echo ""
echo "==> Checking binary path resolution..."
coast_dir="$(dirname "$(readlink -f "$(command -v coast)")")"
coastd_dir="$(dirname "$(readlink -f "$(command -v coastd)")")"
echo "  coast dir:  ${coast_dir}"
echo "  coastd dir: ${coastd_dir}"

if [ "$coast_dir" = "$coastd_dir" ]; then
  printf "${GREEN}Binaries co-located${RESET}\n"
else
  printf "${RED}Binaries in different directories${RESET}\n"
  exit 1
fi
