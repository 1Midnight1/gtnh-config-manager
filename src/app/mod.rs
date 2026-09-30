//! iced `State`/`Message`/`update`/`view` glue. Kept thin - the actual logic lives in the
//! pure `config_store`, `search`, `tree`, `settings`, `changeset`, `tracked_files` and `backups`
//! modules so it stays unit-testable without spinning up the GUI.
//!
//! The views are split by page: `view_configs` (sidebar tree, property pane, search results),
//! `view_profiles` (profile list, diff, ranks.txt) and `dialogs` (modal overlays), with shared
//! styling in `style`.

mod dialogs;
mod style;
mod view_configs;
mod view_profiles;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use iced::widget::operation::{self, RelativeOffset};
use iced::widget::{
    button, center, column, container, opaque, pick_list, row, space, stack, text, text_editor,
};
use iced::{Alignment, Color, Element, Length, Task};

use crate::backups::{self, BackupInfo, RestoreReport};
use crate::changeset::Changeset;
use crate::config_store::{ConfigStore, FileError, PropertyPath};
use crate::forge_cfg::PropertyValue;
use crate::profiles::ProfileStore;
use crate::search::SearchIndex;
use crate::settings::AppSettings;
use crate::tracked_files;
use crate::tree::{self, ModGroup};

/// Cap on how many search results are turned into widgets at once - iced 0.14 has no
/// virtualized list, so an unbounded query match could otherwise build thousands of rows.
const MAX_VISIBLE_RESULTS: usize = 200;

/// Scrollable ids, so `JumpTo` can scroll the sidebar and property pane to the target.
const SIDEBAR_SCROLL: &str = "sidebar";
const PROPERTIES_SCROLL: &str = "properties";

const DEFAULT_PROFILE: &str = "Default";

#[derive(Default)]
pub struct State {
    settings: AppSettings,
    instance_path: Option<PathBuf>,
    /// True while the configs of `instance_path` are being read.
    loading: bool,
    store: Option<ConfigStore>,
    index: SearchIndex,
    search_query: String,
    /// Indices into `index.entries` matching `search_query`, recomputed whenever either changes
    /// rather than on every redraw.
    search_matches: Vec<usize>,
    /// Edits belonging to the selected profile, tracked separately from `store` so they can be
    /// persisted as a named, diff-based profile independent of whatever is on disk.
    changeset: Changeset,
    /// Program-managed, named profiles - persisted to disk independently of any instance.
    profile_store: ProfileStore,
    /// The profile currently being edited. Always `Some` once a folder has been loaded - there
    /// is no way to make edits that aren't attached to some profile. The instance's files hold
    /// this profile's saved state, plus any unsaved edits once they're saved.
    selected_profile: Option<String>,
    /// True while the loaded configs differ from what's on disk / persisted for
    /// `selected_profile`.
    profile_dirty: bool,
    /// Profile shown in the Profiles page's detail pane; falls back to `selected_profile`.
    viewing_profile: Option<String>,
    dialog: Dialog,
    status: Option<String>,
    /// Number of config files that couldn't be parsed on the last load.
    skipped_files: usize,

    page: Page,
    /// Loaded files grouped into mods for the sidebar, rebuilt alongside `index`.
    mods: Vec<ModGroup>,
    /// Category shown in the property pane.
    selection: Option<Location>,
    /// Sidebar nodes currently expanded.
    expanded: HashSet<NodeKey>,
    mod_filter: String,
    /// Property scrolled to by the last jump, drawn with an accent border.
    highlighted: Option<PropertyPath>,
    ranks_source: RanksSource,
    /// Contents of ranks.txt on disk, refreshed whenever the Profiles page is shown or a
    /// profile switch / save touches it. `None` if the file doesn't exist.
    ranks_on_disk: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Configs,
    Profiles,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RanksSource {
    #[default]
    Disk,
    Profile,
}

/// A category within a file - an empty `category_path` means the file's top level.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub file: PathBuf,
    pub category_path: Vec<String>,
}

/// Identifies an expandable sidebar node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Mod(String),
    File(PathBuf),
    Category(PathBuf, Vec<String>),
}

