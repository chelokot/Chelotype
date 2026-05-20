#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
duration_seconds=6
scenario="held-key"
strict=0
release=0
allow_live=0
display_backend="x11"
frame_budget_us=4167
target_refresh_millihz=240000

usage() {
  cat <<'EOF'
Usage: scripts/profile-240hz.sh [--scenario held-key|scroll|scroll-burst|idle|frame-baseline|timer-baseline] [--display-backend x11|weston-headless|native-wayland] [--duration seconds] [--release] [--strict] [--allow-live]

Runs the GTK app with CHELOTYPE_PERF_TRACE enabled and prints frame timing
percentiles. Run the x11 backend under xvfb-run for nested automation. The
weston-headless backend starts an isolated 240 Hz Wayland compositor and uses
app-side automation for supported scenarios. The native-wayland backend uses
the current Wayland session and app-side automation only, with no xdotool,
pointer movement, focus changes, or keyboard events. Use --allow-live only when it is
acceptable for xdotool to focus windows and move the pointer in the current
desktop session.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scenario)
      scenario="${2:?missing scenario}"
      shift 2
      ;;
    --duration)
      duration_seconds="${2:?missing duration}"
      shift 2
      ;;
    --display-backend)
      display_backend="${2:?missing display backend}"
      shift 2
      ;;
    --strict)
      strict=1
      shift
      ;;
    --release)
      release=1
      shift
      ;;
    --allow-live)
      allow_live=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$scenario" in
  held-key|scroll|scroll-burst|idle|frame-baseline|timer-baseline) ;;
  *)
    echo "unknown scenario: $scenario" >&2
    exit 2
    ;;
esac

case "$display_backend" in
  x11|weston-headless|native-wayland) ;;
  *)
    echo "unknown display backend: $display_backend" >&2
    exit 2
    ;;
esac

if [[ "$display_backend" == "x11" ]]; then
  if ! command -v xdotool >/dev/null 2>&1; then
    echo "xdotool is required for x11 profile automation" >&2
    exit 2
  fi
  if [[ "$allow_live" -ne 1 && "${DISPLAY:-}" =~ ^:0($|\.) && -z "${CHELOTYPE_NESTED_PROFILE:-}" ]]; then
    echo "refusing to drive the live desktop session; run through xvfb-run or set CHELOTYPE_NESTED_PROFILE=1" >&2
    echo "example: CHELOTYPE_NESTED_PROFILE=1 xvfb-run -a scripts/profile-240hz.sh --scenario scroll --release" >&2
    exit 2
  fi
elif [[ "$display_backend" == "weston-headless" ]]; then
  if ! command -v weston >/dev/null 2>&1; then
    echo "weston is required for weston-headless profiling" >&2
    exit 2
  fi
  case "$scenario" in
    idle|scroll-burst|frame-baseline|timer-baseline) ;;
    *)
      echo "weston-headless currently supports idle and scroll-burst scenarios" >&2
      exit 2
      ;;
  esac
else
  if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    echo "native-wayland profiling requires WAYLAND_DISPLAY" >&2
    exit 2
  fi
  case "$scenario" in
    idle|scroll-burst|frame-baseline|timer-baseline) ;;
    *)
      echo "native-wayland currently supports idle and scroll-burst scenarios" >&2
      exit 2
      ;;
  esac
fi

build_args=(build)
target_dir="$root/target/debug"
if [[ "$release" -eq 1 ]]; then
  build_args+=(--release)
  target_dir="$root/target/release"
fi

"$root/scripts/with-zig.sh" cargo "${build_args[@]}"

libdir="$(dirname "$(find "$root/target/debug/build" -path '*/ghostty-install/lib/libghostty-vt.so.0' -print -quit)")"
if [[ -z "$libdir" ]]; then
  echo "libghostty-vt build output was not found" >&2
  exit 1
fi

