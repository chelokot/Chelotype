#!/usr/bin/env bash
set -euo pipefail

manifest=com.chelokot.Chelotype.yml
zig_sources=zig-sources.json
extra_sources=packaging/flatpak/zig-extra-sources.json

sys_version="$(awk '$0 == "name = \"libghostty-vt-sys\"" { getline; gsub(/"/, "", $3); print $3 }' Cargo.lock)"
commit="$(curl -fsSL "https://static.crates.io/crates/libghostty-vt-sys/libghostty-vt-sys-${sys_version}.crate" |
  tar -xzO "libghostty-vt-sys-${sys_version}/build.rs" |
  sed -n 's/^const GHOSTTY_COMMIT: &str = "\([0-9a-f]\{40\}\)";$/\1/p')"
archive_url="https://codeload.github.com/ghostty-org/ghostty/tar.gz/${commit}"
archive_sha256="$(curl -fsSL "$archive_url" | sha256sum | cut -d' ' -f1)"

packages="$(curl -fsSL "https://raw.githubusercontent.com/ghostty-org/ghostty/${commit}/flatpak/zig-packages.json")"
git_archives="$(jq -c '.[] | select(.type == "git")' <<<"$packages" | while read -r source; do
  url="$(jq -r '.url' <<<"$source")"
  revision="$(jq -r '.commit' <<<"$source")"
  archive="${url}/archive/${revision}.tar.gz"
  jq -c --arg url "$archive" --arg sha256 "$(curl -fsSL "$archive" | sha256sum | cut -d' ' -f1)" \
    '{type: "archive", url: $url, sha256: $sha256, dest: .dest}' <<<"$source"
done | jq -s '.')"

generated="$(jq --indent 4 --argjson git "$git_archives" --slurpfile extra "$extra_sources" '
  [.[] | select(.type == "archive") | {type, url, sha256, dest}] + $git + $extra[0]
  | map(.dest |= sub("^vendor/p/"; "zig-cache/p/"))' <<<"$packages")"

updated_manifest="$(sed \
  -e "s|^\(        url: https://codeload.github.com/ghostty-org/ghostty/tar.gz/\)[0-9a-f]\{40\}$|\1${commit}|" \
  -e "/codeload.github.com\/ghostty-org\/ghostty/{n;s|^\(        sha256: \)[0-9a-f]\{64\}$|\1${archive_sha256}|}" \
  "$manifest")"

if [[ "${1:-}" == "--check" ]]; then
  if ! cmp -s <(printf '%s\n' "$generated") "$zig_sources" || ! cmp -s <(printf '%s\n' "$updated_manifest") "$manifest"; then
    printf 'Flatpak Ghostty sources do not match libghostty-vt-sys %s (Ghostty %s); run scripts/update-ghostty.sh\n' "$sys_version" "$commit" >&2
    exit 1
  fi
  printf 'Flatpak Ghostty sources match libghostty-vt-sys %s (Ghostty %s)\n' "$sys_version" "$commit"
  exit 0
fi

printf '%s\n' "$generated" >"$zig_sources"
printf '%s\n' "$updated_manifest" >"$manifest"
printf 'Pinned Ghostty %s for libghostty-vt-sys %s\n' "$commit" "$sys_version"
