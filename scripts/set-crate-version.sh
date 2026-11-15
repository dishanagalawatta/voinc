#!/bin/sh
# Stamps the version the release service publishes into Cargo.toml and
# Cargo.lock, so the binaries report it: a dev build's 0.0.0-dev-<run number>
# (as OpenCode numbers its dev builds), a prerelease X.Y.Z-beta.N of the
# checked-out version, or the fork's release X.Y.Z-voinc.N (the
# v<upstream>-voinc.<n> tag). Release builds only; the change is never
# committed.
#
#   set-crate-version.sh <source root> <version>
set -eu

root=$1
version=$2
current=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)
[ "$version" = "$current" ] && exit 0
current_esc=$(printf '%s' "$current" | sed 's/\./\\./g')
printf '%s\n' "$version" | grep -Eq "^(0\.0\.0-dev-[1-9][0-9]*|${current_esc}-beta\.(0|[1-9][0-9]*)|${current_esc}-voinc\.(0|[1-9][0-9]*))\$" || {
  echo "Can only stamp 0.0.0-dev-<run>, a $current-beta.N prerelease, or a $current-voinc.N fork release, not $version." >&2
  exit 1
}
# awk, not sed's GNU-only 0,/re/ address: this also runs on the macOS runner.
awk -v current="$current" -v version="$version" '
  !done && $0 == "version = \"" current "\"" { $0 = "version = \"" version "\""; done = 1 }
  { print }
' "$root/Cargo.toml" > "$root/Cargo.toml.next"
mv "$root/Cargo.toml.next" "$root/Cargo.toml"
awk -v current="$current" -v version="$version" '
  /^name = "voinc"$/ { package = 1 }
  package && $0 == "version = \"" current "\"" { $0 = "version = \"" version "\""; package = 0 }
  { print }
' "$root/Cargo.lock" > "$root/Cargo.lock.next"
mv "$root/Cargo.lock.next" "$root/Cargo.lock"
grep -q "^version = \"$version\"" "$root/Cargo.toml"
echo "Building Voinc as $version." >&2
