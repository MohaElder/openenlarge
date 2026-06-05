#!/usr/bin/env bash
# Regenerate icons/Assets.car from the Icon Composer source (app-logo.icon).
# Requires Xcode 26+ (provides actool). Assets.car carries the macOS 26 "Tahoe"
# Liquid Glass icon; Tauri injects it into Contents/Resources via bundle.macOS.files,
# and Info.plist's CFBundleIconName=app-logo selects it on Tahoe. Older macOS falls
# back to the legacy icon.icns (CFBundleIconFile).
set -euo pipefail
cd "$(dirname "$0")/.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

xcrun actool app-logo.icon \
  --compile "$tmp" \
  --app-icon app-logo \
  --output-partial-info-plist "$tmp/partial.plist" \
  --platform macosx \
  --minimum-deployment-target 26.0 \
  --errors --warnings

cp "$tmp/Assets.car" icons/Assets.car
echo "wrote icons/Assets.car"
