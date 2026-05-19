#!/usr/bin/env bash
set -euo pipefail

root="$(mktemp -d)"
cleanup_root() {
  rm -rf "$root"
}
trap cleanup_root EXIT

xvfb-run -a bash --noprofile --norc -s "$root" "$@" <<'EOF'
set -euo pipefail

root="$1"
shift

runtime_dir="$root/xdg"
weston_log="$root/weston.log"
socket_name="chelotype-wayland-e2e"
mkdir -p "$runtime_dir"
chmod 700 "$runtime_dir"

weston_pid=""
cleanup() {
  if [[ -n "$weston_pid" ]]; then
    kill "$weston_pid" 2>/dev/null || true
    wait "$weston_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

XDG_RUNTIME_DIR="$runtime_dir" weston \
  --backend=x11 \
  --renderer=pixman \
  --shell=kiosk \
  --socket="$socket_name" \
  --width=1200 \
  --height=800 \
  --no-config \
  --idle-time=0 \
  --log="$weston_log" &
weston_pid="$!"

for _ in {1..100}; do
  if [[ -S "$runtime_dir/$socket_name" ]] && grep -F "window id" "$weston_log" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

if [[ ! -S "$runtime_dir/$socket_name" ]]; then
  echo "nested wayland socket did not appear" >&2
  cat "$weston_log" >&2 || true
  exit 1
fi

weston_window_id="$(sed -n 's/.*window id \([0-9][0-9]*\).*/\1/p' "$weston_log" | tail -n 1)"
if [[ -z "$weston_window_id" ]]; then
  echo "nested wayland X window id did not appear" >&2
  cat "$weston_log" >&2 || true
  exit 1
fi

export XDG_RUNTIME_DIR="$runtime_dir"
export WAYLAND_DISPLAY="$socket_name"
export GDK_BACKEND=wayland
export GSK_RENDERER=gl
export GSETTINGS_BACKEND=memory
export NO_AT_BRIDGE=1
export CHELOTYPE_NESTED_WAYLAND_X_WINDOW="$weston_window_id"

"$@"
EOF
