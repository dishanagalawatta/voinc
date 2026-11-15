#!/usr/bin/env bash
# .voinc/rebrand.sh - regenerate voinc branding from a pristine upstream tree.
#
# Branch model:
#   main  = pristine mirror of anomalyco/hex (fast-forward only, no local commits)
#   voinc = main + one generated rebrand commit (this script) + small hand patches
#           (.voinc/patches/*.patch, applied by the sync workflow)
#
# Regeneration used by .github/workflows/sync-upstream.yml:
#   git checkout -B voinc main
#   git checkout <previous voinc tip> -- .voinc    # restore this tooling
#   ./.voinc/rebrand.sh
#
# The script is idempotent: on an already-rebranded tree it changes nothing
# (except the README/NOTICE copies, which are overwritten from .voinc/).
set -euo pipefail
cd "$(dirname "$0")/.."

say() { printf '%s\n' "$*"; }

# ---------------------------------------------------------------------------
# 1. Rename tracked files whose names carry the product identity.
# ---------------------------------------------------------------------------
say "== file renames =="
renamed=0

# anomaly.hex* -> io.voinc.voinc* (desktop entries, icons, service units, ...)
while IFS= read -r -d '' f; do
  d="${f//anomaly.hex/io.voinc.voinc}"
  if [ "$f" != "$d" ] && [ ! -e "$d" ]; then
    mkdir -p "$(dirname "$d")"
    git mv "$f" "$d"
    say "  $f -> $d"
    renamed=$((renamed + 1))
  fi
done < <(git ls-files -z | grep -zZ 'anomaly\.hex' || true)

# term-hex-N.wav vocabulary fixtures -> term-voinc-N.wav
while IFS= read -r -d '' f; do
  d="${f//term-hex-/term-voinc-}"
  if [ "$f" != "$d" ] && [ ! -e "$d" ]; then
    mkdir -p "$(dirname "$d")"
    git mv "$f" "$d"
    say "  $f -> $d"
    renamed=$((renamed + 1))
  fi
done < <(git ls-files -z | grep -zZ 'term-hex-' || true)

# Explicit product-file renames (keep internal names like crates/capture/src/hex.rs).
while IFS='|' read -r src dst; do
  [ -z "$src" ] && continue
  if [ -e "$src" ] && [ ! -e "$dst" ]; then
    mkdir -p "$(dirname "$dst")"
    git mv "$src" "$dst"
    say "  $src -> $dst"
    renamed=$((renamed + 1))
  fi
done <<'EOF'
packaging/windows/hex.iss|packaging/windows/voinc.iss
skills/hex-personal-commands|skills/voinc-personal-commands
.github/release/hex.entitlements|.github/release/voinc.entitlements
.github/assets/hex-icon.png|.github/assets/voinc-icon.png
docs/plans/capture-in-hex.md|docs/plans/capture-in-voinc.md
EOF

