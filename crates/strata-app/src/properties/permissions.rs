// SPDX-License-Identifier: MIT

//! The Permissions editor in Properties (`PermissionEditor` and `request_permission_change` in
//! `src/ui/browser/properties.rs`): symbolic + octal mode, owner/group identities, nine
//! `r`/`w`/`x` toggle buttons, the `+x` checkbox on files, optimistic updates, and rollback with
//! a stacked error dialog on failure.

use gio::prelude::*;
use rune::Tx;
use strata_core::i18n::tr;
use strata_fs::adapters::gio_file_for_location;

use super::{Dialog, show_notice};
use crate::App;

pub(super) struct BitSpec {
    pub(super) id: &'static str,
    pub(super) mask: u32,
    pub(super) glyph: &'static str,
    pub(super) label: &'static str,
    pub(super) description: &'static str,
}

pub(super) struct GroupSpec {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) bits: [BitSpec; 3],
}

pub(super) const GROUPS: [GroupSpec; 3] = [
    GroupSpec {
        id: "owner",
        title: "Owner",
        bits: [
            BitSpec {
                id: "owner-r",
                mask: 0o400,
                glyph: "r",
                label: "Owner read",
                description: "Toggle owner read permission",
            },
            BitSpec {
                id: "owner-w",
                mask: 0o200,
                glyph: "w",
                label: "Owner write",
                description: "Toggle owner write permission",
            },
            BitSpec {
                id: "owner-x",
                mask: 0o100,
                glyph: "x",
                label: "Owner execute",
                description: "Toggle owner execute permission",
            },
        ],
    },
    GroupSpec {
        id: "group",
        title: "permissions.group",
        bits: [
            BitSpec {
                id: "group-r",
                mask: 0o040,
                glyph: "r",
                label: "Group read",
                description: "Toggle group read permission",
            },
            BitSpec {
                id: "group-w",
                mask: 0o020,
                glyph: "w",
                label: "Group write",
                description: "Toggle group write permission",
            },
            BitSpec {
                id: "group-x",
                mask: 0o010,
                glyph: "x",
                label: "Group execute",
                description: "Toggle group execute permission",
            },
        ],
    },
    GroupSpec {
        id: "others",
        title: "Others",
        bits: [
            BitSpec {
                id: "others-r",
                mask: 0o004,
                glyph: "r",
                label: "Others read",
                description: "Toggle others read permission",
            },
            BitSpec {
                id: "others-w",
                mask: 0o002,
                glyph: "w",
                label: "Others write",
                description: "Toggle others write permission",
            },
            BitSpec {
                id: "others-x",
                mask: 0o001,
                glyph: "x",
                label: "Others execute",
                description: "Toggle others execute permission",
            },
        ],
    },
];

pub(super) struct Editor {
    pub(super) mode: Option<u32>,
    pub(super) changing: bool,
    pub(super) owner: String,
    pub(super) group: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            mode: None,
            changing: false,
            owner: "—".to_owned(),
            group: "—".to_owned(),
        }
    }
}

impl Editor {
    pub(super) fn set_mode(&mut self, mode: u32) {
        self.mode = Some(mode);
    }

    pub(super) fn sensitive(&self) -> bool {
        self.mode.is_some() && !self.changing
    }

    pub(super) fn mode_text(&self) -> String {
        self.mode
            .map_or_else(|| "—".to_owned(), crate::details::permissions)
    }

    pub(super) fn identity<'a>(&'a self, group_id: &str) -> &'a str {
        match group_id {
            "owner" => &self.owner,
            "group" => &self.group,
            _ => "—",
        }
    }
}

pub(super) fn toggled_permission(mode: u32, mask: u32) -> u32 {
    mode ^ mask
}

pub(super) fn with_execute_permissions(mode: u32, executable: bool) -> u32 {
    if executable {
        mode | 0o111
    } else {
        mode & !0o111
    }
}

pub(super) fn toggle_bit(app: &mut App, mask: u32) {
    let Some(Dialog::Single(single)) = app.properties.dialog.as_ref() else {
        return;
    };
    let Some(mode) = single.permissions.mode else {
        return;
    };
    request_change(app, toggled_permission(mode, mask));
}

pub(super) fn toggle_executable(app: &mut App) {
    let Some(Dialog::Single(single)) = app.properties.dialog.as_ref() else {
        return;
    };
    let Some(mode) = single.permissions.mode else {
        return;
    };
    request_change(app, with_execute_permissions(mode, mode & 0o111 == 0));
}

fn request_change(app: &mut App, requested_mode: u32) {
    let later = app.properties.later.clone();
    let Some(Dialog::Single(single)) = app.properties.dialog.as_mut() else {
        return;
    };
    if single.permissions.changing {
        return;
    }
    let Some(previous_mode) = single.permissions.mode.replace(requested_mode) else {
        return;
    };
    single.permissions.changing = true;
    let generation = single.generation;
    let file = gio_file_for_location(&single.location);
    let attributes = gio::FileInfo::new();
    attributes.set_attribute_uint32("unix::mode", requested_mode & 0o7777);
    glib::MainContext::default().spawn_local(async move {
        let outcome = file
            .set_attributes_future(
                &attributes,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
            .map_err(|error| error.to_string());
        later.then(move |app: &mut App, _: &mut Tx| {
            finish_change(app, generation, previous_mode, outcome)
        });
    });
}

fn finish_change(
    app: &mut App,
    generation: u64,
    previous_mode: u32,
    outcome: Result<gio::FileInfo, String>,
) -> bool {
    let open = app.properties.open;
    let Some(Dialog::Single(single)) = app.properties.dialog.as_mut() else {
        return false;
    };
    if single.generation != generation {
        return false;
    }
    single.permissions.changing = false;
    if let Err(detail) = outcome {
        single.permissions.mode = Some(previous_mode);
        tracing::warn!(%detail, "unable to change file permissions");
        if open {
            show_notice(app, tr("Unable to change permissions"), detail);
        }
    }
    true
}
