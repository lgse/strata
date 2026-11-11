//! The parent handle an XDG portal passes: `wayland:<xdg-foreign handle>`, `x11:<xid>`, or
//! nothing.

use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use super::outcome::WindowSystem;

/// The longest `wayland:` handle accepted, in bytes. Real handles are 32 to 36 bytes (wlroots'
/// hex, KWin's UUIDs). A string near libwayland's 4096-byte message limit would fail to
/// marshal, and that failure would end winit's whole connection.
pub const MAX_WAYLAND_HANDLE: usize = 1024;

/// The most of an unknown kind kept in [`ParseError::UnknownKind`], in bytes.
const MAX_KIND_SHOWN: usize = 32;

/// A window of another process to parent to, as a portal names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Parent {
    /// No parent: the request carried `""` (zenity, GTK 3 below 3.24.44 on a v2-only
    /// compositor).
    None,
    /// An xdg-foreign handle, exported by a Wayland client.
    Wayland(WaylandHandle),
    /// An X11 window id.
    X11(Xid),
}

/// An xdg-foreign handle, kept opaque: Mutter's are random ids and KWin's UUIDs, so its shape
/// isn't checked. Made only by [`Parent::parse`]: no NUL, at most [`MAX_WAYLAND_HANDLE`] bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WaylandHandle(String);

impl WaylandHandle {
    /// The handle as the exporting client's compositor made it, without the `wayland:`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A non-zero X11 window id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Xid(NonZeroU32);

impl Xid {
    /// The window id, never 0.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// Why a handle isn't one [`Parent::parse`] knows. Treat it as no parent, and log it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// No `kind:` prefix.
    Unrecognised,
    /// A kind other than `wayland` or `x11` (at most 32 bytes of it).
    UnknownKind(String),
    /// `wayland:` with nothing after it.
    EmptyHandle,
    /// A `wayland:` handle with a NUL byte, which no Wayland message can carry.
    BadHandle,
    /// A `wayland:` handle longer than [`MAX_WAYLAND_HANDLE`] bytes (its length).
    TooLong(usize),
    /// An `x11:` id that isn't 1–8 hex digits after one optional `0x`, or is 0.
    BadXid,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Unrecognised => f.write_str("not a `wayland:` or `x11:` parent handle"),
            ParseError::UnknownKind(k) => write!(f, "an unknown parent kind {k:?}"),
            ParseError::EmptyHandle => f.write_str("an empty `wayland:` handle"),
            ParseError::BadHandle => f.write_str("a `wayland:` handle with a NUL byte"),
            ParseError::TooLong(n) => write!(
                f,
                "a `wayland:` handle of {n} bytes, over {MAX_WAYLAND_HANDLE}"
            ),
            ParseError::BadXid => f.write_str("an `x11:` window id that isn't non-zero hex"),
        }
    }
}

impl std::error::Error for ParseError {}

impl Parent {
    /// Parses a portal's `parent_window`. Surrounding whitespace is ignored, and an empty
    /// string is [`Parent::None`]. The kinds are lowercase, as the portal spells them.
    /// `x11:` takes GTK's bare hex (`x11:400003`) and Chromium's `0x` (`x11:0x400003`).
    pub fn parse(s: &str) -> Result<Parent, ParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(Parent::None);
        }
        let (kind, rest) = s.split_once(':').ok_or(ParseError::Unrecognised)?;
        match kind {
            "wayland" => wayland(rest).map(Parent::Wayland),
            "x11" => xid(rest).map(Parent::X11),
            other => Err(ParseError::UnknownKind(truncated(other).to_owned())),
        }
    }

    /// The window system the parent belongs to; None for [`Parent::None`].
    pub fn system(&self) -> Option<WindowSystem> {
        match self {
            Parent::None => None,
            Parent::Wayland(_) => Some(WindowSystem::Wayland),
            Parent::X11(_) => Some(WindowSystem::X11),
        }
    }
}

impl FromStr for Parent {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Parent, ParseError> {
        Parent::parse(s)
    }
}

/// The portal's spelling: `""`, `wayland:<handle>` or `x11:<hex>`.
impl fmt::Display for Parent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Parent::None => Ok(()),
            Parent::Wayland(h) => write!(f, "wayland:{}", h.0),
            Parent::X11(x) => write!(f, "x11:{:x}", x.get()),
        }
    }
}

fn wayland(rest: &str) -> Result<WaylandHandle, ParseError> {
    if rest.is_empty() {
        return Err(ParseError::EmptyHandle);
    }
    if rest.len() > MAX_WAYLAND_HANDLE {
        return Err(ParseError::TooLong(rest.len()));
    }
    if rest.contains('\0') {
        return Err(ParseError::BadHandle);
    }
    Ok(WaylandHandle(rest.to_owned()))
}

fn xid(rest: &str) -> Result<Xid, ParseError> {
    // One prefix at most: `0x0x1` stays an error (no trim_start_matches).
    let digits = rest
        .strip_prefix("0x")
        .or_else(|| rest.strip_prefix("0X"))
        .unwrap_or(rest);
    // from_str_radix alone would take a leading `+`.
    if digits.is_empty() || digits.len() > 8 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ParseError::BadXid);
    }
    u32::from_str_radix(digits, 16)
        .ok()
        .and_then(NonZeroU32::new)
        .map(Xid)
        .ok_or(ParseError::BadXid)
}

fn truncated(s: &str) -> &str {
    if s.len() <= MAX_KIND_SHOWN {
        return s;
    }
    let mut end = MAX_KIND_SHOWN;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
