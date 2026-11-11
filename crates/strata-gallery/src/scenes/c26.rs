//! The gallery page for C26 MountPrompts: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c26-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C26-mount-prompts.md`.
//!
//! The page plays the host: openers for each prompt, a status line saying what the last one
//! answered (a password only by its length), and an End process that answers as the backend
//! does, by sending the shorter list. The golden scenes open a prompt at once.

use rune::{Cx, El, KeyEvent, LiveRegion, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::mount_prompts::{
    BlockingProcess, Choice, MountLabels, MountPromptState, MountPrompts, MountRequest,
    MountResponse, PasswordRequest, ProcessesRequest, QuestionRequest, Secret,
};

use crate::{Gallery, Scene};

/// C26's demo state, held in `Gallery::c26`.
#[derive(Default)]
pub struct Demo {
    pub prompt: MountPromptState<u32>,
    /// What the last prompt answered.
    pub status: String,
    /// The open request, for the status line's choice names and End process's shorter list.
    pub request: Option<MountRequest>,
    next: u32,
}

impl Demo {
    /// Opens `request` as the host does with a request from the bridge.
    pub fn show(&mut self, request: MountRequest) {
        self.next += 1;
        self.request = Some(request.clone());
        let _ = self.prompt.open(request, self.next);
    }
}

/// An SMB share that asks for everything GIO can.
pub fn sign_in() -> PasswordRequest {
    PasswordRequest {
        message: "Password required for share “public” on “nas.local”".into(),
        default_user: "ann".into(),
        default_domain: "WORKGROUP".into(),
        needs_user: true,
        needs_domain: true,
        needs_password: true,
        anonymous_allowed: true,
        saving_allowed: true,
        ..PasswordRequest::default()
    }
}

/// The same share after refused credentials.
pub fn sign_in_retry() -> PasswordRequest {
    let request = sign_in();
    PasswordRequest {
        error: Some(MountLabels::retry_message(&request)),
        ..request
    }
}

/// udisks2's LUKS unlock, as GVfs words it.
pub fn luks() -> PasswordRequest {
    let message = "Enter a passphrase to unlock the volume\nThe passphrase is needed to access \
                   encrypted data on SanDisk Ultra (32 GB).";
    PasswordRequest {
        message: message.into(),
        needs_password: true,
        saving_allowed: true,
        secret: Secret::guess(message),
        ..PasswordRequest::default()
    }
}

/// GVfs's SFTP backend asking for a key's passphrase.
pub fn key_passphrase() -> PasswordRequest {
    let message = "Enter passphrase for secure key “/home/ann/.ssh/id_ed25519”";
    PasswordRequest {
        message: message.into(),
        needs_password: true,
        saving_allowed: true,
        secret: Secret::guess(message),
        ..PasswordRequest::default()
    }
}

/// GVfs's SFTP backend on an unknown host key.
pub fn host_key() -> QuestionRequest {
    QuestionRequest {
        message: "Identity Verification Failed\nVerifying the identity of “files.example.org” \
                  failed, this happens when you log in to a computer the first time.\n\nThe \
                  identity sent by the remote computer is “SHA256:4b2Vd0fA9kq7pL3xYw”. If you \
                  want to be absolutely sure it is safe to continue, contact the system \
                  administrator."
            .into(),
        choices: vec![Choice::new("Log In Anyway"), Choice::new("Cancel Login")],
        host_key: true,
    }
}

/// A question from another backend: no host key.
pub fn question() -> QuestionRequest {
    QuestionRequest {
        message: "Insecure connection\nThe server “ftp.example.org” doesn't support encryption. \
                  Your password and files would be sent in the clear."
            .into(),
        choices: vec![Choice::new("Connect Anyway"), Choice::new("Cancel")],
        host_key: false,
    }
}

/// udisks2's busy volume.
pub fn processes(pids: &[u32]) -> ProcessesRequest {
    let all = [
        (4242, "Files", Some("strata")),
        (5120, "bash", Some("bash")),
        (5133, "vim", Some("vim notes.md")),
    ];
    ProcessesRequest {
        message: "Volume is busy\nOne or more applications are keeping the volume busy.".into(),
        processes: all
            .into_iter()
            .filter(|(pid, ..)| pids.contains(pid))
            .map(|(pid, name, command)| BlockingProcess {
                pid,
                name: name.into(),
                command: command.map(String::from),
            })
            .collect(),
        choices: vec![Choice::new("Unmount Anyway"), Choice::new("Cancel")],
    }
}

const ALL_PIDS: [u32; 3] = [4242, 5120, 5133];

fn scene(g: &mut Gallery, dark: bool, request: MountRequest) {
    g.open("c26", dark);
    g.c26.show(request);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c26", dark)`, then fills in `g.c26`.
pub const SCENES: &[Scene] = &[
    ("c26-sign-in-light", |g| {
        scene(g, false, MountRequest::Password(sign_in()))
    }),
    ("c26-sign-in-dark", |g| {
        scene(g, true, MountRequest::Password(sign_in()))
    }),
    ("c26-sign-in-retry-light", |g| {
        scene(g, false, MountRequest::Password(sign_in_retry()))
    }),
    ("c26-sign-in-retry-dark", |g| {
        scene(g, true, MountRequest::Password(sign_in_retry()))
    }),
    ("c26-luks-light", |g| {
        scene(g, false, MountRequest::Password(luks()))
    }),
    ("c26-luks-dark", |g| {
        scene(g, true, MountRequest::Password(luks()))
    }),
    ("c26-host-key-light", |g| {
        scene(g, false, MountRequest::Question(host_key()))
    }),
    ("c26-host-key-dark", |g| {
        scene(g, true, MountRequest::Question(host_key()))
    }),
    ("c26-processes-light", |g| {
        scene(g, false, MountRequest::Processes(processes(&ALL_PIDS)))
    }),
    ("c26-processes-dark", |g| {
        scene(g, true, MountRequest::Processes(processes(&ALL_PIDS)))
    }),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c26-openers-light", |g| g.open("c26", false)),
    ("c26-openers-dark", |g| g.open("c26", true)),
    ("c26-question-light", |g| {
        scene(g, false, MountRequest::Question(question()))
    }),
    ("c26-question-dark", |g| {
        scene(g, true, MountRequest::Question(question()))
    }),
    ("c26-key-passphrase-light", |g| {
        scene(g, false, MountRequest::Password(key_passphrase()))
    }),
    ("c26-key-passphrase-dark", |g| {
        scene(g, true, MountRequest::Password(key_passphrase()))
    }),
    ("c26-anonymous-light", |g| anonymous(g, false)),
    ("c26-anonymous-dark", |g| anonymous(g, true)),
];

fn anonymous(g: &mut Gallery, dark: bool) {
    scene(g, dark, MountRequest::Password(sign_in()));
    if let Some(form) = g.c26.prompt.password_form_mut() {
        form.anonymous = true;
    }
}

/// What the status line says about an answer: never the password, only its length.
fn describe(request: Option<&MountRequest>, answer: &MountResponse) -> String {
    match answer {
        MountResponse::Password(c) => {
            let mut parts = Vec::new();
            if let Some(user) = &c.user {
                parts.push(format!("user {user}"));
            }
            if let Some(domain) = &c.domain {
                parts.push(format!("domain {domain}"));
            }
            if let Some(password) = &c.password {
                parts.push(format!("secret of {} characters", password.chars().count()));
            }
            parts.push(format!("remember {:?}", c.remember));
            format!("Connect: {}", parts.join(", "))
        }
        MountResponse::Anonymous => "Connect as Anonymous".into(),
        MountResponse::Choice(i) => {
            let choices = match request {
                Some(MountRequest::Question(q)) => &q.choices,
                Some(MountRequest::Processes(p)) => &p.choices,
                _ => return format!("Choice {i}"),
            };
            let label = choices.get(*i).map_or("", |c| c.label.as_str());
            format!("Choice {i}: {label}")
        }
        MountResponse::Abort => "Aborted".into(),
    }
}

fn answered(g: &mut Gallery, _ticket: u32, answer: MountResponse) {
    let d = &mut g.c26;
    d.status = describe(d.request.as_ref(), &answer);
    d.request = None;
}

/// End process: the backend sends the list again without the ended process.
fn end_process(g: &mut Gallery, pid: u32) {
    let d = &mut g.c26;
    let Some(MountRequest::Processes(p)) = &d.request else {
        return;
    };
    let left: Vec<u32> = p
        .processes
        .iter()
        .map(|p| p.pid)
        .filter(|&p| p != pid)
        .collect();
    d.status = format!("Ended {pid}");
    d.show(MountRequest::Processes(processes(&left)));
}

pub fn prompts() -> MountPrompts<Gallery, u32> {
    MountPrompts::new(
        "c26-mount",
        |g: &Gallery| &g.c26.prompt,
        |g: &mut Gallery| &mut g.c26.prompt,
        answered,
    )
    .on_end_process(end_process)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let kit = &look.kit;
    let mut layer = Vec::new();
    prompts()
        .reduced_motion(cx.is_reduced_motion())
        .build(look, g, &mut layer);
    let opener = |key: &str, label: &str, request: fn() -> MountRequest| {
        Button::new(label)
            .variant(ButtonVariant::Neutral)
            .on_press(move |g: &mut Gallery| g.c26.show(request()))
            .build(kit)
            .key(format!("c26-open-{key}"))
    };
    let openers = row()
        .flex_wrap()
        .gap(look.space.md)
        .child(opener("sign-in", "Sign in", || {
            MountRequest::Password(sign_in())
        }))
        .child(opener("retry", "Sign in again", || {
            MountRequest::Password(sign_in_retry())
        }))
        .child(opener("key", "Key passphrase", || {
            MountRequest::Password(key_passphrase())
        }))
        .child(opener("luks", "Unlock volume", || {
            MountRequest::Password(luks())
        }))
        .child(opener("host-key", "Host key", || {
            MountRequest::Question(host_key())
        }))
        .child(opener("question", "Question", || {
            MountRequest::Question(question())
        }))
        .child(opener("processes", "Busy volume", || {
            MountRequest::Processes(processes(&ALL_PIDS))
        }));
    let window = col()
        .w_full()
        .h_full()
        .gap(look.space.lg)
        .padding(look.space.xl)
        .fill(kit.c.bg_base)
        .child(
            text("Mount prompts")
                .font(look.ty.title)
                .color(kit.c.text_base),
        )
        .child(openers)
        .child(
            text(g.c26.status.clone())
                .key("c26-status")
                .font(kit.ty.ui)
                .color(kit.c.text_muted)
                .live_region(LiveRegion::Polite),
        );
    rune_kit::root(window, layer)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
pub fn poll(_g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    false
}
