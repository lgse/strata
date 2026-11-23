// SPDX-License-Identifier: MIT

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One command for the peer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Command {
    Devices {
        pointer: bool,
        keyboard: bool,
        extent: [u32; 2],
    },
    Map {
        title: String,
        width: u32,
        height: u32,
    },
    /// Moves the virtual pointer to a layout point.
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerButton {
        button: Button,
        pressed: bool,
    },
    /// Moves the virtual pointer from where it is to `to` in `steps` motions, `step_ms` apart.
    PointerPath {
        to: [f64; 2],
        steps: u32,
        step_ms: u64,
    },
    Key {
        key: String,
        #[serde(default)]
        pressed: Option<bool>,
    },
    /// Types each character, with Shift where the US layout needs it.
    Type {
        text: String,
    },
    SelectionSet {
        selection: Selection,
        via: Via,
        offers: Vec<Offer>,
    },
    /// The MIME types the current selection offers ([`Targets`]); none when nobody owns it.
    SelectionTargets {
        selection: Selection,
        via: Via,
    },
    /// Reads one type of the current selection ([`Received`]).
    SelectionRead {
        selection: Selection,
        via: Via,
        mime: String,
    },
    SelectionReadMultiple {
        selection: Selection,
        mimes: Vec<String>,
    },
    DragStart {
        offers: Vec<Offer>,
        actions: Vec<Action>,
    },
    DropAccept {
        mimes: Vec<String>,
        actions: Vec<Action>,
        preferred: Action,
        #[serde(default)]
        read_all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ask_answer: Option<Action>,
    },
    DropProxy,
    /// Exports the peer's window for another client to parent to ([`Exported`]).
    Export,
    ClipboardManager {
        answer: bool,
    },
    WindowsOfType {
        ty: String,
    },
    ActivationToken,
    /// Replies once everything sent before it has reached the server and come back.
    Sync,
    /// Exits. End of input does the same.
    Quit,
}

/// A selection. Wayland's data device has only the clipboard; primary needs data control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Selection {
    Clipboard,
    Primary,
}

/// The protocol a selection command goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Via {
    /// `wl_data_device`: needs keyboard focus on the peer's window.
    DataDevice,
    /// `ext_data_control_v1`, else `zwlr_data_control_v1`: no focus needed.
    DataControl,
    /// X11 `CLIPBOARD` and `PRIMARY`.
    X11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Copy,
    Move,
    Ask,
    /// X11 only: Wayland's data device has no link action, and offers none.
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// The evdev code (`BTN_LEFT` and so on).
    pub fn code(self) -> u32 {
        match self {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Wayland,
    X11,
}

/// One MIME type a source offers, with its bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub mime: String,
    pub data: Payload,
}

impl Offer {
    pub fn text(mime: &str, text: &str) -> Self {
        Offer {
            mime: mime.to_owned(),
            data: Payload::Text(text.to_owned()),
        }
    }
}

/// An offer's bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Payload {
    Text(String),
    /// A file's contents, read when a reader asks.
    File(PathBuf),
    /// `len` deterministic bytes from `seed` ([`pattern`]), for large transfers.
    Pattern {
        len: u64,
        seed: u64,
    },
}

