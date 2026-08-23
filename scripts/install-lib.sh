#!/usr/bin/env bash
# sb-agent-core — librería de instalación compartida (Linux/macOS)
#
# No se ejecuta sola. Cada install.sh de un agente (OxiPulse, FerroSentry, ...)
# la descarga y la sourcea, fija sus propias constantes/banner/config, y llama
# a estas funciones. Ver sb-agent-core/TODO.md, sección "Lo que no es Rust".
#
# Uso típico en el install.sh de un agente:
#
#   set -euo pipefail
#   SB_AGENT_LABEL="oxipulse"
#   LIB_URL="https://raw.githubusercontent.com/securyblack/sb-agent-core/main/scripts/install-lib.sh"
#   LIB_TMP="$(mktemp)"
#   curl -fsSL "$LIB_URL" -o "$LIB_TMP" && source "$LIB_TMP"
#
#   sb_require_root
#   sb_require_cmds curl tar systemctl
#   TARGET="$(sb_detect_arch_linux)"
#   VERSION="$(sb_fetch_latest_version "securyblack/oxi-pulse")"
#   TMP_DIR="$(mktemp -d); trap 'rm -rf "$TMP_DIR"' EXIT
#   sb_download_and_verify "https://github.com/.../oxipulse-${TARGET}.tar.gz" "$TMP_DIR/asset.tar.gz"
#   sb_install_binary "$TMP_DIR/asset.tar.gz" "oxipulse" "/usr/local/bin"
#   sb_write_systemd_unit "oxipulse" "OxiPulse monitoring agent" "/usr/local/bin/oxipulse"
#   sb_enable_start_service "oxipulse"

# ─── Colores (una sola vez, no pisan si el agente ya los definió) ────────────
RED="${RED:-\033[0;31m}"
GREEN="${GREEN:-\033[0;32m}"
YELLOW="${YELLOW:-\033[1;33m}"
CYAN="${CYAN:-\033[0;36m}"
BOLD="${BOLD:-\033[1m}"
RESET="${RESET:-\033[0m}"

# ─── Logging — SB_AGENT_LABEL lo fija el script del agente antes de sourcear ─
SB_AGENT_LABEL="${SB_AGENT_LABEL:-sb-agent}"

sb_info()    { echo -e "${CYAN}${BOLD}[${SB_AGENT_LABEL}]${RESET} $*"; }
sb_success() { echo -e "${GREEN}${BOLD}[${SB_AGENT_LABEL}]${RESET} $*"; }
sb_warn()    { echo -e "${YELLOW}${BOLD}[${SB_AGENT_LABEL}]${RESET} $*"; }
sb_die()     { echo -e "${RED}${BOLD}[${SB_AGENT_LABEL}] ERROR:${RESET} $*" >&2; exit 1; }

# ─── Requisitos ───────────────────────────────────────────────────────────────
sb_require_root() {
  [[ "$EUID" -ne 0 ]] && sb_die "This script must be run as root. Try: sudo bash"
  return 0
}

sb_require_cmds() {
  local cmd
  for cmd in "$@"; do
    command -v "$cmd" &>/dev/null || sb_die "Required command not found: ${cmd}"
  done
}

# ─── Arquitectura ─────────────────────────────────────────────────────────────
# Echo del target triple. El agente decide qué hacer con el valor.
sb_detect_arch_linux() {
  local arch target
  arch="$(uname -m)"
  case "$arch" in
    x86_64)          target="x86_64-unknown-linux-gnu"  ;;
    aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
    *) sb_die "Unsupported architecture: ${arch}" ;;
  esac
  sb_info "Detected architecture: ${arch} (${target})" >&2
  echo "$target"
}

# ─── Última versión publicada en GitHub Releases ─────────────────────────────
# sb_fetch_latest_version "securyblack/oxi-pulse"
sb_fetch_latest_version() {
  local repo="$1" version
  sb_info "Fetching latest release from GitHub…" >&2
  version="$(curl -fsSL "https://api.github.com/repos/${repo}/releases/latest" \
    | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\(.*\)".*/\1/')"
  [[ -z "$version" ]] && sb_die "Could not determine latest version. Check your internet connection."
  sb_info "Latest version: ${version}" >&2
  echo "$version"
}

# ─── Descarga + verificación de checksum ─────────────────────────────────────
# sb_download_and_verify "$DOWNLOAD_URL" "$OUT_FILE"
# Descarga $OUT_FILE y, si existe "$DOWNLOAD_URL.sha256", lo verifica.
# La ausencia de checksum es un warning, no un error (mismo comportamiento que
# los 3 instaladores existentes hoy).
sb_download_and_verify() {
  local url="$1" out="$2" asset_name
  asset_name="$(basename "$out")"

  sb_info "Downloading ${asset_name}…"
  curl -fsSL "$url" -o "$out" \
    || sb_die "Download failed. Is the release published with the expected asset name?"

  if curl -fsSL "${url}.sha256" -o "${out}.sha256" 2>/dev/null; then
    sb_info "Verifying checksum…"
    # El .sha256 trae el nombre del asset original; sha256sum -c compara por
    # nombre de fichero, así que hay que ejecutarlo desde el mismo directorio
    # y con el nombre que trae el checksum, no el path completo.
    (cd "$(dirname "$out")" && sha256sum -c "$(basename "${out}.sha256")" --quiet) \
      || sb_die "Checksum verification failed"
    sb_success "Checksum OK"
  else
    sb_warn "No checksum file found, skipping verification"
  fi
}

# ─── Instalación del binario desde un .tar.gz ────────────────────────────────
# sb_install_binary "$TAR_GZ_PATH" "binary_name" "/usr/local/bin"
sb_install_binary() {
  local archive="$1" binary_name="$2" install_dir="$3" extract_dir
  extract_dir="$(dirname "$archive")"

  sb_info "Installing binary to ${install_dir}/${binary_name}…"
  tar -xzf "$archive" -C "$extract_dir"
  install -m 755 "${extract_dir}/${binary_name}" "${install_dir}/${binary_name}"
  sb_success "Binary installed"
}

# ─── Unidad systemd ───────────────────────────────────────────────────────────
# sb_write_systemd_unit "oxipulse" "OxiPulse monitoring agent" "/usr/local/bin/oxipulse" ["/etc/oxipulse"] [restart_sec]
sb_write_systemd_unit() {
  local service_name="$1" description="$2" exec_start="$3" working_dir="${4:-}" restart_sec="${5:-10}"
  local service_file="/etc/systemd/system/${service_name}.service"
  local working_dir_line=""
  [[ -n "$working_dir" ]] && working_dir_line="WorkingDirectory=${working_dir}"

  sb_info "Writing systemd unit ${service_file}…"
  cat > "$service_file" <<EOF
[Unit]
Description=${description}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${exec_start}
Restart=always
RestartSec=${restart_sec}
${working_dir_line}
StandardOutput=journal
StandardError=journal
SyslogIdentifier=${service_name}

[Install]
WantedBy=multi-user.target
EOF
}

sb_enable_start_service() {
  local service_name="$1"
  systemctl daemon-reload
  systemctl enable --now "$service_name"
  sb_success "Service ${service_name} enabled and started"
}