profile_root="$(mktemp -d)"
cleanup() {
  if [[ -n "${app_pid:-}" ]]; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  if [[ -n "${weston_pid:-}" ]]; then
    kill "$weston_pid" 2>/dev/null || true
    wait "$weston_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

perf_trace="$profile_root/perf.tsv"
scroll_trace="$profile_root/scroll.tsv"
snapshot_dir="$profile_root/snapshots"
mkdir -p "$snapshot_dir"

app_env=(
  "LD_LIBRARY_PATH=$libdir:${LD_LIBRARY_PATH:-}"
  "GSETTINGS_BACKEND=memory"
  "NO_AT_BRIDGE=1"
  "CHELOTYPE_SHELL=${CHELOTYPE_SHELL:-/bin/sh}"
  "CHELOTYPE_PERF_TRACE=$perf_trace"
  "CHELOTYPE_SCROLL_TRACE=$scroll_trace"
  "CHELOTYPE_SNAPSHOT_DIR=$snapshot_dir"
)
if [[ "$display_backend" == "weston-headless" ]]; then
  runtime_dir="$profile_root/runtime"
  mkdir -p "$runtime_dir"
  chmod 700 "$runtime_dir"
  export XDG_RUNTIME_DIR="$runtime_dir"
  weston_log="$profile_root/weston.log"
  weston_config="$profile_root/weston.ini"
  weston_socket="chelotype-profile"
  weston_renderer="${CHELOTYPE_WESTON_RENDERER:-gl}"
  cat > "$weston_config" <<'EOF'
[core]
repaint-window=4
EOF
  weston --backend=headless --renderer="$weston_renderer" --width=1200 --height=900 --refresh-rate=240000 --socket="$weston_socket" --config="$weston_config" --idle-time=0 --log="$weston_log" &
  weston_pid="$!"
  for _ in {1..100}; do
    if [[ -S "$XDG_RUNTIME_DIR/$weston_socket" ]]; then
      break
    fi
    sleep 0.05
  done
  if [[ ! -S "$XDG_RUNTIME_DIR/$weston_socket" ]]; then
    echo "weston-headless socket did not appear" >&2
    exit 1
  fi
  app_env+=(
    "GDK_BACKEND=wayland"
    "WAYLAND_DISPLAY=$weston_socket"
    "GSK_RENDERER=gl"
  )
  if [[ "$scenario" == "scroll-burst" ]]; then
    app_env+=("CHELOTYPE_PROFILE_SCROLL_BURST=1")
  fi
  if [[ "$scenario" == "frame-baseline" ]]; then
    app_env+=("CHELOTYPE_PROFILE_FRAME_BASELINE=1")
  fi
  if [[ "$scenario" == "timer-baseline" ]]; then
    app_env+=("CHELOTYPE_PROFILE_TIMER_BASELINE=1")
  fi
elif [[ "$display_backend" == "native-wayland" ]]; then
  app_env+=(
    "GDK_BACKEND=wayland"
    "GSK_RENDERER=${GSK_RENDERER:-gl}"
  )
  if [[ "$scenario" == "scroll-burst" ]]; then
    app_env+=("CHELOTYPE_PROFILE_SCROLL_BURST=1")
  fi
  if [[ "$scenario" == "frame-baseline" ]]; then
    app_env+=("CHELOTYPE_PROFILE_FRAME_BASELINE=1")
  fi
  if [[ "$scenario" == "timer-baseline" ]]; then
    app_env+=("CHELOTYPE_PROFILE_TIMER_BASELINE=1")
  fi
else
  app_env+=("GDK_BACKEND=${GDK_BACKEND:-x11}")
fi

env "${app_env[@]}" "$target_dir/chelotype" &
app_pid="$!"

if [[ "$display_backend" == "weston-headless" || "$display_backend" == "native-wayland" ]]; then
  : > "$perf_trace"
  : > "$scroll_trace"
  if [[ "$scenario" == "scroll-burst" ]]; then
    reached_scroll_idle=0
    for _ in {1..160}; do
      if grep -q $'^idle\t0\t0.00' "$scroll_trace" 2>/dev/null; then
        reached_scroll_idle=1
        break
      fi
      sleep 0.05
    done
    if [[ ! -s "$scroll_trace" ]]; then
      echo "$display_backend scroll-burst produced no scroll trace" >&2
      exit 1
    fi
    if [[ "$reached_scroll_idle" -ne 1 ]]; then
      echo "$display_backend scroll-burst did not settle before timeout" >&2
      tail -n 20 "$scroll_trace" >&2 || true
      exit 1
    fi
  else
    sleep "$duration_seconds"
  fi
else
window_id=""
for _ in {1..100}; do
  window_id="$(
    while IFS= read -r candidate; do
      [[ "$(xdotool getwindowname "$candidate" 2>/dev/null || true)" == "Chelotype Terminal" ]] || continue
      printf '%s\n' "$candidate"
      break
    done < <(xdotool search --pid "$app_pid" 2>/dev/null || true)
  )"
  if [[ -z "$window_id" ]]; then
    window_id="$(xdotool search --name 'Chelotype Terminal' | tail -n 1 || true)"
  fi
  if [[ -n "$window_id" ]]; then
    break
  fi
  sleep 0.1