impl Payload {
    pub fn bytes(&self) -> io::Result<Vec<u8>> {
        match self {
            Payload::Text(t) => Ok(t.as_bytes().to_vec()),
            Payload::File(p) => std::fs::read(p),
            Payload::Pattern { len, seed } => Ok(pattern(*len, *seed)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Received {
    pub len: u64,
    pub fnv1a64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
}

/// The largest data a [`Received`] carries inline as `text`.
pub const INLINE_TEXT: usize = 64 * 1024;

/// The reply to [`Command::Map`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mapped {
    /// The X window, on X11.
    pub xid: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultipleRead {
    pub results: Vec<Option<Received>>,
}

/// The reply to [`Command::SelectionTargets`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Targets {
    pub mimes: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Exported {
    pub handle: String,
}

/// One window in the reply to [`Command::WindowsOfType`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedWindow {
    pub xid: u32,
    pub width: u16,
    pub height: u16,
    pub mapped: bool,
}

/// The reply to [`Command::ActivationToken`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActivationToken {
    pub token: String,
}

/// A line the peer writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Reply(Reply),
    Event(Event),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// The first line: the peer is connected and reads commands.
    Ready {
        backend: Backend,
        pid: u32,
        /// Wayland: `interface vN` for each global. Empty on X11.
        globals: Vec<String>,
        /// Wayland: the data-control protocol bound (`ext` or `wlr`), if any.
        data_control: Option<String>,
    },
    /// Keyboard focus on the peer's window came or went.
    Focus {
        focused: bool,
    },
    /// The pointer entered the peer's window, at window-local points.
    PointerEnter {
        x: f64,
        y: f64,
    },
    /// The pointer moved in the peer's window, at window-local points.
    PointerMotion {
        x: f64,
        y: f64,
    },
    /// The pointer left the peer's window.
    PointerLeave,
    /// A button went down or up on the peer's window.
    PointerButton {
        pressed: bool,
    },
    SelectionChanged {
        selection: Selection,
        via: Via,
        mimes: Option<Vec<String>>,
    },
    /// A reader asked one of the peer's sources for `mime` (selection or drag).
    SourceSend {
        mime: String,
    },
    /// Another client took a selection the peer owned.
    SelectionCancelled {
        selection: Selection,
        via: Via,
    },
    /// Drag source: the target accepts `mime` (Wayland), or nothing.
    DragTarget {
        mime: Option<String>,
    },
    /// Drag source: the action the target and the compositor settled on.
    DragAction {
        action: Option<Action>,
    },
    /// Drag source: the drop happened over a target that accepted it.
    DragDropped,
    /// Drag source: the target read the data and finished.
    DragFinished {
        action: Option<Action>,
    },
    /// Drag source: the drag ended without a drop.
    DragCancelled,
    DropEnter {
        mimes: Vec<String>,
        #[serde(default)]
        proxied: bool,
    },
    /// Drop target: the drag moved, in window-local points.
    DropMotion {
        x: f64,
        y: f64,
    },
    DropLeave,
    /// Drop target: what the drop delivered.
    Dropped {
        mime: String,
        action: Option<Action>,
        data: Received,
    },
    ManagerSaved {
        saved: Vec<String>,
        refused: Vec<String>,
    },
    /// Something failed outside a request.
    Error {
        message: String,
    },
}

pub fn pattern(len: u64, seed: u64) -> Vec<u8> {
    let mut x = seed ^ 0x9e37_79b9_7f4a_7c15;
    let mut out = Vec::with_capacity(len as usize);
    while (out.len() as u64) < len {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let word = x.wrapping_mul(0x2545_f491_4f6c_dd1d).to_le_bytes();
        let take = (len as usize - out.len()).min(8);
        out.extend_from_slice(&word[..take]);
    }
    out
}

/// FNV-1a, 64-bit, as 16 hex digits.
pub fn fnv1a64(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn chord_keys(chord: &str) -> Result<Vec<String>, String> {
    let modifier = |m: &str| match m.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some("Control_L"),
        "shift" => Some("Shift_L"),
        "alt" => Some("Alt_L"),
        "super" | "logo" => Some("Super_L"),
        _ => None,
    };
    let parts: Vec<&str> = chord.split('+').map(str::trim).collect();
    let Some((key, mods)) = parts.split_last() else {
        return Err(format!("empty chord {chord:?}"));
    };
    if key.is_empty() {
        return Err(format!("chord {chord:?} has no key"));
    }
    let mut keys = Vec::with_capacity(parts.len());
    for m in mods {
        let name = modifier(m).ok_or_else(|| format!("chord {chord:?}: unknown modifier {m:?}"))?;
        keys.push(name.to_owned());
    }
    keys.push(modifier(key).map_or_else(|| (*key).to_owned(), str::to_owned));
    Ok(keys)
}
