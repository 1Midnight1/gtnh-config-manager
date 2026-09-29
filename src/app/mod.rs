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
use crate::config_store::{ConfigStore, LoadError, PropertyPath};
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

#[derive(Default)]
pub struct State {
    instance_path: Option<PathBuf>,
    store: Option<ConfigStore>,
    index: SearchIndex,
    search_query: String,
    /// Edits belonging to the selected profile, tracked separately from `store` so they can be
    /// persisted as a named, diff-based profile independent of whatever is on disk.
    changeset: Changeset,
    /// Program-managed, named profiles - persisted to disk independently of any instance.
    profile_store: ProfileStore,
    /// The profile currently being edited. Always `Some` once a folder has been loaded - there
    /// is no way to make edits that aren't attached to some profile.
    selected_profile: Option<String>,
    /// True once `changeset` has diverged from what's persisted for `selected_profile`.
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
    /// Shown (without any buttons) while a restore runs, so nothing else can happen meanwhile.
    Restoring {
        name: String,
    },
    /// Editing a list-valued property, one entry per line.
    EditList {
        path: PropertyPath,
        content: text_editor::Content,
    },
}

#[derive(Debug, Clone)]
enum PendingAction {
    SwitchProfile(String),
    NewProfile,
}

#[derive(Debug, Clone)]
pub enum Message {
    SettingsLoaded(AppSettings),
    SettingsSaved,
    ProfileStoreLoaded(ProfileStore),
    ProfileStoreSaved(Result<(), String>),
    PickFolder,
    FolderPicked(Option<PathBuf>),
    ConfigsLoaded(Result<(ConfigStore, Vec<LoadError>), String>),
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
    CreateNewProfile { from_scratch: bool },
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
}

