#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
zig_dir="$root/.tools/zig-x86_64-linux-0.15.2"
zig_archive="$root/.tools/zig-0.15.2.tar.xz"

if [[ ! -x "$zig_dir/zig" ]]; then
  mkdir -p "$root/.tools"
  curl -L "https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz" -o "$zig_archive"
  tar -xf "$zig_archive" -C "$root/.tools"
fi

PATH="$zig_dir:$PATH" "$@"