#[derive(Debug, Default)]
enum Dialog {
    #[default]
    None,
    UnsavedChanges {
        pending: PendingAction,
    },
    NewProfile {
        name: String,
        error: Option<String>,
    },
    ConfirmDelete {
        name: String,
    },
    RestoreBackup {
        dir: PathBuf,
        backups: Vec<BackupInfo>,
    },
    ConfirmRestore {
        backup: BackupInfo,
    },
    /// Switching to another instance, which writes the selected profile onto its files.
    ConfirmSwitchInstance {
        path: PathBuf,
    },
    MigrateSave {
        saves: Vec<String>,
    },
    ConfirmMigrate {
        world: String,
        target: PathBuf,
    },
    /// Shown (without any buttons) while a restore or migration runs, so nothing else can
    /// happen meanwhile.
    Busy {
        title: String,
        message: String,
    },
    /// Editing a list-valued property, one entry per line.
    EditList {
        path: PropertyPath,
        content: text_editor::Content,
        error: Option<String>,
    },
}

#[derive(Debug, Clone)]
enum PendingAction {
    SwitchProfile(String),
    NewProfile,
    SwitchInstance(PathBuf),
}

#[derive(Debug, Clone)]
pub enum Message {
    PickFolder,
    FolderPicked(Option<PathBuf>),
    ConfigsLoaded {
        minecraft_dir: PathBuf,
        result: Result<(ConfigStore, Vec<FileError>), String>,
        /// Write the selected profile onto the loaded files (a confirmed instance switch)
        /// instead of only applying it in memory (startup).
        apply_to_disk: bool,
    },
    ConfirmSwitchInstance,
    ShowPage(Page),
    SearchChanged(String),
    ModFilterChanged(String),
    NodeClicked(NodeKey),
    Navigate(Location),
    JumpTo(PropertyPath),
    PropertyEdited(PropertyPath, String),
    EditList(PropertyPath),
    ListEditorAction(text_editor::Action),
    ConfirmListEdit,
    Save,
    SelectProfileRequested(String),
    ViewProfile(String),
    SetRanksSource(RanksSource),
    NewProfileRequested,
    NewProfileNameChanged(String),
    CreateNewProfile {
        from_scratch: bool,
    },
    DeleteProfileRequested(String),
    ConfirmDeleteProfile,
    CancelDialog,
    UnsavedChangesSave,
    UnsavedChangesDiscard,
    RestoreBackupRequested,
    BackupsListed(PathBuf, Vec<BackupInfo>),
    RestoreBackupSelected(BackupInfo),
    ConfirmRestore,
    RestoreFinished(Result<RestoreReport, String>),
    MigrateSaveRequested,
    SavesListed(Vec<String>),
    MigrateSaveSelected(String),
    MigrateTargetPicked(String, Option<PathBuf>),
    ConfirmMigrate,
    MigrateFinished {
        world: String,
        target: PathBuf,
        result: Result<RestoreReport, String>,
    },
}

