//! The Profiles page: the list of profiles, and for the one being viewed its changes (each
//! linking back into the config browser) and its ranks.txt.

use std::path::Path;

use iced::widget::{button, column, container, row, rule, scrollable, space, text};
use iced::{Alignment, Element, Font, Length};

use super::{Message, RanksSource, State, style};
use crate::profiles::Profile;
use crate::tracked_files;
use crate::tree;

const LIST_WIDTH: f32 = 280.0;

pub fn view(state: &State) -> Element<'_, Message> {
    let viewing = state
        .viewing_profile
        .as_ref()
        .or(state.selected_profile.as_ref())
        .and_then(|name| state.profile_store.get(name));

    let detail: Element<'_, Message> = match viewing {
        Some(profile) => profile_detail(state, profile),
        None => container(text("No profile selected").style(style::muted_text))
            .padding(24)
            .into(),
    };

    row![
        profile_list(state, viewing.map(|profile| profile.name.as_str())),
        rule::vertical(1),
        container(detail).width(Length::Fill).height(Length::Fill),
    ]
    .into()
}

fn profile_list<'a>(state: &'a State, viewing: Option<&str>) -> Element<'a, Message> {
    let rows: Vec<Element<'_, Message>> = state
        .profile_store
        .profiles
        .iter()
        .map(|profile| {
            let active = state.selected_profile.as_deref() == Some(profile.name.as_str());
            let selected = viewing == Some(profile.name.as_str());

            let mut title = row![text(&profile.name).size(14).font(style::BOLD)]
                .spacing(8)
                .align_y(Alignment::Center);
            if active {
                title = title.push(
                    container(text("active").size(11))
                        .padding([1, 6])
                        .style(style::badge),
                );
            }
            let content = column![
                title,
                text(change_summary(profile))
                    .size(12)
                    .style(style::muted_text),
            ]
            .spacing(2);

            button(content)
                .width(Length::Fill)
                .padding([8, 10])
                .style(move |theme, status| style::list_row(theme, status, selected))
                .on_press(Message::ViewProfile(profile.name.clone()))
                .into()
        })
        .collect();

    container(
        column![
            row![
                text("Profiles").size(16).font(style::BOLD),
                space::horizontal(),
                button(text("New…").size(13))
                    .padding([4, 10])
                    .style(button::secondary)
                    .on_press(Message::NewProfileRequested),
            ]
            .align_y(Alignment::Center),
            scrollable(column(rows).spacing(2)).height(Length::Fill),
        ]
        .spacing(12)
        .padding(12),
    )
    .width(LIST_WIDTH)
    .height(Length::Fill)
    .style(style::panel)
    .into()
}

fn change_summary(profile: &Profile) -> String {
    match profile.changeset.entries.len() {
        1 => "1 change".to_string(),
        n => format!("{n} changes"),
    }
}

fn profile_detail<'a>(state: &'a State, profile: &'a Profile) -> Element<'a, Message> {
    let active = state.selected_profile.as_deref() == Some(profile.name.as_str());

    let mut actions = row![].spacing(8);
    if active {
        actions = actions.push(text("Currently active").size(13).style(style::muted_text));
    } else {
        actions = actions.push(
            button(text("Make active").size(13))
                .padding([5, 12])
                .style(button::primary)
                .on_press(Message::SelectProfileRequested(profile.name.clone())),
        );
    }
    actions = actions.push(
        button(text("Delete").size(13))
            .padding([5, 12])
            .style(button::danger)
            .on_press(Message::DeleteProfileRequested(profile.name.clone())),
    );

    let header = row![
        column![
            text(&profile.name).size(22).font(style::BOLD),
            text(change_summary(profile))
                .size(13)
                .style(style::muted_text),
        ]
        .spacing(2),
        space::horizontal(),
        actions.align_y(Alignment::Center),
    ]
    .align_y(Alignment::Center);

    let mut note = None;
    if active && state.profile_dirty {
        note = Some(
            text("This profile has unsaved edits, which aren't shown below until you save.")
                .size(12)
                .style(style::warning_text),
        );
    }

    let mut content = column![header].spacing(16).padding(20);
    if let Some(note) = note {
        content = content.push(note);
    }
    content = content
        .push(section_title("Changes"))
        .push(changes_table(profile))
        .push(section_title("ranks.txt"))
        .push(ranks_panel(state, profile));

    scrollable(content).height(Length::Fill).into()
}

fn section_title(label: &str) -> Element<'_, Message> {
    text(label).size(15).font(style::BOLD).into()
}

fn changes_table(profile: &Profile) -> Element<'_, Message> {
    if profile.changeset.entries.is_empty() {
        return container(
            text("No settings changed - this profile uses the files as they are on disk.")
                .size(13)
                .style(style::muted_text),
        )
        .padding(12)
        .width(Length::Fill)
        .style(style::section)
        .into();
    }

    let rows: Vec<Element<'_, Message>> = profile
        .changeset
        .entries
        .iter()
        .map(|entry| {
            let original = style::truncate(&entry.original_value.display_text(), 60);
            let new = style::truncate(&entry.new_value.display_text(), 60);
            let content = row![
                column![
                    text(&entry.path.property_name).size(14),
                    text(tree::breadcrumb(&entry.path))
                        .size(12)
                        .style(style::muted_text),
                ]
                .spacing(2)
                .width(Length::FillPortion(3)),
                row![
                    text(original)
                        .size(12)
                        .font(Font::MONOSPACE)
                        .style(style::muted_text),
                    text("→").size(12).style(style::muted_text),
                    text(new).size(12).font(Font::MONOSPACE),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(2)),
            ]
            .spacing(12)
            .align_y(Alignment::Center);

            button(content)
                .width(Length::Fill)
                .padding([6, 10])
                .style(|theme, status| style::list_row(theme, status, false))
                .on_press(Message::JumpTo(entry.path.clone()))
                .into()
        })
        .collect();

    container(column(rows).spacing(2).padding(4))
        .width(Length::Fill)
        .style(style::section)
        .into()
}

fn ranks_panel<'a>(state: &'a State, profile: &'a Profile) -> Element<'a, Message> {
    let segment = |label, source| {
        let active = state.ranks_source == source;
        button(text(label).size(13))
            .padding([4, 12])
            .style(move |theme, status| style::tab(theme, status, active))
            .on_press(Message::SetRanksSource(source))
    };

    let (contents, missing) = match state.ranks_source {
        RanksSource::Disk => (state.ranks_on_disk.as_deref(), "Not present on disk."),
        RanksSource::Profile => (
            profile
                .tracked_files
                .get(Path::new(tracked_files::RANKS_FILE))
                .map(String::as_str),
            "Not stored in this profile - it's captured the next time the profile is saved.",
        ),
    };

    let body: Element<'_, Message> = match contents {
        Some(contents) => scrollable(
            container(text(contents).size(12).font(Font::MONOSPACE))
                .padding(12)
                .width(Length::Fill),
        )
        .height(Length::Fixed(420.0))
        .into(),
        None => container(text(missing).size(13).style(style::muted_text))
            .padding(12)
            .into(),
    };

    column![
        row![
            segment("On disk", RanksSource::Disk),
            segment("Stored in profile", RanksSource::Profile),
            space::horizontal(),
            text(tracked_files::RANKS_FILE)
                .size(12)
                .style(style::muted_text),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
        container(body).width(Length::Fill).style(style::section),
    ]
    .spacing(8)
    .into()
}
