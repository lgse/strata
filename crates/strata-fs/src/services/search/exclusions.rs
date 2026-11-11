// SPDX-License-Identifier: MIT

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchExclusions {
    folder_names: Vec<String>,
    directories: Vec<PathBuf>,
}

impl SearchExclusions {
    pub fn is_directory_path(raw: &str) -> bool {
        raw.starts_with('~') || raw.contains('/')
    }

    pub fn normalize_entry(raw: &str) -> Result<String, &'static str> {
        let input = raw.trim();
        if input.is_empty() {
            return Err("Enter a folder name or directory path.");
        }
        if input.contains('\0') {
            return Err("Exclusions cannot contain NUL characters.");
        }
        let trimmed = input.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "~" {
            return Err("Cannot exclude root or entire home directory.");
        }
        if !Self::is_directory_path(trimmed) {
            if matches!(trimmed, "." | "..") {
                return Err("Enter a folder name, not . or ..");
            }
            return Ok(trimmed.to_owned());
        }
        let path = if let Some(rest) = trimmed.strip_prefix("~/") {
            glib::home_dir().join(rest.trim_start_matches('/'))
        } else if trimmed.starts_with('/') {
            PathBuf::from(trimmed)
        } else {
            return Err("Directory paths must start with / or ~/");
        };
        // Collapsing .. lexically would change the meaning of paths through symlinks.
        if path.components().any(|part| part == Component::ParentDir) {
            return Err("Directory paths cannot contain ..");
        }
        let path: PathBuf = path.components().collect();
        if path == Path::new("/") || path == glib::home_dir() {
            return Err("Cannot exclude root or entire home directory.");
        }
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn from_strings(raw: &[String]) -> Self {
        let mut exclusions = Self::default();
        for item in raw {
            let Ok(normalized) = Self::normalize_entry(item) else {
                continue;
            };
            if Path::new(&normalized).is_absolute() {
                exclusions.directories.push(PathBuf::from(normalized));
            } else {
                exclusions
                    .folder_names
                    .push(normalized.to_ascii_lowercase());
            }
        }
        exclusions.folder_names.sort();
        exclusions.folder_names.dedup();
        exclusions.directories.sort();
        exclusions.directories.dedup();
        exclusions
    }

    pub fn is_excluded(&self, path: &Path, name: &str, is_directory: bool) -> bool {
        self.directories.iter().any(|dir| path.starts_with(dir))
            || (is_directory && self.matches_folder(name))
            || (!self.folder_names.is_empty()
                && path.ancestors().skip(1).any(|parent| {
                    parent
                        .file_name()
                        .is_some_and(|name| self.matches_folder(&name.to_string_lossy()))
                }))
    }

    fn matches_folder(&self, name: &str) -> bool {
        self.folder_names
            .iter()
            .any(|folder| name.eq_ignore_ascii_case(folder))
    }
}

#[cfg(test)]
mod tests;
