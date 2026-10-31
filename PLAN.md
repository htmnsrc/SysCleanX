# SysCleanX v2 — Rust rewrite plan

Source of truth for design decisions and progress. Update the checklist after every phase.

## Goals
- Lightweight, fast, single native exe (Rust + egui, glow backend, no accesskit).
- Cleans *all* common junk (system, browsers, apps, dev caches, heuristics, bloat)
  while never breaking Windows or deleting user data.

## Layout
```
crates/core   engine + all Windows logic (testable on macOS; Windows bits behind cfg(windows))
crates/gui    syscleanx.exe      (eframe/egui app)
crates/core/src/targets.toml     data-driven cleaning targets (embedded at build time)
%APPDATA%\SysCleanX\custom-targets.toml   user-defined extra targets (same format)
```

## Safety rules (enforced in core, not the UI)
1. `guard.rs`: hard blocklist. Never delete drive roots, Windows/System32/SysWOW64/WinSxS,
   Program Files roots, Users root, profile root, Documents/Desktop/Pictures/Videos/Music/Downloads,
   AppData roots, ProgramData root. Nothing inside System32/SysWOW64/WinSxS/Installer is purged.
2. Never follow reparse points (symlinks/junctions) — they are skipped entirely.
3. Purge deletes folder *contents*; the root is kept unless the target sets `remove_root`.
4. Age filter (`min_age_hours`) on files *and* directories (dir age read before purging).
5. Locked files are skipped (optionally scheduled for delete-on-reboot).
6. Read-only files: attribute cleared then retried.
7. `skip_markers`: heuristic targets skip LevelDB/ESE/SQLite-journal directories.
8. Unknown env var in a path => path dropped (never expands to a relative path).
9. Targets needing a closed app (`processes`) are reported as blocked and unchecked by default.
10. Services (`services`) are stopped before and restarted after cleaning.
11. Expert actions require confirmation; restore point offered first.
12. Every deletion is logged to `%APPDATA%\SysCleanX\logs`.
13. User-chosen deletions (leftovers, duplicates, large files) go to the Recycle Bin;
    registry deletions are exported to `.reg` backups first.
14. Deliberately NOT done: registry "cleaning", manual WinSxS deletion, disabling Defender/Update.

## Features / checklist
- [x] Phase 1 — core engine: guard, paths (env + wildcard), fsops (scan/purge), targets model+TOML, engine (parallel scan/clean, cancel, progress), log, settings, unit tests
- [x] Phase 2 — coverage: ~60 targets + Chromium family generator + Electron/WebView2 resolver + specials
      (recycle bin, explorer MRU, cleanmgr handlers, DISM, event logs, restore points, installer orphans, old drivers)
- [x] Phase 3 — uninstaller: HKLM/HKLM-WOW64/HKCU, filters, search/sort, batch + quiet uninstall, leftover scan (files → recycle bin, registry → backup+delete)
- [x] Phase 4 — bloat: AppX debloat (curated + protected lists, deprovision), startup manager (StartupApproved, tasks), broken items (shortcuts, Run entries, uninstall entries, tasks, services, orphaned AppData)
- [x] Phase 5 — extras: disk usage analyzer, duplicate finder, hibernation toggle (CLI + scheduled cleaning removed in round 3)
- [x] GUI: sidebar pages for all of the above, dark theme, confirm dialogs, report window
- [x] Build: cross-compile x86_64-pc-windows-gnu from macOS; README with Windows build steps

## Status (2026-10-31)
All phases implemented. Core: 32 unit tests pass on macOS; whole workspace type-checks and links for
x86_64-pc-windows-gnu. UI verified via debug-only demo screenshots (SYSCLEANX_DEMO=1, SYSCLEANX_PAGE=<page>,
`--features screenshot` + EFRAME_SCREENSHOT_TO). NOT yet run on real Windows.

