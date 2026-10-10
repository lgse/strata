// SPDX-License-Identifier: MIT

use gtk::{gdk::Key, prelude::*};

use super::{DocumentSelection, PreviewUnit, SelectionPoint};

fn text_buffer(unit: &PreviewUnit) -> gtk::TextBuffer {
    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(unit.display_text());
    buffer
}

pub(super) fn clicked_selection(
    units: &[PreviewUnit],
    previous: Option<DocumentSelection>,
    point: SelectionPoint,
    presses: i32,
    extend: bool,
) -> DocumentSelection {
    if extend {
        return DocumentSelection {
            anchor: previous.map_or(point, |selection| selection.anchor),
            focus: point,
        };
    }
    let mut selection = DocumentSelection {
        anchor: point,
        focus: point,
    };
    let Some(unit) = units.get(point.unit) else {
        return selection;
    };
    if presses < 2 {
        return selection;
    }
    let buffer = text_buffer(unit);
    if buffer.char_count() == 0 {
        selection.anchor.offset = 0;
        selection.focus.offset = unit.selection_len();
        return selection;
    }
    let mut start = buffer.iter_at_offset(
        point
            .offset
            .min(buffer.char_count().saturating_sub(1) as usize) as i32,
    );
    let mut end = start;
    if presses == 2 {
        if start.inside_word() || start.starts_word() {
            if !start.starts_word() {
                start.backward_word_start();
            }
            end.forward_word_end();
        } else {
            end.forward_char();
        }
    } else {
        start.set_line_offset(0);
        end.forward_line();
    }
    selection.anchor.offset = start.offset() as usize;
    selection.focus.offset = end.offset() as usize;
    selection
}

pub(super) fn move_selection(
    units: &[PreviewUnit],
    selection: Option<DocumentSelection>,
    key: Key,
    control: bool,
    extend: bool,
) -> Option<DocumentSelection> {
    let first = units.iter().position(|unit| unit.selection_len() > 0)?;
    let last = units.iter().rposition(|unit| unit.selection_len() > 0)?;
    let beginning = SelectionPoint {
        unit: first,
        offset: 0,
    };
    let end = SelectionPoint {
        unit: last,
        offset: units[last].selection_len(),
    };
    let previous = selection.unwrap_or(DocumentSelection {
        anchor: beginning,
        focus: beginning,
    });
    let point = previous.focus;
    let forward = matches!(
        key,
        Key::Right | Key::KP_Right | Key::Down | Key::KP_Down | Key::End | Key::KP_End
    );
    let focus = if !extend
        && !control
        && !previous.is_empty()
        && matches!(key, Key::Left | Key::KP_Left | Key::Right | Key::KP_Right)
    {
        let (start, end) = previous.normalized();
        if forward { end } else { start }
    } else if control && matches!(key, Key::Home | Key::KP_Home | Key::End | Key::KP_End) {
        if forward { end } else { beginning }
    } else if control
        && matches!(key, Key::Right | Key::KP_Right)
        && units[point.unit].display_text().is_empty()
        && point.offset < units[point.unit].selection_len()
    {
        character_step(units, point, true)
    } else if matches!(key, Key::Left | Key::KP_Left | Key::Right | Key::KP_Right) && !control {
        character_step(units, point, forward)
    } else {
        let unit = units.get(point.unit)?;
        let buffer = text_buffer(unit);
        let mut iter = buffer.iter_at_offset(point.offset.min(buffer.char_count() as usize) as i32);
        match key {
            Key::Left | Key::KP_Left => {
                if !iter.backward_word_start() && point.offset == 0 {
                    let mut next = character_step(units, point, false);
                    if next.unit != point.unit {
                        next.offset = units[next.unit].selection_len();
                        return move_selection(
                            units,
                            Some(DocumentSelection {
                                focus: next,
                                ..previous
                            }),
                            key,
                            control,
                            extend,
                        );
                    }
                }
            }
            Key::Right | Key::KP_Right => {
                if !iter.forward_word_end() && point.offset == unit.selection_len() {
                    let mut next = character_step(units, point, true);
                    if next.unit != point.unit {
                        next.offset = 0;
                        return move_selection(
                            units,
                            Some(DocumentSelection {
                                focus: next,
                                ..previous
                            }),
                            key,
                            control,
                            extend,
                        );
                    }
                }
            }
            Key::Home | Key::KP_Home => iter.set_line_offset(0),
            Key::End | Key::KP_End => {
                if !iter.ends_line() {
                    iter.forward_to_line_end();
                }
            }
            Key::Up | Key::KP_Up | Key::Down | Key::KP_Down => {
                let column = iter.line_offset();
                let first_line = iter.line() == 0;
                if forward {
                    iter.forward_line();
                } else {
                    iter.backward_line();
                }
                if (forward && iter.is_end()) || (!forward && first_line) {
                    let adjacent = character_step(
                        units,
                        SelectionPoint {
                            unit: point.unit,
                            offset: if forward { unit.selection_len() } else { 0 },
                        },
                        forward,
                    );
                    if adjacent.unit != point.unit {
                        let next_buffer = text_buffer(&units[adjacent.unit]);
                        let mut next = if forward {
                            next_buffer.start_iter()
                        } else {
                            next_buffer.end_iter()
                        };
                        if !forward && next.line_offset() == 0 {
                            next.backward_char();
                        }
                        next.set_line_offset(0);
                        while next.line_offset() < column && !next.ends_line() {
                            next.forward_char();
                        }
                        let focus = SelectionPoint {
                            unit: adjacent.unit,
                            offset: next.offset() as usize,
                        };
                        return Some(DocumentSelection {
                            anchor: if extend { previous.anchor } else { focus },
                            focus,
                        });
                    }
                }
                iter.set_line_offset(0);
                while iter.line_offset() < column && !iter.ends_line() {
                    iter.forward_char();
                }
            }
            _ => return None,
        }
        SelectionPoint {
            unit: point.unit,
            offset: (iter.offset() as usize).min(unit.selection_len()),
        }
    };
    Some(DocumentSelection {
        anchor: if extend { previous.anchor } else { focus },
        focus,
    })
}

