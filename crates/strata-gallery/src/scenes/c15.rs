//! The gallery page for C15 StrataDialog: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c15-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C15-strata-dialog.md`.
//!
//! The page draws a stand-in window (a header bar, a listing with a focusable cursor row, a
//! path field) and a row of buttons that open each dialog, so scripts can open a dialog from
//! an opener and check where focus returns. The golden scenes open a dialog at once.

use rune::{Cx, El, KeyEvent, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Progress, Select, Switch, TextInput};
use strata_components::dialog::{
    self, DialogShell, Focus, Measure, StrataDialog, Tone, limit_on_change, remaining, text_area,
    trim_to_fit,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The dialog the page has open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    /// A message dialog: Close and Reconnect.
    Basic,
    /// Connect to Server: fields, a select, a limited field and a checkbox.
    Form,
    /// Permanently delete: danger tone, the primary focused.
    Danger,
    /// The error a confirmed delete chains on.
    Error,
    /// Wider than the window, with a body taller than it.
    Small,
    /// A blocking copy in progress.
    Progress,
    /// A Settings-like panel in the shell, fading without a scale.
    Panel,
    /// A dialog that returns focus to the listing's cursor row.
    Return,
    /// A dialog whose primary focuses the path field as it closes.
    Take,
    /// Properties, which opens a second dialog over itself.
    Outer,
    /// A paste conflict: Skip, Cancel and Replace, opening on Cancel.
    Conflict,
}

/// The longest volume label, in the field's unit (a FAT label: 11 bytes).
const LABEL_ROOM: usize = 11;

const PROTOCOLS: [&str; 4] = ["SMB", "SFTP", "FTP", "WebDAV"];

/// C15's demo state, held in `Gallery::c15`.
#[derive(Default)]
pub struct Demo {
    pub open: Option<Open>,
    /// The danger dialog's primary chains on the error dialog.
    pub chain: bool,
    /// The second dialog is open over Properties.
    pub inner: bool,
    /// The second dialog's Apply removed Properties' "Change permissions…" button.
    pub applied: bool,
    /// What the last dialog did: "Cancelled", "Deleted", …
    pub status: String,
    /// Dismissals by Escape, the backdrop or the close button.
    pub dismissals: u32,
    pub address: TextEdit,
    pub label: TextEdit,
    /// The volume label counts characters rather than bytes.
    pub label_chars: bool,
    pub notes: TextEdit,
    pub remember: bool,
    pub protocol: Option<String>,
    pub protocol_open: bool,
    /// A field outside any dialog.
    pub path: TextEdit,
    pub hidden_files: bool,
    /// Puts focus in keyboard mode on the next frame, as a Tab and a Shift+Tab would: the
    /// keyboard scenes show the focus ring.
    pub keyboard: bool,
}

impl Demo {
    fn dirty(&self) -> bool {
        [&self.address, &self.label, &self.notes]
            .iter()
            .any(|e| !e.text.is_empty())
    }

    fn label_measure(&self) -> Measure {
        if self.label_chars {
            Measure::Chars
        } else {
            Measure::Bytes
        }
    }

    /// Opens `which` as its opener button does.
    fn show(&mut self, which: Open) {
        self.open = Some(which);
        match which {
            Open::Form => self.address.focused = true,
            Open::Outer => {
                self.inner = false;
                self.applied = false;
            }
            _ => {}
        }
    }
}

fn open_scene(g: &mut Gallery, dark: bool, which: Open) {
    g.open("c15", dark);
    g.c15.show(which);
}

fn openers_scene(g: &mut Gallery, dark: bool) {
    g.open("c15", dark);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c15", dark)`, then fills in `g.c15`.
pub const SCENES: &[Scene] = &[
    ("c15-basic-light", |g| open_scene(g, false, Open::Basic)),
    ("c15-basic-dark", |g| open_scene(g, true, Open::Basic)),
    ("c15-form-light", |g| open_scene(g, false, Open::Form)),
    ("c15-form-dark", |g| open_scene(g, true, Open::Form)),
    ("c15-danger-light", |g| open_scene(g, false, Open::Danger)),
    ("c15-danger-dark", |g| open_scene(g, true, Open::Danger)),
    ("c15-small-window-light", |g| {
        open_scene(g, false, Open::Small)
    }),
    ("c15-small-window-dark", |g| {
        open_scene(g, true, Open::Small)
    }),
    ("c15-danger-keyboard-light", |g| {
        open_scene(g, false, Open::Danger);
        g.c15.keyboard = true;
    }),
    ("c15-danger-keyboard-dark", |g| {
        open_scene(g, true, Open::Danger);
        g.c15.keyboard = true;
    }),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c15-openers-light", |g| openers_scene(g, false)),
    ("c15-openers-dark", |g| openers_scene(g, true)),
    ("c15-reduced-light", |g| {
        openers_scene(g, false);
        g.look.animations = false;
    }),
    ("c15-reduced-dark", |g| {
        openers_scene(g, true);
        g.look.animations = false;
    }),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let mut layer = Vec::new();
    // Popups (the form's select) go above the dialogs that own them.
    let mut popups = Vec::new();
    dialogs(g, look, cx.is_reduced_motion(), &mut layer, &mut popups);
    layer.extend(popups);
    rune_kit::root(window(g, look), layer)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds. A
/// keyboard scene focuses its dialog's initial focus again with the ring once the dialog is
/// up, shown as the keyboard shows it.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    if !g.c15.keyboard || t <= 0.0 {
        return false;
    }
    g.c15.keyboard = false;
    if let Some(f) = tx.focused().map(str::to_owned) {
        tx.focus(f, true);
    }
    true
}

