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

### Browsing
- A **sidebar** lists every mod, grouped the way the files are laid out: a subfolder of
  `config/` (e.g. `GTNewHorizons/`) is one mod containing all of its files, and a loose file
  (e.g. `ironchest.cfg`) is its own mod. ServerUtilities' configs are listed in their own section.
  Expand a mod to see its files (when it has several) and its nested categories; the sidebar
  can be filtered by mod name.
- Selecting a category shows a clickable breadcrumb (`Mod › file.cfg › category › sub`), the
  category's comment, chips for its subcategories, and a card per setting with its type and
  the comment Forge wrote for it.

### Search
- The global search matches against the mod name, file path, category path, property name, and
  current value (case-insensitive substring match).
- Clicking a result jumps to it: the sidebar expands to its category, the category opens, and
  the setting is scrolled to and highlighted.
- Results are capped at 200 visible rows at a time to keep the UI responsive, since iced has no
  built-in virtualized list widget.

### Editing
- Booleans are toggles, other single values are text fields, and list values are edited in a
  dialog with one entry per line. Every edit immediately updates the in-memory config and the
  search index, and settings changed by the current profile show their original value.
- Edits are tracked as a diff (property → original value → new value), not as full file
  snapshots — untouched properties and files are never rewritten.

### Profiles
Edits always belong to a **profile** (there is no way to make untracked/unattached edits):

- A profile is a named, persisted set of property edits, each recording both the original and
  new value.
- Profiles are managed by the program itself (stored as JSON in the OS config directory), not
  as files the user has to keep track of.
- Exactly one profile is active at a time; switch it from the picker in the top bar or from
  the **Profiles** page. The Profiles page lists all saved profiles and, for the one you pick:
  - **Make active** (its edits are applied on top of a clean slate).
  - Its changes — every property it changes, with the original and new value. Clicking one
    jumps to that setting in the config browser.
  - Its `ranks.txt`, read-only, switchable between the copy on disk and the copy stored in
    the profile.
  - **Delete** (after a confirmation dialog).
- **New profile...** creates a profile either:
  - starting from the currently selected profile's edits, or
  - starting from scratch (reverting every currently-applied edit back to its original value).
- If you try to switch profiles or create a new one while the current profile has unsaved
  edits, a dialog prompts you to **Save**, **Discard**, or **Cancel** first.
- **Save** writes any changed config files to disk and persists the current edits into the
  selected profile.

Because profile switching only replays recorded edits (using the original values captured at
edit time) rather than re-reading files from disk, switching between profiles is instant.

#### Tracked files (ranks.txt)
Some files aren't Forge configs but still belong with a profile. Each profile stores a full copy
of these; currently that's just `serverutilities/server/ranks.txt`, which is edited through
ServerUtilities' in-game GUI.

- **Save** copies the file's current on-disk contents into the selected profile.
- **Selecting** a profile writes its stored copy back to disk. A profile with no stored copy
  (e.g. one started from scratch and never saved) leaves the file alone.
- The file isn't touched when the app starts up.

### Restoring backups
**Backups…** lists the world backups ServerUtilities has written (`yyyy-mm-dd-hh-mm-ss.zip`
files in the folder set by `backup_folder_path` in `serverutilities/serverutilities.cfg`, by
default `.minecraft/backups`), newest first, along with each backup's world name and size.

Close Minecraft before restoring. Restoring a backup:

1. Works out which folders the backup covers: the world save plus the world's folders from
   `additional_backup_files` (JourneyMap, NEI, TCNodeTracker, VisualProspecting, ...).
2. Saves the current contents of exactly those folders as a new timestamped backup in the same
   folder and format, so the restore can be undone by restoring that backup.
3. Extracts the selected backup to a staging folder, then replaces each of those folders with
   the backup's copy. Files created after the backup was taken (e.g. newly generated chunks) are
   removed, not left behind.

ServerUtilities prunes old backups (`backups_to_keep` / `max_folder_size`), so the safety backup
made by a restore may eventually be deleted like any other.

## Project layout

```
src/
  main.rs         Thin entry point wiring the iced application together
  app/            UI (the only module that depends on iced/rfd)
    mod.rs          State, messages, update logic, top and status bars
    view_configs.rs Sidebar tree, property pane, search results
    view_profiles.rs Profiles page (list, diff, ranks.txt viewer)
    dialogs.rs      Modal dialogs
    style.rs        Shared palette-derived widget styles
  forge_cfg.rs    Parser/serializer for Forge's Configuration file format (no I/O, no GUI deps)
  config_store.rs Discovers and loads .cfg files into an in-memory, editable model
  search.rs       Flat, cached search index over a loaded ConfigStore
  tree.rs         Groups files into mods and builds category hierarchies for navigation
  changeset.rs    Diff-based edit tracking (record/apply/revert) shared by profiles
  profiles.rs     Named, persisted collections of changesets ("profiles")
  tracked_files.rs Whole files (ranks.txt) stored verbatim in each profile
  backups.rs      Lists and restores ServerUtilities world backups
  settings.rs     Persisted app settings (currently just the last-used instance folder)
tests/fixtures/   Small, purpose-built .cfg files used by forge_cfg's parser tests
```

All modules except `app/` and `main.rs` are free of `iced`/`rfd` dependencies by design, so
`cargo test` stays fast and doesn't need to compile the GUI/rendering stack.

## Development

```bash
cargo build   # compile
cargo run     # launch the GUI
cargo test    # run the unit test suite (parser, config store, search, profiles, backups, ...)
```
