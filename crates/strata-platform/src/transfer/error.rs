use std::fmt;

/// Why a route or the whole substrate is missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// libwayland-client has no `wl_proxy_get_display` (it arrived in 1.23; Ubuntu 24.04
    /// ships 1.22), so winit's display can't be reached from a Rune window. Below 1.23 is
    /// unsupported (S3 decision 1).
    LibwaylandBelow1_23,
    /// The window is neither Wayland nor X11.
    NotLinux(String),
    /// The compositor lacks a protocol this route needs.
    NoProtocol(&'static str),
    /// Nothing was asked for that this window can have.
    NothingWanted,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unsupported::LibwaylandBelow1_23 => f.write_str(
                "libwayland < 1.23: no wl_proxy_get_display, so winit's display is out of reach",
            ),
            Unsupported::NotLinux(kind) => write!(f, "not a Wayland or X11 window: {kind}"),
            Unsupported::NoProtocol(p) => write!(f, "the compositor has no {p}"),
            Unsupported::NothingWanted => f.write_str("no route asked for is available"),
        }
    }
}

/// A failed transfer call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported(Unsupported),
    /// The window is closed (`WindowHandle::with_raw_handle` failed).
    WindowClosed(String),
    /// The route isn't bound: no such protocol, or not asked for at attach.
    NoRoute(&'static str),
    /// A data-device write needs an input serial, and this client has had no key or button
    /// event yet.
    NoSerial,
    /// The substrate was shut down.
    Shutdown,
    /// A request the protocol forbids here, refused before it reached the server (where it
    /// would end winit's connection).
    Refused(&'static str),
    /// The connection failed.
    Connection(String),
    /// A call failed and the connection lives on: the compositor stopped reading for a
    /// while, or sent an event this client couldn't parse.
    Busy(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(u) => write!(f, "unsupported: {u}"),
            Error::WindowClosed(e) => write!(f, "the window is closed: {e}"),
            Error::NoRoute(r) => write!(f, "no {r} route is bound"),
            Error::NoSerial => f.write_str("no input serial yet: no key or button event"),
            Error::Shutdown => f.write_str("the transfer substrate was shut down"),
            Error::Refused(why) => write!(f, "refused: {why}"),
            Error::Connection(e) => write!(f, "connection failed: {e}"),
            Error::Busy(e) => write!(f, "the call failed, the connection lives on: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Unsupported> for Error {
    fn from(u: Unsupported) -> Self {
        Error::Unsupported(u)
    }
}

/// A failed read of one MIME type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The source sent more than [`ReadLimits::max_bytes`](super::ReadLimits).
    TooLarge,
    /// The source sent nothing for [`ReadLimits::idle`](super::ReadLimits).
    TimedOut,
    /// The offer is no longer current: the selection changed, or the drag ended.
    Gone,
    /// The source doesn't offer this type, or refused it.
    NotOffered,
    /// The substrate was shut down.
    Shutdown,
    /// An offer on winit's display was asked from a thread other than the UI thread.
    OffUiThread,
    Io(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::TooLarge => f.write_str("more data than the read allows"),
            ReadError::TimedOut => f.write_str("the source went quiet"),
            ReadError::Gone => f.write_str("the offer is gone"),
            ReadError::NotOffered => f.write_str("the source doesn't offer that type"),
            ReadError::Shutdown => f.write_str("the transfer substrate was shut down"),
            ReadError::OffUiThread => {
                f.write_str("an offer on winit's display reads from the UI thread only")
            }
            ReadError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReadError {}
