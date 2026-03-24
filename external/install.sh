#!/bin/sh
# Coast installer — https://coasts.dev/install
# Usage: eval "$(curl -fsSL https://coasts.dev/install)"

(
set -eu
unalias -a 2>/dev/null || true

REPO="coast-guard/coasts"
INSTALL_DIR="${HOME}/.coast/bin"

# Colors (only when attached to a terminal)
if [ -t 1 ]; then
  RED='\033[0;31m'
  GREEN='\033[0;32m'
  YELLOW='\033[0;33m'
  BOLD='\033[1m'
  RESET='\033[0m'
else
  RED=''
  GREEN=''
  YELLOW=''
  BOLD=''
  RESET=''
fi

info()  { printf "${BOLD}==>${RESET} %s\n" "$1"; }
warn()  { printf "${YELLOW}warning:${RESET} %s\n" "$1"; }
error() { printf "${RED}error:${RESET} %s\n" "$1" >&2; exit 1; }

# --- Detect OS ---
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
case "$OS" in
  darwin) ;;
  linux)  ;;
  *) error "Unsupported operating system: $OS (expected darwin or linux)" ;;
esac

# --- Detect architecture ---
ARCH=$(uname -m)
case "$ARCH" in
  arm64|aarch64) ARCH="arm64" ;;
  x86_64|amd64)  ARCH="amd64" ;;
  *) error "Unsupported architecture: $ARCH (expected arm64/aarch64 or amd64/x86_64)" ;;
esac

PLATFORM="${OS}-${ARCH}"
info "Detected platform: ${PLATFORM}"

# --- Find latest release with downloadable assets ---
TMPDIR=$(mktemp -d)
trap 'rm -rf "${TMPDIR}"' EXIT

info "Fetching latest release..."
if ! curl -fsSL "https://api.github.com/repos/${REPO}/releases?per_page=5" > "${TMPDIR}/releases.json" 2>/dev/null; then
  error "Failed to fetch release info from GitHub. Check your internet connection."
fi

VERSION=""
URL=""
TARBALL=""
for TAG in $(grep '"tag_name"' "${TMPDIR}/releases.json" | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'); do
  CANDIDATE_TARBALL="coast-${TAG}-${PLATFORM}.tar.gz"
  CANDIDATE_URL="https://github.com/${REPO}/releases/download/${TAG}/${CANDIDATE_TARBALL}"
  if curl -fsSL --head "${CANDIDATE_URL}" >/dev/null 2>&1; then
    VERSION="${TAG}"
    URL="${CANDIDATE_URL}"
    TARBALL="${CANDIDATE_TARBALL}"
    break
  fi
done

if [ -z "${VERSION}" ]; then
  error "No downloadable release found. A new release may still be building — try again in a few minutes."
fi

info "Latest version: ${VERSION}"

# --- Download ---
info "Downloading ${TARBALL}..."
if ! curl -fsSL "${URL}" -o "${TMPDIR}/${TARBALL}"; then
  error "Download failed — no release artifact for ${PLATFORM} at ${VERSION}"
fi

# --- Install ---
mkdir -p "${INSTALL_DIR}"
info "Installing to ${INSTALL_DIR}..."
tar xzf "${TMPDIR}/${TARBALL}" -C "${INSTALL_DIR}" coast coastd
chmod 755 "${INSTALL_DIR}/coast" "${INSTALL_DIR}/coastd"

# --- Add to PATH ---
SHELL_RC_FILE=""
add_to_path() {
  SHELL_RC="$1"
  PATH_LINE='export PATH="$HOME/.coast/bin:$PATH"'

  if [ -f "${SHELL_RC}" ] && grep -qF '.coast/bin' "${SHELL_RC}"; then
    SHELL_RC_FILE="${SHELL_RC}"
    return
  fi

  printf "\n# Coast CLI\n%s\n" "${PATH_LINE}" >> "${SHELL_RC}"
  SHELL_RC_FILE="${SHELL_RC}"
  info "Added ${INSTALL_DIR} to PATH in ${SHELL_RC}"
}

if [ -n "${SHELL:-}" ]; then
  case "${SHELL}" in
    */zsh)  add_to_path "${HOME}/.zshrc" ;;
    */bash)
      if [ "$OS" = "darwin" ]; then
        add_to_path "${HOME}/.bash_profile"
      else
        add_to_path "${HOME}/.bashrc"
      fi
      ;;
    */fish)
      FISH_CONFIG="${HOME}/.config/fish/config.fish"
      if [ ! -f "${FISH_CONFIG}" ] || ! grep -qF '.coast/bin' "${FISH_CONFIG}"; then
        mkdir -p "$(dirname "${FISH_CONFIG}")"
        printf "\n# Coast CLI\nset -gx PATH $HOME/.coast/bin $PATH\n" >> "${FISH_CONFIG}"
        info "Added ${INSTALL_DIR} to PATH in ${FISH_CONFIG}"
      fi
      SHELL_RC_FILE="${FISH_CONFIG}"
      ;;
  esac
fi

# Make available for the verify step below
export PATH="${INSTALL_DIR}:${PATH}"

# --- Install socat dependency ---
if ! command -v socat >/dev/null 2>&1; then
  info "Installing required dependency: socat..."
  case "$OS" in
    darwin)
      if command -v brew >/dev/null 2>&1; then
        brew install socat
      else
        warn "socat is required but Homebrew is not installed"
        printf "  Install Homebrew (https://brew.sh) then run: ${BOLD}brew install socat${RESET}\n"
      fi
      ;;
    linux)
      if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get update -qq && sudo apt-get install -y -qq socat
      elif command -v dnf >/dev/null 2>&1; then
        sudo dnf install -y socat
      elif command -v yum >/dev/null 2>&1; then
        sudo yum install -y socat
      elif command -v pacman >/dev/null 2>&1; then
        sudo pacman -S --noconfirm socat
      elif command -v apk >/dev/null 2>&1; then
        sudo apk add socat
      else
        warn "socat is required but could not detect a package manager"
        printf "  Please install socat manually, then run: ${BOLD}coast help${RESET}\n"
      fi
      ;;
  esac

  if command -v socat >/dev/null 2>&1; then
    info "socat installed"
  else
    warn "socat could not be installed automatically"
    printf "  Coast requires socat to function. Please install it manually.\n"
  fi
fi

# --- Verify ---
if command -v coast >/dev/null 2>&1; then
  INSTALLED_VERSION=$(coast --version 2>/dev/null | head -1 | sed 's/coast /v/' || echo "unknown")
  printf "\n${GREEN}Done!${RESET} Coast installed successfully: ${BOLD}${INSTALLED_VERSION}${RESET}\n"
  printf "  Run ${BOLD}coast help${RESET} to get started\n\n"
else
  warn "coast was installed to ${INSTALL_DIR} but is not on your PATH"
  printf "  Restart your terminal, then run: ${BOLD}coast help${RESET}\n\n"
fi
) >&2

# Make coast available in the current shell (works when invoked via eval)
export PATH="${HOME}/.coast/bin:${PATH}"
