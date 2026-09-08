//! iced `State`/`Message`/`update`/`view` glue. Kept thin - the actual logic lives in the
//! pure `config_store`, `search`, `settings` and `changeset` modules so it stays unit-testable
//! without spinning up the GUI.

use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Task};

use crate::changeset::Changeset;
use crate::config_store::{ConfigStore, LoadError, PropertyPath};
use crate::forge_cfg::PropertyValue;
use crate::profiles::{Profile, ProfileStore};
use crate::search::{self, IndexedProperty, SearchIndex};
use crate::settings::AppSettings;

/// Cap on how many search results are turned into widgets at once - iced 0.14 has no
/// virtualized list, so an unbounded query match could otherwise build thousands of rows.
const MAX_VISIBLE_RESULTS: usize = 200;

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
    /// Name of the profile currently expanded to show its diff, if any.
    viewing_profile: Option<String>,
    dialog: Dialog,
    status: Option<String>,
}

#[derive(Debug, Clone, Default)]
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
    SearchChanged(String),
    PropertyEdited(PropertyPath, String),
    Save,
    SelectProfileRequested(String),
    ViewProfile(String),
    NewProfileRequested,
    NewProfileNameChanged(String),
    CreateNewProfile { from_scratch: bool },
    DeleteProfileRequested(String),
    ConfirmDeleteProfile,
    CancelDialog,
    UnsavedChangesSave,
    UnsavedChangesDiscard,
}

pub fn boot() -> (State, Task<Message>) {
    let settings_task = Task::perform(load_settings(), Message::SettingsLoaded);
    let profiles_task = Task::perform(load_profile_store(), Message::ProfileStoreLoaded);
    (State::default(), Task::batch([settings_task, profiles_task]))
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
            let starting_dir =
                state.instance_path.as_deref().and_then(|path| path.parent()).map(|parent| parent.to_path_buf());
            Task::perform(pick_folder(starting_dir), Message::FolderPicked)
        }
        Message::FolderPicked(Some(path)) => {
            let load_task = start_loading_configs(state, path.clone());
            let settings = AppSettings { last_instance: Some(path) };
            let save_task = Task::perform(save_settings(settings), |()| Message::SettingsSaved);
            Task::batch([load_task, save_task])
        }
        Message::FolderPicked(None) => Task::none(),
        Message::ConfigsLoaded(Ok((mut store, errors))) => {
            let mut status_parts = Vec::new();
            // Some mods ship configs in formats other than Forge's Configuration class (raw
            // JSON, ChickenBones' older format, etc). Those are expected to fail here and are
            // simply left unmanaged rather than treated as corruption.
            if !errors.is_empty() {
                status_parts.push(format!("{} config file(s) use an unsupported format and were skipped", errors.len()));
            }

            // A profile must always be selected - fall back to an existing one, or create an
            // empty "Default" profile the first time the program is ever used.
            let mut created_default = false;
            if state.selected_profile.is_none() {
                if state.profile_store.profiles.is_empty() {
                    state.profile_store.upsert("Default".to_string(), Changeset::default());
                    created_default = true;
                }
                state.selected_profile = state.profile_store.profiles.first().map(|profile| profile.name.clone());
            }

            let active_changeset = state
                .selected_profile
                .as_ref()
                .and_then(|name| state.profile_store.get(name))
                .map(|profile| profile.changeset.clone())
                .unwrap_or_default();

            let unresolved = active_changeset.apply(&mut store);
            if !unresolved.is_empty() {
                status_parts.push(format!("{} edit(s) in the selected profile no longer apply", unresolved.len()));
            }
            state.changeset = active_changeset;
            state.profile_dirty = false;

            state.index = SearchIndex::build(&store);
            state.store = Some(store);
            state.status = (!status_parts.is_empty()).then(|| status_parts.join("; "));

            if created_default {
                Task::perform(save_profile_store(state.profile_store.clone()), Message::ProfileStoreSaved)
            } else {
                Task::none()
            }
        }
        Message::ConfigsLoaded(Err(message)) => {
            state.status = Some(message);
            Task::none()
        }
        Message::SearchChanged(query) => {
            state.search_query = query;
            Task::none()
        }
        Message::PropertyEdited(path, value) => {
            let new_value = PropertyValue::Single(value);
            if let Some(store) = &mut state.store {
                let original_value = store.get_property(&path).map(|property| property.value.clone());
                if let Some(original_value) = original_value {
                    if store.set_property_value(&path, new_value.clone()) {
                        state.index.update_value(&path, &new_value);
                        state.changeset.record(path, original_value, new_value);
                        state.profile_dirty = true;
                    }
                }
            }
            Task::none()
        }
        Message::Save => persist_current_changeset(state),
        Message::SelectProfileRequested(name) => {
            if state.selected_profile.as_deref() == Some(name.as_str()) {
                return Task::none();
            }
            if state.profile_dirty {
                state.dialog = Dialog::UnsavedChanges { pending: PendingAction::SwitchProfile(name) };
            } else {
                switch_to_profile(state, &name);
            }
            Task::none()
        }
        Message::ViewProfile(name) => {
            state.viewing_profile =
                if state.viewing_profile.as_deref() == Some(name.as_str()) { None } else { Some(name) };
            Task::none()
        }
        Message::NewProfileRequested => {
            if state.store.is_none() {
                state.status = Some("Select a modpack folder first.".to_string());
                return Task::none();
            }
            if state.profile_dirty {
                state.dialog = Dialog::UnsavedChanges { pending: PendingAction::NewProfile };
            } else {
                state.dialog = Dialog::NewProfile { name: String::new(), error: None };
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

            let new_changeset = if from_scratch {
                if let Some(store) = &mut state.store {
                    state.changeset.revert(store);
                }
                Changeset::default()
            } else {
                state.changeset.clone()
            };

            state.profile_store.upsert(name.clone(), new_changeset.clone());
            state.selected_profile = Some(name);
            state.changeset = new_changeset;
            state.profile_dirty = false;
            state.dialog = Dialog::None;
            if let Some(store) = &state.store {
                state.index = SearchIndex::build(store);
            }

            Task::perform(save_profile_store(state.profile_store.clone()), Message::ProfileStoreSaved)
        }
        Message::DeleteProfileRequested(name) => {
            state.dialog = Dialog::ConfirmDelete { name };
            Task::none()
        }
        Message::ConfirmDeleteProfile => {
            let Dialog::ConfirmDelete { name } = std::mem::replace(&mut state.dialog, Dialog::None) else {
                return Task::none();
            };
            state.profile_store.remove(&name);
            if state.viewing_profile.as_deref() == Some(name.as_str()) {
                state.viewing_profile = None;
            }

            if state.selected_profile.as_deref() == Some(name.as_str()) {
                state.selected_profile = None;
                if state.profile_store.profiles.is_empty() {
                    state.profile_store.upsert("Default".to_string(), Changeset::default());
                }
                if let Some(fallback) = state.profile_store.profiles.first().map(|profile| profile.name.clone()) {
                    switch_to_profile(state, &fallback);
                }
            }

            Task::perform(save_profile_store(state.profile_store.clone()), Message::ProfileStoreSaved)
        }
        Message::CancelDialog => {
            state.dialog = Dialog::None;
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
    }
}

