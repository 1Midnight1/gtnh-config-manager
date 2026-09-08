//! Parser for Forge's `Configuration` file format (net.minecraftforge.common.config.Configuration),
//! used by Forge mods on Minecraft 1.7.10 (and thus by GTNH). The AST preserves comments,
//! ordering and nesting so a parsed file can be written back out with edits applied.

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

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub prop_type: PropertyType,
    pub name: String,
    pub value: PropertyValue,
    /// Comment lines directly preceding this property, with the leading '#' stripped.
    pub comment: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub name: String,
    /// Comment lines directly preceding this category, with the leading '#' stripped.
    pub comment: Vec<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Property(Property),
    Category(Category),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfigFile {
    /// Comment block found at the very top of the file, before any category or property.
    pub header_comment: Vec<String>,
    /// Raw text of an optional `~CONFIG_VERSION: <value>` directive Forge writes at the top
    /// of some files, outside of any category.
    pub config_version: Option<String>,
    pub items: Vec<Item>,
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
    let mut parser = Parser { lines: &lines, pos: 0 };

    let mut pending_comment: Vec<String> = Vec::new();
    let mut header_comment: Vec<String> = Vec::new();
    let mut config_version: Option<String> = None;
    let items = parser.parse_items(&mut pending_comment, &mut header_comment, &mut config_version, true)?;

    Ok(ConfigFile { header_comment, config_version, items })
}

struct Parser<'a> {
    lines: &'a [&'a str],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn line_number(&self) -> usize {
        self.pos + 1
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError { line: self.line_number(), message: message.into() }
    }

    /// Parses a sequence of items until a closing `}` (consumed) or end of input.
    /// `is_root` controls whether an orphaned comment block, followed by a blank line
    /// before any item has been parsed, is captured as the file's `header_comment`.
    fn parse_items(
        &mut self,
        pending_comment: &mut Vec<String>,
        header_comment: &mut Vec<String>,
        config_version: &mut Option<String>,
        is_root: bool,
    ) -> Result<Vec<Item>, ParseError> {
        let mut items = Vec::new();

        while self.pos < self.lines.len() {
            let raw = self.lines[self.pos];
            let trimmed = raw.trim();

            if trimmed.is_empty() {
                if is_root && items.is_empty() && header_comment.is_empty() && !pending_comment.is_empty() {
                    header_comment.append(pending_comment);
                } else {
                    pending_comment.clear();
                }
                self.pos += 1;
                continue;
            }

            if let Some(comment) = trimmed.strip_prefix('#') {
                pending_comment.push(comment.trim().to_string());
                self.pos += 1;
                continue;
            }

            if is_root && config_version.is_none() {
                if let Some(version) = trimmed.strip_prefix("~CONFIG_VERSION:") {
                    *config_version = Some(version.trim().to_string());
                    pending_comment.clear();
                    self.pos += 1;
                    continue;
                }
            }

            if trimmed == "}" {
                self.pos += 1;
                return Ok(items);
            }

            if let Some(name) = trimmed.strip_suffix('{').map(|s| s.trim().to_string()) {
                self.pos += 1;
                let comment = std::mem::take(pending_comment);
                let sub_items = self.parse_items(pending_comment, header_comment, config_version, false)?;
                items.push(Item::Category(Category { name, comment, items: sub_items }));
                continue;
            }

            let property = self.parse_property(trimmed, std::mem::take(pending_comment))?;
            items.push(Item::Property(property));
        }

        if is_root {
            Ok(items)
        } else {
            Err(self.error("unexpected end of file, expected '}'"))
        }
    }

    fn parse_property(&mut self, line: &str, comment: Vec<String>) -> Result<Property, ParseError> {
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
        let value = if let Some(after_eq) = remainder.strip_prefix('=') {
            self.pos += 1;
            PropertyValue::Single(after_eq.trim().to_string())
        } else if let Some(rest) = remainder.strip_prefix('<') {
            if rest.trim().is_empty() {
                self.expect_list_value()?
            } else {
                return Err(self.error("unexpected content after '<' in list value"));
            }
        } else {
            return Err(self.error(format!("expected '=' or '<' in property line '{line}'")));
        };

        Ok(Property { prop_type, name, value, comment })
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

impl fmt::Display for ConfigFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.header_comment {
            writeln!(f, "# {line}")?;
        }
        if !self.header_comment.is_empty() {
            writeln!(f)?;
        }
        if let Some(version) = &self.config_version {
            writeln!(f, "~CONFIG_VERSION: {version}")?;
            writeln!(f)?;
        }
        write_items(f, &self.items, 0)
    }
}