/// Settings and profiles are small JSON files, so they're read before the first frame. Loading
/// them before any configs also means a profile can never be picked from a half-loaded store.
pub fn boot() -> (State, Task<Message>) {
    let (profile_store, warning) = ProfileStore::load();
    let settings = AppSettings::load();
    let last_instance = settings.last_instance.clone();
    let mut state = State {
        settings,
        profile_store,
        status: warning,
        ..State::default()
    };
    let task = match last_instance {
        Some(path) => start_loading_configs(&mut state, path, false),
        None => Task::none(),
    };
    (state, task)
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::PickFolder => {
            Task::perform(pick_folder(picker_start_dir(state)), Message::FolderPicked)
        }
        Message::FolderPicked(Some(path)) => {
            // With nothing loaded there is no profile state to carry over, so just load it.
            if state.store.is_none() {
                return load_instance(state, path, false);
            }
            state.dialog = if state.profile_dirty {
                Dialog::UnsavedChanges {
                    pending: PendingAction::SwitchInstance(path),
                }
            } else {
                Dialog::ConfirmSwitchInstance { path }
            };
            Task::none()
        }
        Message::FolderPicked(None) => Task::none(),
        Message::ConfirmSwitchInstance => {
            let Dialog::ConfirmSwitchInstance { path } =
                std::mem::replace(&mut state.dialog, Dialog::None)
            else {
                return Task::none();
            };
            load_instance(state, path, true)
        }
        Message::ConfigsLoaded {
            minecraft_dir,
            result,
            apply_to_disk,
        } => {
            // A load of a folder that has since been replaced by another pick.
            let current = state
                .instance_path
                .as_ref()
                .map(|path| path.join(".minecraft"));
            if !state.loading || current.as_ref() != Some(&minecraft_dir) {
                return Task::none();
            }
            state.loading = false;
            match result {
                Ok((store, errors)) => configs_loaded(state, store, errors.len(), apply_to_disk),
                Err(message) => report(state, message),
            }
            Task::none()
        }
        Message::ShowPage(page) => {
            if page == Page::Profiles {
                refresh_ranks_on_disk(state);
            }
            state.page = page;
            Task::none()
        }
        Message::SearchChanged(query) => {
            state.search_query = query;
            refresh_search(state);
            Task::none()
        }
        Message::ModFilterChanged(filter) => {
            state.mod_filter = filter;
            Task::none()
        }
        Message::NodeClicked(key) => {
            node_clicked(state, key);
            Task::none()
        }
        Message::Navigate(location) => {
            reveal(state, &location);
            state.selection = Some(location);
            state.highlighted = None;
            operation::snap_to(PROPERTIES_SCROLL, RelativeOffset::START)
        }
        Message::JumpTo(path) => jump_to(state, path),
        Message::PropertyEdited(path, value) => {
            edit_property(state, path, PropertyValue::Single(value));
            Task::none()
        }
        Message::EditList(path) => {
            let entries = match state.store.as_ref().and_then(|s| s.get_property(&path)) {
                Some(property) => match &property.value {
                    PropertyValue::List(values) => values.join("\n"),
                    PropertyValue::Single(_) => return Task::none(),
                },
                None => return Task::none(),
            };
            state.dialog = Dialog::EditList {
                path,
                content: text_editor::Content::with_text(&entries),
                error: None,
            };
            Task::none()
        }
        Message::ListEditorAction(action) => {
            if let Dialog::EditList { content, .. } = &mut state.dialog {
                content.perform(action);
            }
            Task::none()
        }
        Message::ConfirmListEdit => {
            let Dialog::EditList {
                path,
                content,
                error,
            } = &mut state.dialog
            else {
                return Task::none();
            };
            let values: Vec<String> = content
                .text()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            // A lone '>' closes a list in Forge's format, so it can't be stored as an entry.
            if values.iter().any(|value| value == ">") {
                *error = Some("An entry can't be just \">\".".to_string());
                return Task::none();
            }
            let path = path.clone();
            state.dialog = Dialog::None;
            edit_property(state, path, PropertyValue::List(values));
            Task::none()
        }
        Message::Save => {
            state.status = None;
            save_current_profile(state);
            Task::none()
        }
        Message::SelectProfileRequested(name) => {
            if state.selected_profile.as_deref() == Some(name.as_str()) {
                return Task::none();
            }
            if state.profile_dirty {
                state.dialog = Dialog::UnsavedChanges {
                    pending: PendingAction::SwitchProfile(name),
                };
            } else {
                state.status = None;
                activate_profile(state, &name, true);
            }
            Task::none()
        }
        Message::ViewProfile(name) => {
            state.viewing_profile = Some(name);
            Task::none()
        }
        Message::SetRanksSource(source) => {
            if source == RanksSource::Disk {
                refresh_ranks_on_disk(state);
            }
            state.ranks_source = source;
            Task::none()
        }
        Message::NewProfileRequested => {
            if state.store.is_none() {
                state.status = Some("Select a modpack folder first.".to_string());
                return Task::none();
            }
            state.dialog = if state.profile_dirty {
                Dialog::UnsavedChanges {
                    pending: PendingAction::NewProfile,
                }
            } else {
                new_profile_dialog()
            };
            Task::none()
        }
        Message::NewProfileNameChanged(name) => {
            if let Dialog::NewProfile { name: current, .. } = &mut state.dialog {
                *current = name;
            }
            Task::none()
        }
        Message::CreateNewProfile { from_scratch } => {
            create_new_profile(state, from_scratch);
            Task::none()
        }
        Message::DeleteProfileRequested(name) => {
            state.dialog = Dialog::ConfirmDelete { name };
            Task::none()
        }
        Message::ConfirmDeleteProfile => {
            let Dialog::ConfirmDelete { name } = std::mem::replace(&mut state.dialog, Dialog::None)
            else {
                return Task::none();
            };
            state.status = None;
            state.profile_store.remove(&name);
            if state.viewing_profile.as_deref() == Some(name.as_str()) {
                state.viewing_profile = None;
            }
            if state.selected_profile.as_deref() == Some(name.as_str()) {
                // The deleted profile's edits are undone on disk by activating another one.
                let fallback = fallback_profile(state);
                activate_profile(state, &fallback, true);
            }
            persist_profiles(state);
            Task::none()
        }
        Message::CancelDialog => {
            // A running restore or migration can't be cancelled.
            if !matches!(state.dialog, Dialog::Busy { .. }) {
                state.dialog = Dialog::None;
            }
            Task::none()
        }
        Message::UnsavedChangesSave => {
            state.status = None;
            if save_current_profile(state) {
                resolve_pending_dialog_action(state)
            } else {
                // Don't switch away from edits that couldn't be written.
                state.dialog = Dialog::None;
                Task::none()
            }
        }
        Message::UnsavedChangesDiscard => {
            state.status = None;
            if let Some(name) = state.selected_profile.clone() {
                // Reset back to the profile's last-saved state, discarding unsaved edits.
                activate_profile(state, &name, false);
            }
            resolve_pending_dialog_action(state)
        }
        Message::RestoreBackupRequested => {
            let Some(store) = &state.store else {
                state.status = Some("Select a modpack folder first.".to_string());
                return Task::none();
            };
            let dir = backups::backups_dir(store);
            Task::perform(list_backups(dir.clone()), move |found| {
                Message::BackupsListed(dir.clone(), found)
            })
        }
        Message::BackupsListed(dir, backups) => {
            state.dialog = Dialog::RestoreBackup { dir, backups };
            Task::none()
        }
        Message::RestoreBackupSelected(backup) => {
            state.dialog = Dialog::ConfirmRestore { backup };
            Task::none()
        }
        Message::ConfirmRestore => {
            let Dialog::ConfirmRestore { backup } =
                std::mem::replace(&mut state.dialog, Dialog::None)
            else {
                return Task::none();
            };
            let Some(store) = &state.store else {
                return Task::none();
            };
            let minecraft_dir = store.minecraft_dir.clone();
            let dir = backups::backups_dir(store);
            state.dialog = Dialog::Busy {
                title: "Restoring…".to_string(),
                message: format!(
                    "Restoring {}. This can take a while for large worlds.",
                    backup.name
                ),
            };
            Task::perform(
                run_blocking(move || backups::restore(&minecraft_dir, &dir, &backup)),
                Message::RestoreFinished,
            )
        }
        Message::RestoreFinished(result) => {
            state.dialog = Dialog::None;
            state.status = Some(match result {
                Ok(report) => format!(
                    "Backup restored ({} folder(s) replaced). The previous state was saved as {}.",
                    report.roots.len(),
                    report.safety_backup.display()
                ),
                Err(err) => err,
            });
            Task::none()
        }
        Message::MigrateSaveRequested => {
            let Some(store) = &state.store else {
                state.status = Some("Select a modpack folder first.".to_string());
                return Task::none();
            };
            Task::perform(
                list_saves(store.minecraft_dir.clone()),
                Message::SavesListed,
            )
        }
        Message::SavesListed(saves) => {
            state.dialog = Dialog::MigrateSave { saves };
            Task::none()
        }
        Message::MigrateSaveSelected(world) => {
            state.dialog = Dialog::None;
            Task::perform(pick_folder(picker_start_dir(state)), move |target| {
                Message::MigrateTargetPicked(world.clone(), target)
            })
        }
        Message::MigrateTargetPicked(world, Some(target)) => {
            let target_minecraft = target.join(".minecraft");
            if !target_minecraft.is_dir() {
                state.status = Some(format!(
                    "{} does not exist - is this a GTNH instance folder?",
                    target_minecraft.display()
                ));
            } else if state
                .instance_path
                .as_deref()
                .is_some_and(|current| backups::same_dir(&target, current))
            {
                state.status = Some(format!("\"{world}\" is already in this instance."));
            } else {
                state.dialog = Dialog::ConfirmMigrate { world, target };
            }
            Task::none()
        }
        Message::MigrateTargetPicked(_, None) => Task::none(),
        Message::ConfirmMigrate => {
            let Dialog::ConfirmMigrate { world, target } =
                std::mem::replace(&mut state.dialog, Dialog::None)
            else {
                return Task::none();
            };
            let Some(store) = &state.store else {
                return Task::none();
            };
            let source_minecraft = store.minecraft_dir.clone();
            state.dialog = Dialog::Busy {
                title: "Migrating…".to_string(),
                message: format!(
                    "Copying \"{world}\" to {}. This can take a while for large worlds.",
                    target.display()
                ),
            };
            let (migrate_world, target_minecraft) = (world.clone(), target.join(".minecraft"));
            Task::perform(
                run_blocking(move || {
                    backups::migrate(&source_minecraft, &migrate_world, &target_minecraft)
                }),
                move |result| Message::MigrateFinished {
                    world: world.clone(),
                    target: target.clone(),
                    result,
                },
            )
        }
        Message::MigrateFinished {
            world,
            target,
            result,
        } => {
            state.dialog = Dialog::None;
            state.status = Some(match result {
                Ok(report) => format!(
                    "Migrated \"{world}\" to {} ({} folder(s) replaced). The target's previous state was saved as {}.",
                    target.display(),
                    report.roots.len(),
                    report.safety_backup.display()
                ),
                Err(err) => err,
            });
            Task::none()
        }
    }
}

