// SPDX-License-Identifier: MIT

use super::{PATHOLOGICAL_TEXT_UNIT_BYTES, SOURCE_UNIT_BYTES, SOURCE_UNIT_LINES, SourceUnit};

pub(super) fn source_units(content: &str) -> (Vec<SourceUnit>, bool) {
    let mut units = Vec::new();
    let mut group = String::new();
    let mut group_line = 1;
    let mut group_lines = 0;
    let mut split_lines = false;

    for (line_index, line) in content.split_inclusive('\n').enumerate() {
        let next_line = line_index + 1;
        let content_end = line.len() - usize::from(line.ends_with('\n'));
        if content_end > PATHOLOGICAL_TEXT_UNIT_BYTES {
            push_source_group(&mut units, &mut group, group_line, group_lines);
            group_lines = 0;
            push_source_line_chunks(&mut units, line, next_line);
            split_lines = true;
        } else {
            if !group.is_empty()
                && (group.len() + line.len() > SOURCE_UNIT_BYTES
                    || group_lines >= SOURCE_UNIT_LINES)
            {
                push_source_group(&mut units, &mut group, group_line, group_lines);
                group_line = next_line;
                group_lines = 0;
            }
            if group.is_empty() {
                group_line = next_line;
            }
            group.push_str(line);
            group_lines += 1;
        }
    }
    push_source_group(&mut units, &mut group, group_line, group_lines);
    if units.is_empty() {
        units.push(SourceUnit::new(String::new(), 1, 1, false));
    }
    (units, split_lines)
}

pub(super) fn push_source_group(
    units: &mut Vec<SourceUnit>,
    group: &mut String,
    first_line: usize,
    line_count: usize,
) {
    if group.is_empty() {
        return;
    }
    units.push(SourceUnit::new(
        std::mem::take(group),
        first_line,
        line_count,
        false,
    ));
}

pub(super) fn push_source_line_chunks(units: &mut Vec<SourceUnit>, line: &str, first_line: usize) {
    let content_end = line.len() - usize::from(line.ends_with('\n'));
    let mut start = 0;
    let mut continuation = false;
    while content_end - start > PATHOLOGICAL_TEXT_UNIT_BYTES {
        let remaining = content_end - start;
        let chunk_bytes = if remaining < PATHOLOGICAL_TEXT_UNIT_BYTES * 2 {
            remaining.div_ceil(2)
        } else {
            PATHOLOGICAL_TEXT_UNIT_BYTES
        };
        let mut end = start + chunk_bytes;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        units.push(SourceUnit::new(
            line[start..end].to_owned(),
            first_line,
            usize::from(!continuation),
            continuation,
        ));
        start = end;
        continuation = true;
    }
    units.push(SourceUnit::new(
        line[start..].to_owned(),
        first_line,
        usize::from(!continuation),
        continuation,
    ));
}

pub(super) fn source_line_numbers(unit: &SourceUnit) -> String {
    if unit.continuation {
        return "↳".to_owned();
    }
    (unit.first_line..unit.first_line + unit.line_count.max(1))
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
