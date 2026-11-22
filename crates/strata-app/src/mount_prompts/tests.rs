// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gio::prelude::*;
use rune::{Key, Mods};
use serde_json::Value;
use strata_components::mount_prompts::{
    Choice, MountRequest, MountResponse, PasswordRequest, QuestionRequest, Remember, Secret,
};
use strata_kit::icons::IconName;

use super::{
    PromptTicket, ReleaseOverlaySpec, next_id, open_prompt,
    operation::{
        BoundOperation, MountCredentials, MountPromptDetails, OperationOptions,
        authentication_retry_message,
    },
    show_forget_password, show_release_overlay, show_release_safe_to_remove, show_unlock_progress,
    withdraw_prompt,
};
use crate::{
    shell,
    test_support::{
        PATIENCE, hosted,
        view::{all, idle, press, text},
    },
};

const MODULE: &str = "mount_prompts::tests";

fn keyed<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    all(tree, &|node| node["key"] == key).into_iter().next()
}

fn click_key(host: &mut crate::host::offscreen::Offscreen, key: &str) {
    let elements = host.runtime().elements();
    let target = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("element {key} not mounted"));
    let [x, y, w, h] = target.visible;
    let (x, y) = (x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

#[test]
fn retry_message_names_only_the_requested_fields_and_distinguishes_passphrase() {
    let luks = authentication_retry_message(
        gio::AskPasswordFlags::NEED_PASSWORD | gio::AskPasswordFlags::SAVING_SUPPORTED,
        "Enter a passphrase to unlock the volume",
        Secret::Passphrase,
    );
    assert!(luks.contains("passphrase"), "LUKS retry: {luks}");
    assert!(!luks.contains("username"), "LUKS retry: {luks}");

    let smb = authentication_retry_message(
        gio::AskPasswordFlags::NEED_USERNAME
            | gio::AskPasswordFlags::NEED_DOMAIN
            | gio::AskPasswordFlags::NEED_PASSWORD,
        "Enter password for share",
        Secret::Password,
    );
    assert!(smb.contains("username"), "SMB retry: {smb}");
    assert!(smb.contains("domain"), "SMB retry: {smb}");
    assert!(smb.contains("password"), "SMB retry: {smb}");

    let empty = authentication_retry_message(gio::AskPasswordFlags::empty(), "", Secret::Password);
    assert_eq!(empty, "That attempt wasn’t accepted. Try again.");
}

#[test]
fn luks_passphrase_prompt_submits_secret_and_storage_choice_to_gio_operation() {
    hosted(
        MODULE,
        "luks_passphrase_prompt_submits_secret_and_storage_choice_to_gio_operation",
        |mut host| {
            let later = shell::window_later(host.app());
            let submitted_hook = Rc::new(Cell::new(false));
            let prompt_opened = Rc::new(Cell::new(false));
            let bound = BoundOperation::new(
                later,
                OperationOptions {
                    encrypted: true,
                    supplied: None,
                    on_prompt_open: Some({
                        let flag = Rc::clone(&prompt_opened);
                        Rc::new(move || flag.set(true))
                    }),
                    on_submitted: Some({
                        let flag = Rc::clone(&submitted_hook);
                        Rc::new(move || flag.set(true))
                    }),
                    on_unmount_progress: None,
                },
            );
            type ReplyRecord = (gio::MountOperationResult, Option<String>, gio::PasswordSave);
            let replied: Rc<RefCell<Option<ReplyRecord>>> = Rc::default();
            let reply_sink = Rc::clone(&replied);
            bound.operation().connect_reply(move |op, result| {
                reply_sink.replace(Some((
                    result,
                    op.password().map(String::from),
                    op.password_save(),
                )));
            });

            let flags =
                gio::AskPasswordFlags::NEED_PASSWORD | gio::AskPasswordFlags::SAVING_SUPPORTED;
            bound.operation().emit_by_name::<()>(
                "ask-password",
                &[
                    &"Enter a passphrase to unlock 31 GB Encrypted",
                    &"",
                    &"",
                    &flags,
                ],
            );

            let opened = host.run_until(PATIENCE, |host| {
                keyed(&host.tree(), "mount-password-dialog").is_some()
            });
            assert!(opened, "passphrase prompt should open");
            assert!(prompt_opened.get());

            let tree = host.tree();
            let dialog = keyed(&tree, "mount-password-dialog").expect("dialog");
            assert_eq!(text(dialog, "name"), "Passphrase required");

            host.runtime().update(|app, _| {
                let form = app
                    .mount_prompts
                    .prompt
                    .password_form_mut()
                    .expect("password form");
                form.remember = Remember::ForSession;
            });
            host.turn();
            press(&mut host, Key::Text("top-secret".into()), Mods::default());
            press(&mut host, Key::Enter, Mods::default());

            let answered = host.run_until(PATIENCE, |_| replied.borrow().is_some());
            assert!(answered, "GIO operation should receive reply");
            assert!(submitted_hook.get());
            assert_eq!(
                replied.borrow().as_ref(),
                Some(&(
                    gio::MountOperationResult::Handled,
                    Some("top-secret".to_owned()),
                    gio::PasswordSave::ForSession,
                ))
            );
            let outcome = bound.finish(Ok(()));
            assert_eq!(
                outcome.attempted.as_ref().map(|c| c.password.as_str()),
                Some("top-secret")
            );
            assert!(outcome.details.is_some());
        },
    );
}

#[test]
fn escape_aborts_passphrase_prompt_and_aborted_signal_withdraws_without_reply() {
    hosted(
        MODULE,
        "escape_aborts_passphrase_prompt_and_aborted_signal_withdraws_without_reply",
        |mut host| {
            // Part 1: Escape replies Aborted.
            let answer = Rc::new(RefCell::new(None));
            let sink = Rc::clone(&answer);
            let id = next_id();
            let ticket = PromptTicket::new(id, Rc::new(Cell::new(false)), move |r| {
                sink.replace(Some(r));
            });
            let request = MountRequest::Password(PasswordRequest {
                message: "Enter passphrase".into(),
                needs_password: true,
                secret: Secret::Passphrase,
                ..PasswordRequest::default()
            });
            host.runtime().update(|app, _| {
                open_prompt(app, request, ticket);
            });
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "mount-password-dialog").is_some()
            }));
            press(&mut host, Key::Escape, Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "mount-password-dialog").is_none()
            }));
            assert_eq!(answer.borrow().as_ref(), Some(&MountResponse::Abort));

            // Part 2: Withdrawal closes without calling reply.
            let called = Rc::new(Cell::new(false));
            let called_flag = Rc::clone(&called);
            let id2 = next_id();
            let ticket2 = PromptTicket::new(id2, Rc::new(Cell::new(false)), move |_| {
                called_flag.set(true);
            });
            let q = MountRequest::Question(QuestionRequest {
                message: "Continue?".into(),
                choices: vec![Choice::new("Continue"), Choice::new("Cancel")],
                host_key: false,
            });
            host.runtime().update(|app, _| open_prompt(app, q, ticket2));
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "mount-question-dialog").is_some()
            }));
            host.runtime().update(|app, _| withdraw_prompt(app, id2));
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "mount-question-dialog").is_none()
            }));
            assert!(!called.get(), "withdrawn prompt must not reply");
        },
    );
}