/// Resolves whatever action was deferred behind the "unsaved changes" dialog: either switching
/// to the target profile, or opening the "new profile" dialog with a clean slate.
fn resolve_pending_dialog_action(state: &mut State) -> Task<Message> {
    let Dialog::UnsavedChanges { pending } = std::mem::replace(&mut state.dialog, Dialog::None) else {
        return Task::none();
    };
    match pending {
        PendingAction::SwitchProfile(name) => switch_to_profile(state, &name),
        PendingAction::NewProfile => {
            state.dialog = Dialog::NewProfile { name: String::new(), error: None };
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
    state.profile_store.upsert(name, state.changeset.clone());
    state.profile_dirty = false;
    Task::perform(save_profile_store(state.profile_store.clone()), Message::ProfileStoreSaved)
}

/// Synchronously resets the loaded config store from whatever `changeset` currently reflects
/// to `name`'s saved state, by reverting the former and re-applying the latter in memory - no
/// disk re-read needed, since `original_value`s already capture the true on-disk values.
fn switch_to_profile(state: &mut State, name: &str) {
    let Some(store) = &mut state.store else { return };

    state.changeset.revert(store);
    let target = state.profile_store.get(name).map(|profile| profile.changeset.clone()).unwrap_or_default();
    let unresolved = target.apply(store);

    state.changeset = target;
    state.selected_profile = Some(name.to_string());
    state.profile_dirty = false;
    state.index = SearchIndex::build(store);
    state.status =
        (!unresolved.is_empty()).then(|| format!("{} edit(s) in this profile no longer apply", unresolved.len()));
}

fn start_loading_configs(state: &mut State, instance_path: PathBuf) -> Task<Message> {
    state.instance_path = Some(instance_path.clone());
    state.store = None;
    state.index = SearchIndex::default();
    state.status = None;

    let minecraft_dir = instance_path.join(".minecraft");
    Task::perform(load_configs(minecraft_dir), Message::ConfigsLoaded)
}

pub fn view(state: &State) -> Element<'_, Message> {
    if !matches!(state.dialog, Dialog::None) {
        return dialog_view(state);
    }

    let path_label = match &state.instance_path {
        Some(path) => text(path.display().to_string()),
        None => text("No folder selected"),
    };

    let save_button = button("Save").on_press_maybe(state.profile_dirty.then_some(Message::Save));
    let new_profile_button = button("New profile...").on_press(Message::NewProfileRequested);

    let mut content = column![
        row![button("Select modpack folder").on_press(Message::PickFolder), save_button, new_profile_button]
            .spacing(10),
        path_label,
        profiles_panel(state),
        text_input("Search configs...", &state.search_query).on_input(Message::SearchChanged),
    ]
    .spacing(10)
    .padding(20);

    if let Some(status) = &state.status {
        content = content.push(text(status));
    }

    let matches = state.index.filter(&state.search_query);
    let total = matches.len();
    let rows: Vec<Element<'_, Message>> =
        matches.into_iter().take(MAX_VISIBLE_RESULTS).map(property_row).collect();

    content = content.push(scrollable(column(rows).spacing(6)).height(Length::Fill));

    if total > MAX_VISIBLE_RESULTS {
        content = content.push(text(format!("{} more result(s) not shown - refine your search", total - MAX_VISIBLE_RESULTS)));
    }

    container(content).into()
}

/// Renders the active dialog as the entire window content, blocking interaction with the rest
/// of the UI until it's resolved - iced 0.14 has no built-in modal/overlay primitive, so this
/// is the simplest way to force the user to address it first.
fn dialog_view(state: &State) -> Element<'_, Message> {
    let content: Element<'_, Message> = match &state.dialog {
        Dialog::None => unreachable!("dialog_view is only called when a dialog is active"),
        Dialog::UnsavedChanges { .. } => column![
            text("You have unsaved changes in the current profile. Save them before continuing?"),
            row![
                button("Save").on_press(Message::UnsavedChangesSave),
                button("Discard").on_press(Message::UnsavedChangesDiscard),
                button("Cancel").on_press(Message::CancelDialog),
            ]
            .spacing(10),
        ]
        .spacing(10)
        .into(),
        Dialog::NewProfile { name, error } => {
            let mut dialog = column![
                text("Create new profile"),
                text_input("Profile name...", name).on_input(Message::NewProfileNameChanged),
            ]
            .spacing(10);

            if let Some(error) = error {
                dialog = dialog.push(text(error));
            }

            dialog
                .push(
                    row![
                        button("Start from current profile").on_press(Message::CreateNewProfile { from_scratch: false }),
                        button("Start from scratch").on_press(Message::CreateNewProfile { from_scratch: true }),
                        button("Cancel").on_press(Message::CancelDialog),
                    ]
                    .spacing(10),
                )
                .into()
        }
        Dialog::ConfirmDelete { name } => column![
            text(format!("Delete profile \"{name}\"? This cannot be undone.")),
            row![
                button("Delete").on_press(Message::ConfirmDeleteProfile),
                button("Cancel").on_press(Message::CancelDialog),
            ]
            .spacing(10),
        ]
        .spacing(10)
        .into(),
    };

    container(content).padding(20).into()
}