done

if [[ -z "$window_id" ]]; then
  echo "Chelotype window did not appear" >&2
  exit 1
fi

xdotool windowactivate --sync "$window_id" || xdotool windowfocus "$window_id" || true
sleep 0.25

case "$scenario" in
  held-key)
    : > "$perf_trace"
    : > "$scroll_trace"
    xdotool keydown --window "$window_id" a
    sleep "$duration_seconds"
    xdotool keyup --window "$window_id" a
    sleep 0.5
    ;;
  scroll|scroll-burst)
    xdotool type --window "$window_id" --delay 1 "for n in \$(seq 1 160); do echo SCROLL_PROFILE_\$n; done"
    xdotool key --window "$window_id" Return
    sleep 1.0
    eval "$(xdotool getwindowgeometry --shell "$window_id")"
    xdotool mousemove "$((X + WIDTH / 2))" "$((Y + HEIGHT / 2))"
    : > "$perf_trace"
    : > "$scroll_trace"
    if [[ "$scenario" == "scroll-burst" ]]; then
      xdotool click --repeat 8 --delay 0 4
    else
      for _ in {1..8}; do
        xdotool click 4
        sleep 0.12
      done
    fi
    sleep 0.8
    if [[ ! -s "$scroll_trace" ]]; then
      echo "scroll scenario produced no scroll events; profile automation did not hit the terminal viewport" >&2
      exit 1
    fi
    ;;
  idle)
    : > "$perf_trace"
    : > "$scroll_trace"
    sleep "$duration_seconds"
    ;;
  frame-baseline)
    : > "$perf_trace"
    : > "$scroll_trace"
    sleep "$duration_seconds"
    ;;
  timer-baseline)
    : > "$perf_trace"
    : > "$scroll_trace"
    sleep "$duration_seconds"
    ;;
esac
fi

summarize_duration() {
  local event="$1"
  awk -F '\t' -v event="$event" '
    $1 == event { values[++count] = $2 }
    END {
      if (count == 0) {
        printf "%-24s count=0\n", event
        exit
      }
      asort(values)
      p50 = values[int((count - 1) * 0.50) + 1]
      p95 = values[int((count - 1) * 0.95) + 1]
      p99 = values[int((count - 1) * 0.99) + 1]
      max = values[count]
      if (event == "gtk_frame_interval" || event == "gtk_tick_wall_interval") {
        fps = p50 > 0 ? 1000000 / p50 : 0
        printf "%-24s count=%-5d p50=%7dus fps=%6.1f p95=%7dus p99=%7dus max=%7dus\n", event, count, p50, fps, p95, p99, max
      } else {
        printf "%-24s count=%-5d p50=%7dus p95=%7dus p99=%7dus max=%7dus\n", event, count, p50, p95, p99, max
      }
    }
  ' "$perf_trace"
}

summarize_counter() {
  local event="$1"
  awk -F '\t' -v event="$event" '
    $1 == event { values[++count] = $2 }
    END {
      if (count == 0) {
        printf "%-24s count=0\n", event
        exit
      }
      asort(values)
      p95 = values[int((count - 1) * 0.95) + 1]
      p99 = values[int((count - 1) * 0.99) + 1]
      max = values[count]
      printf "%-24s count=%-5d p95=%8d p99=%8d max=%8d\n", event, count, p95, p99, max
    }
  ' "$perf_trace"
}

