#!/usr/bin/env bash
# GitPilot Universal Installer
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/temesgensida-code/gitpilot/main/install.sh | bash

set -e

# Repository configuration
REPO_OWNER="${REPO_OWNER:-temesgensida-code}"
REPO_NAME="${REPO_NAME:-gitpilot}"
BINARY_NAME="gitpilot"
ALT_BINARY_NAME="Gitpilot"

# Colors for UI
RESET="\033[0m"
BOLD="\033[1m"
GREEN="\033[38;2;200;255;0m" # GitPilot accent lemon green
CYAN="\033[36m"
RED="\033[31m"
GRAY="\033[90m"

log_info() {
    printf "${GREEN}==>${RESET} ${BOLD}%b${RESET}\n" "$1"
}

log_step() {
    printf "  ${CYAN}•${RESET} %b\n" "$1"
}

log_warn() {
    printf "  ${RED}!${RESET} %b\n" "$1"
}

log_error() {
    printf "${RED}Error:${RESET} %b\n" "$1" >&2
}

# 1. Determine installation directory
if [ -n "$CUSTOM_INSTALL_DIR" ]; then
    INSTALL_DIR="$CUSTOM_INSTALL_DIR"
elif [ "$(id -u)" -eq 0 ]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="$HOME/.local/bin"
fi

mkdir -p "$INSTALL_DIR"

log_info "Installing GitPilot into ${INSTALL_DIR}..."

# 2. Detect OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        OS_TARGET="unknown-linux-gnu"
        ;;
    Darwin)
        OS_TARGET="apple-darwin"
        ;;
    *)
        log_error "Unsupported operating system: $OS"
        exit 1
        ;;
esac

case "$ARCH" in
    x86_64|amd64)
        ARCH_TARGET="x86_64"
        ;;
    aarch64|arm64)
        ARCH_TARGET="aarch64"
        ;;
    *)
        log_error "Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"
log_step "Detected platform: ${BOLD}${TARGET}${RESET}"

# Helper to check for command availability
has_cmd() {
    command -v "$1" >/dev/null 2>&1
}

# Helper to download a URL to a file
download() {
    local url="$1"
    local dest="$2"
    if has_cmd curl; then
        curl -fsSL "$url" -o "$dest"
    elif has_cmd wget; then
        wget -q -O "$dest" "$url"
    else
        log_error "Neither curl nor wget was found on your system."
        return 1
    fi
}

INSTALLED=0

# 3. Strategy A: If running inside local GitPilot repo, build and install directly
if [ -f "./Cargo.toml" ] && grep -q 'name = "gitpilot"' "./Cargo.toml" 2>/dev/null && has_cmd cargo; then
    log_step "Local GitPilot source repository detected. Building release binary..."
    cargo build --release --quiet
    if [ -f "./target/release/${BINARY_NAME}" ]; then
        cp "./target/release/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
        chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
        INSTALLED=1
        log_step "Installed local release binary to ${INSTALL_DIR}/${BINARY_NAME}"
    fi
fi

# 4. Strategy B: Try downloading prebuilt binary from GitHub Releases
if [ "$INSTALLED" -eq 0 ]; then
    TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'gitpilot-install')"
    cleanup() {
        rm -rf "$TMP_DIR"
    }
    trap cleanup EXIT

    log_step "Checking for precompiled release binaries on GitHub..."
    RELEASE_URL="https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/latest/download/gitpilot-${TARGET}.tar.gz"
    ARCHIVE="${TMP_DIR}/gitpilot-${TARGET}.tar.gz"

    if download "$RELEASE_URL" "$ARCHIVE" 2>/dev/null && [ -s "$ARCHIVE" ]; then
        log_step "Downloading pre-built release package..."
        tar -xzf "$ARCHIVE" -C "$TMP_DIR"
        if [ -f "$TMP_DIR/${BINARY_NAME}" ]; then
            mv "$TMP_DIR/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
            chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
            INSTALLED=1
            log_step "Extracted release binary to ${INSTALL_DIR}/${BINARY_NAME}"
        fi
    fi
fi

