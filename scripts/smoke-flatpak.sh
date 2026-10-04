#!/usr/bin/env bash
set -euo pipefail

build_dir="${1:-build-dir}"
runtime_dir="$(mktemp -d)"
export XDG_RUNTIME_DIR="$runtime_dir"
weston --backend=headless --socket=chelotype-smoke --idle-time=0 >/dev/null 2>&1 &
compositor=$!
trap 'kill "$compositor"; rm -rf "$runtime_dir"' EXIT
for _ in $(seq 50); do [[ -S "$runtime_dir/chelotype-smoke" ]] && break; sleep 0.1; done

status=0
WAYLAND_DISPLAY=chelotype-smoke timeout 10 flatpak-builder --run "$build_dir" com.chelokot.Chelotype.yml chelotype || status=$?
if [[ "$status" -ne 124 ]]; then
  printf 'Chelotype exited with status %s during the Flatpak smoke run\n' "$status" >&2
  exit 1
fi
printf 'Chelotype stayed up for 10 seconds in the Flatpak build\n'