## Round 2 (2026-10-31, after first Windows test)
User feedback: UI felt vague/cluttered with no interaction; wanted restore-point toggle on the Cleaner page.
- UI: animated widgets in theme.rs (check, toggle, hover_row, pill buttons, segmented, slim progress,
  animated_bytes); sliding sidebar highlight; page fade; toast slide-in; Cleaner rebuilt as hero card +
  collapsible categories (details in tooltips); Safe/Recommended/Custom presets; restore-point switch on hero
  with "Clean without it" fallback modal; theme pinned (set_theme) so OS theme switches can't override it.
- Bugs fixed: recycle now warns instead of silently nuking on drives without Recycle Bin (FOF_WANTNUKEWARNING);
  running apps re-checked at clean time (Options.force); service stop fails fast; exclusions apply to file
  roots and accept %ENV%; cleanmgr flags cleared before each run; restore-point cleanup limited to system
  volume; Disk Cleanup targets no longer show empty/0 B; orphaned-uninstall check skips MS/system/bundles;
  Unicode slicing panic in split_command; Duplicates per-frame clone; hiberfil read per frame; Cleaner jobs
  now progress on any page (tick()).

## Round 3 (2026-10-31)
User asked: remove the CLI entirely; more minimal yet more informative UI with new accents; another bug pass.
- CLI crate + weekly scheduled cleaning removed. App deletes the old "SysCleanX\Weekly Clean" task on start (admin).
- Nav reduced to 5: Clean, Apps (Programs | Store apps tabs), Startup, Tools (Repair | Disk usage | Duplicates), Settings.
- Accent colours selectable (Violet default, Teal, Blue, Rose, Amber); borderless dark surfaces.
- Informative: drive bar with junk share + "+X after cleaning", last clean / total freed stats (Settings.last_clean,
  total_freed), category summaries + share bars, safety-level dots + legend, item descriptions inline,
  selected size in Apps, broken-entry count in Startup, drive space in sidebar.
- Bugs fixed: labels swallowed row clicks (selectable_labels off); broad sweeps double-counted folders owned by
  other targets (Filter.skip_dirs from resolve_all's claimed list); logs grew forever (keep newest 30);
  COM initialised for Recycle Bin operations; Disk usage on a missing path showed 0 B.

## Round 4 (2026-10-31): installer
- installer/syscleanx.nsi (NSIS 3, MUI2): Program Files (x64, Win10+ check), Start menu + optional desktop
  shortcut, Uninstall key in 64-bit view (DisplayName/Version/Publisher/Icon/InstallLocation/InstallDate/
  EstimatedSize/QuietUninstallString/NoModify/NoRepair), App Paths, upgrade into previous InstallLocation,
  "app is running" retry check, uninstaller with optional %APPDATA%\SysCleanX removal; RMDir never recursive on $INSTDIR.
- installer/build.sh builds exe + installer into dist/. Exe now embeds VERSIONINFO (build.rs).
- Not yet signed (SmartScreen warning). Signing later: osslsigncode (brew) or signtool with a cert / Azure Trusted Signing.

## Round 5 (2026-10-31): repo migration + docs
- Old Python app removed; the Rust project now lives in the original git repo folder `SysCleanX/`
  (remote github.com/htmnsrc/SysCleanX, history kept, nothing committed yet). `SysCleanX-rs/` no longer exists.
- README rewritten in depth (cleaning table generated from targets.toml).
- docs/make_previews.sh regenerates docs/screenshots/*.png + docs/social-preview.png from the debug preview mode
  (APPDATA is pointed at a temp folder so real settings never leak into images).

## Next: verify on Windows (user's laptop)
- Elevation prompt, fonts/icons (Segoe UI + Segoe UI Emoji fallback), scan numbers vs. reality
- Specials: cleanmgr /sagerun, DISM, Get-WindowsDriver, installer-orphans PowerShell COM script, restore points
- Uninstaller: Inno/NSIS relaunch-to-temp waiting, leftover matching quality
- Startup enable/disable reflected in Task Manager; AppX removal; broken-item fixes + backups

## Build
- macOS checks: `cargo test -p syscleanx-core`, `cargo check --target x86_64-pc-windows-gnu`
- Windows exe from macOS: `cargo build --release --target x86_64-pc-windows-gnu` (needs `brew install mingw-w64`)
- On Windows: `cargo build --release` (MSVC toolchain)
