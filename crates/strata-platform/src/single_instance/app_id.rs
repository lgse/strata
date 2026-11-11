//! The app id: the well-known bus name the primary owns, and the object path its interfaces
//! sit at.

use std::fmt;

/// An application id, such as `org.example.Files`: checked as GLib's
/// `g_application_id_is_valid` checks one, so a GApplication of the same id meets this one on
/// the bus.
///
/// It is also the desktop entry's name and the window's app id, so the desktop can match the
/// running app, its windows and its launcher.
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
    /// Two or more elements of `A`–`Z`, `a`–`z`, `0`–`9`, `_` and `-`, separated by dots, none
    /// starting with a digit, at most 255 bytes in all.
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

    /// Where the app's interfaces are exported, as GApplication spells it: `/` first, every
    /// `.` a `/` and every `-` a `_` (`org.example.my-app` is `/org/example/my_app`).
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