summarize_refresh_gate() {
  awk -F '\t' -v target_refresh_millihz="$target_refresh_millihz" -v frame_budget_us="$frame_budget_us" '
    $1 == "gdk_monitor_refresh_millihz" { monitor[++monitor_count] = $2 }
    $1 == "gtk_frame_interval" { frame[++frame_count] = $2 }
    $1 == "gtk_tick_wall_interval" { wall[++wall_count] = $2 }
    END {
      if (monitor_count > 0) {
        asort(monitor)
        monitor_max = monitor[monitor_count]
        monitor_hz = monitor_max / 1000.0
        target_hz = target_refresh_millihz / 1000.0
        monitor_status = monitor_max >= target_refresh_millihz ? "ok" : "below-target"
        printf "%-24s monitor_max=%6.1fHz target=%6.1fHz status=%s\n", "display_cadence", monitor_hz, target_hz, monitor_status
      } else {
        printf "%-24s monitor_max=unknown target=%6.1fHz status=unknown\n", "display_cadence", target_refresh_millihz / 1000.0
      }
      if (frame_count > 0) {
        asort(frame)
        frame_p50 = frame[int((frame_count - 1) * 0.50) + 1]
        frame_hz = frame_p50 > 0 ? 1000000.0 / frame_p50 : 0
        frame_status = frame_p50 <= frame_budget_us ? "ok" : "below-target"
        printf "%-24s p50=%7dus fps=%6.1f budget=%dus status=%s\n", "frame_cadence", frame_p50, frame_hz, frame_budget_us, frame_status
      } else {
        printf "%-24s p50=unknown budget=%dus status=unknown\n", "frame_cadence", frame_budget_us
      }
      if (wall_count > 0) {
        asort(wall)
        wall_p50 = wall[int((wall_count - 1) * 0.50) + 1]
        wall_hz = wall_p50 > 0 ? 1000000.0 / wall_p50 : 0
        wall_status = wall_p50 <= frame_budget_us ? "ok" : "below-target"
        printf "%-24s p50=%7dus fps=%6.1f budget=%dus status=%s\n", "wall_cadence", wall_p50, wall_hz, frame_budget_us, wall_status
      } else {
        printf "%-24s p50=unknown budget=%dus status=unknown\n", "wall_cadence", frame_budget_us
      }
    }
  ' "$perf_trace"
}

echo "profile root: $profile_root"
profile_mode="$([[ "$release" -eq 1 ]] && echo release || echo debug)"
if [[ "$display_backend" == "weston-headless" ]]; then
  echo "scenario: $scenario duration=${duration_seconds}s backend=$display_backend renderer=$weston_renderer profile=$profile_mode"
else
  echo "scenario: $scenario duration=${duration_seconds}s backend=$display_backend profile=$profile_mode"
fi
echo
summarize_refresh_gate
summarize_duration gtk_frame_interval
summarize_duration gtk_tick_wall_interval
summarize_duration gtk_tick_work
summarize_duration gtk_cursor_tick
summarize_duration gtk_smooth_scroll_tick
summarize_duration gtk_snapshot_dirty
summarize_duration gtk_snapshot_forced
summarize_duration gdk_refresh_interval
summarize_duration gdk_next_presentation_delta
summarize_duration glib_timeout_interval
summarize_duration gtk_paint
summarize_duration gtk_render
summarize_duration input_to_render
summarize_duration gtk_metrics
summarize_counter gdk_frame_clock_fps_millihz
summarize_counter gdk_monitor_refresh_millihz
summarize_counter gtk_render_allocs
summarize_counter gtk_render_alloc_bytes
summarize_counter gtk_paint_rows
summarize_counter gtk_layout_cache_hits
summarize_counter gtk_layout_cache_misses

if [[ -s "$scroll_trace" ]]; then
  awk -F '\t' '
    $1 == "frame" {
      frame_count++
      if ($2 == 0) pixel_only++
      if ($2 != 0) line_steps++
      durations[++duration_count] = $5
    }
    END {
      if (frame_count > 0) {
        asort(durations)
        p50 = durations[int((duration_count - 1) * 0.50) + 1]
        p95 = durations[int((duration_count - 1) * 0.95) + 1]
        printf "%-24s count=%-5d pixel_only=%-5d line_steps=%-5d p50=%7dus p95=%7dus\n", "scroll_frames", frame_count, pixel_only, line_steps, p50, p95
      }
    }
  ' "$scroll_trace"
fi