/// Installs a freshly loaded store and activates a profile on it: the current one when
/// switching instances, otherwise the one remembered from the last run.
fn configs_loaded(state: &mut State, store: ConfigStore, skipped: usize, apply_to_disk: bool) {
    // Some mods ship configs in formats other than Forge's Configuration class (raw JSON,
    // ChickenBones' older format, etc). Those are expected to fail to parse and are simply
    // left unmanaged rather than treated as corruption.
    state.skipped_files = skipped;
    state.mods = tree::group_mods(&store);
    state.store = Some(store);
    // The previous changeset was applied to another store, so there's nothing to revert here.
    state.changeset = Changeset::default();

    let remembered = state
        .selected_profile
        .clone()
        .or_else(|| state.settings.active_profile.clone())
        .filter(|name| state.profile_store.get(name).is_some());
    let name = match remembered {
        Some(name) => name,
        None => fallback_profile(state),
    };
    activate_profile(state, &name, apply_to_disk);
    if apply_to_disk {
        report(
            state,
            format!("Applied profile \"{name}\" to this instance"),
        );
    }
}

/// The first profile, creating (and persisting) an empty "Default" one if there are none, so
/// a profile can always be selected.
fn fallback_profile(state: &mut State) -> String {
    if let Some(profile) = state.profile_store.profiles.first() {
        return profile.name.clone();
    }
    state
        .profile_store
        .upsert(DEFAULT_PROFILE.to_string(), Changeset::default());
    persist_profiles(state);
    DEFAULT_PROFILE.to_string()
}