fn demo(g: &mut Gallery) -> &mut Demo {
    &mut g.c15
}

/// Closes the dialog with `status`.
fn finish(g: &mut Gallery, status: &str) {
    let d = demo(g);
    d.status = status.into();
    d.open = None;
}

/// Escape, the backdrop or the close button.
fn dismissed(g: &mut Gallery) {
    let d = demo(g);
    d.dismissals += 1;
    d.status = "Dismissed".into();
    d.open = None;
}

/// The stand-in window behind the dialogs.
fn window(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let d = &g.c15;
    let header = row()
        .key("c15-header")
        .w_full()
        .h(look.frame.bar)
        .px(look.space.lg)
        .gap(look.space.md)
        .center_y()
        .fill(kit.c.layer_01)
        .border_sides([0.0, 0.0, 1.0, 0.0], kit.c.border_muted)
        .child(icon(look.icon(IconName::Back), look.icon_size.md).color(kit.c.icon_base))
        .child(icon(look.icon(IconName::Forward), look.icon_size.md).color(kit.c.icon_muted))
        .child(text("Projects").font(look.ty.title).color(kit.c.text_base))
        .child(el().grow(1.0))
        .child(icon(look.icon(IconName::Search), look.icon_size.md).color(kit.c.icon_base));

    let names = [
        "notes.md",
        "budget.ods",
        "photo.raw",
        "report.pdf",
        "slides.odp",
        "archive.tar.gz",
        "build.log",
        "readme.txt",
    ];
    let listing = col()
        .key("c15-listing")
        .w(look.row.list * 12.0)
        .gap(look.space.xxs)
        .padding(look.space.lg)
        .children(names.iter().enumerate().map(|(i, name)| {
            let r = row()
                .h(look.row.list)
                .px(look.space.md)
                .gap(look.space.md)
                .center_y()
                .radius(look.kit.radius.sm)
                .child(icon(look.icon(IconName::File), look.icon_size.md).color(kit.c.icon_muted))
                .child(text(*name).font(kit.ty.ui).color(kit.c.text_base));
            if i == 0 {
                r.key("c15-cursor")
                    .label(*name)
                    .focusable(true)
                    .fill(kit.c.layer_02)
            } else {
                r
            }
        }));

    let opener = |key: &str, label: &str, which: Open| {
        Button::new(label)
            .on_press(move |g: &mut Gallery| g.c15.show(which))
            .build(kit)
            .key(format!("c15-open-{key}"))
    };
    let chained = Button::new("Delete, then fail")
        .on_press(|g: &mut Gallery| {
            g.c15.chain = true;
            g.c15.show(Open::Danger);
        })
        .build(kit)
        .key("c15-open-chained");
    let openers = row()
        .key("c15-openers")
        .w_full()
        .flex_wrap()
        .gap(look.space.md)
        .child(opener("basic", "Message", Open::Basic))
        .child(opener("form", "Connect to Server", Open::Form))
        .child(opener("danger", "Delete", Open::Danger))
        .child(chained)
        .child(opener("small", "Leftovers", Open::Small))
        .child(opener("progress", "Copy", Open::Progress))
        .child(opener("panel", "Settings", Open::Panel))
        .child(opener("return", "Move to Trash", Open::Return))
        .child(opener("take", "Edit path", Open::Take))
        .child(opener("outer", "Properties", Open::Outer))
        .child(opener("conflict", "Paste", Open::Conflict));

    let dismissals = match d.dismissals {
        1 => "Dismissed 1 time".to_owned(),
        n => format!("Dismissed {n} times"),
    };
    let side = col()
        .key("c15-side")
        .grow(1.0)
        .gap(look.space.lg)
        .padding(look.space.lg)
        .child(openers)
        .child(
            TextInput::new(&d.path)
                .label("Path")
                .placeholder("/home/me/projects")
                .on_edit(|g: &mut Gallery| &mut g.c15.path)
                .build(kit)
                .key("c15-path"),
        )
        .child(
            text(if d.status.is_empty() {
                "Ready"
            } else {
                d.status.as_str()
            })
            .key("c15-status")
            .font(kit.ty.ui)
            .color(kit.c.text_base),
        )
        .child(
            text(dismissals)
                .key("c15-dismissals")
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );

    col()
        .key("c15-window")
        .w_full()
        .h_full()
        .child(header)
        .child(row().w_full().grow(1.0).child(listing).child(side))
}

