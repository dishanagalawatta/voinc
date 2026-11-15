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
