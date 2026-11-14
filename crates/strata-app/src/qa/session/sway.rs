// SPDX-License-Identifier: MIT

//! sway's IPC, as the session host needs it: where this process's window is, how large the
//! output is, and a command to resize the window.

use std::{
    io::{Read as _, Write as _},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use serde_json::Value;

const SOCKET_VARIABLE: &str = "SWAYSOCK";
const MAGIC: &[u8; 6] = b"i3-ipc";
const RUN_COMMAND: u32 = 0;
const GET_OUTPUTS: u32 = 3;
const GET_TREE: u32 = 4;

/// A box on the output, in its logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Area {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

impl Area {
    fn of(rect: &Value) -> Option<Self> {
        Some(Self {
            x: rect["x"].as_f64()?,
            y: rect["y"].as_f64()?,
            width: rect["width"].as_f64()?,
            height: rect["height"].as_f64()?,
        })
    }
}

pub(super) struct Sway {
    socket: PathBuf,
}

impl Sway {
    pub(super) fn from_environment() -> Result<Self, String> {
        let socket = std::env::var_os(SOCKET_VARIABLE)
            .map(PathBuf::from)
            .ok_or("the session host needs sway's SWAYSOCK")?;
        Ok(Self { socket })
    }

    fn ask(&self, kind: u32, payload: &str) -> Result<Value, String> {
        let io = |error: std::io::Error| format!("sway IPC: {error}");
        let mut stream = UnixStream::connect(&self.socket).map_err(io)?;
        let length = u32::try_from(payload.len()).map_err(|error| error.to_string())?;
        let mut message = MAGIC.to_vec();
        message.extend(length.to_ne_bytes());
        message.extend(kind.to_ne_bytes());
        message.extend(payload.as_bytes());
        stream.write_all(&message).map_err(io)?;
        let mut header = [0; 14];
        stream.read_exact(&mut header).map_err(io)?;
        let mut length = [0; 4];
        length.copy_from_slice(&header[6..10]);
        let length = usize::try_from(u32::from_ne_bytes(length)).map_err(|e| e.to_string())?;
        let mut reply = vec![0; length];
        stream.read_exact(&mut reply).map_err(io)?;
        serde_json::from_slice(&reply).map_err(|error| format!("sway IPC: {error}"))
    }

    /// The size of the first active output.
    pub(super) fn output(&self) -> Result<(u32, u32), String> {
        let outputs = self.ask(GET_OUTPUTS, "")?;
        let output = outputs
            .as_array()
            .and_then(|outputs| outputs.iter().find(|output| output["active"] == true))
            .and_then(|output| Area::of(&output["rect"]))
            .ok_or("sway has no active output")?;
        let pixels = |value: f64| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "an output is a few thousand pixels wide"
            )]
            let pixels = value.round() as u32;
            pixels
        };
        Ok((pixels(output.width), pixels(output.height)))
    }

    /// Where the content of `pid`'s window is on the output.
    pub(super) fn window(&self, pid: u32) -> Result<Area, String> {
        let tree = self.ask(GET_TREE, "")?;
        let node = find(&tree, &|node| node["pid"].as_u64() == Some(u64::from(pid)))
            .ok_or("sway shows no window of this process")?;
        let outer = Area::of(&node["rect"]).ok_or("sway gave the window no rect")?;
        let inner = Area::of(&node["window_rect"]).unwrap_or(Area {
            x: 0.0,
            y: 0.0,
            ..outer
        });
        Ok(Area {
            x: outer.x + inner.x,
            y: outer.y + inner.y,
            width: inner.width,
            height: inner.height,
        })
    }

    pub(super) fn command(&self, command: &str) -> Result<(), String> {
        let results = self.ask(RUN_COMMAND, command)?;
        let failed = results
            .as_array()
            .and_then(|results| results.iter().find(|result| result["success"] != true));
        match failed {
            Some(failure) => Err(format!("sway refused {command:?}: {failure}")),
            None => Ok(()),
        }
    }
}

/// The first node of `tree`, depth first, that `wanted` picks.
fn find<'a>(tree: &'a Value, wanted: &dyn Fn(&Value) -> bool) -> Option<&'a Value> {
    if wanted(tree) {
        return Some(tree);
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|field| tree[field].as_array())
        .flatten()
        .find_map(|child| find(child, wanted))
}
