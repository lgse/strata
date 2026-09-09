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
            if !show_hidden && entry.file_name().as_encoded_bytes().starts_with(b".") {
                continue;
            }
            if count >= max_entries {
                coverage.entry_limit = true;
                break 'roots;
            }
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => {
                    coverage.unreadable = true;
                    continue;
                }
            };
            pending.push(SearchItem::new(entry.path(), &root, kind.is_dir()));
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
