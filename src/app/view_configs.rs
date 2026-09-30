//! The Configs page: a sidebar tree (mod → file → category), the property pane for the
//! selected category, and global search results that jump into the tree.

use std::path::Path;

use iced::widget::{
    button, center, column, container, row, rule, scrollable, space, text, text_input, toggler,
};
use iced::{Alignment, Element, Font, Length};

use super::{
    Location, MAX_VISIBLE_RESULTS, Message, NodeKey, PROPERTIES_SCROLL, SIDEBAR_SCROLL, State,
    style,
};
use crate::config_store::{ConfigStore, PropertyPath};
use crate::forge_cfg::{Item, Property, PropertyType, PropertyValue};
use crate::tree::{self, CategoryNode};

const SIDEBAR_WIDTH: f32 = 300.0;
const INDENT: f32 = 14.0;
const EDITOR_WIDTH: f32 = 280.0;
/// How many entries of a list value are shown before "… and N more".
const LIST_PREVIEW: usize = 6;

/// One visible line of the sidebar, flattened from the mod/file/category tree according to
/// which nodes are expanded. Also used by `jump_to` to work out where to scroll the sidebar.
pub struct SidebarRow {
    pub kind: RowKind,
    pub depth: u16,
    pub label: String,
    pub detail: Option<String>,
    pub expandable: bool,
    pub expanded: bool,
    pub selected: bool,
}

pub enum RowKind {
    Header,
    Node(NodeKey),
}

pub fn sidebar_rows(state: &State) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    let Some(store) = &state.store else {
        return rows;
    };
    let filter = state.mod_filter.trim().to_lowercase();
    let mut current_section = None;

    for group in state
        .mods
        .iter()
        .filter(|group| filter.is_empty() || group.display_name.to_lowercase().contains(&filter))
    {
        if current_section != Some(group.section) {
            current_section = Some(group.section);
            rows.push(SidebarRow {
                kind: RowKind::Header,
                depth: 0,
                label: group.section.label().to_uppercase(),
                detail: None,
                expandable: false,
                expanded: false,
                selected: false,
            });
        }

        let key = NodeKey::Mod(group.id.clone());
        let expanded = state.expanded.contains(&key);
        let single_file = (group.files.len() == 1).then(|| group.files[0].as_path());
        rows.push(SidebarRow {
            kind: RowKind::Node(key),
            depth: 0,
            label: group.display_name.clone(),
            detail: (group.files.len() > 1).then(|| format!("{} files", group.files.len())),
            expandable: true,
            expanded,
            selected: single_file.is_some_and(|file| is_selected(state, file, &[])),
        });
        if !expanded {
            continue;
        }

        match single_file {
            Some(file) => push_categories(state, store, file, 1, &mut rows),
            None => {
                for file in &group.files {
                    let key = NodeKey::File(file.clone());
                    let expanded = state.expanded.contains(&key);
                    rows.push(SidebarRow {
                        kind: RowKind::Node(key),
                        depth: 1,
                        label: tree::file_name(file),
                        detail: None,
                        expandable: true,
                        expanded,
                        selected: is_selected(state, file, &[]),
                    });
                    if expanded {
                        push_categories(state, store, file, 2, &mut rows);
                    }
                }
            }
        }
    }
    rows
}

fn push_categories(
    state: &State,
    store: &ConfigStore,
    file: &Path,
    depth: u16,
    rows: &mut Vec<SidebarRow>,
) {
    let Some(entry) = store.entry(file) else {
        return;
    };
    for node in tree::category_tree(&entry.ast) {
        push_category(state, file, &node, depth, rows);
    }
}

fn push_category(
    state: &State,
    file: &Path,
    node: &CategoryNode,
    depth: u16,
    rows: &mut Vec<SidebarRow>,
) {
    let key = NodeKey::Category(file.to_path_buf(), node.path.clone());
    let expanded = state.expanded.contains(&key);
    rows.push(SidebarRow {
        kind: RowKind::Node(key),
        depth,
        label: node.name.clone(),
        detail: (node.property_count > 0).then(|| node.property_count.to_string()),
        expandable: !node.children.is_empty(),
        expanded,
        selected: is_selected(state, file, &node.path),
    });
    if expanded {
        for child in &node.children {
            push_category(state, file, child, depth + 1, rows);
        }
    }
}