# 5. Strategy C: If no precompiled binary found, compile via Cargo from remote Git
if [ "$INSTALLED" -eq 0 ]; then
    log_step "No precompiled binary found for release yet. Checking for Cargo..."
    if has_cmd cargo; then
        log_step "Rust toolchain detected! Building from source with Cargo..."
        INSTALL_ROOT="$(dirname "$INSTALL_DIR")"
        cargo install --git "https://github.com/${REPO_OWNER}/${REPO_NAME}.git" --root "$INSTALL_ROOT" --force --quiet
        if [ -f "${INSTALL_ROOT}/bin/${BINARY_NAME}" ]; then
            if [ "${INSTALL_ROOT}/bin" != "$INSTALL_DIR" ]; then
                cp "${INSTALL_ROOT}/bin/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
            fi
            INSTALLED=1
            log_step "Successfully compiled and installed via Cargo"
        fi
    else
        log_warn "Precompiled binaries not yet published and 'cargo' was not found."
        log_error "Please install Rust (https://rustup.rs) or check back once GitHub Releases are published."
        exit 1
    fi
fi

# 6. Create case-insensitive alias: Gitpilot -> gitpilot
if [ -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    ln -sf "${INSTALL_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${ALT_BINARY_NAME}"
    log_step "Created alias: ${BOLD}${ALT_BINARY_NAME}${RESET} -> ${BINARY_NAME}"
fi

# 7. Ensure install directory is in PATH
PATH_CONFIGURED=0
case ":$PATH:" in
    *":${INSTALL_DIR}:"*)
        PATH_CONFIGURED=1
        ;;
esac

if [ "$PATH_CONFIGURED" -eq 0 ]; then
    log_warn "${INSTALL_DIR} is not in your current PATH."
    DETECTED_SHELL="$(basename "${SHELL:-bash}")"
    CONFIG_FILE=""
    case "$DETECTED_SHELL" in
        zsh)
            CONFIG_FILE="${HOME}/.zshrc"
            ;;
        bash)
            if [ -f "${HOME}/.bashrc" ]; then
                CONFIG_FILE="${HOME}/.bashrc"
            elif [ -f "${HOME}/.bash_profile" ]; then
                CONFIG_FILE="${HOME}/.bash_profile"
            fi
            ;;
        fish)
            CONFIG_FILE="${HOME}/.config/fish/config.fish"
            ;;
    esac

    WRITTEN=0
    if [ -n "$CONFIG_FILE" ] && { [ -w "$CONFIG_FILE" ] || { [ ! -e "$CONFIG_FILE" ] && [ -w "$(dirname "$CONFIG_FILE")" ]; }; }; then
        if [ "$DETECTED_SHELL" = "fish" ]; then
            if echo "fish_add_path ${INSTALL_DIR}" >> "$CONFIG_FILE" 2>/dev/null; then
                WRITTEN=1
            fi
        else
            if echo "export PATH=\"${INSTALL_DIR}:\$PATH\"" >> "$CONFIG_FILE" 2>/dev/null; then
                WRITTEN=1
            fi
        fi
        if [ "$WRITTEN" -eq 1 ]; then
            log_step "Added ${INSTALL_DIR} to ${CONFIG_FILE}"
            log_warn "Run 'source ${CONFIG_FILE}' or restart your terminal to reload PATH."
        fi
    fi

    if [ "$WRITTEN" -eq 0 ]; then
        log_warn "Add the following line to your shell configuration:"
        printf "    export PATH=\"%s:\$PATH\"\n" "${INSTALL_DIR}"
    fi
fi

# 8. Verification & Success Announcement
VERSION_OUTPUT="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null || echo "GitPilot")"

printf "\n"
printf "${GREEN}   ___  _ _   ___  _ _        _    ${RESET}\n"
printf "${GREEN}  / __|(_) |_| _ \(_) |___  _| |_  ${RESET}\n"
printf "${GREEN} | (_ || |  _|  _/| | / _ \/ _  _| ${RESET}\n"
printf "${GREEN}  \___||_|\__|_|  |_|_\___/\__|_\_  ${RESET}\n"
printf "\n"
printf "${BOLD}GitPilot installed successfully!${RESET} (${GRAY}%s${RESET})\n\n" "$VERSION_OUTPUT"
printf "Usage:\n"
printf "  ${CYAN}cd /path/to/any/git/repo${RESET}\n"
printf "  ${GREEN}gitpilot${RESET}  ${GRAY}# or 'Gitpilot'${RESET}\n\n"
