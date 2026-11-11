//! The clipboard's public types.

use std::fmt;
use std::time::Duration;

use crate::transfer;

/// Which selection: the clipboard (Ctrl+C, Ctrl+V), or the primary selection (select, then
/// middle-click). The primary selection needs data control on Wayland, or X11.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Selection {
    Clipboard,
    Primary,
}

impl From<Selection> for transfer::Selection {
    fn from(s: Selection) -> Self {
        match s {
            Selection::Clipboard => transfer::Selection::Clipboard,
            Selection::Primary => transfer::Selection::Primary,
        }
    }
}

impl From<transfer::Selection> for Selection {
    fn from(s: transfer::Selection) -> Self {
        match s {
            transfer::Selection::Clipboard => Selection::Clipboard,
            transfer::Selection::Primary => Selection::Primary,
        }
    }
}

/// The protocol the clipboard goes through here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// Wayland data control (`ext_data_control_v1`, else `zwlr_data_control_v1`): heard and
    /// written without keyboard focus.
    DataControl,
    /// Wayland `wl_data_device` on the app's own connection (mutter, which has no data
    /// control): another app's copy is heard only at the app's next keyboard focus, and a
    /// write needs the serial of the key or click that asked for it.
    DataDevice,
    /// X11 selections, on X11, or through Xwayland where Wayland has no route at all.
    X11,
}

impl Route {
    pub(crate) fn transfer(self) -> transfer::Route {
        match self {
            Route::DataControl => transfer::Route::DataControl,
            Route::DataDevice => transfer::Route::DataDevice,
            Route::X11 => transfer::Route::X11,
        }
    }
}

/// What the clipboard can do here, once attached.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Status {
    /// `wayland` or `x11`: the window's backend.
    pub backend: &'static str,
    pub route: Route,
    /// Another app's copy is heard while the app has no keyboard focus.
    pub hears_unfocused: bool,
    /// The primary selection can be written and read.
    pub primary: bool,
    /// A copy can outlive the app: X11, where a clipboard manager runs. On Wayland nothing
    /// client-side can (see the module docs).
    pub keeps_after_exit: bool,
    /// A Wayland window whose clipboard goes through Xwayland: Wayland had no route.
    pub xwayland: bool,
    /// Why a better route is missing, when one is: libwayland < 1.23, no data control.
    pub unsupported: Option<String>,
}

/// Bounds on one read, so another app can neither exhaust memory nor hold a read forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most bytes taken; more fails the read with [`ReadError::TooLarge`].
    pub max_bytes: usize,
    /// The longest the other app may stay silent, before its first byte or between two.
    pub idle: Duration,
}

impl Default for Limits {
    /// 64 MiB, and 5 s of silence.
    fn default() -> Self {
        let d = transfer::ReadLimits::default();
        Limits {
            max_bytes: d.max_bytes,
            idle: d.idle,
        }
    }
}

impl From<Limits> for transfer::ReadLimits {
    fn from(l: Limits) -> Self {
        transfer::ReadLimits {
            max_bytes: l.max_bytes,
            idle: l.idle,
        }
    }
}

/// One type read from a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Data {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Data {
    /// The bytes as text, if this is a text type ([`super::TEXT_TYPES`]) that decodes:
    /// `STRING` and `TEXT` as ISO Latin-1 (ICCCM), the others as UTF-8.
    pub fn text(&self) -> Option<String> {
        match self.mime.as_str() {
            "STRING" | "TEXT" => Some(self.bytes.iter().map(|&b| char::from(b)).collect()),
            m if super::TEXT_TYPES.contains(&m) => String::from_utf8(self.bytes.clone()).ok(),
            _ => None,
        }
    }
}