fn dialogs(g: &Gallery, look: &Look, reduced: bool, layer: &mut Vec<El>, popups: &mut Vec<El>) {
    let d = &g.c15;
    let open = d.open;
    let kit = &look.kit;

    StrataDialog::new("c15-basic", "Connection lost", dismissed)
        .open(open == Some(Open::Basic))
        .reduced_motion(reduced)
        .subtitle("smb://nas.local/projects")
        .icon(IconName::Network)
        .width(dialog::MESSAGE_WIDTH)
        .message(
            "Strata lost the connection to nas.local. Reconnect to keep working with its files.",
        )
        .secondary("Close", |g: &mut Gallery| finish(g, "Closed"))
        .primary("Reconnect", |g: &mut Gallery| finish(g, "Reconnected"))
        .build(look, layer);

    StrataDialog::new("c15-danger", "Permanently delete 3 items?", dismissed)
        .open(open == Some(Open::Danger))
        .reduced_motion(reduced)
        .subtitle("3 items · 4.2 MB will be permanently deleted")
        .icon(IconName::Trash)
        .tone(Tone::Danger)
        .message("These items will be permanently deleted. This action cannot be undone.")
        .child(file_list(look))
        .secondary("Cancel", |g: &mut Gallery| finish(g, "Cancelled"))
        .primary("Permanently delete 3 items", |g: &mut Gallery| {
            if g.c15.chain {
                g.c15.status = "Deleted 2 of 3 items".into();
                g.c15.open = Some(Open::Error);
            } else {
                finish(g, "Deleted");
            }
        })
        .initial_focus(Focus::Primary)
        .build(look, layer);

    StrataDialog::new("c15-error", "Couldn't delete 1 item", dismissed)
        .open(open == Some(Open::Error))
        .reduced_motion(reduced)
        .subtitle("photo.raw")
        .icon(IconName::Error)
        .message("The file is open in another application.")
        .secondary("Close", |g: &mut Gallery| finish(g, "Closed"))
        .primary("Retry", |g: &mut Gallery| finish(g, "Retried"))
        .initial_focus(Focus::Primary)
        .build(look, layer);

    form(d, look, open == Some(Open::Form), reduced, layer, popups);

    StrataDialog::new("c15-conflict", "Replace photo.raw?", dismissed)
        .open(open == Some(Open::Conflict))
        .reduced_motion(reduced)
        .subtitle("A file with this name already exists in Pictures.")
        .icon(IconName::Paste)
        .action("skip", "Skip", |g: &mut Gallery| finish(g, "Skipped"))
        .secondary("Cancel", |g: &mut Gallery| finish(g, "Cancelled"))
        .primary("Replace", |g: &mut Gallery| finish(g, "Replaced"))
        .initial_focus(Focus::Secondary)
        .build(look, layer);

    StrataDialog::new("c15-small", "Review leftovers", dismissed)
        .open(open == Some(Open::Small))
        .reduced_motion(reduced)
        .subtitle("40 folders left by apps you removed")
        .icon(IconName::Folder)
        .width(1000.0)
        .children((0..40).map(|i| {
            text(format!("~/.config/app-{i:02} · {} MB", 3 + i * 7 % 50))
                .key(format!("c15-small-row-{i}"))
                .font(kit.ty.ui)
                .color(kit.c.text_base)
        }))
        .secondary("Cancel", |g: &mut Gallery| finish(g, "Cancelled"))
        .primary("Delete leftovers", |g: &mut Gallery| {
            finish(g, "Deleted leftovers")
        })
        .build(look, layer);

    StrataDialog::new("c15-progress", "Copying 1,204 files", |g: &mut Gallery| {
        finish(g, "Moved to the dock")
    })
    .open(open == Some(Open::Progress))
    .reduced_motion(reduced)
    .subtitle("To /mnt/backup")
    .icon(IconName::Copy)
    .loading(Some("Copying…".into()))
    .blocking(true)
    .child(Progress::new(Some(0.34)).label("Copied").build(kit))
    .message("412 of 1,204 files · 1.2 GB of 3.4 GB")
    .secondary("Cancel", |g: &mut Gallery| finish(g, "Copy cancelled"))
    .build(look, layer);

    DialogShell::new("c15-panel", "Settings", dismissed)
        .open(open == Some(Open::Panel))
        .reduced_motion(reduced)
        .fade_only()
        .width(640.0)
        .child(
            col()
                .w_full()
                .gap(look.space.lg)
                .padding(look.space.xl)
                .child(
                    text("Settings")
                        .font(look.ty.title)
                        .color(kit.c.text_base)
                        .a11y_hidden(),
                )
                .child(
                    Switch::new(d.hidden_files)
                        .label("Show hidden files")
                        .on_toggle(|g: &mut Gallery| g.c15.hidden_files = !g.c15.hidden_files)
                        .build(kit)
                        .key("c15-panel-hidden"),
                ),
        )
        .build(look, layer);

    StrataDialog::new("c15-return", "Move 3 items to Trash?", dismissed)
        .open(open == Some(Open::Return))
        .reduced_motion(reduced)
        .subtitle("They can be restored from Trash.")
        .icon(IconName::Trash)
        .secondary("Cancel", |g: &mut Gallery| finish(g, "Cancelled"))
        .primary("Move to Trash", |g: &mut Gallery| {
            finish(g, "Moved to Trash")
        })
        .initial_focus(Focus::Primary)
        .return_focus("c15-cursor")
        .build(look, layer);

    StrataDialog::new("c15-take", "Edit the path?", dismissed)
        .open(open == Some(Open::Take))
        .reduced_motion(reduced)
        .message("The path field takes focus as this dialog closes.")
        .secondary("Cancel", |g: &mut Gallery| finish(g, "Cancelled"))
        .primary("Edit path", |g: &mut Gallery| {
            finish(g, "Editing the path");
            g.c15.path.focused = true;
        })
        .initial_focus(Focus::Primary)
        .build(look, layer);

    let outer_body = if d.applied {
        text("Permissions changed")
            .key("c15-outer-applied")
            .font(kit.ty.ui)
            .color(kit.c.text_muted)
    } else {
        Button::new("Change permissions…")
            .variant(ButtonVariant::Outline)
            .on_press(|g: &mut Gallery| g.c15.inner = true)
            .build(kit)
            .key("c15-outer-permissions")
    };
    StrataDialog::new("c15-outer", "Properties", dismissed)
        .open(open == Some(Open::Outer))
        .reduced_motion(reduced)
        .subtitle("notes.md")
        .icon(IconName::Properties)
        .child(outer_body)
        .secondary("Close", |g: &mut Gallery| finish(g, "Closed"))
        .build(look, layer);

    StrataDialog::new("c15-inner", "Change permissions?", |g: &mut Gallery| {
        g.c15.inner = false
    })
    .open(open == Some(Open::Outer) && d.inner)
    .reduced_motion(reduced)
    .stacked(true)
    .message("Others will be able to read notes.md.")
    .secondary("Cancel", |g: &mut Gallery| g.c15.inner = false)
    .primary("Apply", |g: &mut Gallery| {
        g.c15.inner = false;
        g.c15.applied = true;
        g.c15.status = "Permissions changed".into();
    })
    .initial_focus(Focus::Primary)
    .build(look, layer);
}