# Any remaining product-named files at a path-segment start: hex.<ext>, hex-<name>
# (crates/capture's internal hex.rs module and historical plan images stay).
while IFS= read -r -d '' f; do
  case "$f" in
    crates/capture/*) continue ;;
    docs/plans/rune-ui/*) continue ;;
    .voinc/*) continue ;;
  esac
  d=$(printf '%s' "$f" | sed -E 's#(^|/)hex([.-])#\1voinc\2#g')
  if [ "$f" != "$d" ] && [ ! -e "$d" ]; then
    mkdir -p "$(dirname "$d")"
    git mv "$f" "$d"
    say "  $f -> $d"
    renamed=$((renamed + 1))
  fi
done < <(git ls-files -z | grep -zZ '(^|/)hex[.-]' || true)

say "  ($renamed files renamed)"

# ---------------------------------------------------------------------------
# 2. Content transforms on text files.
# ---------------------------------------------------------------------------
say "== content transforms =="

# Exclusions: upstream's site, release history, published SDK, locks, licenses.
# .github IS in scope so env names (HEX_* -> VOINC_*) stay consistent between
# workflows and build scripts; our sync patches replace workflows wholesale.
mapfile -d '' FILES < <(
  git ls-files -z |
    grep -zZv -E '(^|/)(site|release-notes|sdk|\.voinc)(/|$)' |
    grep -zZv -E '^(Cargo\.lock|flake\.lock|LICENSE|LICENSE\.*)$' |
    grep -zZv -E '^THIRD_PARTY_NOTICES\.md$' |
    grep -zZv -E '\.(png|PNG|icns|ico|jpg|jpeg|webp|gif|svgz|woff2?|ttf|otf|eot|mp3|wav|ogg|flac|m4a|mp4|mov|heic|bin|dat|der|node|so|dylib|a|rlib|pdf|zip|gz|tgz|br|7z|jar|cur|ani|psd|blend)$' || true
)

if [ "${#FILES[@]}" -gt 0 ]; then
  printf '%s\0' "${FILES[@]}" | xargs -0 -r perl -0777 -i -pe '
    my %m; my $n = 0;
    # --- protect (mask) tokens that must keep their exact upstream spelling ---
    s{<!-- voinc-fork-banner start -->.*?<!-- voinc-fork-banner end -->\n?}{ $m{"B".++$n}=$&; "\x01B$n\x01" }sge;  # banner text written by this script
    s{(github\.com/anomalyco/hex/(?:issues|pull|discussions)\S*)}{ $m{"P".++$n}=$1; "\x01P$n\x01" }ge;  # upstream conversation links
    s{anomalyco/hex}{dishanagalawatta/voinc}g;                             # our repo replaces theirs
    s{(github\.com/anomalyco\S*)}{ $m{"G".++$n}=$1; "\x01G$n\x01" }ge;      # upstream deps: rune, transcribe.cpp, taps
    s{([\w.-]*anoma\.ly\S*)}{ $m{"A".++$n}=$1; "\x01A$n\x01" }ge;           # upstream service URLs and prose
    s{(com\.kitlangton\.[hH]ex\d?(?:\.[\w-]+)*)}{ $m{"K".++$n}=$1; "\x01K$n\x01" }ge;  # legacy bundle ids
    s{(downloads\.hex\.kitlangton\.[\w.-]+)}{ $m{"D".++$n}=$1; "\x01D$n\x01" }ge;      # legacy download host
    s{(\@kitlangton/hex\b)}{ $m{"T".++$n}=$1; "\x01T$n\x01" }ge;            # upstream npm package
    s{(x-hex-[a-z-]+)}{ $m{"X".++$n}=$1; "\x01X$n\x01" }ge;                 # local API header names
    s{(X-Hex-[A-Za-z-]+)}{ $m{"Y".++$n}=$1; "\x01Y$n\x01" }ge;              # icon PNG text chunk
    s{\b(Hexagon\w*)\b}{ $m{"H".++$n}=$1; "\x01H$n\x01" }ge;                # shape words, not the product
    s{^(\s*(?:pub\s+)?mod\s+)hex(\s*;)}{ $m{"M".++$n}=$&; "\x01M$n\x01" }gme;          # capture crate module name
    s{^(use\s+crate::hex;)}{ $m{"U".++$n}=$1; "\x01U$n\x01" }gme;
    # --- renames, most specific first ---
    s{anomaly\.hex}{io.voinc.voinc}g;                                      # app id / bundle id / portal id
    s{anomaly-hex-identity}{voinc-identity}g;                              # settings migration key
    s{\bANOMALY_}{VOINC_}g;                                                # const prefix
    s{\bHEX_}{VOINC_}g;                                                    # env var prefix
    s{hex:notes}{voinc:notes}g;                                            # meeting Markdown marker
    s{hex://}{voinc://}g;                                                  # deep-link scheme
    s{\bHex\b}{Voinc}g;                                                    # display name
    s{\bHEX\b}{VOINC}g;                                                    # display name caps
    s{(?<![0-9A-Za-z_])hex(?![0-9A-Za-z_:])}{voinc}g;                      # binary name, paths, CLI, dirs
    # --- restore protected tokens ---
    s{\x01([ABGKPDTPXYHMU])(\d+)\x01}{ $m{$1.$2} }ge;
  '
fi
say "  (done: ${#FILES[@]} files)"

# ---------------------------------------------------------------------------
# 3. Voinc-owned files that replace upstream counterparts.
# ---------------------------------------------------------------------------
cp .voinc/README.md README.md
cp .voinc/NOTICE.md NOTICE.md
say "== copied .voinc/README.md and .voinc/NOTICE.md =="

# Fork banner at the top of AGENTS.md (idempotent). The markers delimit the
# block and the content transforms mask it, so a later run never rewrites the
# banner's own text.
if [ -f AGENTS.md ] && ! grep -q 'voinc-fork-banner start' AGENTS.md; then
  tmp=$(mktemp)
  {
    printf '<!-- voinc-fork-banner start -->\n'
    printf '# Voinc\n\n'
    printf '> **VOINC FORK NOTE.** Voinc is a community fork of Anomaly'"'"'s Hex (MIT).\n'
    printf '> `main` is a pristine mirror of upstream release tags; the `voinc` branch applies\n'
    printf '> `.voinc/rebrand.sh` plus `.voinc/patches/*`. The architecture and invariant\n'
    printf '> sections below describe the inherited codebase and still apply; sections about\n'
    printf '> Hex'"'"'s own development process (PRs to `dev`, signing, the release service,\n'
    printf '> goldens policy) describe upstream, not this fork.\n'
    printf '<!-- voinc-fork-banner end -->\n\n'
    cat AGENTS.md
  } > "$tmp"
  mv "$tmp" AGENTS.md
  say "== AGENTS.md fork banner added =="
fi

# ---------------------------------------------------------------------------
# 4. Residual report (review input; not a hard gate).
# ---------------------------------------------------------------------------
say "== residual brand tokens in scope (should be explainable) =="
git grep -n 'anomaly\.hex' -- ':!site' ':!release-notes' ':!sdk' ':!.voinc' ':!Cargo.lock' || true
git grep -nE '\bHex\b' -- ':!site' ':!release-notes' ':!sdk' ':!.voinc' ':!Cargo.lock' | head -20 || true
git grep -nE '\bhex\b' -- ':!site' ':!release-notes' ':!sdk' ':!.voinc' ':!Cargo.lock' | grep -vE 'hex_|hex::|Hexagon|kitlangton|x-hex-|X-Hex-|mod hex|use crate::hex' | head -30 || true

say "== rebrand complete =="