fn character_step(units: &[PreviewUnit], point: SelectionPoint, forward: bool) -> SelectionPoint {
    let len = units[point.unit].selection_len();
    if forward {
        if point.offset < len {
            let buffer = text_buffer(&units[point.unit]);
            let mut iter =
                buffer.iter_at_offset(point.offset.min(buffer.char_count() as usize) as i32);
            iter.forward_cursor_position();
            return SelectionPoint {
                offset: (iter.offset() as usize).max(point.offset + 1).min(len),
                ..point
            };
        }
        if let Some(unit) =
            (point.unit + 1..units.len()).find(|index| units[*index].selection_len() > 0)
        {
            let buffer = text_buffer(&units[unit]);
            let mut iter = buffer.start_iter();
            iter.forward_cursor_position();
            return SelectionPoint {
                unit,
                offset: (iter.offset() as usize)
                    .max(1)
                    .min(units[unit].selection_len()),
            };
        }
    } else {
        if point.offset > 0 {
            let buffer = text_buffer(&units[point.unit]);
            let mut iter =
                buffer.iter_at_offset(point.offset.min(buffer.char_count() as usize) as i32);
            iter.backward_cursor_position();
            return SelectionPoint {
                offset: (iter.offset() as usize).min(point.offset - 1),
                ..point
            };
        }
        if let Some(unit) = (0..point.unit)
            .rev()
            .find(|index| units[*index].selection_len() > 0)
        {
            let buffer = text_buffer(&units[unit]);
            let mut iter = buffer.end_iter();
            iter.backward_cursor_position();
            return SelectionPoint {
                unit,
                offset: (iter.offset() as usize).min(units[unit].selection_len() - 1),
            };
        }
    }
    point
}

pub(in crate::ui) fn match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: String = query.chars().flat_map(char::to_lowercase).collect();
    let mut folded = String::with_capacity(text.len());
    let mut positions = Vec::new();
    for (offset, character) in text.chars().enumerate() {
        for lower in character.to_lowercase() {
            positions.push((folded.len(), offset));
            folded.push(lower);
        }
    }
    let mut position = 0;
    folded
        .match_indices(&needle)
        .map(|(start, _)| {
            while positions[position].0 < start {
                position += 1;
            }
            let from = positions[position].1;
            let end = start + needle.len();
            while position < positions.len() && positions[position].0 < end {
                position += 1;
            }
            (from, positions[position - 1].1 + 1)
        })
        .collect()
}

#[cfg(test)]
mod tests;