fn is_selected(state: &State, file: &Path, category_path: &[String]) -> bool {
    state
        .selection
        .as_ref()
        .is_some_and(|selection| selection.file == file && selection.category_path == category_path)
}

pub fn view(state: &State) -> Element<'_, Message> {
    row![
        sidebar(state),
        rule::vertical(1),
        container(main_pane(state))
            .width(Length::Fill)
            .height(Length::Fill)
    ]
    .into()
}

fn sidebar(state: &State) -> Element<'_, Message> {
    let rows: Vec<Element<'_, Message>> =
        sidebar_rows(state).into_iter().map(sidebar_row).collect();

    let list: Element<'_, Message> = if rows.is_empty() {
        container(text("No matching mods").size(13).style(style::muted_text))
            .padding(12)
            .into()
    } else {
        scrollable(column(rows).spacing(1).padding([4, 8]))
            .id(SIDEBAR_SCROLL)
            .height(Length::Fill)
            .into()
    };

    container(
        column![
            container(
                text_input("Filter mods…", &state.mod_filter)
                    .on_input(Message::ModFilterChanged)
                    .size(13)
                    .padding(6)
            )
            .padding([10, 8]),
            list
        ]
        .height(Length::Fill),
    )
    .width(SIDEBAR_WIDTH)
    .height(Length::Fill)
    .style(style::panel)
    .into()
}

