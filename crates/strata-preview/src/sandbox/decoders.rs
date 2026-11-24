// SPDX-License-Identifier: MIT

//! Decoder capabilities of the FFmpeg the media sandbox runs.

use std::{
    process::{Command, Stdio},
    sync::OnceLock,
};

#[cfg(test)]
mod tests;

/// Animated WebP decoding arrived in FFmpeg 9.0. Asked once per process; the
/// listing comes from a trusted runtime binary given no input.
pub fn decodes_animated_webp() -> bool {
    static DECODES: OnceLock<bool> = OnceLock::new();
    *DECODES.get_or_init(|| {
        super::sandbox_tool("ffmpeg").is_some_and(|ffmpeg| {
            Command::new(ffmpeg)
                .args(["-hide_banner", "-decoders"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|output| {
                    output.status.success()
                        && lists_decoder(&String::from_utf8_lossy(&output.stdout), "webp_anim")
                })
        })
    })
}

fn lists_decoder(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name))
}
