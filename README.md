# GTNH Config Manager

A desktop GUI (built with [iced](https://github.com/iced-rs/iced)) for browsing, searching, and
editing the `.cfg` files in a GregTech: New Horizons (GTNH) modpack instance, with a
profile system for tracking and re-applying sets of edits independently of the modpack's
on-disk defaults.

## Features

### Instance selection
- Pick a modpack instance folder via a native file picker (powered by
  [rfd](https://github.com/PolyMeilex/rfd)).
- The last selected folder is remembered automatically (persisted to a small JSON settings
  file in the OS config directory) and reloaded the next time the app starts.

### Config discovery
- Scans `<instance>/.minecraft/config` **and** `<instance>/.minecraft/serverutilities`, each up
  to one subdirectory deep, for `.cfg` files.
- Parses Forge's `Configuration` file format (`net.minecraftforge.common.config.Configuration`,
  used by most GTNH mods on Minecraft 1.7.10) — including nested categories, `B:`/`I:`/`D:`/`S:`
  typed properties, list values (`Name <` ... `>`), quoted property names, and the
  `~CONFIG_VERSION:` directive.
- Files in other formats (raw JSON, the older ChickenBones/CodeChicken config format, NEI's
  plain-list format, mod-specific recipe DSLs, etc.) are recognized as unsupported and skipped
  rather than treated as errors — the status bar reports how many were skipped.
- Parsing preserves comments, ordering, and nesting, so edited files are written back
  byte-for-byte equivalent to the original aside from the values that were actually changed.

### Search
- A live text filter matches against the file path, category path, property name, and current
  value (case-insensitive substring match).
- Results are capped at 200 visible rows at a time to keep the UI responsive, since iced has no
  built-in virtualized list widget.

### Editing
- Every matched property is shown with an inline text field; typing a new value immediately
  updates the in-memory config and the search index.
- Edits are tracked as a diff (property → original value → new value), not as full file
  snapshots — untouched properties and files are never rewritten.

### Profiles
Edits always belong to a **profile** (there is no way to make untracked/unattached edits):

- A profile is a named, persisted set of property edits, each recording both the original and
  new value.
- Profiles are managed by the program itself (stored as JSON in the OS config directory), not
  as files the user has to keep track of.
- Exactly one profile is selected at a time. The UI lists all saved profiles and lets you:
  - **Select** a profile to make it active (its edits are applied on top of a clean slate).
  - **View / Hide** a profile's diff — a read-only list of every property it changes, showing
    the original value and the value it changes it to.
  - **Delete** a profile (after a confirmation dialog).
- **New profile...** creates a profile either:
  - starting from the currently selected profile's edits, or
  - starting from scratch (reverting every currently-applied edit back to its original value).
- If you try to switch profiles or create a new one while the current profile has unsaved
  edits, a dialog prompts you to **Save**, **Discard**, or **Cancel** first.
- **Save** writes any changed config files to disk and persists the current edits into the
  selected profile.

Because profile switching only replays recorded edits (using the original values captured at
edit time) rather than re-reading files from disk, switching between profiles is instant.

## Project layout

```
src/
  main.rs         Thin entry point wiring the iced application together
  app.rs          UI state, messages, update/view logic (the only module that depends on iced/rfd)
  forge_cfg.rs    Parser/serializer for Forge's Configuration file format (no I/O, no GUI deps)
  config_store.rs Discovers and loads .cfg files into an in-memory, editable model
  search.rs       Flat, cached search index over a loaded ConfigStore
  changeset.rs    Diff-based edit tracking (record/apply/revert) shared by profiles
  profiles.rs     Named, persisted collections of changesets ("profiles")
  settings.rs     Persisted app settings (currently just the last-used instance folder)
tests/fixtures/   Small, purpose-built .cfg files used by forge_cfg's parser tests
```

All modules except `app.rs` and `main.rs` are free of `iced`/`rfd` dependencies by design, so
`cargo test` stays fast and doesn't need to compile the GUI/rendering stack.

## Development

```bash
cargo build   # compile
cargo run     # launch the GUI
cargo test    # run the unit test suite (parser, config store, search, profiles, settings)
```