/// Connect to Server: a focused address field, a protocol select, a byte-limited label with
/// its caption, notes and a checkbox. Connect needs an address, and once anything is typed a
/// click outside no longer dismisses.
fn form(
    d: &Demo,
    look: &Look,
    open: bool,
    reduced: bool,
    layer: &mut Vec<El>,
    popups: &mut Vec<El>,
) {
    let kit = &look.kit;
    let measure = d.label_measure();
    let left = remaining(&d.label.text, LABEL_ROOM, measure);
    let caption = match left {
        1 => "1 char remaining".to_owned(),
        n => format!("{n} chars remaining"),
    };
    let protocol = Select::new("c15-protocol", &PROTOCOLS)
        .label("Protocol")
        .current(Some(d.protocol.as_deref().unwrap_or("SMB")))
        .open(d.protocol_open)
        .on_toggle(|g: &mut Gallery| g.c15.protocol_open = !g.c15.protocol_open)
        .on_close(|g: &mut Gallery| g.c15.protocol_open = false)
        .on_choose(|g: &mut Gallery, v: &str, _tx: &mut Tx| {
            g.c15.protocol = Some(v.to_owned());
            g.c15.protocol_open = false;
        })
        .build(kit, popups);
    let address = d.address.text.trim().to_owned();
    let lines = match d.notes.text.lines().count() {
        1 => "1 line".to_owned(),
        n => format!("{n} lines"),
    };

    StrataDialog::new("c15-form", "Connect to Server", dismissed)
        .open(open)
        .reduced_motion(reduced)
        .subtitle("Browse files on another computer")
        .icon(IconName::Network)
        .blocking(d.dirty())
        .child(field(
            look,
            "Server address",
            TextInput::new(&d.address)
                .label("Server address")
                .placeholder("smb://server/share")
                .on_edit(|g: &mut Gallery| &mut g.c15.address)
                .build(kit)
                .key("c15-form-address"),
        ))
        .child(field(look, "Protocol", protocol))
        .child(field(
            look,
            "Volume label",
            col()
                .w_full()
                .gap(look.space.xs)
                .child(
                    TextInput::new(&d.label)
                        .label("Volume label")
                        .on_edit(|g: &mut Gallery| &mut g.c15.label)
                        .on_change(limit_on_change(
                            |g: &mut Gallery| &mut g.c15.label,
                            |g: &Gallery| (LABEL_ROOM, g.c15.label_measure()),
                        ))
                        .build(kit)
                        .key("c15-form-label"),
                )
                .child(
                    text(caption)
                        .key("c15-form-label-left")
                        .font(kit.ty.small)
                        .color(kit.c.text_muted),
                ),
        ))
        .child(
            Checkbox::new(d.label_chars)
                .label("Count the label in characters")
                .on_toggle(|g: &mut Gallery| {
                    let d = &mut g.c15;
                    d.label_chars = !d.label_chars;
                    let measure = d.label_measure();
                    trim_to_fit(&mut d.label, LABEL_ROOM, measure);
                })
                .build(kit)
                .key("c15-form-chars"),
        )
        .child(field(
            look,
            "Notes",
            col()
                .w_full()
                .gap(look.space.xs)
                .child(
                    text_area(&d.notes, |g: &mut Gallery| &mut g.c15.notes)
                        .label("Notes")
                        .rows(3)
                        .build(kit)
                        .key("c15-form-notes"),
                )
                .child(
                    text(lines)
                        .key("c15-form-notes-lines")
                        .font(kit.ty.small)
                        .color(kit.c.text_muted),
                ),
        ))
        .child(
            Checkbox::new(d.remember)
                .label("Remember password")
                .on_toggle(|g: &mut Gallery| g.c15.remember = !g.c15.remember)
                .build(kit)
                .key("c15-form-remember"),
        )
        .secondary("Cancel", |g: &mut Gallery| finish(g, "Cancelled"))
        // Before `primary`: either order disables it.
        .primary_enabled(!address.is_empty())
        .primary("Connect", move |g: &mut Gallery| {
            let to = g.c15.address.text.trim().to_owned();
            finish(g, &format!("Connected to {to}"));
        })
        .build(look, layer);
}

