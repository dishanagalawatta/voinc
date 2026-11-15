# Patches

Hand-made source changes applied after `.voinc/rebrand.sh` on every upstream
sync. The sync workflow regenerates the `voinc` branch as:

1. `git checkout -B voinc main`
2. restore `.voinc/` from the previous `voinc` tip
3. `./.voinc/rebrand.sh` (mechanical branding)
4. `git apply .voinc/patches/*.patch` (these files, in name order)

Generating a patch: make the change by hand on the rebranded tree, then

```sh
git diff -- <paths> > .voinc/patches/<name>.patch
git checkout -- <paths>
git apply .voinc/patches/<name>.patch   # verify it round-trips
```

| Patch | What it does |
| --- | --- |
| `P1-disable-upstream-updater.patch` | The inherited self-updater checks Anomaly's release service manifest, which serves Hex releases. Its checks now stay off unless `VOINC_RELEASE_BASE_URL` (HTTPS only) names an origin that serves a Voinc manifest; without it the app reports updates unavailable and updates arrive through GitHub releases instead. |
| `P2-own-workflows.patch` | Replaces upstream's CI with this fork's: adds `.github/workflows/sync-upstream.yml` (daily cron, also dispatchable: finds the newest upstream `vX.Y.Z` tag, fast-forwards `main`, regenerates the `voinc` branch, opens a PR) and deletes upstream-only workflows (`release.yml`, `deploy.yml`, `test-builds.yml`, `release-typescript.yml`, `aur-check.yml`, `windows-installer*.yml`). Kept as-is: `compile.yml` (PR compile check), `check-{linux,macos,windows}.yml`, `build-desktop.yml` (workflow_call, reused by P3), `agent-policy.yml`. The repository's default branch must be `voinc`: scheduled workflows read the default branch's copy of the file, and the pristine `main` mirror carries no workflows. |
| `P3-release-workflow.patch` | Adds `.github/workflows/release.yml` and widens `scripts/set-crate-version.sh`. The release workflow publishes one GitHub release per tag `v<upstream>-voinc.<n>` (e.g. `v3.4.0-voinc.1`): `plan` validates the tag and resolves the commit it points at (peels annotated tags); `build` reuses `build-desktop.yml` for Linux x64/ARM64 (executables + `.deb` + `.rpm`) and Windows x64/ARM64 (`.exe`/`.com`); `build-macos` builds an ad-hoc-signed `Voinc.app` on `macos-26` (unsigned v1: no Developer ID, no DMG, no sign job); `publish` downloads every artifact, verifies the digests its jobs reported, renames the app zip to `voinc-<version>-macos-arm64.zip`, writes one `SHA256SUMS` over all assets, and runs `gh release create --latest` with notes covering the unsigned builds (Gatekeeper/SmartScreen), the upstream release link, and Hugging Face model downloads. `workflow_dispatch` rehearses: it builds the dispatched commit and publishes only when the `publish` input is set (then it targets the built commit when the tag does not exist yet). `set-crate-version.sh` accepts the fork's `X.Y.Z-voinc.N` (and keeps `0.0.0-dev-N` and upstream's `X.Y.Z-beta.N`); without this the release builds could not stamp their version, and the strict-semver counter (`0|[1-9][0-9]*`) rejects leading zeros. The binaries stay unsigned on Windows (no Authenticode); the updater itself stays off (P1) since no manifest origin is configured. |
| `P4-fork-identity.patch` | Fork-facing metadata the mechanical rebrand deliberately leaves pointing upstream (it protects anoma.ly URLs, the word "Anomaly", and upstream conversation links): `packaging/linux/nfpm.yaml` (maintainer and vendor are the fork, homepage is the fork repository, and the package-managed-install comment no longer promises an update command from the release service); `packaging/windows/voinc.iss` (a fresh AppId GUID so the fork never collides with an installed upstream Hex in Apps & Features, fork publisher, and fork support URL); `packaging/aur/PKGBUILD` (fork maintainer, description names the community fork and disambiguates against the AUR `hex` dump tool, and the comment says the release's asset URLs instead of the release service's); `.github/ISSUE_TEMPLATE/config.yml` (the contact link points at the fork README, and the `blob/dev/` links — a branch this fork does not have — point at `voinc`); `CONTRIBUTING.md` (fork policy: product behavior goes upstream to Anomaly, fork mechanics take PRs against `voinc`). |

The fork's brand assets are not a patch: `.voinc/rebrand.sh` section 3b copies
the logo and the derived icon set (Linux hicolor sizes, Windows `.ico`/glyph
PNGs/tray raster, the macOS Icon Composer layer, `.github/assets/voinc-icon.png`)
from `.voinc/assets/` over upstream's artwork on every sync. Derivation rules
and the regeneration recipe live in [`.voinc/assets/README.md`](../assets/README.md).

## Deferred (not patched yet)

- `.github/release/install-linux.sh` and `scripts/release-aur.sh` /
  `packaging/homebrew/voinc.rb` still read the upstream release service
  (anoma.ly manifest and ed25519 signature). Nothing in the fork calls them
  today (P2 deleted the workflows that did); rewrite them for GitHub
  releases before advertising a one-line install or an AUR/Homebrew package.
- The Inno Setup installer is not wired to any workflow in v1; P4 fixed its
  identity for later.
- macOS/Windows code signing, and a P1-style updater origin if the fork ever
  serves a manifest.
