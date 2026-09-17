// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct SearchExclusions {
    pub(crate) folder_names: Vec<String>,
    pub(crate) directories: Vec<PathBuf>,
}

impl SearchExclusions {
    pub fn is_directory_path(raw: &str) -> bool {
        raw.starts_with('~') || raw.starts_with('/') || raw.contains('/')
    }

    pub fn from_strings(raw: &[String]) -> Self {
        let mut folder_names = Vec::new();
        let mut directories = Vec::new();
        let home = glib::home_dir();
        for item in raw {
            let trimmed = item.trim().trim_end_matches('/');
            if trimmed.is_empty() || trimmed == "~" || trimmed == "/" {
                continue;
            }
            if Self::is_directory_path(trimmed) {
                let resolved = if let Some(rest) = trimmed.strip_prefix("~/") {
                    home.join(rest)
                } else if trimmed.starts_with('/') {
                    PathBuf::from(trimmed)
                } else {
                    // Relative directory paths with slashes resolve relative to home directory.
                    home.join(trimmed)
                };
                directories.push(resolved);
            } else {
                folder_names.push(trimmed.to_owned());
            }
        }
        folder_names.sort_by_key(|a| a.to_lowercase());
        folder_names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        directories.sort();
        directories.dedup();
        Self {
            folder_names,
            directories,
        }
    }

    pub fn is_excluded(&self, path: &Path, name: &str, is_directory: bool) -> bool {
        if is_directory {
            if self
                .folder_names
                .iter()
                .any(|folder| name.eq_ignore_ascii_case(folder))
            {
                return true;
            }
            if self
                .directories
                .iter()
                .any(|dir| path == dir || path.starts_with(dir))
            {
                return true;
            }
        } else {
            if self.directories.iter().any(|dir| path.starts_with(dir)) {
                return true;
            }
            if !self.folder_names.is_empty() {
                for parent in path.ancestors().skip(1) {
                    if let Some(parent_name) = parent.file_name() {
                        let name_str = parent_name.to_string_lossy();
                        if self
                            .folder_names
                            .iter()
                            .any(|folder| name_str.eq_ignore_ascii_case(folder))
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