/// Makes `name` the selected profile: undoes the current changeset in memory and applies
/// `name`'s saved one. With `write_to_disk` the result and the profile's tracked files are
/// written to the instance right away; without it (startup, discarding edits) only the
/// in-memory state changes, and anything that then differs from disk shows as unsaved.
fn activate_profile(state: &mut State, name: &str, write_to_disk: bool) {
    let Some(store) = &mut state.store else {
        return;
    };

    state.changeset.revert(store);
    let (mut changeset, tracked) = state
        .profile_store
        .get(name)
        .map(|profile| (profile.changeset.clone(), profile.tracked_files.clone()))
        .unwrap_or_default();
    let unresolved = changeset.apply(store);

    let mut errors = Vec::new();
    if write_to_disk {
        errors = store.save_dirty();
        errors.extend(tracked_files::write_all(&store.minecraft_dir, &tracked));
    }
    let differs_from_disk = store.has_unsaved_changes();
    state.index = SearchIndex::build(store);
    refresh_search(state);

    // `apply` may have rebased original values onto what this instance really holds; keep
    // those so reverting later restores the right values.
    if state
        .profile_store
        .get(name)
        .is_some_and(|profile| profile.changeset != changeset)
    {
        state
            .profile_store
            .upsert(name.to_string(), changeset.clone());
        persist_profiles(state);
    }
    state.changeset = changeset;
    state.selected_profile = Some(name.to_string());
    state.profile_dirty = differs_from_disk;
    refresh_ranks_on_disk(state);

    if !unresolved.is_empty() {
        report(
            state,
            format!(
                "{} edit(s) in profile \"{name}\" no longer apply",
                unresolved.len()
            ),
        );
    }
    if differs_from_disk && !write_to_disk {
        report(
            state,
            format!("Some files don't match profile \"{name}\" yet - Save to write them"),
        );
    }
    report_file_errors(state, &errors);

    if state.settings.active_profile.as_deref() != Some(name) {
        state.settings.active_profile = Some(name.to_string());
        persist_settings(state);
    }
}

/// Writes dirty config files to disk and stores the current changeset and tracked files in
/// the selected profile. Returns false if any config file couldn't be written.
fn save_current_profile(state: &mut State) -> bool {
    let (Some(store), Some(name)) = (&mut state.store, state.selected_profile.clone()) else {
        return false;
    };
    let errors = store.save_dirty();
    let tracked = tracked_files::capture(&store.minecraft_dir);

    state
        .profile_store
        .upsert(name.clone(), state.changeset.clone());
    state.profile_store.set_tracked_files(&name, tracked);
    persist_profiles(state);
    refresh_ranks_on_disk(state);

    let saved = errors.is_empty();
    state.profile_dirty = !saved;
    if saved {
        report(state, "Saved.");
    }
    report_file_errors(state, &errors);
    saved
}