/// A failed read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The selection offers none of the types asked for (or nothing that reads as files).
    NothingOffered,
    /// The offer went away before the read: another copy replaced it.
    Gone,
    /// More than [`Limits::max_bytes`].
    TooLarge,
    /// The other app stayed silent for [`Limits::idle`].
    TimedOut,
    /// The clipboard isn't attached, or was shut down.
    NotAttached,
    Platform(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::NothingOffered => f.write_str("the selection offers none of those types"),
            ReadError::Gone => f.write_str("the offer is gone"),
            ReadError::TooLarge => f.write_str("more data than the read allows"),
            ReadError::TimedOut => f.write_str("the other app went quiet"),
            ReadError::NotAttached => f.write_str("the clipboard isn't attached"),
            ReadError::Platform(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<transfer::ReadError> for ReadError {
    fn from(e: transfer::ReadError) -> Self {
        use transfer::ReadError as T;
        match e {
            T::TooLarge => ReadError::TooLarge,
            T::TimedOut => ReadError::TimedOut,
            T::Gone => ReadError::Gone,
            T::NotOffered => ReadError::NothingOffered,
            T::Shutdown => ReadError::NotAttached,
            other => ReadError::Platform(other.to_string()),
        }
    }
}

/// A failed clipboard call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// This session has no route for it: not Wayland or X11, or Wayland with no route and no
    /// Xwayland.
    Unsupported(String),
    /// The clipboard isn't attached ([`super::Clipboard::attach`]), or was shut down.
    NotAttached,
    /// A copy with no types.
    Empty,
    /// A request refused before it reached the platform (the primary selection where there
    /// is none, a call off the UI thread).
    Refused(&'static str),
    /// A data-device write needs a key or click first: the app has had none.
    NoSerial,
    Platform(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(why) => write!(f, "the clipboard is unsupported here: {why}"),
            Error::NotAttached => f.write_str("the clipboard isn't attached"),
            Error::Empty => f.write_str("a copy with no types"),
            Error::Refused(why) => write!(f, "refused: {why}"),
            Error::NoSerial => f.write_str("no key or click to write the clipboard with"),
            Error::Platform(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<transfer::Error> for Error {
    fn from(e: transfer::Error) -> Self {
        use transfer::Error as T;
        match e {
            T::Unsupported(u) => Error::Unsupported(u.to_string()),
            T::Shutdown => Error::NotAttached,
            T::NoSerial => Error::NoSerial,
            T::Refused(r) => Error::Refused(r),
            other => Error::Platform(other.to_string()),
        }
    }
}

/// One copy this process made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CopyId(pub(crate) u64);

impl CopyId {
    /// The id logged as `n` by [`CopyId::get`]: for strata-platform's own tests, which name
    /// copies across a process boundary.
    #[doc(hidden)]
    pub fn from_raw(n: u64) -> CopyId {
        CopyId(n)
    }

    /// A number for logs.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Where a copy stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyState {
    /// Written, not yet confirmed. On X11 the server's timestamp is a round trip away; on
    /// Wayland the compositor's echo is. On wlroots a data-device write made while the app had
    /// no keyboard focus is never confirmed, as no echo comes either way (mutter: a hand test).
    /// Reads of the selection get this copy meanwhile.
    Pending,
    /// The selection holds it.
    Holding,
    /// It no longer holds the selection, and never will again.
    Lost(Lost),
}

/// Why a copy no longer holds the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lost {
    /// Another client took the selection: another app's copy, or Rune's own `Tx::copy`.
    Taken,
    /// A later copy of this process's replaced it.
    Replaced,
    /// [`super::Clipboard::clear`].
    Cleared,
    /// The compositor dropped the write (a data-device write with a stale serial).
    Refused,
    /// The platform connection failed.
    Failed,
}

/// What a selection now offers, as last heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    /// Its MIME types (and X11 target names), in the order offered.
    pub mimes: Vec<String>,
    /// This process's own copy, when it is the one offered.
    pub ours: Option<CopyId>,
}

/// What changed, as `on_change` hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Another client's copy now holds `selection`.
    Offered {
        selection: Selection,
        mimes: Vec<String>,
    },
    /// Nobody holds `selection` now.
    Emptied { selection: Selection },
    /// One of this process's copies now holds `selection`.
    Holding { selection: Selection, copy: CopyId },
    /// One of this process's copies no longer holds `selection`.
    Lost {
        selection: Selection,
        copy: CopyId,
        why: Lost,
    },
}

/// How handing the clipboard to a clipboard manager went, at shutdown (X11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandOff {
    /// The manager took every type: the copy outlives the app.
    Saved,
    /// No clipboard manager runs.
    NoManager,
    /// The app held no clipboard copy that asked to be kept.
    NotHeld,
    /// The manager answered that it saved nothing.
    Refused,
    /// The manager didn't answer within the copy's wait.
    TimedOut,
    /// The X11 side was gone.
    Gone,
}

impl From<transfer::HandOff> for HandOff {
    fn from(h: transfer::HandOff) -> Self {
        use transfer::HandOff as T;
        match h {
            T::Saved => HandOff::Saved,
            T::NoManager => HandOff::NoManager,
            T::NotHeld => HandOff::NotHeld,
            T::Refused => HandOff::Refused,
            T::TimedOut => HandOff::TimedOut,
            T::Gone => HandOff::Gone,
        }
    }
}

/// What [`super::shutdown`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed {
    /// A transfer substrate was bound, and is now torn down.
    pub bound: bool,
    /// The X11 hand-off to a clipboard manager, when one was tried.
    pub hand_off: Option<HandOff>,
}
