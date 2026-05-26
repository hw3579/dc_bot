#!/usr/bin/env bash
set -euo pipefail

SYSTEMD_USER_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
INSTALL_BASE="${XDG_DATA_HOME:-$HOME/.local/share}/dc-bot"
RELAY_BIN="$INSTALL_BASE/bin/ib-options-relay"
WATCHER_SERVICE_NAME="dc-bot-discord-watch.service"
RELAY_SERVICE_NAME="dc-bot-ib-relay.service"

REMOVE_WATCHER=1
REMOVE_RELAY=1
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage: scripts/uninstall-systemd-user.sh [options]

Remove systemd user services installed by scripts/install-systemd-user.sh.

Options:
  --watcher-only   Remove only dc-bot-discord-watch.service
  --relay-only     Remove only dc-bot-ib-relay.service and its installed binary
  --dry-run        Print the actions without changing files or systemd state
  -h, --help       Show this help message

Notes:
  - Default behavior removes both services.
  - This script does not delete your repository .env, watcher cursor files, or audit JSONL files.
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

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --watcher-only)
        REMOVE_RELAY=0
        ;;
      --relay-only)
        REMOVE_WATCHER=0
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

  if [[ "$REMOVE_WATCHER" -eq 0 && "$REMOVE_RELAY" -eq 0 ]]; then
    log "At least one component must be selected."
    exit 1
  fi
}

remove_service() {
  local service_name=$1
  local service_path="$SYSTEMD_USER_DIR/$service_name"

  if [[ -f "$service_path" ]]; then
    run_cmd systemctl --user disable --now "$service_name"
    run_cmd rm -f "$service_path"
  fi
}

main() {
  parse_args "$@"

  if [[ "$DRY_RUN" -eq 0 ]] && ! command -v systemctl >/dev/null 2>&1; then
    log "Missing required command: systemctl"
    exit 1
  fi

  if [[ "$REMOVE_WATCHER" -eq 1 ]]; then
    remove_service "$WATCHER_SERVICE_NAME"
  fi

  if [[ "$REMOVE_RELAY" -eq 1 ]]; then
    remove_service "$RELAY_SERVICE_NAME"
    if [[ -f "$RELAY_BIN" ]]; then
      run_cmd rm -f "$RELAY_BIN"
    fi
    if [[ -f "$INSTALL_BASE/bin/options-relay-state.json" ]]; then
      run_cmd rm -f "$INSTALL_BASE/bin/options-relay-state.json"
    fi
  fi

  run_cmd systemctl --user daemon-reload
  run_cmd systemctl --user reset-failed

  if [[ "$REMOVE_RELAY" -eq 1 ]]; then
    if [[ "$DRY_RUN" -eq 0 ]]; then
      rmdir "$INSTALL_BASE/bin" 2>/dev/null || true
      rmdir "$INSTALL_BASE" 2>/dev/null || true
    else
      log "+ try to remove empty directories $INSTALL_BASE/bin and $INSTALL_BASE"
    fi
  fi

  log "Uninstall complete."
}

main "$@"