// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, process::Command};

use gtk::{gio, prelude::*};

use super::{ORIGINAL_GDK_DEBUG, restore_command_from, restore_launch_context_from};

const RESTARTED_DEBUG: &str = "misc:no-portals";

#[test]
fn launched_commands_see_the_original_gdk_debug() -> Result<(), Box<dyn std::error::Error>> {
    for (original, expected) in [
        (Some("misc"), "misc|unset"),
        (Some(""), "unset|unset"),
        (None, "misc:no-portals|unset"),
    ] {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "printf '%s|%s' \"${GDK_DEBUG-unset}\" \"${STRATA_ORIGINAL_GDK_DEBUG-unset}\"",
            ])
            .env("GDK_DEBUG", RESTARTED_DEBUG);
        if let Some(original) = original {
            command.env(ORIGINAL_GDK_DEBUG, original);
        }
        restore_command_from(&mut command, original.map(OsStr::new));
        let output = command.output()?;
        assert_eq!(String::from_utf8(output.stdout)?, expected, "{original:?}");
    }
    Ok(())
}

#[test]
fn launch_contexts_pass_the_original_gdk_debug() {
    for (original, expected) in [
        (Some("misc"), Some("GDK_DEBUG=misc")),
        (Some(""), None),
        (None, Some("GDK_DEBUG=misc:no-portals")),
    ] {
        let context = gio::AppLaunchContext::new();
        context.setenv("GDK_DEBUG", RESTARTED_DEBUG);
        if let Some(original) = original {
            context.setenv(ORIGINAL_GDK_DEBUG, original);
        }
        restore_launch_context_from(&context, original.map(OsStr::new));
        let environment = context.environment();
        let debug = environment
            .iter()
            .find(|entry| entry.as_encoded_bytes().starts_with(b"GDK_DEBUG="));
        assert_eq!(
            debug.map(|entry| entry.to_str()),
            expected.map(Some),
            "{original:?}"
        );
        assert!(
            !environment.iter().any(|entry| entry
                .as_encoded_bytes()
                .starts_with(ORIGINAL_GDK_DEBUG.as_bytes())),
            "{original:?}"
        );
    }
}
