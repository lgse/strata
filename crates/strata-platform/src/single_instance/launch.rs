//! What a launch carries to the primary: its arguments, its working directory and the
//! desktop's activation token, in GApplication's platform data.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use zbus::{OwnedVariant, Str, Variant};

/// The platform data dictionary (`a{sv}`) of GApplication's methods.
pub(super) type PlatformData = HashMap<String, OwnedVariant>;

/// GApplication's platform data keys: the working directory as a byte string, and the
/// launcher's token under the name GTK reads on each windowing system.
const CWD: &str = "cwd";
const ACTIVATION_TOKEN: &str = "activation-token";
const DESKTOP_STARTUP_ID: &str = "desktop-startup-id";

/// The token a launcher hands the app it starts, so the compositor lets the app's window take
/// focus: `XDG_ACTIVATION_TOKEN` on Wayland (xdg-activation), `DESKTOP_STARTUP_ID` on X11
/// (startup notification). A launch from a terminal usually has neither.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activation {
    pub xdg_activation_token: Option<String>,
    pub desktop_startup_id: Option<String>,
}

impl Activation {
    /// This process's own tokens, from the environment. Empty values count as none.
    pub fn from_env() -> Activation {
        let var = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
        Activation {
            xdg_activation_token: var("XDG_ACTIVATION_TOKEN"),
            desktop_startup_id: var("DESKTOP_STARTUP_ID"),
        }
    }

    /// The token to pass to `rune::AppHandle::activate`: the one for the windowing system this
    /// process uses (Wayland when `WAYLAND_DISPLAY` is set, as winit picks it), else the other.
    pub fn token(&self) -> Option<&str> {
        self.token_for(std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()))
    }

    /// [`Activation::token`] for a Wayland (`true`) or X11 (`false`) process.
    pub fn token_for(&self, wayland: bool) -> Option<&str> {
        let (first, second) = if wayland {
            (&self.xdg_activation_token, &self.desktop_startup_id)
        } else {
            (&self.desktop_startup_id, &self.xdg_activation_token)
        };
        first.as_deref().or(second.as_deref())
    }

    pub fn is_empty(&self) -> bool {
        self.xdg_activation_token.is_none() && self.desktop_startup_id.is_none()
    }

    /// Reads the tokens from GApplication's platform data. Missing, mistyped or empty entries
    /// are none: platform data comes from any process on the bus.
    pub(super) fn from_platform_data(data: &PlatformData) -> Activation {
        let text = |key| {
            data.get(key)
                .and_then(|v| <&str>::try_from(&**v).ok())
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        Activation {
            xdg_activation_token: text(ACTIVATION_TOKEN),
            desktop_startup_id: text(DESKTOP_STARTUP_ID),
        }
    }

    pub(super) fn write_platform_data(&self, data: &mut PlatformData) {
        for (key, value) in [
            (ACTIVATION_TOKEN, &self.xdg_activation_token),
            (DESKTOP_STARTUP_ID, &self.desktop_startup_id),
        ] {
            if let Some(value) = value {
                data.insert(key.to_owned(), OwnedVariant::from(Str::from(value.clone())));
            }
        }
    }
}

/// One launch of the app: what it was started with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// The whole argument vector, the program first, as GApplication passes it. Arguments
    /// need not be UTF-8.
    pub args: Vec<OsString>,
    /// The launch's working directory, for resolving relative paths in `args`.
    pub cwd: Option<PathBuf>,
    pub activation: Activation,
}

impl Launch {
    /// This process's launch: `std::env::args_os`, the current directory and the tokens in the
    /// environment.
    pub fn from_env() -> Launch {
        Launch {
            args: std::env::args_os().collect(),
            cwd: std::env::current_dir().ok(),
            activation: Activation::from_env(),
        }
    }

    /// The arguments as GApplication's `aay`: each one NUL-terminated, as GLib's byte strings
    /// are (it reads one without the NUL as empty).
    pub(super) fn wire_args(&self) -> Vec<Vec<u8>> {
        self.args.iter().map(|a| bytestring(a.as_bytes())).collect()
    }

    pub(super) fn platform_data(&self) -> PlatformData {
        let mut data = PlatformData::new();
        // A working directory that won't convert is left out: the primary then has none.
        if let Some(cwd) = self.cwd.as_ref().and_then(|cwd| {
            OwnedVariant::try_from(Variant::from(bytestring(cwd.as_os_str().as_bytes()))).ok()
        }) {
            data.insert(CWD.to_owned(), cwd);
        }
        self.activation.write_platform_data(&mut data);
        data
    }

    /// The launch a `CommandLine` call describes.
    pub(super) fn from_wire(args: Vec<Vec<u8>>, data: &PlatformData) -> Launch {
        let cwd = data
            .get(CWD)
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| Vec::<u8>::try_from(v).ok())
            .map(unbytestring)
            .filter(|c| !c.is_empty())
            .map(|c| PathBuf::from(OsString::from_vec(c)));
        Launch {
            args: args
                .into_iter()
                .map(|a| OsString::from_vec(unbytestring(a)))
                .collect(),
            cwd,
            activation: Activation::from_platform_data(data),
        }
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }
}

fn bytestring(bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(bytes.len() + 1);
    v.extend_from_slice(bytes);
    v.push(0);
    v
}

/// A GLib byte string's bytes: up to its first NUL, which GLib always sends and other callers
/// may leave out.
fn unbytestring(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(nul) = bytes.iter().position(|b| *b == 0) {
        bytes.truncate(nul);
    }
    bytes
}
