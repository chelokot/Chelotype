#!/bin/sh
set -eu

prefix="${PREFIX:-/usr}"
destdir="${DESTDIR:-}"
app_id="${APP_ID:-com.chelokot.Chelotype}"
license_root="$destdir$prefix/share/licenses/$app_id"

install_license_files() {
    group="$1"
    source_root="$2"

    [ -d "$source_root" ] || return 0

    find "$source_root" -type f \( \
        -iname 'LICENSE*' -o \
        -iname 'COPYING*' -o \
        -iname 'NOTICE*' \
    \) | while IFS= read -r license_file; do
        relative_path="${license_file#"$source_root"/}"
        install -Dm644 "$license_file" "$license_root/$group/$relative_path"
    done
}

install_license_files cargo cargo/vendor
install_license_files ghostty ghostty-src
install_license_files zig-packages zig-cache/p
