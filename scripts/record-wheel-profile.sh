#!/usr/bin/env bash
set -euo pipefail

if ! command -v wev >/dev/null 2>&1; then
  echo "wev is required to record wheel input on Wayland" >&2
  exit 2
fi

if [[ -n "${CHELOTYPE_CONFIG_DIR:-}" ]]; then
  config_dir="$CHELOTYPE_CONFIG_DIR"
elif [[ -n "${XDG_CONFIG_HOME:-}" ]]; then
  config_dir="$XDG_CONFIG_HOME/chelotype"
else
  config_dir="$HOME/.config/chelotype"
fi

output="${1:-$config_dir/wheel-profile.tsv}"
mkdir -p "$(dirname "$output")"
raw="$(mktemp)"
tmp_output="$output.tmp"

cleanup() {
  rm -f "$raw"
  rm -f "$tmp_output"
}
trap cleanup EXIT

cat <<EOF
Recording wheel profile.

1. Move the pointer over the wev window.
2. Scroll naturally: one flick, several notches, whatever feels real.
3. Press Ctrl+C in this terminal when done.

Output: $output
EOF

wev_pid=""
stop_recording() {
  printf "\nStopping recording...\n" >&2
  if [[ -n "$wev_pid" ]]; then
    kill -INT "$wev_pid" 2>/dev/null || true
  fi
}
trap stop_recording INT TERM

set +e
wev -f wl_pointer:axis -f wl_pointer:axis_value120 >"$raw" &
wev_pid=$!
wait "$wev_pid"
status=$?
wev_pid=""
set -e
trap - INT TERM

if [[ "$status" -ne 0 && "$status" -ne 130 && "$status" -ne 143 ]]; then
  echo "wev exited with status $status" >&2
  exit "$status"
fi

awk '
  /wl_pointer.*axis:/ && /time:/ && /axis: 0/ {
    if (match($0, /time: [0-9]+/)) {
      last_time = substr($0, RSTART + 6, RLENGTH - 6) + 0
    }
  }
  /wl_pointer.*axis_value120:/ && /axis: 0/ && /value120:/ {
    if (last_time == "") next
    if (!match($0, /value120: -?[0-9]+/)) next
    value120 = substr($0, RSTART + 10, RLENGTH - 10) + 0
    if (start_time == "") start_time = last_time
    elapsed = last_time - start_time
    lines = -value120 / 120.0 * 3.0
    if (lines != 0) {
      printf "%d\t%.4f\n", elapsed, lines
      count++
    }
  }
  END {
    if (count == 0) exit 1
  }
' "$raw" >"$tmp_output" || {
  echo "No vertical wheel events were recorded." >&2
  echo "Make sure the pointer is over the wev window before scrolling." >&2
  exit 1
}

{
  printf "# chelotype wheel profile v1\n"
  printf "# elapsed_ms\tlines\n"
  cat "$tmp_output"
} >"$output"
rm -f "$tmp_output"

event_count="$(awk 'BEGIN { count = 0 } $1 !~ /^#/ { count++ } END { print count }' "$output")"
duration_ms="$(awk '$1 !~ /^#/ { last = $1 } END { print last + 0 }' "$output")"
printf "Saved %s events over %sms to %s\n" "$event_count" "$duration_ms" "$output"