#[test]
fn forget_saved_password_dialog_confirms_and_cancels_cleanly() {
    hosted(
        MODULE,
        "forget_saved_password_dialog_confirms_and_cancels_cleanly",
        |mut host| {
            let confirmed = Rc::new(Cell::new(false));
            let cancelled = Rc::new(Cell::new(false));
            let (c1, k1) = (Rc::clone(&confirmed), Rc::clone(&cancelled));
            host.runtime().update(|app, _| {
                show_forget_password(
                    app,
                    "Vault".into(),
                    move || c1.set(true),
                    move || k1.set(true),
                );
            });
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "forget-saved-password-dialog").is_some()
            }));
            click_key(&mut host, "forget-saved-password-primary");
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "forget-saved-password-dialog").is_none()
            }));
            assert!(confirmed.get());
            assert!(!cancelled.get());

            let cancelled2 = Rc::new(Cell::new(false));
            let k2 = Rc::clone(&cancelled2);
            host.runtime().update(|app, _| {
                show_forget_password(app, "Vault".into(), || {}, move || k2.set(true));
            });
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "forget-saved-password-dialog").is_some()
            }));
            press(&mut host, Key::Escape, Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "forget-saved-password-dialog").is_none()
            }));
            assert!(cancelled2.get());
        },
    );
}

#[test]
fn release_overlay_transitions_to_safe_to_remove_and_unlock_progress_hides() {
    hosted(
        MODULE,
        "release_overlay_transitions_to_safe_to_remove_and_unlock_progress_hides",
        |mut host| {
            let release_id = next_id();
            host.runtime().update(|app, _| {
                show_release_overlay(
                    app,
                    ReleaseOverlaySpec {
                        id: release_id,
                        title: "Ejecting".into(),
                        subtitle: "Backup".into(),
                        body: "Writing data to the drive.".into(),
                        icon: IconName::Eject,
                    },
                    || {},
                );
            });
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "device-release-dialog").is_some()
            }));
            host.runtime().update(|app, _| {
                assert!(show_release_safe_to_remove(app, release_id, "Backup"));
            });
            idle(&mut host);
            let tree = host.tree();
            let dialog = keyed(&tree, "device-release-dialog").expect("dialog");
            assert_eq!(text(dialog, "name"), "Safe to remove");
            click_key(&mut host, "device-release-primary");
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "device-release-dialog").is_none()
            }));

            let hidden = Rc::new(Cell::new(false));
            let flag = Rc::clone(&hidden);
            let unlock_id = next_id();
            host.runtime().update(|app, _| {
                show_unlock_progress(app, unlock_id, "Vault".into(), move || flag.set(true));
            });
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "unlock-progress-dialog").is_some()
            }));
            idle(&mut host);
            click_key(&mut host, "unlock-progress-primary");
            assert!(host.run_until(PATIENCE, |h| {
                keyed(&h.tree(), "unlock-progress-dialog").is_none()
            }));
            assert!(hidden.get());
        },
    );
}

#[test]
fn mount_prompt_details_preserves_previous_user_and_domain_on_retry() {
    let details = MountPromptDetails {
        message: "Password required".into(),
        default_user: "guest".into(),
        default_domain: "OLD".into(),
        flags: gio::AskPasswordFlags::NEED_USERNAME
            | gio::AskPasswordFlags::NEED_DOMAIN
            | gio::AskPasswordFlags::NEED_PASSWORD,
        encrypted: false,
    };
    let prev = MountCredentials {
        anonymous: false,
        username: "alice".into(),
        domain: "CORP".into(),
        password: "wrong".into(),
        save: gio::PasswordSave::Never,
    };
    let request = details.password_request(true, Some(&prev));
    assert_eq!(request.default_user, "alice");
    assert_eq!(request.default_domain, "CORP");
    assert!(request.error.is_some());
}
