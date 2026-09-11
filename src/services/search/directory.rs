// SPDX-License-Identifier: MIT

use super::*;

#[cfg(test)]
mod tests;

pub(super) fn build_index(
    index: &SharedIndex,
    roots: Vec<PathBuf>,
    show_hidden: bool,
    max_entries: usize,
    time_budget: Duration,
) {
    let start = Instant::now();
    let mut last_publish = start;
    let mut count = 0;
    let mut pending = Vec::with_capacity(256);
    let mut coverage = SearchCoverage::default();
    // Depth-limited recursive walkers can still open child directories. A plain
    // directory iterator also keeps immediate ignored/generated entries eligible.
    'roots: for root in roots {
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(_) => {
                coverage.unreadable = true;
                continue;
            }
        };
        let hidden_names = native_hidden_names(&root);
        for entry in entries {
            if index.is_retired() {
                return;
            }
            if start.elapsed() >= time_budget {
                coverage.time_limit = true;
                break 'roots;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    coverage.unreadable = true;
                    continue;
                }
            };
            if !show_hidden && is_hidden_name(&entry.file_name(), &hidden_names) {
                continue;
            }
            if count >= max_entries {
                coverage.entry_limit = true;
                break 'roots;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    coverage.unreadable = true;
                    continue;
                }
            };
            let path = entry.path();
            let is_directory = matches!(
                native_kind(file_type, &path),
                EntryKind::Directory | EntryKind::DirectorySymbolicLink
            );
            pending.push(SearchItem::new(path, &root, is_directory));
            count += 1;
            if pending.len() >= 256 {
                append_index_items(index, &mut pending, true, coverage);
            }
            if last_publish.elapsed() >= PUBLISH_INTERVAL {
                append_index_items(index, &mut pending, true, coverage);
                index.broadcast_change();
                last_publish = Instant::now();
            }
        }
    }
    append_index_items(index, &mut pending, false, coverage);
    index.broadcast_change();
}
