#!/usr/bin/env bash
set -euo pipefail

prefix="${PREFIX:-/usr}"
destdir="${DESTDIR:-}"
datadir="$destdir$prefix/share"
app_id="com.chelokot.Chelotype"

install -Dm644 "data/$app_id.desktop" "$datadir/applications/$app_id.desktop"
install -Dm644 "data/$app_id.metainfo.xml" "$datadir/metainfo/$app_id.metainfo.xml"
install -Dm644 "data/icons/hicolor/scalable/apps/$app_id.svg" "$datadir/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 "data/icons/hicolor/symbolic/apps/$app_id-symbolic.svg" "$datadir/icons/hicolor/symbolic/apps/$app_id-symbolic.svg"
