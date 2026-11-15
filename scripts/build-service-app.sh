#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
team_id=${VOINC_TEAM_ID:?Set VOINC_TEAM_ID to the Apple Developer signing team}
. "$root/scripts/macos-signing.sh"
identity=$(hex_codesign_identity "$team_id")
bundle="$root/target/VOINC Service.app"
executable="$bundle/Contents/MacOS/voinc-service"
version=$(hex_version "$root")
build_number=$(hex_build_number "$version")

cargo build --release --manifest-path "$root/Cargo.toml"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS"
cp "$root/target/release/voinc" "$executable"
cp "$root/app/ServiceInfo.plist" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build_number" "$bundle/Contents/Info.plist"

codesign --force --timestamp --options runtime --sign "$identity" "$bundle"
codesign --verify --deep --strict --verbose=2 "$bundle"

echo "$bundle"
