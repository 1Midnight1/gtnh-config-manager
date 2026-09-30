# GTNH Config Manager

A desktop GUI (built with [iced](https://github.com/iced-rs/iced)) for browsing, searching, and
editing the `.cfg` files in a GregTech: New Horizons (GTNH) modpack instance, with a
profile system for tracking and re-applying sets of edits independently of the modpack's
on-disk defaults.

## Features

### Instance selection
- Pick a modpack instance folder via a native file picker (powered by
  [rfd](https://github.com/PolyMeilex/rfd)).
- The last selected folder and the active profile are remembered automatically (persisted to a
  small JSON settings file in the OS config directory) and restored the next time the app starts.

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
- Parsing keeps every line (comments, blank lines, indentation, line endings), so edited files
  are written back byte-for-byte identical to the original aside from the lines of the values
  that were actually changed. Files are only rewritten when their contents really change, and
  every write goes to a temporary file first, so a crash can't leave a config half-written.

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
  - **Make active**: the current profile's edits are undone and this one's applied, and the
    result is written to the instance's files straight away - the files on disk always match the
    active profile.
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
- If the files on disk no longer match the active profile when the app starts (for example after
  a modpack update reset some configs), the profile's values are applied in memory and shown as
  unsaved; **Save** writes them.
- Profiles are saved to disk immediately after every change. If the profiles file is ever
  damaged, it's moved aside (`profiles.json.corrupt-<timestamp>`) instead of being overwritten.
- **Switching instances** (after one is already loaded) asks for confirmation, then writes the
  active profile's edits and tracked files onto the new instance. Unsaved edits trigger the
  Save/Discard prompt first; cancelling keeps the current instance loaded.

Because profile switching only replays recorded edits rather than re-reading files from disk,
switching between profiles is instant. Whenever a profile is applied, each edit's original value
is updated to what the files actually contain, so undoing it later restores the right value even
after a modpack update or on another instance.

#### Tracked files (ranks.txt)
Some files aren't Forge configs but still belong with a profile. Each profile stores a full copy
of these; currently that's just `serverutilities/server/ranks.txt`, which is edited through
ServerUtilities' in-game GUI.

- **Save** copies the file's current on-disk contents into the selected profile.
- **Selecting** a profile writes its stored copy back to disk. A profile with no stored copy
  (e.g. one started from scratch and never saved) leaves the file alone.
- **Switching instances** writes the active profile's stored copy to the new instance.
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

The replaced folders are moved aside rather than deleted until every folder has been swapped in;
if anything fails, the swap is undone and the world is left exactly as it was.

ServerUtilities prunes old backups (`backups_to_keep` / `max_folder_size`), so the safety backup
made by a restore may eventually be deleted like any other.

### Migrating saves
**Migrate save…** copies a world from the loaded instance into another instance (for example a
fresh install of a newer pack version). Pick the save, then the target instance folder, and
confirm. Close Minecraft in both instances first.

A migration is the same as restoring a backup of the save in the target instance:

1. The save's folders are found on disk: `saves/<world>` plus world-named folders from other
   mods (`journeymap/data/sp/<world>`, `visualprospecting/server/<world>_*`, ...).
2. They're zipped into a temporary backup inside the target's `.minecraft`.
3. That backup is restored into the target as described above, so any existing copy of the
   world there is first saved to the target's backups folder. The temporary zip is then
   deleted. The source instance is only read.

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
  fsutil.rs       Crash-safe file writes and shared JSON persistence
  backups.rs      Lists and restores ServerUtilities world backups
  settings.rs     Persisted app settings (last-used instance folder, active profile)
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
