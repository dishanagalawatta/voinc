# Contributing to Voinc

Voinc is a community fork of [Hex](https://github.com/anomalyco/hex) that
tracks upstream stable releases: `main` mirrors upstream tags, and the
`voinc` branch applies `.voinc/rebrand.sh` and `.voinc/patches/` to them
automatically.

- **Product behavior** — bugs and ideas about the app itself belong
  upstream: [open an issue with Anomaly](https://github.com/anomalyco/hex/issues/new/choose).
  Fixes land in Hex and reach Voinc through the automatic sync, so a change
  made only here would be overwritten by the next release.
- **Fork mechanics** — the rebrand script, patches, workflows, and packaging:
  pull requests are welcome against the `voinc` branch.
- **Diagnoses** — redacted logs, the code you suspect, or a patch as a diff
  snippet are welcome in either tracker, and credit follows the fix.

To build Voinc yourself, see [`AGENTS.md`](AGENTS.md) and [`docs/`](docs/README.md).
