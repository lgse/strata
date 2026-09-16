// SPDX-License-Identifier: MIT

/// Match folded filenames and queries. Plain text is a substring; a query with
/// `*` is anchored to the whole filename, with each star consuming zero or more characters.
pub(crate) fn filter_name_matches(name: &str, query: &str) -> bool {
    let Some((prefix, rest)) = query.split_once('*') else {
        return name.contains(query);
    };
    let Some(name) = name.strip_prefix(prefix) else {
        return false;
    };
    let (middle, suffix) = rest.rsplit_once('*').unwrap_or(("", rest));
    let Some(mut remaining) = name.strip_suffix(suffix) else {
        return false;
    };
    // Reserve the anchored suffix first so interior matches cannot consume it.
    // Literal segments advance through disjoint slices without wildcard backtracking.
    for segment in middle.split('*').filter(|segment| !segment.is_empty()) {
        let Some(position) = remaining.find(segment) else {
            return false;
        };
        remaining = &remaining[position + segment.len()..];
    }
    true
}

#[cfg(test)]
mod tests;
