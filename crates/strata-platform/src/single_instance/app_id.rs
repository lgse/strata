// SPDX-License-Identifier: MIT

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId(String);

/// Why a string is not an [`AppId`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidAppId {
    pub id: String,
    pub reason: &'static str,
}

impl fmt::Display for InvalidAppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not an application id: {}", self.id, self.reason)
    }
}

impl std::error::Error for InvalidAppId {}

impl AppId {
    pub fn new(id: &str) -> Result<AppId, InvalidAppId> {
        let fail = |reason| {
            Err(InvalidAppId {
                id: id.to_owned(),
                reason,
            })
        };
        if id.is_empty() || id.len() > 255 {
            return fail("it must be 1 to 255 bytes long");
        }
        let elements: Vec<&str> = id.split('.').collect();
        if elements.len() < 2 {
            return fail("it needs at least two elements, separated by dots");
        }
        for element in elements {
            if element.is_empty() {
                return fail("an element is empty");
            }
            if element.starts_with(|c: char| c.is_ascii_digit()) {
                return fail("an element starts with a digit");
            }
            if !element
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return fail("only A-Z, a-z, 0-9, _ and - may appear between the dots");
            }
        }
        Ok(AppId(id.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn object_path(&self) -> String {
        let mut path = String::with_capacity(self.0.len() + 1);
        path.push('/');
        path.extend(self.0.chars().map(|c| match c {
            '.' => '/',
            '-' => '_',
            c => c,
        }));
        path
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
