#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "Usage: $0 OUTPUT_DIR BASE_URL [PUBLIC_KEY_FILE]" >&2
  exit 2
fi

output_dir="$1"
base_url="${2%/}"
public_key_file="${3:-packaging/flatpak/chelotype-flatpak-public.gpg}"
repo_url="$base_url/repo/"
gpg_key="$(base64 -w0 "$public_key_file")"
icon_url="$base_url/com.chelokot.Chelotype.svg"

install -d "$output_dir"
install -Dm644 data/icons/hicolor/scalable/apps/com.chelokot.Chelotype.svg \
  "$output_dir/com.chelokot.Chelotype.svg"

cat > "$output_dir/chelotype.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Chelotype
Url=$repo_url
Homepage=https://github.com/chelokot/Chelotype
Comment=Opinionated GTK terminal
Description=Chelotype is an opinionated GTK terminal for Linux desktops.
Icon=$icon_url
GPGKey=$gpg_key
EOF

cat > "$output_dir/com.chelokot.Chelotype.flatpakref" <<EOF
[Flatpak Ref]
Name=com.chelokot.Chelotype
Branch=stable
Title=Chelotype
Url=$repo_url
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
GPGKey=$gpg_key
EOF
