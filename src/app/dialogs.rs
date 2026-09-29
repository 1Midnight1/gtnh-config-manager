//! Modal dialogs, drawn as a card over a dimmed copy of the page (see `super::view`).

use iced::widget::{
    button, column, container, row, scrollable, space, text, text_editor, text_input,
};
use iced::{Alignment, Element, Font, Length, font};

use super::{Dialog, Message, State, style};
use crate::tree;

const BOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

/// The active dialog's card, or `None` when no dialog is open.
pub fn view(state: &State) -> Option<Element<'_, Message>> {
    let (title, body, buttons, width): (String, Element<'_, Message>, Vec<Element<'_, Message>>, f32) =
        match &state.dialog {
            Dialog::None => return None,
            Dialog::UnsavedChanges { .. } => (
                "Unsaved changes".to_string(),
                text("The current profile has unsaved changes. Save them before continuing?")
                    .into(),
                vec![
                    cancel(),
                    secondary("Discard", Message::UnsavedChangesDiscard),
                    primary("Save", Message::UnsavedChangesSave),
                ],
                420.0,
            ),
            Dialog::NewProfile { name, error } => {
                let mut body = column![
                    text_input("Profile name", name)
                        .on_input(Message::NewProfileNameChanged)
                        .on_submit(Message::CreateNewProfile {
                            from_scratch: false
                        })
                        .padding(8),
                    text("Start from a copy of the current profile's changes, or from the files as they are on disk.")
                        .size(12)
                        .style(style::muted_text),
                ]
                .spacing(8);
                if let Some(error) = error {
                    body = body.push(text(error).size(13).style(style::danger_text));
                }
                (
                    "New profile".to_string(),
                    body.into(),
                    vec![
                        cancel(),
                        secondary(
                            "Start from scratch",
                            Message::CreateNewProfile { from_scratch: true },
                        ),
                        primary(
                            "Copy current",
                            Message::CreateNewProfile {
                                from_scratch: false,
                            },
                        ),
                    ],
                    460.0,
                )
            }
            Dialog::ConfirmDelete { name } => (
                "Delete profile".to_string(),
                text(format!("Delete profile \"{name}\"? This cannot be undone.")).into(),
                vec![
                    cancel(),
                    button(text("Delete"))
                        .padding([6, 14])
                        .style(button::danger)
                        .on_press(Message::ConfirmDeleteProfile)
                        .into(),
                ],
                400.0,
            ),
            Dialog::RestoreBackup { dir, backups } => {
                let list: Element<'_, Message> = if backups.is_empty() {
                    text("No backups found.").style(style::muted_text).into()
                } else {
                    let rows: Vec<Element<'_, Message>> = backups
                        .iter()
                        .map(|backup| {
                            let content = row![
                                column![
                                    text(&backup.name).size(14),
                                    text(backup.world.as_deref().unwrap_or("(unknown world)"))
                                        .size(12)
                                        .style(style::muted_text),
                                ]
                                .spacing(2),
                                space::horizontal(),
                                text(format_size(backup.size))
                                    .size(12)
                                    .style(style::muted_text),
                            ]
                            .align_y(Alignment::Center);
                            button(content)
                                .width(Length::Fill)
                                .padding([6, 10])
                                .style(|theme, status| style::list_row(theme, status, false))
                                .on_press(Message::RestoreBackupSelected(backup.clone()))
                                .into()
                        })
                        .collect();
                    scrollable(column(rows).spacing(2))
                        .height(Length::Fixed(360.0))
                        .into()
                };
                (
                    "Restore a backup".to_string(),
                    column![
                        text(format!("Backups in {}", dir.display()))
                            .size(12)
                            .style(style::muted_text),
                        list,
                    ]
                    .spacing(8)
                    .into(),
                    vec![cancel()],
                    560.0,
                )
            }
            Dialog::ConfirmRestore { backup } => (
                "Restore backup".to_string(),
                column![
                    text(format!(
                        "Restore backup {} of world \"{}\"?",
                        backup.name,
                        backup.world.as_deref().unwrap_or("(unknown world)")
                    )),
                    text(
                        "Close Minecraft before continuing. The world's current folders will first be \
                         backed up to a new zip in the backups folder, then replaced with this backup's \
                         contents."
                    )
                    .size(13)
                    .style(style::muted_text),
                ]
                .spacing(8)
                .into(),
                vec![cancel(), primary("Restore", Message::ConfirmRestore)],
                460.0,
            ),
            Dialog::Restoring { name } => (
                "Restoring…".to_string(),
                text(format!(
                    "Restoring {name}. This can take a while for large worlds."
                ))
                .into(),
                Vec::new(),
                420.0,
            ),
            Dialog::EditList { path, content } => (
                format!("Edit {}", path.property_name),
                column![
                    text(tree::breadcrumb(path)).size(12).style(style::muted_text),
                    text("One entry per line. Blank lines are ignored.")
                        .size(12)
                        .style(style::muted_text),
                    text_editor(content)
                        .on_action(Message::ListEditorAction)
                        .font(Font::MONOSPACE)
                        .size(13)
                        .height(Length::Fixed(320.0)),
                ]
                .spacing(8)
                .into(),
                vec![cancel(), primary("Apply", Message::ConfirmListEdit)],
                520.0,
            ),
        };

    let mut card = column![text(title).size(18).font(BOLD), body].spacing(14);
    if !buttons.is_empty() {
        card = card.push(
            row![space::horizontal()]
                .extend(buttons)
                .spacing(8)
                .align_y(Alignment::Center),
        );
    }

    Some(
        container(card)
            .padding(20)
            .width(width)
            .style(style::section)
            .into(),
    )
}

fn cancel<'a>() -> Element<'a, Message> {
    secondary("Cancel", Message::CancelDialog)
}

fn primary(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label))
        .padding([6, 14])
        .style(button::primary)
        .on_press(message)
        .into()
}

fn secondary(label: &str, message: Message) -> Element<'_, Message> {
    button(text(label))
        .padding([6, 14])
        .style(button::secondary)
        .on_press(message)
        .into()
}

fn format_size(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