pub fn boot() -> (State, Task<Message>) {
    let settings_task = Task::perform(load_settings(), Message::SettingsLoaded);
    let profiles_task = Task::perform(load_profile_store(), Message::ProfileStoreLoaded);
    (
        State::default(),
        Task::batch([settings_task, profiles_task]),
    )
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SettingsLoaded(settings) => match settings.last_instance {
            Some(path) => start_loading_configs(state, path),
            None => Task::none(),
        },
        Message::SettingsSaved => Task::none(),
        Message::ProfileStoreLoaded(profile_store) => {
            state.profile_store = profile_store;
            Task::none()
        }
        Message::ProfileStoreSaved(result) => {
            if let Err(err) = result {
                state.status = Some(format!("Failed to persist profiles: {err}"));
            }
            Task::none()
        }
        Message::PickFolder => {
            let starting_dir = state
                .instance_path
                .as_deref()
                .and_then(|path| path.parent())
                .map(|parent| parent.to_path_buf());
            Task::perform(pick_folder(starting_dir), Message::FolderPicked)
        }
        Message::FolderPicked(Some(path)) => {
            let load_task = start_loading_configs(state, path.clone());
            let settings = AppSettings {
                last_instance: Some(path),
            };
            let save_task = Task::perform(save_settings(settings), |()| Message::SettingsSaved);
            Task::batch([load_task, save_task])
        }
        Message::FolderPicked(None) => Task::none(),
        Message::ConfigsLoaded(Ok((mut store, errors))) => {
            let mut status_parts = Vec::new();
            // Some mods ship configs in formats other than Forge's Configuration class (raw
            // JSON, ChickenBones' older format, etc). Those are expected to fail here and are
            // simply left unmanaged rather than treated as corruption.
            state.skipped_files = errors.len();

            // A profile must always be selected - fall back to an existing one, or create an
            // empty "Default" profile the first time the program is ever used.
            let mut created_default = false;
            if state.selected_profile.is_none() {
                if state.profile_store.profiles.is_empty() {
                    state
                        .profile_store
                        .upsert("Default".to_string(), Changeset::default());
                    created_default = true;
                }
                state.selected_profile = state
                    .profile_store
                    .profiles
                    .first()
                    .map(|profile| profile.name.clone());
            }

            let active_changeset = state
                .selected_profile
                .as_ref()
                .and_then(|name| state.profile_store.get(name))
                .map(|profile| profile.changeset.clone())
                .unwrap_or_default();

            let unresolved = active_changeset.apply(&mut store);
            if !unresolved.is_empty() {
                status_parts.push(format!(
                    "{} edit(s) in the selected profile no longer apply",
                    unresolved.len()
                ));
            }
            state.changeset = active_changeset;
            state.profile_dirty = false;

            state.ranks_on_disk =
                tracked_files::read(&store.minecraft_dir, Path::new(tracked_files::RANKS_FILE));
            state.index = SearchIndex::build(&store);
            state.mods = tree::group_mods(&store);
            state.store = Some(store);
            state.status = (!status_parts.is_empty()).then(|| status_parts.join("; "));

            if created_default {
                Task::perform(
                    save_profile_store(state.profile_store.clone()),
                    Message::ProfileStoreSaved,
                )
            } else {
                Task::none()
            }
        }
        Message::ConfigsLoaded(Err(message)) => {
            state.status = Some(message);
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
            let Dialog::EditList { path, content } =
                std::mem::replace(&mut state.dialog, Dialog::None)
            else {
                return Task::none();
            };
            let values = content
                .text()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            edit_property(state, path, PropertyValue::List(values));
            Task::none()
        }
        Message::Save => persist_current_changeset(state),
        Message::SelectProfileRequested(name) => {
            if state.selected_profile.as_deref() == Some(name.as_str()) {
                return Task::none();
            }
            if state.profile_dirty {
                state.dialog = Dialog::UnsavedChanges {
                    pending: PendingAction::SwitchProfile(name),
                };
            } else {
                switch_to_profile(state, &name);
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
            if state.profile_dirty {
                state.dialog = Dialog::UnsavedChanges {
                    pending: PendingAction::NewProfile,
                };
            } else {
                state.dialog = Dialog::NewProfile {
                    name: String::new(),
                    error: None,
                };
            }
            Task::none()
        }
        Message::NewProfileNameChanged(name) => {
            if let Dialog::NewProfile { name: current, .. } = &mut state.dialog {
                *current = name;
            }
            Task::none()
        }
        Message::CreateNewProfile { from_scratch } => {
            let Dialog::NewProfile { name, .. } = &state.dialog else {
                return Task::none();
            };
            let name = name.trim().to_string();

            if name.is_empty() {
                set_new_profile_error(state, "Enter a profile name.");
                return Task::none();
            }
            if state.profile_store.get(&name).is_some() {
                set_new_profile_error(state, "A profile with that name already exists.");
                return Task::none();
            }

            let (new_changeset, new_tracked_files) = if from_scratch {
                if let Some(store) = &mut state.store {
                    state.changeset.revert(store);
                }
                (Changeset::default(), Default::default())
            } else {
                let current_tracked_files = state
                    .selected_profile
                    .as_ref()
                    .and_then(|current| state.profile_store.get(current))
                    .map(|profile| profile.tracked_files.clone())
                    .unwrap_or_default();
                (state.changeset.clone(), current_tracked_files)
            };

            state
                .profile_store
                .upsert(name.clone(), new_changeset.clone());
            state
                .profile_store
                .set_tracked_files(&name, new_tracked_files);
            state.selected_profile = Some(name.clone());
            state.viewing_profile = Some(name);
            state.changeset = new_changeset;
            state.profile_dirty = false;
            state.dialog = Dialog::None;
            if let Some(store) = &state.store {
                state.index = SearchIndex::build(store);
            }

            Task::perform(
                save_profile_store(state.profile_store.clone()),
                Message::ProfileStoreSaved,
            )
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
            state.profile_store.remove(&name);
            if state.viewing_profile.as_deref() == Some(name.as_str()) {
                state.viewing_profile = None;
            }

            if state.selected_profile.as_deref() == Some(name.as_str()) {
                state.selected_profile = None;
                if state.profile_store.profiles.is_empty() {
                    state
                        .profile_store
                        .upsert("Default".to_string(), Changeset::default());
                }
                if let Some(fallback) = state
                    .profile_store
                    .profiles
                    .first()
                    .map(|profile| profile.name.clone())
                {
                    switch_to_profile(state, &fallback);
                }
            }

            Task::perform(
                save_profile_store(state.profile_store.clone()),
                Message::ProfileStoreSaved,
            )
        }
        Message::CancelDialog => {
            // A running restore can't be cancelled.
            if !matches!(state.dialog, Dialog::Restoring { .. }) {
                state.dialog = Dialog::None;
            }
            Task::none()
        }
        Message::UnsavedChangesSave => {
            let save_task = persist_current_changeset(state);
            let follow_up_task = resolve_pending_dialog_action(state);
            Task::batch([save_task, follow_up_task])
        }
        Message::UnsavedChangesDiscard => {
            if let Some(name) = state.selected_profile.clone() {
                // Reset back to the profile's last-saved state, discarding unsaved edits.
                switch_to_profile(state, &name);
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
            state.dialog = Dialog::Restoring {
                name: backup.name.clone(),
            };
            Task::perform(
                restore_backup(minecraft_dir, dir, backup),
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
    }
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
    let same_shape = matches!(
        (&original_value, &new_value),
        (PropertyValue::Single(_), PropertyValue::Single(_))
            | (PropertyValue::List(_), PropertyValue::List(_))
    );
    if same_shape
        && original_value != new_value
        && store.set_property_value(&path, new_value.clone())
    {
        state.index.update_value(&path, &new_value);
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
        state.ranks_on_disk =
            tracked_files::read(&store.minecraft_dir, Path::new(tracked_files::RANKS_FILE));
    }
}

/// Resolves whatever action was deferred behind the "unsaved changes" dialog: either switching
/// to the target profile, or opening the "new profile" dialog with a clean slate.
fn resolve_pending_dialog_action(state: &mut State) -> Task<Message> {
    let Dialog::UnsavedChanges { pending } = std::mem::replace(&mut state.dialog, Dialog::None)
    else {
        return Task::none();
    };
    match pending {
        PendingAction::SwitchProfile(name) => switch_to_profile(state, &name),
        PendingAction::NewProfile => {
            state.dialog = Dialog::NewProfile {
                name: String::new(),
                error: None,
            };
        }
    }
    Task::none()
}

fn set_new_profile_error(state: &mut State, message: &str) {
    if let Dialog::NewProfile { error, .. } = &mut state.dialog {
        *error = Some(message.to_string());
    }
}

/// Writes dirty config files to disk and persists the current changeset into the selected
/// profile.
fn persist_current_changeset(state: &mut State) -> Task<Message> {
    if let Some(store) = &mut state.store {
        let errors = store.save_dirty();
        state.status = if errors.is_empty() {
            Some("Saved.".to_string())
        } else {
            Some(format!("Failed to save {} file(s)", errors.len()))
        };
    }

    let Some(name) = state.selected_profile.clone() else {
        return Task::none();
    };
    state
        .profile_store
        .upsert(name.clone(), state.changeset.clone());
    if let Some(store) = &state.store {
        state
            .profile_store
            .set_tracked_files(&name, tracked_files::capture(&store.minecraft_dir));
    }
    refresh_ranks_on_disk(state);
    state.profile_dirty = false;
    Task::perform(
        save_profile_store(state.profile_store.clone()),
        Message::ProfileStoreSaved,
    )
}

/// Synchronously resets the loaded config store from whatever `changeset` currently reflects
/// to `name`'s saved state, by reverting the former and re-applying the latter in memory - no
/// disk re-read needed, since `original_value`s already capture the true on-disk values. Also
/// writes the profile's stored whole files (e.g. ranks.txt) straight to disk.
fn switch_to_profile(state: &mut State, name: &str) {
    let Some(store) = &mut state.store else {
        return;
    };

    state.changeset.revert(store);
    let (target, target_files) = state
        .profile_store
        .get(name)
        .map(|profile| (profile.changeset.clone(), profile.tracked_files.clone()))
        .unwrap_or_default();
    let unresolved = target.apply(store);
    let file_errors = tracked_files::write_all(&store.minecraft_dir, &target_files);

    state.changeset = target;
    state.selected_profile = Some(name.to_string());
    state.profile_dirty = false;
    state.index = SearchIndex::build(store);
    refresh_ranks_on_disk(state);

    let mut status_parts = Vec::new();
    if !unresolved.is_empty() {
        status_parts.push(format!(
            "{} edit(s) in this profile no longer apply",
            unresolved.len()
        ));
    }
    for error in file_errors {
        status_parts.push(format!(
            "Failed to write {}: {}",
            error.relative_path.display(),
            error.message
        ));
    }
    state.status = (!status_parts.is_empty()).then(|| status_parts.join("; "));
}

fn start_loading_configs(state: &mut State, instance_path: PathBuf) -> Task<Message> {
    state.instance_path = Some(instance_path.clone());
    state.store = None;
    state.index = SearchIndex::default();
    state.mods.clear();
    state.selection = None;
    state.expanded.clear();
    state.highlighted = None;
    state.status = None;

    let minecraft_dir = instance_path.join(".minecraft");
    Task::perform(load_configs(minecraft_dir), Message::ConfigsLoaded)
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
    let content = if state.instance_path.is_some() && state.status.is_none() {
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

async fn load_settings() -> AppSettings {
    AppSettings::load()
}

async fn save_settings(settings: AppSettings) {
    let _ = settings.save();
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

async fn load_configs(minecraft_dir: PathBuf) -> Result<(ConfigStore, Vec<LoadError>), String> {
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

/// Runs the restore on its own thread - backups can be gigabytes, and blocking an executor
/// thread for that long would stall other tasks.
async fn restore_backup(
    minecraft_dir: PathBuf,
    backups_dir: PathBuf,
    backup: BackupInfo,
) -> Result<RestoreReport, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(backups::restore(&minecraft_dir, &backups_dir, &backup));
    });
    receiver
        .await
        .unwrap_or_else(|_| Err("The restore stopped unexpectedly.".to_string()))
}

async fn load_profile_store() -> ProfileStore {
    ProfileStore::load()
}

async fn save_profile_store(profile_store: ProfileStore) -> Result<(), String> {
    profile_store.save().map_err(|err| err.to_string())
}