fn create_new_profile(state: &mut State, from_scratch: bool) {
    let Dialog::NewProfile { name, error } = &mut state.dialog else {
        return;
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        *error = Some("Enter a profile name.".to_string());
        return;
    }
    if state.profile_store.get(&name).is_some() {
        *error = Some("A profile with that name already exists.".to_string());
        return;
    }
    state.dialog = Dialog::None;
    state.status = None;
    state.viewing_profile = Some(name.clone());

    if from_scratch {
        // Starts from the files as they'd be without any profile, which undoes the current
        // profile's edits on disk. With no stored tracked files, ranks.txt is left alone.
        state
            .profile_store
            .upsert(name.clone(), Changeset::default());
        persist_profiles(state);
        activate_profile(state, &name, true);
    } else {
        // A copy of the current profile, which is already what's on disk.
        let tracked = state
            .selected_profile
            .as_ref()
            .and_then(|current| state.profile_store.get(current))
            .map(|profile| profile.tracked_files.clone())
            .unwrap_or_default();
        state
            .profile_store
            .upsert(name.clone(), state.changeset.clone());
        state.profile_store.set_tracked_files(&name, tracked);
        persist_profiles(state);
        state.selected_profile = Some(name.clone());
        state.settings.active_profile = Some(name);
        persist_settings(state);
    }
}

fn new_profile_dialog() -> Dialog {
    Dialog::NewProfile {
        name: String::new(),
        error: None,
    }
}

/// Appends `message` to the status line.
fn report(state: &mut State, message: impl Into<String>) {
    let message = message.into();
    state.status = Some(match state.status.take() {
        Some(existing) => format!("{existing}; {message}"),
        None => message,
    });
}

fn report_file_errors(state: &mut State, errors: &[FileError]) {
    for error in errors {
        report(
            state,
            format!(
                "Failed to write {}: {}",
                error.relative_path.display(),
                error.message
            ),
        );
    }
}

/// Saves the profile store right away, so saves always land in the order they happen.
fn persist_profiles(state: &mut State) {
    if let Err(err) = state.profile_store.save() {
        report(state, format!("Failed to save profiles: {err}"));
    }
}

fn persist_settings(state: &mut State) {
    if let Err(err) = state.settings.save() {
        report(state, format!("Failed to save settings: {err}"));
    }
}

fn refresh_search(state: &mut State) {
    state.search_matches = if state.search_query.trim().is_empty() {
        Vec::new()
    } else {
        state.index.filter(&state.search_query)
    };
}

