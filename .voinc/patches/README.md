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
| `P2-own-workflows.patch` | Replaces upstream's CI with this fork's: adds `.github/workflows/sync-upstream.yml` (daily cron, also dispatchable: finds the newest upstream `vX.Y.Z` tag, fast-forwards `main`, regenerates the `voinc` branch, opens a PR) and deletes upstream-only workflows (`release.yml`, `deploy.yml`, `test-builds.yml`, `release-typescript.yml`, `aur-check.yml`, `windows-installer*.yml`). Kept as-is: `compile.yml` (PR compile check), `check-{linux,macos,windows}.yml`, `build-desktop.yml` (workflow_call, for a later release workflow), `agent-policy.yml`. The repository's default branch must be `voinc`: scheduled workflows read the default branch's copy of the file, and the pristine `main` mirror carries no workflows. |
