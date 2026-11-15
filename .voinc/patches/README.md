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
