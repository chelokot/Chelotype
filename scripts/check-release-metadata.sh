#!/usr/bin/env bash
set -euo pipefail

cargo_version="$(sed -n 's/^version = "\([0-9.]*\)"$/\1/p' Cargo.toml | head -n1)"
fedora_version="$(sed -n 's/^Version:[[:space:]]*\([0-9.]*\)$/\1/p' packaging/fedora/chelotype.spec.in)"
metainfo_version="$(sed -n 's/.*<release version="\([0-9.]*\)".*/\1/p' data/com.chelokot.Chelotype.metainfo.xml | head -n1)"
flatpak_source_commit="$(sed -n 's/^        commit: \([0-9a-f]\{40\}\)$/\1/p' com.chelokot.Chelotype.yml)"

if ! [[ "$cargo_version" == "$fedora_version" && "$cargo_version" == "$metainfo_version" ]]; then
  printf 'Release metadata mismatch: Cargo=%s Fedora=%s AppStream=%s\n' \
    "$cargo_version" "$fedora_version" "$metainfo_version" >&2
  exit 1
fi

git cat-file -e "${flatpak_source_commit}^{commit}"
for source_file in Cargo.toml Cargo.lock cargo-sources.json; do
  if ! cmp -s <(git show "${flatpak_source_commit}:${source_file}") "$source_file"; then
    printf 'Flatpak source %s does not match %s\n' "$flatpak_source_commit" "$source_file" >&2
    exit 1
  fi
done

printf 'Release metadata synchronized at %s from %s\n' "$cargo_version" "$flatpak_source_commit"
