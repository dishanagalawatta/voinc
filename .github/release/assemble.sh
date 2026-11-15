#!/usr/bin/env bash
# assemble.sh <in> <dist>: gathers the release files from the verified
# artifacts in <in>/<name>/ into <dist>, refusing any file that is not one of
# the release service targets for VOINC_VERSION (docs/plans/release-service.md)
# or that two artifacts both hold.
. "$(dirname "$0")/lib.sh"

in=$(posix "$1")
dist=$(posix "$2")
v=$VOINC_VERSION
mkdir -p "$dist"
for file in "$in"/*/*; do
  name=$(basename "$file")
  [ "$name" = SHA256SUMS ] && continue
  case "$name" in
    "voinc-$v-linux-x64" | "voinc-$v-linux-x64.deb" | "voinc-$v-linux-x64.rpm" | \
      "voinc-$v-linux-arm64" | "voinc-$v-linux-arm64.deb" | "voinc-$v-linux-arm64.rpm" | \
      "voinc-$v-macos-arm64.zip" | "voinc-$v-macos-arm64.dmg" | \
      "voinc-$v-windows-x64.exe" | "voinc-$v-windows-arm64.exe" | \
      "voinc-$v-windows-x64.com" | "voinc-$v-windows-arm64.com" | \
      "voinc-setup-$v-windows-x64.exe" | "voinc-setup-$v-windows-arm64.exe") ;;
    *) die "$name is not a release file of Voinc $v." ;;
  esac
  [ ! -e "$dist/$name" ] || die "Two artifacts hold $name."
  cp "$file" "$dist/$name"
done
(cd "$dist" && ls -l && sha256 -- *) | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
