# Test Builds

Every push to `dev` runs the [Test builds](https://github.com/dishanagalawatta/voinc/actions/workflows/test-builds.yml)
workflow (`.github/workflows/test-builds.yml`; run it by hand with **Run
workflow**). Open the latest green run: its summary links each download.
Artifacts are kept 14 days, and downloading one requires a GitHub login with
access to the repository.

```ts
voinc-test-<commit>-macos-arm64       // voinc-test-<commit>-arm64.dmg
voinc-test-<commit>-windows-x86_64    // voinc-setup-<version>-windows-x64.exe + bare voinc-<version>-windows-x64.exe
voinc-test-<commit>-windows-aarch64   // voinc-setup-<version>-windows-arm64.exe + bare voinc-<version>-windows-arm64.exe
voinc-test-<commit>-linux-x86_64      // voinc-<version>-linux-x64
```

None of these are code-signed, notarized, or published. They never update
themselves, so the signed release cannot replace the build you are testing.
GitHub wraps each artifact in a zip; unzip it first.

## macOS (Apple silicon, macOS 15 or later)

The test app is **Voinc Test.app** (bundle ID `io.voinc.voinc.test`),
ad-hoc signed and never checks for updates. Its own name and bundle ID let it sit
beside a released `Voinc.app` without touching that app's permissions, login
item, or updates. It also keeps its own data: settings, models, History, and
meetings live in `~/Library/Application Support/voinc-test`, personal commands in
`~/.config/voinc-test`, and meeting Markdown in `~/Documents/Voinc Test Meetings`,
so it starts with onboarding and never reads or changes the installed Voinc's
data. Both apps listen to the same shortcut and microphone, so run one at a
time.

1. Quit Voinc from its menu-bar icon.
2. Open the DMG and drag **Voinc Test** to Applications.
3. Clear the download quarantine, which macOS otherwise reports as an app that
   cannot be opened or is damaged:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Voinc Test.app"
   ```

   Alternatively, open it once, then choose **Open Anyway** in System Settings →
   Privacy & Security.
4. Open Voinc Test and grant Microphone and Accessibility again. Ad-hoc
   signatures change with every build, so each new test build needs these
   grants again: remove the old **Voinc Test** entries in System Settings and
   add the new app.

To go back, quit Voinc Test, delete it, and open Voinc.

## Windows (10 or 11, x64 or ARM64)

The installer and executable are unsigned, so SmartScreen warns: choose **More
info**, then **Run anyway**.

1. Quit Voinc from its tray icon.
2. Run `voinc-setup-<version>-windows-<x64|arm64>.exe`. It replaces an installed Voinc of
   the same or an older version in the usual per-user layout, and refuses to
   replace a newer one: uninstall that first from Apps & Features (settings are
   kept). Or skip the installer and run the bare `voinc-<version>-windows-<arch>.exe` from any folder.
3. The test build never self-updates. To return to the release, run the
   release installer from <https://anoma.ly/hex>.

## Linux (x86_64)

An installed Voinc starts through its systemd user unit, so launching the test
binary normally would just start the installed one. Run the test binary as the
instance directly instead:

```sh
voinc quit
chmod +x voinc-*-linux-x64
./voinc-*-linux-x64 service     # runs Voinc in the foreground; Ctrl-C quits
./voinc-*-linux-x64             # in another terminal: opens the window
```

After Ctrl-C, `voinc` starts the installed Voinc again.

## Reporting

Include the commit from the artifact name, the OS and architecture, and the
logs: `~/Library/Application Support/voinc/logs/` on macOS,
`%LOCALAPPDATA%\voinc\logs\` on Windows, and
`~/.local/share/voinc/logs/` on Linux.
