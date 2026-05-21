#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
original_args=("$@")
backend="mutter"
release=0
build=1
capture_duration="4.25"

usage() {
  cat <<'EOF'
Usage: scripts/capture-flathub-media.sh [--backend mutter|x11] [--release] [--no-build]

Captures Flathub/README media:
- data/screenshots/chelotype-preferences.webp
- docs/media/chelotype-preferences.webm

The default backend is a nested headless Wayland compositor:
host Mutter + virtual monitor + Mutter ScreenCast/PipeWire.
Use --backend x11 only as a fallback.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --backend)
      backend="${2:-}"
      shift 2
      ;;
    --release)
      release=1
      shift
      ;;
    --no-build)
      build=0
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

case "$backend" in
  mutter|x11) ;;
  *)
    echo "unsupported backend: $backend" >&2
    usage >&2
    exit 2
    ;;
esac

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required to capture Chelotype media" >&2
    exit 2
  fi
}

require_host_command() {
  if ! distrobox-host-exec sh -lc "command -v '$1' >/dev/null 2>&1"; then
    echo "host $1 is required to capture Chelotype media with the mutter backend" >&2
    exit 2
  fi
}

if [[ "$backend" == "x11" && "${CHELOTYPE_MEDIA_UNDER_XVFB:-0}" != 1 ]]; then
  require_command xvfb-run
  exec xvfb-run -a -s "-screen 0 1920x1080x24 -nolisten tcp" \
    env CHELOTYPE_MEDIA_UNDER_XVFB=1 "$0" "${original_args[@]}"
fi

require_command find
require_command ffmpeg
require_command perl

if [[ "$backend" == "mutter" ]]; then
  require_command dbus-daemon
  require_command distrobox-host-exec
  require_command python3
  require_host_command mutter
  require_host_command gst-launch-1.0
  require_host_command timeout
else
  require_command xdotool
fi

target_profile="debug"
cargo_build_args=(build)
if [[ "$release" -eq 1 ]]; then
  target_profile="release"
  cargo_build_args+=(--release)
fi

if [[ "$build" -eq 1 ]]; then
  "$root/scripts/with-zig.sh" cargo "${cargo_build_args[@]}"
fi

bin="$root/target/$target_profile/chelotype"
if [[ ! -x "$bin" ]]; then
  echo "Chelotype binary was not found at $bin" >&2
  exit 1
fi

libdir="$(dirname "$(find "$root/target/$target_profile/build" -path '*/ghostty-install/lib/libghostty-vt.so.0' -print -quit)")"
if [[ -z "$libdir" || ! -d "$libdir" ]]; then
  echo "libghostty-vt build output was not found for $target_profile" >&2
  exit 1
fi

output_screenshot="$root/data/screenshots/chelotype-preferences.webp"
output_video="$root/docs/media/chelotype-preferences.webm"
mkdir -p "$(dirname "$output_screenshot")" "$(dirname "$output_video")"

workdir="$(mktemp -d)"
cleanup() {
  if [[ -n "${app_pid:-}" ]]; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  if [[ -n "${mutter_pid:-}" ]]; then
    kill "$mutter_pid" 2>/dev/null || true
    wait "$mutter_pid" 2>/dev/null || true
  fi
  if [[ -n "${bus_pid:-}" ]]; then
    kill "$bus_pid" 2>/dev/null || true
  fi
  rm -rf "$workdir"
}
trap cleanup EXIT

config_dir="$workdir/config"
mkdir -p "$config_dir"
cat > "$config_dir/config" <<'EOF'
startup_launch_target=host
cursor_shape=bar
cursor_style=neovide
cursor_animation_duration_ms=100
smooth_scrolling=true
EOF