/// A form row: a visible label over its control, which carries the same accessible name.
fn field(look: &Look, label: &str, control: El) -> El {
    col()
        .w_full()
        .gap(look.space.xs)
        .child(
            text(label)
                .font(look.kit.ty.small_medium)
                .color(look.kit.c.text_muted)
                .a11y_hidden(),
        )
        .child(control)
}

/// The delete confirmation's list of the items it asks about.
fn file_list(look: &Look) -> El {
    let kit = &look.kit;
    let files = [
        ("report.pdf", "2.1 MB"),
        ("photo.raw", "1.9 MB"),
        ("notes.md", "200 kB"),
    ];
    col()
        .key("c15-danger-files")
        .w_full()
        .gap(look.space.xxs)
        .px(look.space.md)
        .py(look.space.sm)
        .radius(look.round(3.0))
        .fill(kit.c.layer_02)
        .children(files.iter().map(|(name, size)| {
            row()
                .w_full()
                .h(look.row.list)
                .gap(look.space.md)
                .center_y()
                .label(format!("{name}, {size}"))
                .child(icon(look.icon(IconName::File), look.icon_size.md).color(kit.c.icon_muted))
                .child(
                    text(*name)
                        .grow(1.0)
                        .font(kit.ty.ui)
                        .color(kit.c.text_base)
                        .a11y_hidden(),
                )
                .child(
                    text(*size)
                        .font(kit.ty.small)
                        .color(kit.c.text_muted)
                        .a11y_hidden(),
                )
        }))
}
