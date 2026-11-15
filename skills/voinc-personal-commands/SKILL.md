---
name: voinc-personal-commands
description: Configure Voinc personal deterministic transcript transformations in `~/.config/voinc/voinc.config.ts`. Use when the user wants to add or change custom text transformations for the Voinc voice app.
license: MIT
---

# Voinc Personal Transformations

This skill targets current Voinc (dev builds now, and the next stable release).
Voinc 2.3.1 uses an older runtime; its matching skill installs with
`npx skills add dishanagalawatta/voinc#app-v2.3.1 --skill voinc-personal-commands`.

Voinc runs `~/.config/voinc/voinc.config.ts` itself, in an embedded JavaScript
engine. There is no package to install and no build step.

1. Check for `~/.config/voinc/voinc.config.ts` (or `voinc.config.js`). If it is
   missing, ask the user to open Voinc's Modes pane and choose Create Config,
   or run `voinc commands init` if the `voinc` CLI is available.
2. Read `~/.config/voinc/AGENTS.md` for the current API and limits, and
   `voinc.d.ts` for exact types. Voinc owns both and rewrites them; do not edit
   them.
3. Edit only the config and files it imports. Scripts are plain TypeScript or
   JavaScript: they can import `@voinc/commands` and files in the folder, not
   npm packages, Effect, or Node APIs, and they have no filesystem, network,
   or timer access. Enums, namespaces, and constructor parameter properties are
   rejected.
4. Use `defineVoincConfig` from `@voinc/commands` and declare pure `transformations`
   entries (`{ id, name, description?, transform(text, context) }`).
5. Transformations must be selected in the mode's Transformations section.
   They run after mode corrections and optional AI rewriting, not for meetings.
6. Run `voinc commands check` (macOS) after editing. It loads the config as Voinc
   does and prints what it registers or the error with file, line, and column.
   Optionally type-check with `npx -y -p typescript tsc` in `~/.config/voinc`. A
   running Voinc reloads the config on save and shows load errors in Modes'
   Transformations section.
   If `voinc` reports `commands check` as an unrecognized subcommand, stop: the
   user is on an older Voinc (such as 2.3.1). Tell them to install the matching
   skill with `npx skills add dishanagalawatta/voinc#app-v2.3.1 --skill voinc-personal-commands`,
   or to switch to the dev build (https://github.com/dishanagalawatta/voinc#downloads).

Completion criterion: the requested behavior is represented in the config,
`voinc commands check` succeeds (or, without the CLI, the reload succeeded in
Voinc), and any verification gap is reported.