fn sidebar_row<'a>(entry: SidebarRow) -> Element<'a, Message> {
    let key = match entry.kind {
        RowKind::Header => {
            return container(text(entry.label).size(11).style(style::muted_text))
                .padding([8, 8])
                .into();
        }
        RowKind::Node(key) => key,
    };

    let chevron = match (entry.expandable, entry.expanded) {
        (false, _) => " ",
        (true, false) => "▸",
        (true, true) => "▾",
    };
    let mut content = row![
        space().width(entry.depth as f32 * INDENT),
        text(chevron).size(12).width(12).style(style::muted_text),
        text(entry.label).size(13),
        space::horizontal(),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if let Some(detail) = entry.detail {
        content = content.push(text(detail).size(11).style(style::muted_text));
    }

    let selected = entry.selected;
    button(content)
        .width(Length::Fill)
        .padding([3, 6])
        .style(move |theme, status| style::list_row(theme, status, selected))
        .on_press(Message::NodeClicked(key))
        .into()
}

fn main_pane(state: &State) -> Element<'_, Message> {
    let mut search_row = row![
        text_input(
            "Search all settings by mod, file, category, name or value…",
            &state.search_query
        )
        .on_input(Message::SearchChanged)
        .padding(8)
        .size(14)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if !state.search_query.is_empty() {
        search_row = search_row.push(
            button(text("Clear").size(13))
                .padding([6, 12])
                .style(button::secondary)
                .on_press(Message::SearchChanged(String::new())),
        );
    }

    let content: Element<'_, Message> = match (&state.store, &state.selection) {
        (_, _) if !state.search_query.trim().is_empty() => search_results(state),
        (Some(store), Some(selection)) => category_view(state, store, selection),
        _ => center(
            column![
                text("Select a mod in the sidebar").size(18),
                text("or search for a setting above.").style(style::muted_text),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .into(),
    };

    column![container(search_row).padding([12, 16]), content]
        .height(Length::Fill)
        .into()
}

fn search_results(state: &State) -> Element<'_, Message> {
    let total = state.search_matches.len();

    let rows: Vec<Element<'_, Message>> = state
        .search_matches
        .iter()
        .take(MAX_VISIBLE_RESULTS)
        .filter_map(|&index| state.index.entries.get(index))
        .map(|entry| {
            let value = style::truncate(&entry.display_value, 80);
            let content = column![
                row![
                    text(&entry.path.property_name).size(14).font(style::BOLD),
                    space::horizontal(),
                    text(value)
                        .size(12)
                        .font(Font::MONOSPACE)
                        .style(style::muted_text),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
                text(tree::breadcrumb(&entry.path))
                    .size(12)
                    .style(style::muted_text),
            ]
            .spacing(2);
            button(content)
                .width(Length::Fill)
                .padding([6, 10])
                .style(|theme, status| style::list_row(theme, status, false))
                .on_press(Message::JumpTo(entry.path.clone()))
                .into()
        })
        .collect();

    let summary = match total {
        0 => "No matches".to_string(),
        1 => "1 match".to_string(),
        n if n > MAX_VISIBLE_RESULTS => {
            format!("{n} matches - showing the first {MAX_VISIBLE_RESULTS}, refine your search")
        }
        n => format!("{n} matches"),
    };

    column![
        container(text(summary).size(12).style(style::muted_text)).padding([0, 16]),
        scrollable(column(rows).spacing(2).padding([4, 12])).height(Length::Fill),
    ]
    .spacing(4)
    .into()
}

fn category_view<'a>(
    state: &'a State,
    store: &'a ConfigStore,
    selection: &'a Location,
) -> Element<'a, Message> {
    let Some(items) = store.items_at(&selection.file, &selection.category_path) else {
        return center(text("This category no longer exists.").style(style::muted_text)).into();
    };

    let mut header = column![breadcrumb(selection)].spacing(8);

    let description = description(store, selection);
    if !description.is_empty() {
        header = header.push(text(description).size(13).style(style::muted_text));
    }

    let subcategories: Vec<Element<'_, Message>> = items
        .iter()
        .filter_map(|item| match item {
            Item::Category(category) => {
                let mut category_path = selection.category_path.clone();
                category_path.push(category.name.clone());
                Some(
                    button(text(&category.name).size(13))
                        .padding([4, 12])
                        .style(style::chip)
                        .on_press(Message::Navigate(Location {
                            file: selection.file.clone(),
                            category_path,
                        }))
                        .into(),
                )
            }
            Item::Property(_) | Item::Trivia(_) => None,
        })
        .collect();
    if !subcategories.is_empty() {
        header = header.push(row(subcategories).spacing(6).wrap().vertical_spacing(6));
    }

    let cards: Vec<Element<'_, Message>> = items
        .iter()
        .filter_map(|item| match item {
            Item::Property(property) => Some(property_card(state, selection, property)),
            Item::Category(_) | Item::Trivia(_) => None,
        })
        .collect();

    let properties: Element<'_, Message> = if cards.is_empty() {
        container(
            text("No settings directly in this category - pick a subcategory above.")
                .style(style::muted_text),
        )
        .padding(16)
        .into()
    } else {
        scrollable(column(cards).spacing(8).padding([8, 16]))
            .id(PROPERTIES_SCROLL)
            .height(Length::Fill)
            .into()
    };

    column![container(header).padding([0, 16]), properties]
        .spacing(8)
        .height(Length::Fill)
        .into()
}

/// Clickable "Mod › file.cfg › category › sub" path to the current location.
fn breadcrumb(selection: &Location) -> Element<'_, Message> {
    let crumbs = tree::crumbs(&selection.file, &selection.category_path);
    let last = crumbs.len() - 1;
    let mut parts = row![].spacing(2).align_y(Alignment::Center);
    for (i, (label, depth)) in crumbs.into_iter().enumerate() {
        let location = Location {
            file: selection.file.clone(),
            category_path: selection.category_path[..depth].to_vec(),
        };
        if i > 0 {
            parts = parts.push(
                text(tree::CRUMB_SEPARATOR)
                    .size(16)
                    .style(style::muted_text),
            );
        }
        parts = if i == last {
            parts.push(text(label).size(18).font(style::BOLD))
        } else {
            parts.push(
                button(text(label).size(16))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::Navigate(location)),
            )
        };
    }
    parts.wrap().into()
}