/// Where folder pickers open: next to the current instance folder.
fn picker_start_dir(state: &State) -> Option<PathBuf> {
    state
        .instance_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// Applies an edit to the store, the search index and the changeset. Edits that would change a
/// property between a single value and a list are ignored, since that would change its syntax.
fn edit_property(state: &mut State, path: PropertyPath, new_value: PropertyValue) {
    let Some(store) = &mut state.store else {
        return;
    };
    let Some(original_value) = store
        .get_property(&path)
        .map(|property| property.value.clone())
    else {
        return;
    };
    if original_value.same_shape(&new_value)
        && original_value != new_value
        && store.set_property_value(&path, new_value.clone())
    {
        state.index.update_value(&path, &new_value);
        refresh_search(state);
        state.changeset.record(path, original_value, new_value);
        state.profile_dirty = true;
    }
}

/// Sidebar click: mods and files toggle open (a single-file mod also selects its file), and
/// categories are selected, collapsing only when clicked again while already selected.
fn node_clicked(state: &mut State, key: NodeKey) {
    match &key {
        NodeKey::Mod(id) => {
            let opening = !state.expanded.contains(&key);
            toggle(&mut state.expanded, key.clone());
            let single_file = state
                .mods
                .iter()
                .find(|group| &group.id == id)
                .filter(|group| group.files.len() == 1)
                .map(|group| group.files[0].clone());
            if let (true, Some(file)) = (opening, single_file) {
                select(
                    state,
                    Location {
                        file,
                        category_path: Vec::new(),
                    },
                );
            }
        }
        NodeKey::File(file) => {
            let location = Location {
                file: file.clone(),
                category_path: Vec::new(),
            };
            if state.selection.as_ref() == Some(&location) {
                toggle(&mut state.expanded, key.clone());
            } else {
                state.expanded.insert(key.clone());
                select(state, location);
            }
        }
        NodeKey::Category(file, category_path) => {
            let location = Location {
                file: file.clone(),
                category_path: category_path.clone(),
            };
            if state.selection.as_ref() == Some(&location) {
                toggle(&mut state.expanded, key.clone());
            } else {
                state.expanded.insert(key.clone());
                select(state, location);
            }
        }
    }
}

fn select(state: &mut State, location: Location) {
    state.selection = Some(location);
    state.highlighted = None;
}

fn toggle(set: &mut HashSet<NodeKey>, key: NodeKey) {
    if !set.remove(&key) {
        set.insert(key);
    }
}

/// Expands every sidebar node above (and including) `location`.
fn reveal(state: &mut State, location: &Location) {
    state
        .expanded
        .insert(NodeKey::Mod(tree::mod_id_for(&location.file)));
    state.expanded.insert(NodeKey::File(location.file.clone()));
    for depth in 1..=location.category_path.len() {
        state.expanded.insert(NodeKey::Category(
            location.file.clone(),
            location.category_path[..depth].to_vec(),
        ));
    }
}

/// Opens the property's category in the browser, reveals it in the sidebar, highlights it and
/// scrolls both panes to it.
fn jump_to(state: &mut State, path: PropertyPath) -> Task<Message> {
    let Some(store) = &state.store else {
        return Task::none();
    };
    let Some((index, count)) = tree::property_position(store, &path) else {
        state.status = Some(format!(
            "{} no longer exists in {}",
            path.property_name,
            tree::breadcrumb(&path)
        ));
        return Task::none();
    };

    let location = Location {
        file: path.relative_path.clone(),
        category_path: path.category_path.clone(),
    };
    state.page = Page::Configs;
    state.search_query.clear();
    refresh_search(state);
    state.mod_filter.clear();
    reveal(state, &location);
    state.selection = Some(location);
    state.highlighted = Some(path);

    let rows = view_configs::sidebar_rows(state);
    let sidebar_task = match rows.iter().position(|row| row.selected) {
        Some(row) => operation::snap_to(SIDEBAR_SCROLL, relative_offset(row, rows.len())),
        None => Task::none(),
    };
    Task::batch([
        sidebar_task,
        operation::snap_to(PROPERTIES_SCROLL, relative_offset(index, count)),
    ])
}

fn relative_offset(index: usize, count: usize) -> RelativeOffset {
    RelativeOffset {
        x: 0.0,
        y: if count > 1 {
            index as f32 / (count - 1) as f32
        } else {
            0.0
        },
    }
}

fn refresh_ranks_on_disk(state: &mut State) {
    if let Some(store) = &state.store {
        state.ranks_on_disk = tracked_files::read_ranks(&store.minecraft_dir);
    }
}

/// Resolves whatever action was deferred behind the "unsaved changes" dialog.
fn resolve_pending_dialog_action(state: &mut State) -> Task<Message> {
    let Dialog::UnsavedChanges { pending } = std::mem::replace(&mut state.dialog, Dialog::None)
    else {
        return Task::none();
    };
    match pending {
        PendingAction::SwitchProfile(name) => activate_profile(state, &name, true),
        PendingAction::NewProfile => state.dialog = new_profile_dialog(),
        PendingAction::SwitchInstance(path) => {
            state.dialog = Dialog::ConfirmSwitchInstance { path };
        }
    }
    Task::none()
}

/// Loads `path` as the instance and remembers it as the last one used.
fn load_instance(state: &mut State, path: PathBuf, apply_to_disk: bool) -> Task<Message> {
    state.status = None;
    state.settings.last_instance = Some(path.clone());
    persist_settings(state);
    start_loading_configs(state, path, apply_to_disk)
}

fn start_loading_configs(
    state: &mut State,
    instance_path: PathBuf,
    apply_to_disk: bool,
) -> Task<Message> {
    state.instance_path = Some(instance_path.clone());
    state.loading = true;
    state.store = None;
    state.index = SearchIndex::default();
    refresh_search(state);
    state.mods.clear();
    state.selection = None;
    state.expanded.clear();
    state.highlighted = None;

    let minecraft_dir = instance_path.join(".minecraft");
    Task::perform(load_configs(minecraft_dir.clone()), move |result| {
        Message::ConfigsLoaded {
            minecraft_dir: minecraft_dir.clone(),
            result,
            apply_to_disk,
        }
    })
}

pub fn view(state: &State) -> Element<'_, Message> {
    let body: Element<'_, Message> = if state.store.is_none() {
        empty_view(state)
    } else {
        match state.page {
            Page::Configs => view_configs::view(state),
            Page::Profiles => view_profiles::view(state),
        }
    };

    let base = column![
        top_bar(state),
        container(body).height(Length::Fill),
        status_bar(state)
    ];

    match dialogs::view(state) {
        Some(dialog) => stack![
            base,
            opaque(
                center(dialog).style(|_| container::Style::default().background(Color {
                    a: 0.5,
                    ..Color::BLACK
                }))
            )
        ]
        .into(),
        None => base.into(),
    }
}

