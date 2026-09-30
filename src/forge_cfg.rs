//! Parser for Forge's `Configuration` file format (net.minecraftforge.common.config.Configuration),
//! used by Forge mods on Minecraft 1.7.10 (and thus by GTNH).
//!
//! The AST keeps every source line: comments, blank lines and the `~CONFIG_VERSION:` directive
//! are `Item::Trivia`, and categories and properties remember the exact lines they were parsed
//! from. Writing a file back out therefore reproduces the original bytes, except for the lines of
//! properties whose value was changed.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyType {
    Bool,
    Int,
    Double,
    String,
}

impl PropertyType {
    fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "B" => Some(Self::Bool),
            "I" => Some(Self::Int),
            "D" => Some(Self::Double),
            "S" => Some(Self::String),
            _ => None,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Bool => "B",
            Self::Int => "I",
            Self::Double => "D",
            Self::String => "S",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropertyValue {
    Single(String),
    List(Vec<String>),
}

impl PropertyValue {
    /// The value as one line of text (list entries joined with ", "), for display and search.
    pub fn display_text(&self) -> String {
        match self {
            PropertyValue::Single(value) => value.clone(),
            PropertyValue::List(values) => values.join(", "),
        }
    }

    /// Whether both are scalars or both are lists. Changing a property's shape would change
    /// its syntax, so edits are only ever applied between values of the same shape.
    pub fn same_shape(&self, other: &PropertyValue) -> bool {
        matches!(
            (self, other),
            (PropertyValue::Single(_), PropertyValue::Single(_))
                | (PropertyValue::List(_), PropertyValue::List(_))
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub prop_type: PropertyType,
    pub name: String,
    pub value: PropertyValue,
    /// Comment lines directly preceding this property, with the leading '#' stripped. For
    /// display only - the comment lines themselves are `Item::Trivia` in the parent.
    pub comment: Vec<String>,
    source: Source,
}

/// The lines a property was parsed from, so an unchanged property is written back verbatim and
/// a changed one keeps its original layout.
#[derive(Debug, Clone, PartialEq)]
struct Source {
    /// One line for a scalar; the `name <` line, the entries and the closing `>` for a list.
    lines: Vec<String>,
    /// The value `lines` encode.
    value: PropertyValue,
    /// For a scalar, the byte offset in `lines[0]` just past the `=`.
    value_start: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub name: String,
    /// Comment lines directly preceding this category, with the leading '#' stripped. For
    /// display only - the comment lines themselves are `Item::Trivia` in the parent.
    pub comment: Vec<String>,
    pub items: Vec<Item>,
    open_line: String,
    close_line: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Property(Property),
    Category(Category),
    /// A blank line, comment line or `~CONFIG_VERSION:` directive, exactly as read.
    Trivia(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigFile {
    /// Comment block at the very top of the file, before any category or property. For
    /// display only.
    pub header_comment: Vec<String>,
    pub items: Vec<Item>,
    line_ending: &'static str,
    final_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<ConfigFile, ParseError> {
    let lines: Vec<&str> = input.lines().collect();
    let mut parser = Parser {
        lines: &lines,
        pos: 0,
    };
    let (items, _) = parser.parse_items(true)?;

    Ok(ConfigFile {
        header_comment: header_comment(&items),
        items,
        line_ending: if input.contains("\r\n") { "\r\n" } else { "\n" },
        final_newline: input.ends_with('\n'),
    })
}

struct Parser<'a> {
    lines: &'a [&'a str],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            line: self.pos + 1,
            message: message.into(),
        }
    }

    /// Parses a sequence of items until a closing `}` (consumed and returned, for a nested
    /// category) or the end of input (only allowed at the root).
    fn parse_items(&mut self, is_root: bool) -> Result<(Vec<Item>, String), ParseError> {
        let mut items = Vec::new();

        while let Some(&raw) = self.lines.get(self.pos) {
            let trimmed = raw.trim();

            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || (is_root && trimmed.starts_with("~CONFIG_VERSION:"))
            {
                items.push(Item::Trivia(raw.to_string()));
                self.pos += 1;
            } else if trimmed == "}" {
                if is_root {
                    return Err(self.error("unexpected '}' outside of any category"));
                }
                self.pos += 1;
                return Ok((items, raw.to_string()));
            } else if is_property_line(trimmed) {
                // Checked before categories, so a value ending in '{' stays a value.
                let comment = comment_block(&items, false).1;
                let property = self.parse_property(raw, comment)?;
                items.push(Item::Property(property));
            } else if let Some(name) = trimmed.strip_suffix('{') {
                let (start, comment) = comment_block(&items, true);
                // A block at the very top, separated by a blank line, is the file's header.
                let comment = if is_root && start == 0 && is_blank(items.last()) {
                    Vec::new()
                } else {
                    comment
                };
                self.pos += 1;
                let (sub_items, close_line) = self.parse_items(false)?;
                items.push(Item::Category(Category {
                    name: name.trim().to_string(),
                    comment,
                    items: sub_items,
                    open_line: raw.to_string(),
                    close_line,
                }));
            } else {
                return Err(self.error(format!(
                    "expected a category, property or comment, got '{trimmed}'"
                )));
            }
        }

        if is_root {
            Ok((items, String::new()))
        } else {
            Err(self.error("unexpected end of file, expected '}'"))
        }
    }

    fn parse_property(&mut self, raw: &str, comment: Vec<String>) -> Result<Property, ParseError> {
        let line = raw.trim();
        let (prefix, rest) = line
            .split_once(':')
            .ok_or_else(|| self.error(format!("expected 'TYPE:name=value', got '{line}'")))?;

        let prop_type = PropertyType::from_prefix(prefix)
            .ok_or_else(|| self.error(format!("unknown property type prefix '{prefix}'")))?;

        // A property name is followed by either '=value' (scalar) or '<' (start of a list),
        // with no '=' at all in the list case.
        let (name, remainder) = if let Some(after_quote_start) = rest.strip_prefix('"') {
            let end = after_quote_start
                .find('"')
                .ok_or_else(|| self.error("unterminated quoted property name"))?;
            let name = after_quote_start[..end].to_string();
            (name, after_quote_start[end + 1..].trim_start())
        } else {
            let split_idx = rest.find(['=', '<']).unwrap_or(rest.len());
            let name = rest[..split_idx].trim().to_string();
            (name, rest[split_idx..].trim_start())
        };

        // Once an '=' is present, the rest of the line is always the literal scalar value -
        // even if it's empty or happens to start with '<' (e.g. `S:formatMessage=<%u> %m`).
        // Only a name with no '=' at all followed by '<' opens a list.
        let start = self.pos;
        let (value, value_start) = if let Some(after_eq) = remainder.strip_prefix('=') {
            self.pos += 1;
            // `after_eq` is a subslice of `raw`, so this is where the value begins.
            let value_start = after_eq.as_ptr() as usize - raw.as_ptr() as usize;
            (
                PropertyValue::Single(after_eq.trim().to_string()),
                value_start,
            )
        } else if let Some(rest) = remainder.strip_prefix('<') {
            if !rest.trim().is_empty() {
                return Err(self.error("unexpected content after '<' in list value"));
            }
            (self.expect_list_value()?, 0)
        } else {
            return Err(self.error(format!("expected '=' or '<' in property line '{line}'")));
        };

        Ok(Property {
            prop_type,
            name,
            source: Source {
                lines: self.lines[start..self.pos]
                    .iter()
                    .map(|line| line.to_string())
                    .collect(),
                value: value.clone(),
                value_start,
            },
            value,
            comment,
        })
    }

    fn expect_list_value(&mut self) -> Result<PropertyValue, ParseError> {
        let mut values = Vec::new();
        loop {
            self.pos += 1;
            let raw = self
                .lines
                .get(self.pos)
                .ok_or_else(|| self.error("unexpected end of file inside list value"))?;
            let trimmed = raw.trim();
            if trimmed == ">" {
                self.pos += 1;
                return Ok(PropertyValue::List(values));
            }
            values.push(trimmed.to_string());
        }
    }
}

/// Whether `trimmed` starts like a property (`B:`, `I:`, `D:` or `S:`). Forge only writes
/// category names containing ':' in quotes, so this never matches a category.
fn is_property_line(trimmed: &str) -> bool {
    matches!(trimmed.as_bytes(), [b'B' | b'I' | b'D' | b'S', b':', ..])
}

fn comment_text(item: &Item) -> Option<&str> {
    match item {
        Item::Trivia(line) => line.trim().strip_prefix('#').map(str::trim),
        _ => None,
    }
}

fn is_blank(item: Option<&Item>) -> bool {
    matches!(item, Some(Item::Trivia(line)) if line.trim().is_empty())
}

/// The run of comment lines at the end of `items` (the comment of whatever item comes next),
/// with the index where it starts. With `allow_blank`, one blank line may separate the comment
/// from the item, which is how Forge lays out category comments.
fn comment_block(items: &[Item], allow_blank: bool) -> (usize, Vec<String>) {
    let end = if allow_blank && is_blank(items.last()) {
        items.len() - 1
    } else {
        items.len()
    };
    let start = items[..end]
        .iter()
        .rposition(|item| comment_text(item).is_none())
        .map_or(0, |index| index + 1);
    let lines = items[start..end]
        .iter()
        .filter_map(comment_text)
        .map(str::to_string)
        .collect();
    (start, lines)
}

/// The comment block at the very top of the file, if a blank line separates it from what follows
/// (otherwise it's the comment of the first item).
fn header_comment(items: &[Item]) -> Vec<String> {
    let end = items
        .iter()
        .position(|item| comment_text(item).is_none())
        .unwrap_or(items.len());
    if end > 0 && is_blank(items.get(end)) {
        items[..end]
            .iter()
            .filter_map(comment_text)
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    }
}

/// Writes lines separated by the file's line ending, so the last one only gets one if the
/// original file ended with one.
struct LineWriter<'a, 'b> {
    f: &'a mut fmt::Formatter<'b>,
    ending: &'static str,
    first: bool,
}

impl LineWriter<'_, '_> {
    fn line(&mut self, line: &str) -> fmt::Result {
        if !self.first {
            self.f.write_str(self.ending)?;
        }
        self.first = false;
        self.f.write_str(line)
    }