if [[ "$scenario" == "scroll-burst" ]]; then
  awk -F '\t' '
    $1 == "enqueue" {
      enqueue_count++
      if (frame_count == 0) enqueues_before_first_frame++
      if ($3 > max_pending_px) max_pending_px = $3
    }
    $1 == "frame" {
      frame_count++
      absolute_step = $2 < 0 ? -$2 : $2
      if (absolute_step > max_line_step) max_line_step = absolute_step
    }
    END {
      printf "%-24s enqueues=%-5d before_first_frame=%-5d max_pending_px=%7.2f max_line_step=%d\n", "scroll_burst", enqueue_count, enqueues_before_first_frame, max_pending_px, max_line_step
      if (enqueue_count < 8) {
        print "scroll-burst profile failed: did not record all wheel enqueues" > "/dev/stderr"
        exit 1
      }
      if (enqueues_before_first_frame < 4) {
        print "scroll-burst profile failed: wheel events did not coalesce before the first animation frame" > "/dev/stderr"
        exit 1
      }
      if (max_pending_px < 200.0) {
        printf "scroll-burst profile failed: pending target %.2fpx is too small to prove accumulated scrolling\n", max_pending_px > "/dev/stderr"
        exit 1
      }
      if (max_line_step < 2) {
        printf "scroll-burst profile failed: max line step %d did not accelerate with target distance\n", max_line_step > "/dev/stderr"
        exit 1
      }
    }
  ' "$scroll_trace"
fi

if [[ "$strict" -eq 1 ]]; then
  awk -F '\t' \
    -v frame_budget_us="$frame_budget_us" \
    -v target_refresh_millihz="$target_refresh_millihz" \
    -v require_monitor_refresh="$([[ "$display_backend" == "weston-headless" || "$display_backend" == "native-wayland" ]] && echo 1 || echo 0)" '
    $1 == "gtk_frame_interval" { frame[++frame_count] = $2 }
    $1 == "gtk_tick_wall_interval" { wall[++wall_count] = $2 }
    $1 == "gtk_paint" { paint[++paint_count] = $2 }
    $1 == "gdk_monitor_refresh_millihz" { monitor[++monitor_count] = $2 }
    END {
      if (require_monitor_refresh == 1) {
        if (monitor_count == 0) {
          print "strict profile failed: no gdk_monitor_refresh_millihz samples" > "/dev/stderr"
          exit 1
        }
        asort(monitor)
        monitor_max = monitor[monitor_count]
        if (monitor_max < target_refresh_millihz) {
          printf "strict profile failed: monitor refresh %.1fHz below required %.1fHz\n", monitor_max / 1000.0, target_refresh_millihz / 1000.0 > "/dev/stderr"
          exit 1
        }
      }
      if (frame_count == 0) {
        print "strict profile failed: no gtk_frame_interval samples" > "/dev/stderr"
        exit 1
      }
      asort(frame)
      frame_p50 = frame[int((frame_count - 1) * 0.50) + 1]
      if (frame_p50 > frame_budget_us) {
        printf "strict profile failed: gtk_frame_interval p50 %dus exceeds 240Hz budget %dus\n", frame_p50, frame_budget_us > "/dev/stderr"
        exit 1
      }
      if (wall_count == 0) {
        print "strict profile failed: no gtk_tick_wall_interval samples" > "/dev/stderr"
        exit 1
      }
      asort(wall)
      wall_p50 = wall[int((wall_count - 1) * 0.50) + 1]
      if (wall_p50 > frame_budget_us) {
        printf "strict profile failed: gtk_tick_wall_interval p50 %dus exceeds 240Hz budget %dus\n", wall_p50, frame_budget_us > "/dev/stderr"
        exit 1
      }
      if (paint_count > 0) {
        asort(paint)
        paint_p50 = paint[int((paint_count - 1) * 0.50) + 1]
        if (paint_p50 > frame_budget_us) {
          printf "strict profile failed: gtk_paint p50 %dus exceeds 240Hz budget %dus\n", paint_p50, frame_budget_us > "/dev/stderr"
          exit 1
        }
      }
    }
  ' "$perf_trace"
  if [[ "$scenario" == "scroll" || "$scenario" == "scroll-burst" ]]; then
    awk -F '\t' -v frame_budget_us="$frame_budget_us" '
      $1 == "frame" { scroll_frame[++scroll_frame_count] = $5 }
      END {
        if (scroll_frame_count == 0) {
          print "strict profile failed: no smooth scroll frame samples" > "/dev/stderr"
          exit 1
        }
        asort(scroll_frame)
        scroll_frame_p50 = scroll_frame[int((scroll_frame_count - 1) * 0.50) + 1]
        if (scroll_frame_p50 > frame_budget_us) {
          printf "strict profile failed: scroll_frames p50 %dus exceeds 240Hz budget %dus\n", scroll_frame_p50, frame_budget_us > "/dev/stderr"
          exit 1
        }
      }
    ' "$scroll_trace"
  fi
fi