fn top_bar(state: &State) -> Element<'_, Message> {
    let loaded = state.store.is_some();
    let tab = |label, page| {
        button(text(label).size(14))
            .padding([6, 14])
            .style(move |theme, status| style::tab(theme, status, state.page == page))
            .on_press_maybe(loaded.then_some(Message::ShowPage(page)))
    };

    let profile_names: Vec<String> = state
        .profile_store
        .profiles
        .iter()
        .map(|profile| profile.name.clone())
        .collect();
    let profile_picker = pick_list(
        profile_names,
        state.selected_profile.clone(),
        Message::SelectProfileRequested,
    )
    .text_size(14)
    .padding([5, 10]);

    let dirty_marker: Element<'_, Message> = if state.profile_dirty {
        text("● Unsaved").size(13).style(style::warning_text).into()
    } else {
        space().into()
    };

    let bar = row![
        row![
            tab("Configs", Page::Configs),
            tab("Profiles", Page::Profiles)
        ]
        .spacing(4),
        space::horizontal(),
        dirty_marker,
        text("Profile").size(13).style(style::muted_text),
        profile_picker,
        button(text("Save").size(14))
            .padding([6, 16])
            .style(button::primary)
            .on_press_maybe(state.profile_dirty.then_some(Message::Save)),
        button(text("Backups…").size(14))
            .padding([6, 12])
            .style(button::secondary)
            .on_press_maybe(loaded.then_some(Message::RestoreBackupRequested)),
        button(text("Migrate save…").size(14))
            .padding([6, 12])
            .style(button::secondary)
            .on_press_maybe(loaded.then_some(Message::MigrateSaveRequested)),
        button(text("Instance…").size(14))
            .padding([6, 12])
            .style(button::secondary)
            .on_press(Message::PickFolder),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    container(bar)
        .padding([8, 16])
        .width(Length::Fill)
        .style(style::bar)
        .into()
}

fn status_bar(state: &State) -> Element<'_, Message> {
    let left = text(
        state
            .instance_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    )
    .size(12)
    .style(style::muted_text);

    let mut summary = Vec::new();
    if let Some(store) = &state.store {
        summary.push(format!(
            "{} mods · {} files · {} settings",
            state.mods.len(),
            store.files.len(),
            state.index.entries.len()
        ));
    }
    if state.skipped_files > 0 {
        summary.push(format!("{} unsupported files skipped", state.skipped_files));
    }

    let mut bar = row![left, space::horizontal()]
        .spacing(16)
        .align_y(Alignment::Center);
    if let Some(status) = &state.status {
        bar = bar.push(text(status).size(12));
    }
    bar = bar.push(text(summary.join(" · ")).size(12).style(style::muted_text));

    container(bar)
        .padding([4, 16])
        .width(Length::Fill)
        .style(style::bar)
        .into()
}

fn empty_view(state: &State) -> Element<'_, Message> {
    let content = if state.loading {
        column![text("Loading configs…").size(18)]
    } else {
        let mut content = column![
            text("No modpack loaded").size(22),
            text("Choose your GTNH instance folder (the one that contains .minecraft).")
                .style(style::muted_text),
            button(text("Choose instance folder…"))
                .padding([8, 16])
                .style(button::primary)
                .on_press(Message::PickFolder),
        ];
        if let Some(status) = &state.status {
            content = content.push(text(status).style(style::danger_text));
        }
        content
    };
    center(content.spacing(12).align_x(Alignment::Center)).into()
}

async fn pick_folder(starting_dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new();
    if let Some(dir) = starting_dir {
        dialog = dialog.set_directory(dir);
    }

    dialog
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

async fn load_configs(minecraft_dir: PathBuf) -> Result<(ConfigStore, Vec<FileError>), String> {
    if !minecraft_dir.is_dir() {
        return Err(format!(
            "{} does not exist - is this a GTNH instance folder?",
            minecraft_dir.display()
        ));
    }
    Ok(ConfigStore::load(&minecraft_dir))
}

async fn list_backups(dir: PathBuf) -> Vec<BackupInfo> {
    backups::list_backups(&dir)
}

async fn list_saves(minecraft_dir: PathBuf) -> Vec<String> {
    backups::list_saves(&minecraft_dir)
}

/// Runs `job` on its own thread - restores and migrations can copy gigabytes, and blocking an
/// executor thread for that long would stall other tasks.
async fn run_blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(job());
    });
    receiver
        .await
        .unwrap_or_else(|_| Err("The operation stopped unexpectedly.".to_string()))
}
