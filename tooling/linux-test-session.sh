#!/usr/bin/env bash
# Isolated Linux desktop/keyring for source tests. Never use the user's keyring.
set -euo pipefail
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [ "$#" -eq 0 ]; then
  echo 'Usage: bash tooling/linux-test-session.sh <command> [args...]' >&2
  exit 2
fi
if [ "${STUDIO_LINUX_TEST_SESSION:-}" != 1 ]; then
  # Keep the package cache stable while isolating the app's XDG directories.
  # The public launcher performs a frozen install before opening the desktop.
  if command -v pnpm >/dev/null 2>&1; then
    studio_pnpm_store=$(pnpm --dir "$repo_dir" store path)
    export npm_config_store_dir
    npm_config_store_dir=$(dirname -- "$studio_pnpm_store")
  fi
  mkdir -p "$repo_dir/.local/test-runs"
  run_dir=$(mktemp -d "$repo_dir/.local/test-runs/linux-session-XXXXXX")
  export STUDIO_LINUX_TEST_SESSION=1 STUDIO_LINUX_TEST_ROOT="$run_dir"
  export XDG_DATA_HOME="$run_dir/data" XDG_CONFIG_HOME="$run_dir/config"
  export XDG_CACHE_HOME="$run_dir/cache" XDG_RUNTIME_DIR="$run_dir/runtime"
  export TMPDIR="$run_dir/tmp"
  mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$TMPDIR"
  chmod 700 "$XDG_RUNTIME_DIR" "$XDG_DATA_HOME"
  # Native checks use their own X server and window manager, including on WSLg.
  export GDK_BACKEND=x11
  unset WAYLAND_DISPLAY
  exec dbus-run-session -- xvfb-run -a -s '-screen 0 2560x1440x24' bash "$0" "$@"
fi
run_dir=${STUDIO_LINUX_TEST_ROOT:?Missing isolated test directory}
openbox >"$run_dir/window-manager.log" 2>&1 &
window_manager_pid=$!
printf 'dataset-studio-isolated-test-keyring\n' | gnome-keyring-daemon \
  --unlock --components=secrets --foreground --control-directory "$run_dir/keyring-control" \
  >"$run_dir/keyring.log" 2>&1 &
keyring_pid=$!
trap 'kill "$keyring_pid" "$window_manager_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 100); do
  if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner org.freedesktop.secrets 2>/dev/null | grep -q true; then
    break
  fi
  sleep 0.1
done
echo "Linux test session: $run_dir"
"$@"