    fn items(&mut self, items: &[Item]) -> fmt::Result {
        for item in items {
            match item {
                Item::Trivia(line) => self.line(line)?,
                Item::Category(category) => {
                    self.line(&category.open_line)?;
                    self.items(&category.items)?;
                    self.line(&category.close_line)?;
                }
                Item::Property(property) => self.property(property)?,
            }
        }
        Ok(())
    }

    fn property(&mut self, property: &Property) -> fmt::Result {
        let source = &property.source;
        if property.value == source.value {
            for line in &source.lines {
                self.line(line)?;
            }
            return Ok(());
        }

        let indent = leading_whitespace(&source.lines[0]);
        match (&property.value, &source.value) {
            (PropertyValue::Single(value), PropertyValue::Single(_)) => self.line(&format!(
                "{}{value}",
                &source.lines[0][..source.value_start]
            )),
            (PropertyValue::List(values), PropertyValue::List(_)) => {
                // Keep the `name <` and `>` lines, indenting entries like the original ones.
                let (open, rest) = source.lines.split_first().expect("list has an open line");
                let (close, old_entries) = rest.split_last().expect("list has a close line");
                let entry_indent = match old_entries.first() {
                    Some(entry) => leading_whitespace(entry).to_string(),
                    None => format!("{indent}    "),
                };
                self.line(open)?;
                for value in values {
                    self.line(&format!("{entry_indent}{value}"))?;
                }
                self.line(close)
            }
            // The shape changed, so there's no layout to reuse; write it the way Forge would.
            (value, _) => {
                let prefix = property.prop_type.prefix();
                let name = quote_name(&property.name);
                match value {
                    PropertyValue::Single(value) => {
                        self.line(&format!("{indent}{prefix}:{name}={value}"))
                    }
                    PropertyValue::List(values) => {
                        self.line(&format!("{indent}{prefix}:{name} <"))?;
                        for value in values {
                            self.line(&format!("{indent}    {value}"))?;
                        }
                        self.line(&format!("{indent} >"))
                    }
                }
            }
        }
    }
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Quotes a property name the way Forge does: whenever it has a character other than a letter,
/// digit, '.', '_' or '-'.
fn quote_name(name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if plain {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

impl fmt::Display for ConfigFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut writer = LineWriter {
            f,
            ending: self.line_ending,
            first: true,
        };
        writer.items(&self.items)?;
        if self.final_newline {
            writer.f.write_str(self.line_ending)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &[(&str, &str)] = &[
        (
            "config_version_directive",
            include_str!("../tests/fixtures/config_version_directive.cfg"),
        ),
        (
            "dreamcraft",
            include_str!("../tests/fixtures/dreamcraft.cfg"),
        ),
        (
            "forge_category_comments",
            include_str!("../tests/fixtures/forge_category_comments.cfg"),
        ),
        (
            "header_comment",
            include_str!("../tests/fixtures/header_comment.cfg"),
        ),
        (
            "list_value",
            include_str!("../tests/fixtures/list_value.cfg"),
        ),
        (
            "literal_value_edge_cases",
            include_str!("../tests/fixtures/literal_value_edge_cases.cfg"),
        ),
        (
            "nested_categories",
            include_str!("../tests/fixtures/nested_categories.cfg"),
        ),
        (
            "quoted_names",
            include_str!("../tests/fixtures/quoted_names.cfg"),
        ),
        (
            "quoted_property_name",
            include_str!("../tests/fixtures/quoted_property_name.cfg"),
        ),
        (
            "scalar_property_with_comment",
            include_str!("../tests/fixtures/scalar_property_with_comment.cfg"),
        ),
        (
            "value_ending_in_brace",
            include_str!("../tests/fixtures/value_ending_in_brace.cfg"),
        ),
    ];

    fn find_property<'a>(items: &'a [Item], name: &str) -> &'a Property {
        items
            .iter()
            .find_map(|item| match item {
                Item::Property(property) if property.name == name => Some(property),
                _ => None,
            })
            .unwrap_or_else(|| panic!("property '{name}' not found"))
    }

    fn find_property_mut<'a>(items: &'a mut [Item], name: &str) -> &'a mut Property {
        items
            .iter_mut()
            .find_map(|item| match item {
                Item::Property(property) if property.name == name => Some(property),
                _ => None,
            })
            .unwrap_or_else(|| panic!("property '{name}' not found"))
    }