fn profiles_panel(state: &State) -> Element<'_, Message> {
    let mut panel = column![].spacing(6);

    for profile in &state.profile_store.profiles {
        let is_selected = state.selected_profile.as_deref() == Some(profile.name.as_str());
        let is_viewing = state.viewing_profile.as_deref() == Some(profile.name.as_str());
        let label = if is_selected { format!("* {} (selected)", profile.name) } else { profile.name.clone() };

        panel = panel.push(
            row![
                text(label).width(Length::FillPortion(2)),
                text(format!("{} change(s)", profile.changeset.entries.len())).width(Length::FillPortion(1)),
                button("Select")
                    .on_press_maybe((!is_selected).then(|| Message::SelectProfileRequested(profile.name.clone()))),
                button(if is_viewing { "Hide" } else { "View" }).on_press(Message::ViewProfile(profile.name.clone())),
                button("Delete").on_press(Message::DeleteProfileRequested(profile.name.clone())),
            ]
            .spacing(10),
        );

        if is_viewing {
            panel = panel.push(profile_diff_view(profile));
        }
    }

    container(panel).into()
}

fn profile_diff_view(profile: &Profile) -> Element<'_, Message> {
    let rows: Vec<Element<'_, Message>> = profile
        .changeset
        .entries
        .iter()
        .map(|entry| {
            let label = format!(
                "{}  {}  {}: {} -> {}",
                entry.path.relative_path.display(),
                entry.path.category_path.join("/"),
                entry.path.property_name,
                search::display_value(&entry.original_value),
                search::display_value(&entry.new_value),
            );
            text(label).into()
        })
        .collect();

    container(column(rows).spacing(4).padding(10)).into()
}

fn property_row(entry: &IndexedProperty) -> Element<'_, Message> {
    let label = format!(
        "{}  {}  {}",
        entry.path.relative_path.display(),
        entry.path.category_path.join("/"),
        entry.path.property_name
    );

    row![
        text(label).width(Length::FillPortion(3)),
        text_input("value", &entry.display_value)
            .on_input({
                let path = entry.path.clone();
                move |value| Message::PropertyEdited(path.clone(), value)
            })
            .width(Length::FillPortion(2)),
    ]
    .spacing(10)
    .into()
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

    dialog.pick_folder().await.map(|handle| handle.path().to_path_buf())
}

async fn load_configs(minecraft_dir: PathBuf) -> Result<(ConfigStore, Vec<LoadError>), String> {
    if !minecraft_dir.is_dir() {
        return Err(format!("{} does not exist - is this a GTNH instance folder?", minecraft_dir.display()));
    }
    Ok(ConfigStore::load(&minecraft_dir))
}

async fn load_profile_store() -> ProfileStore {
    ProfileStore::load()
}

async fn save_profile_store(profile_store: ProfileStore) -> Result<(), String> {
    profile_store.save().map_err(|err| err.to_string())
}
