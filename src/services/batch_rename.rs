// SPDX-License-Identifier: MIT

//! Finder-inspired batch rename planning.
//!
//! Pure name mapping shared by the batch rename dialog and the application
//! controller; filesystem access and operation dispatch stay in [`crate::app::Browser`].

#[cfg(test)]
mod tests;
/// Batch rename mode mirroring Finder's rename dialog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchRenameMode {
    /// Replace every occurrence of `find` with `replace_with`.
    ReplaceText { find: String, replace_with: String },
    /// Prepend `text`, or insert it before the extension.
    AddText { text: String, before_name: bool },
    /// Replace each name with `custom_name` plus a counter, index, or date.
    /// The extension is preserved; numbering follows selection order.
    Format {
        custom_name: String,
        style: FormatStyle,
        start_number: usize,
        before_name: bool,
    },
}

/// Numbering style for [`BatchRenameMode::Format`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatStyle {
    /// `{name} {start:05}`, … zero-padded like Finder.
    Counter,
    /// `{name} {start}`, `{name} {start + 1}`, … in selection order.
    Index,
    /// `{name} {timestamp}` for every item.
    Date,
}

/// Maps each input name to its renamed form, preserving order.
///
/// Callers pass display names in view order and pair the output back with
/// their entries. Unchanged results (empty pattern, empty custom name) keep
/// the original name so callers can skip no-ops. `timestamp` is the
/// date-time stamp for [`FormatStyle::Date`].
pub fn plan_batch_rename(names: &[String], mode: &BatchRenameMode, timestamp: &str) -> Vec<String> {
    match mode {
        BatchRenameMode::ReplaceText { find, replace_with } => names
            .iter()
            .map(|name| {
                if find.is_empty() {
                    name.clone()
                } else {
                    name.replace(find.as_str(), replace_with.as_str())
                }
            })
            .collect(),
        BatchRenameMode::AddText { text, before_name } => names
            .iter()
            .map(|name| {
                if *before_name {
                    format!("{text}{name}")
                } else {
                    let (stem, extension) = split_stem_extension(name);
                    format!("{stem}{text}{extension}")
                }
            })
            .collect(),
        BatchRenameMode::Format {
            custom_name,
            style,
            start_number,
            before_name,
        } => {
            if custom_name.is_empty() {
                return names.to_vec();
            }
            names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let (_, extension) = split_stem_extension(name);
                    // Date stamps are identical for the whole batch, so they
                    // always carry the position number to stay unique.
                    let (stamp, numbered) = match style {
                        FormatStyle::Counter => {
                            (format!("{:05}", start_number.saturating_add(index)), false)
                        }
                        FormatStyle::Index => {
                            (format!("{}", start_number.saturating_add(index)), false)
                        }
                        FormatStyle::Date => (timestamp.to_owned(), true),
                    };
                    if numbered {
                        let position = index + 1;
                        if *before_name {
                            format!("{stamp} {position} {custom_name}{extension}")
                        } else {
                            format!("{custom_name} {stamp} {position}{extension}")
                        }
                    } else if *before_name {
                        format!("{stamp} {custom_name}{extension}")
                    } else {
                        format!("{custom_name} {stamp}{extension}")
                    }
                })
                .collect()
        }
    }
}

/// Splits `photo.jpg` into `("photo", ".jpg")` so appended text lands before
/// the extension. Extensionless names and leading-dot files keep the whole
/// name as the stem.
fn split_stem_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(position) if position > 0 => name.split_at(position),
        _ => (name, ""),
    }
}
