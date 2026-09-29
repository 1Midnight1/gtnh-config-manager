//! Shared widget styles, all derived from the active theme's palette so the app stays
//! consistent (and works in both light and dark themes).

use iced::widget::{button, container, text};
use iced::{Background, Border, Color, Theme, border};

/// Top and bottom bars.
pub fn bar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default()
        .background(palette.background.weakest.color)
        .border(border::width(1).color(palette.background.strong.color))
}

/// The sidebar and other secondary panels.
pub fn panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default().background(palette.background.weakest.color)
}

/// A property card. `highlighted` draws an accent border (the target of a jump), `edited`
/// a subtler one for properties changed by the current profile.
pub fn card(theme: &Theme, highlighted: bool, edited: bool) -> container::Style {
    let palette = theme.extended_palette();
    let border_color = if highlighted {
        palette.primary.base.color
    } else if edited {
        palette.primary.weak.color
    } else {
        palette.background.strong.color
    };
    container::Style::default()
        .background(palette.background.base.color)
        .border(
            border::rounded(6)
                .width(if highlighted { 2 } else { 1 })
                .color(border_color),
        )
}

/// Plain bordered box (profile detail sections, ranks viewer).
pub fn section(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default()
        .background(palette.background.base.color)
        .border(
            border::rounded(6)
                .width(1)
                .color(palette.background.strong.color),
        )
}

/// Small pill showing a property's type.
pub fn badge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default()
        .background(palette.background.weak.color)
        .border(border::rounded(4))
        .color(palette.background.weak.text)
}

/// Page tabs in the top bar.
pub fn tab(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let palette = theme.extended_palette();
    let background = match (active, status) {
        (true, _) => Some(Background::Color(palette.primary.weak.color)),
        (false, button::Status::Hovered) => Some(Background::Color(palette.background.weak.color)),
        _ => None,
    };
    button::Style {
        background,
        text_color: if active {
            palette.primary.weak.text
        } else {
            palette.background.base.text
        },
        border: border::rounded(6),
        ..button::Style::default()
    }
}

/// A row in a list (sidebar nodes, search results, profiles): flat until hovered, tinted
/// when selected.
pub fn list_row(theme: &Theme, status: button::Status, selected: bool) -> button::Style {
    let palette = theme.extended_palette();
    let background = match (selected, status) {
        (true, _) => Some(Background::Color(palette.primary.weak.color)),
        (false, button::Status::Hovered | button::Status::Pressed) => {
            Some(Background::Color(palette.background.weak.color))
        }
        _ => None,
    };
    button::Style {
        background,
        text_color: if selected {
            palette.primary.weak.text
        } else {
            palette.background.base.text
        },
        border: border::rounded(4),
        ..button::Style::default()
    }
}

/// Clickable text, used for breadcrumb segments and subcategory chips.
pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => palette.primary.strong.color,
            _ => palette.primary.base.color,
        },
        ..button::Style::default()
    }
}

/// A subcategory chip.
pub fn chip(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => palette.background.strong.color,
            _ => palette.background.weak.color,
        })),
        text_color: palette.background.base.text,
        border: Border {
            radius: 12.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

pub fn muted_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(muted(theme)),
    }
}

pub fn accent_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().primary.base.color),
    }
}

pub fn warning_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().warning.base.color),
    }
}

pub fn danger_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().danger.base.color),
    }
}

fn muted(theme: &Theme) -> Color {
    theme
        .extended_palette()
        .background
        .base
        .text
        .scale_alpha(0.6)
}
