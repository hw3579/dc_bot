#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
SYSTEMD_TEMPLATE_DIR="$REPO_ROOT/systemd/user"
SYSTEMD_USER_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
INSTALL_BASE="${XDG_DATA_HOME:-$HOME/.local/share}/dc-bot"
RELAY_INSTALL_DIR="$INSTALL_BASE/bin"
RELAY_BIN="$RELAY_INSTALL_DIR/ib-options-relay"
WATCHER_SERVICE_NAME="dc-bot-discord-watch.service"
RELAY_SERVICE_NAME="dc-bot-ib-relay.service"
UV_BIN=""

INSTALL_WATCHER=1
INSTALL_RELAY=1
START_SERVICES=1
SKIP_BUILD=0
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage: scripts/install-systemd-user.sh [options]

Install systemd user services for the Discord watcher and/or the headless broker relay.

Options:
  --watcher-only   Install only dc-bot-discord-watch.service
  --relay-only     Install only dc-bot-ib-relay.service
  --skip-build     Reuse an existing release relay binary instead of rebuilding it
  --no-start       Enable the services but do not start/restart them immediately
  --dry-run        Print the actions without changing files or systemd state
  -h, --help       Show this help message

Notes:
  - Default behavior installs both services.
    - Relay installation builds app/src-tauri/target/release/ib-options-relay and copies it to:
      ~/.local/share/dc-bot/bin/ib-options-relay
    - Relay mode may need the repo Python runtime as well when RELAY_BROKER=moomoo
  - User services only survive logout when linger is enabled:
      sudo loginctl enable-linger "$USER"
EOF
}

log() {
  printf '%s\n' "$*"
}

run_cmd() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '+ '
    printf '%q ' "$@"
    printf '\n'
    return 0
  fi

  "$@"
}

run_in_dir() {
  local dir=$1
  shift

  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '+ (cd %q && ' "$dir"
    printf '%q ' "$@"
    printf ')\n'
    return 0
  fi

  (
    cd "$dir"
    "$@"
  )
}

require_command() {
  local command_name=$1
  if ! command -v "$command_name" >/dev/null 2>&1; then
    log "Missing required command: $command_name"
    exit 1
  fi
}

escape_sed_replacement() {
  printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'
}

render_template() {
  local template_path=$1
  local output_path=$2
  local repo_root_escaped
  local uv_bin_escaped
  local relay_bin_escaped

  repo_root_escaped=$(escape_sed_replacement "$REPO_ROOT")
  uv_bin_escaped=$(escape_sed_replacement "$UV_BIN")
  relay_bin_escaped=$(escape_sed_replacement "$RELAY_BIN")

  if [[ "$DRY_RUN" -eq 1 ]]; then
    log "+ render $template_path -> $output_path"
    return 0
  fi

  mkdir -p "$(dirname "$output_path")"
  sed \
    -e "s|__REPO_ROOT__|$repo_root_escaped|g" \
    -e "s|__UV_BIN__|$uv_bin_escaped|g" \
    -e "s|__RELAY_BIN__|$relay_bin_escaped|g" \
    "$template_path" > "$output_path"
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --watcher-only)
        INSTALL_RELAY=0
        ;;
      --relay-only)
        INSTALL_WATCHER=0
        ;;
      --skip-build)
        SKIP_BUILD=1
        ;;
      --no-start)
        START_SERVICES=0
        ;;
      --dry-run)
        DRY_RUN=1
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        log "Unknown option: $1"
        usage
        exit 1
        ;;
    esac
    shift
  done

  if [[ "$INSTALL_WATCHER" -eq 0 && "$INSTALL_RELAY" -eq 0 ]]; then
    log "At least one component must be selected."
    exit 1
  fi
}

build_relay_binary() {
  local source_binary="$REPO_ROOT/app/src-tauri/target/release/ib-options-relay"

  require_command pnpm
  require_command cargo

  if [[ "$SKIP_BUILD" -eq 0 ]]; then
    run_in_dir "$REPO_ROOT/app" pnpm install --frozen-lockfile
    run_in_dir "$REPO_ROOT/app" pnpm build
    run_in_dir "$REPO_ROOT/app" cargo build --release --manifest-path src-tauri/Cargo.toml
  fi

  if [[ "$DRY_RUN" -eq 0 && ! -f "$source_binary" ]]; then
    log "Release relay binary not found: $source_binary"
    exit 1
  fi

  run_cmd mkdir -p "$RELAY_INSTALL_DIR"
  run_cmd install -m 755 "$source_binary" "$RELAY_BIN"
}

prepare_watcher_runtime() {
  require_command uv
  UV_BIN=$(command -v uv)

  if [[ "$SKIP_BUILD" -eq 0 ]]; then
    run_in_dir "$REPO_ROOT" uv sync
  fi
}

install_services() {
  local service_names=()

  require_command systemctl

  if [[ "$INSTALL_WATCHER" -eq 1 || "$INSTALL_RELAY" -eq 1 ]]; then
    prepare_watcher_runtime
  fi

  run_cmd mkdir -p "$SYSTEMD_USER_DIR"

  if [[ "$INSTALL_WATCHER" -eq 1 ]]; then
    render_template \
      "$SYSTEMD_TEMPLATE_DIR/$WATCHER_SERVICE_NAME.tpl" \
      "$SYSTEMD_USER_DIR/$WATCHER_SERVICE_NAME"
    service_names+=("$WATCHER_SERVICE_NAME")
  fi

  if [[ "$INSTALL_RELAY" -eq 1 ]]; then
    build_relay_binary
    render_template \
      "$SYSTEMD_TEMPLATE_DIR/$RELAY_SERVICE_NAME.tpl" \
      "$SYSTEMD_USER_DIR/$RELAY_SERVICE_NAME"
    service_names+=("$RELAY_SERVICE_NAME")
  fi

  run_cmd systemctl --user daemon-reload

  for service_name in "${service_names[@]}"; do
    run_cmd systemctl --user enable "$service_name"
    if [[ "$START_SERVICES" -eq 1 ]]; then
      run_cmd systemctl --user restart "$service_name"
    fi
  done

  log "Installed services: ${service_names[*]}"
  if [[ "$START_SERVICES" -eq 0 ]]; then
    log "Services were enabled but not started."
  fi
  log "If you want these user services to stay alive after logout, enable linger:"
  log "  sudo loginctl enable-linger \"$USER\""
}

main() {
  parse_args "$@"
  install_services
}

main "$@"