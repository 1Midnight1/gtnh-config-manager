# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Rust (edition 2024) desktop GUI built on iced 0.14 for browsing, searching, and editing the Forge-format `.cfg` files of a GregTech: New Horizons (Minecraft 1.7.10) modpack instance, with named "profiles" that track edits as diffs. See README.md for the user-facing feature list.

## Commands

```bash
cargo build
cargo run                               # launch the GUI
cargo test                              # all unit tests (inline #[cfg(test)] modules)
cargo test forge_cfg::tests             # one module's tests
cargo test round_trips_full_sample      # a single test by name
cargo fmt / cargo clippy
```

## Architecture

The data flow is: `forge_cfg` (parse/serialize one file) → `config_store` (load all files into a mutable model) → `search` (flat index over the store), `tree` (mod grouping + category hierarchy for the sidebar) and `changeset` (diff of edits) → `profiles` (named, persisted changesets, plus verbatim copies of `tracked_files` such as ranks.txt) → `app` (iced state machine tying it together). `backups` (restoring ServerUtilities world backups) is independent of the config pipeline apart from reading its folder setting from the store.

- **GUI isolation:** only `app/` and `main.rs` may depend on `iced`/`rfd`. Keep all other modules GUI-free so `cargo test` stays fast; put new logic in a non-GUI module and test it there.
- **Lossless round-trip is a core invariant.** `forge_cfg::parse` builds an AST (`ConfigFile` → `Item::{Category, Property, ...}`) that preserves comments, ordering, nesting, and the `~CONFIG_VERSION:` directive; `impl Display for ConfigFile` must reproduce the original bytes except for changed values. Parser tests load fixtures from `tests/fixtures/` via `include_str!`. Add a fixture there when handling a new syntax edge case.
- **Unsupported formats = parse errors.** Many `.cfg` files in GTNH aren't Forge format (JSON, CodeChicken, NEI lists, etc.). `ConfigStore::load` returns them as `LoadError`s alongside the successfully parsed store, and the app reports them as "skipped" in the status bar rather than failing. Don't make the parser lenient in ways that would risk mangling a non-Forge file on write.
- **`PropertyPath`** (file path relative to `.minecraft` + category path + property name) is the stable identity key used by the search index, changesets, and persisted profiles. It's serialized into `profiles.json`, so changing its shape breaks saved profiles.
- **Changesets store `original_value` and `new_value`.** `record` keeps the first-seen original. `apply`/`revert` return unresolved paths instead of failing (files may change on disk after a modpack update). Profile switching (`app::switch_to_profile`) reverts the current changeset and applies the target in memory, with no disk re-read. That depends on the `original_value`s being accurate.
- **Every edit belongs to a profile.** `State.selected_profile` is always `Some` once configs are loaded; a "Default" profile is created on first run. `profile_dirty` tracks divergence from the persisted profile, and the unsaved-changes dialog guards switching or creating profiles. Saving writes dirty config files (`ConfigStore::save_dirty`) and persists the profile store.
- **Persistence:** `settings.rs` and `profiles.rs` share one pattern. They store JSON under `directories::ProjectDirs` (`settings.json`, `profiles.json`), with public `load()`/`save()` wrapping private `load_from(path)`/`save_to(path)`. Tests use the `*_from`/`*_to` variants with `std::env::temp_dir()` paths so they never touch the real config dir.
- **Tracked files** (`tracked_files::TRACKED_FILES`, currently `serverutilities/server/ranks.txt`) are captured into the selected profile on Save and written to disk only by `switch_to_profile`, never on startup. `Profile.tracked_files` is `#[serde(default)]` so older `profiles.json` files still load.
- **Backup restore** (`backups::restore`) replaces whole "restore roots" rather than extracting over existing files. `restore_roots` derives them from the zip's entries: the shortest prefix named `<world>` or `<world>_*`, otherwise the entry's parent folder, and never a top-level `.minecraft` folder. The order is: write a safety backup of those roots in ServerUtilities' zip layout (paths relative to `.minecraft`, world name as zip comment, plus directory entries for empty folders) → extract into `.minecraft/.gtnh-restore-staging` → swap each root in. Nothing is deleted until extraction has succeeded. `app` runs restores on a separate `std::thread` because backups can be gigabytes.
- **Discovery** scans `.minecraft/config` and `.minecraft/serverutilities` (`SCAN_ROOTS`), each only one subdirectory deep.
- **UI:** `app/mod.rs` holds `State`, `Message`, `boot`/`update` and the top/status bars; views are split into `view_configs` (sidebar tree, property pane, search results), `view_profiles` (profile list, diff, ranks.txt viewer) and `dialogs` (modal cards drawn over the page with `stack`/`opaque`), with palette-derived styles in `style`. Async work (file picker, loading, persisting) goes through `Task::perform`. Search results are capped at 200 rendered rows because iced has no virtualized list.
- **Navigation:** mods are grouped by `tree::group_mods` (a scan-root subfolder = one mod, a loose file = its own mod; `tree::mod_id_for` must agree with it). The sidebar is flattened into rows by `view_configs::sidebar_rows`, driven by `State.expanded` (`NodeKey`s) and `State.selection` (a `Location`: file + category path). `JumpTo(PropertyPath)` expands the ancestors, selects the category, highlights the property and scrolls both scrollables by relative offset (`SIDEBAR_SCROLL`/`PROPERTIES_SCROLL` ids).
- **Editing:** scalar values go through `Message::PropertyEdited` (bools as a toggler), list values through the `EditList` dialog (one entry per line). `edit_property` refuses to change a property between single and list shape.