/// The comment above the selected category, or the file's header comment at the top level.
fn description(store: &ConfigStore, selection: &Location) -> String {
    let lines = match selection.category_path.split_last() {
        Some((name, parent)) => store
            .items_at(&selection.file, parent)
            .and_then(|items| {
                items.iter().find_map(|item| match item {
                    Item::Category(category) if &category.name == name => {
                        Some(category.comment.as_slice())
                    }
                    _ => None,
                })
            })
            .unwrap_or_default(),
        None => store
            .entry(&selection.file)
            .map(|entry| entry.ast.header_comment.as_slice())
            .unwrap_or_default(),
    };
    join_comment(lines)
}

fn join_comment(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.chars().all(|c| c == '#' || c == '-' || c == '='))
        .collect::<Vec<_>>()
        .join("\n")
}

fn property_card<'a>(
    state: &'a State,
    selection: &Location,
    property: &'a Property,
) -> Element<'a, Message> {
    let path = PropertyPath {
        relative_path: selection.file.clone(),
        category_path: selection.category_path.clone(),
        property_name: property.name.clone(),
    };
    let highlighted = state.highlighted.as_ref() == Some(&path);
    let changed_from = state
        .changeset
        .entries
        .iter()
        .find(|entry| entry.path == path && entry.original_value != property.value)
        .map(|entry| entry.original_value.display_text());

    let type_label = match (&property.value, property.prop_type) {
        (PropertyValue::List(_), _) => "list",
        (_, PropertyType::Bool) => "bool",
        (_, PropertyType::Int) => "int",
        (_, PropertyType::Double) => "decimal",
        (_, PropertyType::String) => "text",
    };

    let mut info = column![
        row![
            text(&property.name).size(15).font(style::BOLD),
            container(text(type_label).size(11))
                .padding([1, 6])
                .style(style::badge),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .spacing(4)
    .width(Length::Fill);

    let comment = join_comment(&property.comment);
    if !comment.is_empty() {
        info = info.push(text(comment).size(12).style(style::muted_text));
    }
    let edited = changed_from.is_some();
    if let Some(original) = changed_from {
        let original = style::truncate(&original, 80);
        info = info.push(
            text(format!("Changed in this profile (was {original})"))
                .size(12)
                .style(style::accent_text),
        );
    }

    let content = row![
        info,
        container(value_editor(path, property)).width(EDITOR_WIDTH)
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    container(content)
        .padding(12)
        .width(Length::Fill)
        .style(move |theme| style::card(theme, highlighted, edited))
        .into()
}

fn value_editor(path: PropertyPath, property: &Property) -> Element<'_, Message> {
    match &property.value {
        PropertyValue::Single(value)
            if property.prop_type == PropertyType::Bool
                && (value == "true" || value == "false") =>
        {
            container(
                toggler(value == "true")
                    .label(value.as_str())
                    .size(20)
                    .on_toggle(move |on| Message::PropertyEdited(path.clone(), on.to_string())),
            )
            .align_right(Length::Fill)
            .into()
        }
        PropertyValue::Single(value) => text_input("", value)
            .on_input(move |value| Message::PropertyEdited(path.clone(), value))
            .font(Font::MONOSPACE)
            .size(13)
            .padding(6)
            .into(),
        PropertyValue::List(values) => {
            let mut preview: Vec<&str> = values
                .iter()
                .take(LIST_PREVIEW)
                .map(String::as_str)
                .collect();
            let more = values.len().saturating_sub(LIST_PREVIEW);
            let more_label = format!("… and {more} more");
            if more > 0 {
                preview.push(&more_label);
            }
            let preview = if values.is_empty() {
                "(empty)".to_string()
            } else {
                preview.join("\n")
            };
            column![
                text(preview).size(12).font(Font::MONOSPACE),
                button(text(format!("Edit list ({})…", values.len())).size(13))
                    .padding([4, 10])
                    .style(button::secondary)
                    .on_press(Message::EditList(path)),
            ]
            .spacing(6)
            .into()
        }
    }
}