    fn find_category<'a>(items: &'a [Item], name: &str) -> &'a Category {
        items
            .iter()
            .find_map(|item| match item {
                Item::Category(category) if category.name == name => Some(category),
                _ => None,
            })
            .unwrap_or_else(|| panic!("category '{name}' not found"))
    }

    fn find_category_mut<'a>(items: &'a mut [Item], name: &str) -> &'a mut Category {
        items
            .iter_mut()
            .find_map(|item| match item {
                Item::Category(category) if category.name == name => Some(category),
                _ => None,
            })
            .unwrap_or_else(|| panic!("category '{name}' not found"))
    }

    /// Lines of `before` and `after` that differ, as (line number, before, after).
    fn changed_lines<'a>(before: &'a str, after: &'a str) -> Vec<(usize, &'a str, &'a str)> {
        assert_eq!(before.lines().count(), after.lines().count());
        before
            .lines()
            .zip(after.lines())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(index, (a, b))| (index + 1, a, b))
            .collect()
    }

    #[test]
    fn every_fixture_round_trips_byte_for_byte() {
        for (name, input) in FIXTURES {
            let config = parse(input).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(&config.to_string(), input, "{name} (LF)");

            let crlf = input.replace('\n', "\r\n");
            let config = parse(&crlf).unwrap_or_else(|err| panic!("{name} (CRLF): {err}"));
            assert_eq!(config.to_string(), crlf, "{name} (CRLF)");
        }
    }

    #[test]
    fn editing_a_value_changes_only_its_line() {
        let input = include_str!("../tests/fixtures/dreamcraft.cfg");
        let mut config = parse(input).unwrap();
        let modfixes = find_category_mut(&mut config.items, "modfixes");
        find_property_mut(&mut modfixes.items, "GenerateOil").value =
            PropertyValue::Single("false".to_string());

        let output = config.to_string();
        assert_eq!(
            changed_lines(input, &output),
            [(11, "    B:GenerateOil=true", "    B:GenerateOil=false")]
        );
    }

    #[test]
    fn edits_keep_crlf_line_endings() {
        let input = "general {\r\n    I:Count=1\r\n}\r\n";
        let mut config = parse(input).unwrap();
        let general = find_category_mut(&mut config.items, "general");
        find_property_mut(&mut general.items, "Count").value =
            PropertyValue::Single("2".to_string());
        assert_eq!(config.to_string(), "general {\r\n    I:Count=2\r\n}\r\n");
    }

    #[test]
    fn editing_a_list_keeps_its_layout() {
        let input = include_str!("../tests/fixtures/list_value.cfg");
        let mut config = parse(input).unwrap();
        let oilgen = find_category_mut(&mut config.items, "oilgen");
        find_property_mut(&mut oilgen.items, "OilBiomeIDBlackList").value =
            PropertyValue::List(vec!["1".to_string(), "2".to_string()]);
        assert_eq!(
            config.to_string(),
            "oilgen {\n    I:OilBiomeIDBlackList <\n        1\n        2\n     >\n}\n"
        );
    }

    #[test]
    fn a_changed_shape_is_written_like_forge() {
        let mut config = parse("general {\n    S:\"a:b\"=x\n}\n").unwrap();
        let general = find_category_mut(&mut config.items, "general");
        find_property_mut(&mut general.items, "a:b").value =
            PropertyValue::List(vec!["y".to_string()]);
        assert_eq!(
            config.to_string(),
            "general {\n    S:\"a:b\" <\n        y\n     >\n}\n"
        );
        assert_eq!(quote_name("plain.name-ok_1"), "plain.name-ok_1");
        assert_eq!(quote_name("with space"), "\"with space\"");
    }

    #[test]
    fn parses_scalar_properties_with_comments() {
        let input = include_str!("../tests/fixtures/scalar_property_with_comment.cfg");
        let config = parse(input).unwrap();
        let modfixes = find_category(&config.items, "modfixes");
        let prop = find_property(&modfixes.items, "GenerateOil");
        assert_eq!(prop.prop_type, PropertyType::Bool);
        assert_eq!(prop.value, PropertyValue::Single("true".to_string()));
        assert_eq!(
            prop.comment,
            vec!["Set to true to enable OilSpawn".to_string()]
        );
    }

    #[test]
    fn attaches_forge_category_comments_and_the_header() {
        let input = include_str!("../tests/fixtures/forge_category_comments.cfg");
        let config = parse(input).unwrap();
        assert_eq!(config.header_comment, ["Configuration file"]);

        let first = find_category(&config.items, "first");
        assert!(first.comment.contains(&"The first category".to_string()));
        assert!(!first.comment.contains(&"Configuration file".to_string()));
        let second = find_category(&config.items, "second");
        assert!(second.comment.contains(&"The second category".to_string()));
        assert!(
            !second
                .comment
                .contains(&"A comment right before the closing brace".to_string())
        );

        let enabled = find_property(&first.items, "Enabled");
        assert_eq!(enabled.comment, ["Enables the thing [default: true]"]);
    }

    #[test]
    fn a_category_without_a_comment_does_not_take_the_header() {
        let config = parse(include_str!("../tests/fixtures/header_comment.cfg")).unwrap();
        assert_eq!(config.header_comment, ["Configuration file"]);
        assert!(find_category(&config.items, "modules").comment.is_empty());
    }

    #[test]
    fn parses_nested_categories() {
        let input = include_str!("../tests/fixtures/nested_categories.cfg");
        let config = parse(input).unwrap();
        let modfixes = find_category(&config.items, "modfixes");
        let oilgen = find_category(&modfixes.items, "oilgen");
        let prop = find_property(&oilgen.items, "OilSphereChance");
        assert_eq!(prop.value, PropertyValue::Single("60.0".to_string()));
    }

    #[test]
    fn scalar_values_are_never_mistaken_for_list_or_category_starts() {
        let input = include_str!("../tests/fixtures/literal_value_edge_cases.cfg");
        let config = parse(input).unwrap();
        let debug = find_category(&config.items, "debug");
        let icon = find_property(&debug.items, "icon");
        assert_eq!(icon.value, PropertyValue::Single(String::new()));
        let format_message = find_property(&debug.items, "formatMessage");
        assert_eq!(
            format_message.value,
            PropertyValue::Single("<%u> %m".to_string())
        );

        let input = include_str!("../tests/fixtures/value_ending_in_brace.cfg");
        let config = parse(input).unwrap();
        let general = find_category(&config.items, "general");
        let pattern = find_property(&general.items, "pattern");
        assert_eq!(pattern.value, PropertyValue::Single("abc{".to_string()));
        let closing = find_property(&general.items, "closing");
        assert_eq!(closing.value, PropertyValue::Single("}".to_string()));
    }

    #[test]
    fn parses_the_config_version_directive_as_trivia() {
        let input = include_str!("../tests/fixtures/config_version_directive.cfg");
        let config = parse(input).unwrap();
        assert!(
            config
                .items
                .contains(&Item::Trivia("~CONFIG_VERSION: 1".to_string()))
        );
        assert_eq!(config.header_comment, ["Configuration file"]);
    }

    #[test]
    fn parses_list_values() {
        let input = include_str!("../tests/fixtures/list_value.cfg");
        let config = parse(input).unwrap();
        let oilgen = find_category(&config.items, "oilgen");
        let prop = find_property(&oilgen.items, "OilBiomeIDBlackList");
        assert_eq!(
            prop.value,
            PropertyValue::List(vec!["16".to_string(), "21".to_string(), "29".to_string()])
        );
    }

    #[test]
    fn parses_quoted_property_names() {
        let input = include_str!("../tests/fixtures/quoted_property_name.cfg");
        let config = parse(input).unwrap();
        let modules = find_category(&config.items, "modules");
        let prop = find_property(&modules.items, "GTNH Pause menu buttons");
        assert_eq!(prop.value, PropertyValue::Single("true".to_string()));

        let config = parse(include_str!("../tests/fixtures/quoted_names.cfg")).unwrap();
        let general = find_category(&config.items, "general");
        find_property(&general.items, "weird:name+x");
        find_property(&general.items, "plain.name-ok_1");
    }

    #[test]
    fn rejects_structural_errors() {
        assert!(parse("}\ngeneral {\n}\n").is_err());
        assert!(parse("general {\n    B:A=true\n").is_err());
        assert!(parse("general {\n    I:L <\n        1\n}\n").is_err());
        assert!(parse("{\"json\": true}\n").is_err());
    }
}