fn write_items(f: &mut fmt::Formatter<'_>, items: &[Item], depth: usize) -> fmt::Result {
    let indent = "    ".repeat(depth);
    for item in items {
        match item {
            Item::Category(category) => {
                for line in &category.comment {
                    writeln!(f, "{indent}# {line}")?;
                }
                writeln!(f, "{indent}{} {{", category.name)?;
                write_items(f, &category.items, depth + 1)?;
                writeln!(f, "{indent}}}")?;
                writeln!(f)?;
            }
            Item::Property(property) => {
                for line in &property.comment {
                    writeln!(f, "{indent}# {line}")?;
                }
                let name = if property.name.contains(' ') {
                    format!("\"{}\"", property.name)
                } else {
                    property.name.clone()
                };
                match &property.value {
                    PropertyValue::Single(value) => {
                        writeln!(f, "{indent}{}:{}={}", property.prop_type.prefix(), name, value)?;
                    }
                    PropertyValue::List(values) => {
                        writeln!(f, "{indent}{}:{} <", property.prop_type.prefix(), name)?;
                        for value in values {
                            writeln!(f, "{indent}    {value}")?;
                        }
                        writeln!(f, "{indent} >")?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_property<'a>(items: &'a [Item], name: &str) -> &'a Property {
        items
            .iter()
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

    #[test]
    fn parses_scalar_properties_with_comments() {
        let input = include_str!("../tests/fixtures/scalar_property_with_comment.cfg");
        let config = parse(input).unwrap();
        let modfixes = find_category(&config.items, "modfixes");
        let prop = find_property(&modfixes.items, "GenerateOil");
        assert_eq!(prop.prop_type, PropertyType::Bool);
        assert_eq!(prop.value, PropertyValue::Single("true".to_string()));
        assert_eq!(prop.comment, vec!["Set to true to enable OilSpawn".to_string()]);
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
    fn scalar_values_are_never_mistaken_for_list_starts() {
        let input = include_str!("../tests/fixtures/literal_value_edge_cases.cfg");
        let config = parse(input).unwrap();
        let debug = find_category(&config.items, "debug");

        let icon = find_property(&debug.items, "icon");
        assert_eq!(icon.value, PropertyValue::Single(String::new()));

        let format_message = find_property(&debug.items, "formatMessage");
        assert_eq!(format_message.value, PropertyValue::Single("<%u> %m".to_string()));
    }

    #[test]
    fn parses_config_version_directive_and_round_trips_it() {
        let input = include_str!("../tests/fixtures/config_version_directive.cfg");
        let config = parse(input).unwrap();
        assert_eq!(config.config_version, Some("1".to_string()));

        let reparsed = parse(&config.to_string()).unwrap();
        assert_eq!(config, reparsed);
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
    }

    #[test]
    fn parses_header_comment() {
        let input = include_str!("../tests/fixtures/header_comment.cfg");
        let config = parse(input).unwrap();
        assert_eq!(config.header_comment, vec!["Configuration file".to_string()]);
    }

    #[test]
    fn round_trips_full_sample() {
        let input = include_str!("../tests/fixtures/dreamcraft.cfg");
        let config = parse(input).unwrap();

        let modules = find_category(&config.items, "modules");
        let version = find_property(&modules.items, "ModPackVersion");
        assert_eq!(version.value, PropertyValue::Single("2.9.0-beta-3".to_string()));

        // Re-parsing the serialized output should yield an identical structure.
        let rendered = config.to_string();
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(config, reparsed);
    }
}