media_shell="$workdir/chelotype-media-shell"
prompt_path="$root"
home_candidates=("${HOME:-}" "$(getent passwd "$(id -un)" | cut -d: -f6 || true)" "/var/home/$(id -un)")
for home_candidate in "${home_candidates[@]}"; do
  if [[ -n "$home_candidate" && "$prompt_path" == "$home_candidate"/* ]]; then
    prompt_path="~/${prompt_path#"$home_candidate"/}"
    break
  fi
done
cat > "$media_shell" <<EOF
#!/usr/bin/env bash
export PS1='\\[\\e[38;2;126;211;252m\\]$prompt_path\\[\\e[0m\\] on \\[\\e[38;2;203;166;247m\\]main\\[\\e[0m\\] is \\[\\e[38;2;251;146;60m\\]v0.1.0\\[\\e[0m\\] via rust 1.95.0\n> '
printf '\033]0;fedora-toolbox\007'
exec /bin/bash --noprofile --norc -i
EOF
chmod +x "$media_shell"

write_readme_media_block() {
  readme_block="$workdir/readme-media.md"
  cat > "$readme_block" <<'EOF'
<!-- chelotype-media-start -->
<p align="center">
  <video src="docs/media/chelotype-preferences.webm" autoplay loop muted playsinline controls width="960"></video>
</p>
<!-- chelotype-media-end -->
EOF

  readme_tmp="$workdir/README.md"
  awk -v block_file="$readme_block" '
  BEGIN {
    while ((getline line < block_file) > 0) {
      block = block line "\n"
    }
  }
  /^<!-- chelotype-media-start -->$/ {
    if (!inserted) {
      printf "%s", block
      inserted = 1
    }
    skipping = 1
    next
  }
  /^<!-- chelotype-media-end -->$/ {
    skipping = 0
    after_block = 1
    next
  }
  skipping {
    next
  }
  after_block && /^[[:space:]]*$/ {
    next
  }
  after_block {
    print ""
    after_block = 0
  }
  NR == 1 {
    print
    if (!inserted) {
      print ""
      printf "%s", block
      print ""
      inserted = 1
    }
    next
  }
  {
    print
  }
  ' "$root/README.md" > "$readme_tmp"
  mv "$readme_tmp" "$root/README.md"
  perl -0pi -e 's/(<!-- chelotype-media-end -->)\n+/$1\n\n/s' "$root/README.md"
}

capture_x11() {
  export LD_LIBRARY_PATH="$libdir:${LD_LIBRARY_PATH:-}"
  export GDK_BACKEND=x11
  export GSETTINGS_BACKEND=memory
  export NO_AT_BRIDGE=1
  export ADW_DEBUG_COLOR_SCHEME=prefer-dark
  export CHELOTYPE_CONFIG_DIR="$config_dir"
  export CHELOTYPE_SHELL="$media_shell"
  export CHELOTYPE_MEDIA_OPEN_PREFERENCES=1

  "$bin" &
  app_pid="$!"

  app_window=""
  for _ in {1..100}; do
    app_window="$(xdotool search --name 'Chelotype Terminal' | head -n 1 || true)"
    if [[ -n "$app_window" ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ -z "$app_window" ]]; then
    echo "Chelotype window did not appear" >&2
    exit 1
  fi

  xdotool windowsize "$app_window" 1760 990
  xdotool windowmove "$app_window" 80 45

  settings_window=""
  for _ in {1..100}; do
    settings_window="$(xdotool search --name 'Settings' | head -n 1 || true)"
    if [[ -n "$settings_window" ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ -z "$settings_window" ]]; then
    echo "Chelotype settings window did not appear" >&2
    exit 1
  fi

  xdotool windowsize "$settings_window" 960 960
  xdotool windowmove "$settings_window" 480 60
  xdotool mousemove 1910 1070
  sleep 0.8

  display_input="$DISPLAY"
  if [[ "$display_input" != *.* ]]; then
    display_input="$display_input.0"
  fi
  display_input="$display_input+0,0"

  ffmpeg -y -hide_banner -loglevel warning \
    -f x11grab -draw_mouse 0 -framerate 1 -video_size 1920x1080 \
    -i "$display_input" -frames:v 1 \
    -c:v libwebp -lossless 1 -compression_level 6 "$output_screenshot"

  ffmpeg -y -hide_banner -loglevel warning \
    -f x11grab -draw_mouse 0 -framerate 30 -video_size 1920x1080 \
    -i "$display_input" -t "$capture_duration" \
    -c:v libvpx-vp9 -pix_fmt yuv444p -b:v 0 -crf 6 "$output_video"
}

write_pipewire_capture_helper() {
  pipewire_capture="$workdir/capture-pipewire.py"
  cat > "$pipewire_capture" <<'PY'
import os
import subprocess
import sys

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLib", "2.0")
from gi.repository import Gio, GLib

bus_address, mode, output_path, duration = sys.argv[1:5]
connection = Gio.DBusConnection.new_for_address_sync(
    bus_address,
    Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT
    | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
    None,
    None,
)
flags = Gio.DBusCallFlags.NONE
session_path = connection.call_sync(
    "org.gnome.Mutter.ScreenCast",
    "/org/gnome/Mutter/ScreenCast",
    "org.gnome.Mutter.ScreenCast",
    "CreateSession",
    GLib.Variant("(a{sv})", ({},)),
    GLib.VariantType.new("(o)"),
    flags,
    -1,
    None,
).unpack()[0]
stream_path = connection.call_sync(
    "org.gnome.Mutter.ScreenCast",
    session_path,
    "org.gnome.Mutter.ScreenCast.Session",
    "RecordArea",
    GLib.Variant("(iiiia{sv})", (0, 0, 1920, 1080, {})),
    GLib.VariantType.new("(o)"),
    flags,
    -1,
    None,
).unpack()[0]

loop = GLib.MainLoop()
node_id = {"value": None}


def on_signal(_connection, _sender, _path, _interface, signal, parameters):
    if signal == "PipeWireStreamAdded":
        node_id["value"] = parameters.unpack()[0]
        loop.quit()


connection.signal_subscribe(
    "org.gnome.Mutter.ScreenCast",
    "org.gnome.Mutter.ScreenCast.Stream",
    "PipeWireStreamAdded",
    stream_path,
    None,
    Gio.DBusSignalFlags.NONE,
    on_signal,
)
connection.call_sync(
    "org.gnome.Mutter.ScreenCast",
    session_path,
    "org.gnome.Mutter.ScreenCast.Session",
    "Start",
    None,
    None,
    flags,
    -1,
    None,
)
GLib.timeout_add(3000, loop.quit)
loop.run()
if node_id["value"] is None:
    raise SystemExit("Mutter ScreenCast did not expose a PipeWire node")

if mode == "webp":
    command = [
        "distrobox-host-exec",
        "timeout",
        "5s",
        "gst-launch-1.0",
        "-q",
        "-e",
        "pipewiresrc",
        f"path={node_id['value']}",
        "do-timestamp=true",
        "num-buffers=1",
        "!",
        "videoconvert",
        "!",
        "webpenc",
        "lossless=true",
        "quality=100",
        "speed=6",
        "preset=text",
        "!",
        "filesink",
        f"location={output_path}",
    ]
else:
    record_timeout = f"{float(duration)}s"
    command = [
        "distrobox-host-exec",
        "timeout",
        "--signal=INT",
        "--kill-after=2s",
        record_timeout,
        "gst-launch-1.0",
        "-q",
        "-e",
        "pipewiresrc",
        f"path={node_id['value']}",
        "do-timestamp=true",
        "!",
        "videorate",
        "!",
        "video/x-raw,framerate=30/1",
        "!",
        "videoconvert",
        "!",
        "video/x-raw,format=Y444",
        "!",
        "vp9enc",
        "deadline=1",
        "cpu-used=4",
        "target-bitrate=0",
        "end-usage=cq",
        "cq-level=0",
        "min-quantizer=0",
        "max-quantizer=0",
        "static-threshold=100",
        "row-mt=true",
        "threads=8",
        "!",
        "webmmux",
        "!",
        "filesink",
        f"location={output_path}",
    ]

environment = os.environ.copy()
environment.pop("DBUS_SESSION_BUS_ADDRESS", None)
result = subprocess.run(command, env=environment)
if result.returncode not in (0, 124):
    raise SystemExit(result.returncode)

connection.call_sync(
    "org.gnome.Mutter.ScreenCast",
    session_path,
    "org.gnome.Mutter.ScreenCast.Session",
    "Stop",
    None,
    None,
    flags,
    -1,
    None,
)
PY
}

capture_mutter() {
  write_pipewire_capture_helper

  bus_path="$workdir/session-bus"
  bus_pid="$(dbus-daemon --session --fork --address="unix:path=$bus_path" --print-pid)"
  bus_address="unix:path=$bus_path"
  wayland_display="chelotype-media-$$"
  mutter_log="$workdir/mutter.log"
  runtime_dir="/run/user/$(id -u)"

  distrobox-host-exec env \
    DBUS_SESSION_BUS_ADDRESS="$bus_address" \
    LD_LIBRARY_PATH="$libdir:${LD_LIBRARY_PATH:-}" \
    GDK_BACKEND=wayland \
    WAYLAND_DISPLAY="$wayland_display" \
    XDG_RUNTIME_DIR="$runtime_dir" \
    GSETTINGS_BACKEND=memory \
    NO_AT_BRIDGE=1 \
    ADW_DEBUG_COLOR_SCHEME=prefer-dark \
    CHELOTYPE_CONFIG_DIR="$config_dir" \
    CHELOTYPE_SHELL="$media_shell" \
    CHELOTYPE_MEDIA_OPEN_PREFERENCES=1 \
    CHELOTYPE_MEDIA_PREFERENCES_HEIGHT=890 \
    mutter --headless --virtual-monitor 1920x1080 --wayland --no-x11 \
      --wayland-display "$wayland_display" -- "$bin" \
      >"$mutter_log" 2>&1 &
  mutter_pid="$!"

  for _ in {1..120}; do
    if grep -q "Using Wayland display name" "$mutter_log"; then
      break
    fi
    sleep 0.25
  done
  if ! grep -q "Using Wayland display name" "$mutter_log"; then
    echo "Mutter did not start a Wayland display" >&2
    sed -n '1,120p' "$mutter_log" >&2
    exit 1
  fi

  sleep 4
  python3 "$pipewire_capture" "$bus_address" webp "$output_screenshot" "$capture_duration"
  python3 "$pipewire_capture" "$bus_address" webm "$output_video" "$capture_duration"
}

if [[ "$backend" == "mutter" ]]; then
  capture_mutter
else
  capture_x11
fi

write_readme_media_block

printf 'Wrote %s\n' "$output_screenshot"
printf 'Wrote %s\n' "$output_video"
