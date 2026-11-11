use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};
use strata_kit::look::Look;

use super::state::submission;
use super::view::{Footer, footer, split};
use super::*;

/// A host holding one prompt, with `u32` tickets, and the answers it got.
#[derive(Default)]
struct Host {
    prompt: MountPromptState<u32>,
    answers: Vec<(u32, MountResponse)>,
}

fn view(s: &Host, _cx: &mut Cx<Host>) -> El {
    let look = Look::fallback(false);
    let mut layer = Vec::new();
    MountPrompts::new(
        "m",
        |s: &Host| &s.prompt,
        |s: &mut Host| &mut s.prompt,
        |s: &mut Host, ticket, answer| s.answers.push((ticket, answer)),
    )
    .build(&look, s, &mut layer);
    rune_kit::root(col(), layer)
}

fn mount(request: MountRequest) -> Runtime<Host> {
    let mut host = Host::default();
    assert!(host.prompt.open(request, 7).is_none());
    let mut rt = Runtime::new(host, view, (800.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(0.3);
    rt
}

fn press(rt: &mut Runtime<Host>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    // A key the dialog sends on lands on the next frame.
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
}

fn keys(rt: &Runtime<Host>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .collect()
}

fn focused(rt: &Runtime<Host>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn smb() -> PasswordRequest {
    PasswordRequest {
        message: "Password required for share public on nas".into(),
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

fn host_key() -> QuestionRequest {
    QuestionRequest {
        message: "Identity Verification Failed\nThe identity sent is “SHA256:abc”.".into(),
        choices: vec![Choice::new("Log In Anyway"), Choice::new("Cancel Login")],
        host_key: true,
    }
}

#[test]
fn a_password_prompt_shows_only_the_fields_and_choices_its_request_allows() {
    let parts = [
        "user",
        "domain",
        "secret",
        "reveal",
        "connect-as",
        "remember",
    ];
    let shown = |rt: &Runtime<Host>| -> Vec<&str> {
        let keys = keys(rt);
        parts
            .into_iter()
            .filter(|p| keys.contains(&format!("m-password-{p}")))
            .collect()
    };
    assert_eq!(shown(&mount(MountRequest::Password(smb()))), parts);
    let key_only = PasswordRequest {
        needs_password: true,
        secret: Secret::Passphrase,
        ..PasswordRequest::default()
    };
    assert_eq!(
        shown(&mount(MountRequest::Password(key_only))),
        ["secret", "reveal"]
    );
}

#[test]
fn typing_the_secret_and_enter_answer_with_the_credentials_asked_for() {
    let mut rt = mount(MountRequest::Password(smb()));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-password-secret"),
        "a default username sends focus to the password"
    );
    press(&mut rt, Key::Text("hunter2".into()));
    press(&mut rt, Key::Enter);
    let credentials = Credentials {
        user: Some("ann".into()),
        domain: Some("WORKGROUP".into()),
        password: Some("hunter2".into()),
        remember: Remember::Never,
    };
    assert_eq!(
        rt.state.answers,
        [(7, MountResponse::Password(credentials))]
    );
    assert!(!rt.state.prompt.is_open());
}

#[test]
fn the_prompt_opens_on_an_empty_username_else_the_secret_else_connect() {
    let empty_user = PasswordRequest {
        default_user: String::new(),
        ..smb()
    };
    let rt = mount(MountRequest::Password(empty_user));
    assert_eq!(focused(&rt).as_deref(), Some("m-password-user"));
    let user_only = PasswordRequest {
        needs_user: true,
        default_user: "ann".into(),
        ..PasswordRequest::default()
    };
    let rt = mount(MountRequest::Password(user_only));
    assert_eq!(focused(&rt).as_deref(), Some("m-password-primary"));
}

#[test]
fn a_submission_carries_only_what_was_asked_and_saving_only_when_allowed() {
    let request = PasswordRequest {
        needs_domain: false,
        saving_allowed: false,
        ..smb()
    };
    let mut state = MountPromptState::default();
    let _ = state.open(MountRequest::Password(request.clone()), 1);
    let form = state.password_form_mut().expect("a password prompt");
    form.domain.replace_text("TYPED");
    form.secret.replace_text("pw");
    form.remember = Remember::Permanently;
    let form = state.password_form().expect("a password prompt");
    assert_eq!(
        submission(&request, form),
        MountResponse::Password(Credentials {
            user: Some("ann".into()),
            domain: None,
            password: Some("pw".into()),
            remember: Remember::Never,
        })
    );

    let mut state = MountPromptState::default();
    let _ = state.open(MountRequest::Password(smb()), 1);
    let form = state.password_form_mut().expect("a password prompt");
    form.secret.replace_text("pw");
    form.anonymous = true;
    assert_eq!(state.submit(), Some((1, MountResponse::Anonymous)));
}

#[test]
fn choosing_anonymous_disables_the_fields_and_the_storage_choice() {
    let mut host = Host::default();
    let _ = host.prompt.open(MountRequest::Password(smb()), 7);
    let form = host.prompt.password_form_mut().expect("a password prompt");
    form.anonymous = true;
    let mut rt = Runtime::new(host, view, (800.0, 700.0), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(0.3);
    let focusable = |key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .is_some_and(|e| e.focusable)
    };
    for key in ["m-password-user", "m-password-domain", "m-password-secret"] {
        assert!(!focusable(key), "{key} is disabled");
    }
    assert!(focusable("m-password-primary"), "Connect still connects");
    let segments: Vec<bool> = rt
        .elements()
        .into_iter()
        .filter(|e| e.role == "radio" && e.path.contains("m-password-remember"))
        .map(|e| e.focusable)
        .collect();
    assert_eq!(segments, [false; 3], "the storage choice is disabled");
}

#[test]
fn escape_answers_abort_and_a_choice_answers_with_its_index_in_the_request() {
    let mut rt = mount(MountRequest::Password(smb()));
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.answers, [(7, MountResponse::Abort)]);

    let mut rt = mount(MountRequest::Question(host_key()));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("m-question-secondary"),
        "the declining choice has focus"
    );
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.answers, [(7, MountResponse::Choice(1))]);
}

#[test]
fn the_footer_keeps_each_choice_s_index_and_puts_declining_ones_first() {
    let choices = [
        Choice::new("Unmount Anyway"),
        Choice::new("Eject Anyway"),
        Choice::new("Cancel"),
        Choice::new("Do not eject"),
    ];
    assert_eq!(
        footer(&choices),
        Footer {
            primary: Some(0),
            secondary: Some(2),
            actions: vec![3, 1],
        }
    );
}

#[test]
fn a_process_list_over_a_process_list_updates_in_place() {
    let busy = |pids: &[u32]| {
        MountRequest::Processes(ProcessesRequest {
            message: "Volume is busy\nOne or more applications are keeping the volume busy.".into(),
            processes: pids
                .iter()
                .map(|&pid| BlockingProcess {
                    pid,
                    name: format!("app{pid}"),
                    command: None,
                })
                .collect(),
            choices: vec![Choice::new("Unmount Anyway"), Choice::new("Cancel")],
        })
    };
    let mut rt = mount(busy(&[10, 11]));
    let focus = focused(&rt);
    assert_eq!(focus.as_deref(), Some("m-processes-secondary"));
    let mut replaced = None;
    rt.update(|s, _| replaced = s.prompt.open(busy(&[11]), 8));
    assert_eq!(replaced, Some(7));
    rt.frame(1.0 / 60.0);
    rt.frame(1.0 / 60.0);
    let keys = keys(&rt);
    assert!(!keys.contains(&"m-processes-process-10".to_owned()));
    assert!(keys.contains(&"m-processes-process-11".to_owned()));
    assert_eq!(focused(&rt), focus, "focus stays on Cancel");
    assert!(rt.state.answers.is_empty());
}

#[test]
fn opening_over_a_prompt_hands_back_its_ticket_and_withdrawing_answers_nothing() {
    let mut state = MountPromptState::default();
    assert_eq!(state.open(MountRequest::Password(smb()), 1), None);
    assert_eq!(state.open(MountRequest::Question(host_key()), 2), Some(1));
    assert_eq!(state.ticket(), Some(&2));
    assert_eq!(state.withdraw(), Some(2));
    assert!(!state.is_open());
    assert_eq!(state.withdraw(), None);
}

#[test]
fn credentials_never_print_their_password() {
    let answer = MountResponse::Password(Credentials {
        user: Some("ann".into()),
        domain: None,
        password: Some("hunter2".into()),
        remember: Remember::ForSession,
    });
    let printed = format!("{answer:?}");
    assert!(!printed.contains("hunter2"), "{printed}");
    assert!(printed.contains("<redacted>"));
}

#[test]
fn the_retry_message_names_only_the_fields_shown() {
    assert_eq!(
        MountLabels::retry_message(&smb()),
        "Those credentials weren’t accepted. Check username, domain and password and try again."
    );
    let luks = PasswordRequest {
        needs_password: true,
        secret: Secret::guess("Enter a passphrase to unlock the volume"),
        ..PasswordRequest::default()
    };
    assert_eq!(
        MountLabels::retry_message(&luks),
        "Those credentials weren’t accepted. Check passphrase and try again."
    );
}

#[test]
fn a_backend_message_splits_into_a_title_and_a_body_at_its_first_line() {
    assert_eq!(
        split("Volume is busy\nOne or more applications\nkeep it busy.\n"),
        (
            "Volume is busy".into(),
            Some("One or more applications\nkeep it busy.".into())
        )
    );
    assert_eq!(split("Continue?"), ("Continue?".into(), None));
}

#[test]
fn declining_choices_are_told_by_their_leading_word() {
    for label in ["Cancel Login", "cancel", "Don't trust", "No", "Disconnect"] {
        assert!(looks_like_rejection(label), "{label}");
    }
    for label in ["Log In Anyway", "Nothing", "Unmount Anyway", "Notify"] {
        assert!(!looks_like_rejection(label), "{label}");
    }
}

#[test]
fn each_refusal_is_a_new_node_so_a_second_one_is_heard_too() {
    let refused = || {
        let request = smb();
        MountRequest::Password(PasswordRequest {
            error: Some(MountLabels::retry_message(&request)),
            ..request
        })
    };
    let mut rt = mount(refused());
    let errors = |rt: &Runtime<Host>| -> Vec<String> {
        keys(rt)
            .into_iter()
            .filter(|k| k.starts_with("m-password-error-"))
            .collect()
    };
    let first = errors(&rt);
    assert_eq!(first.len(), 1);
    rt.update(|s, _| {
        let _ = s.prompt.open(refused(), 8);
    });
    rt.frame(1.0 / 60.0);
    let second = errors(&rt);
    assert_eq!(second.len(), 1);
    assert_ne!(first, second);
}

#[test]
fn with_no_choice_marked_declining_a_question_opens_on_close_and_enter_aborts() {
    let german = [
        Choice::new("Trotzdem anmelden"),
        Choice::new("Anmeldung abbrechen"),
    ];
    for choices in [german.to_vec(), german.iter().rev().cloned().collect()] {
        let question = QuestionRequest {
            choices,
            ..host_key()
        };
        assert!(question.choices.iter().all(|c| !c.rejects));
        let mut rt = mount(MountRequest::Question(question));
        assert_eq!(focused(&rt).as_deref(), Some("m-question-close"));
        press(&mut rt, Key::Enter);
        assert_eq!(rt.state.answers, [(7, MountResponse::Abort)]);
    }
}

#[test]
fn an_edit_that_reaches_the_spare_is_wiped_once_it_lands() {
    let mut state = MountPromptState::<u32>::default();
    // A field's edit handler, run after its prompt has gone, then its change handler.
    state.edit_mut(PromptField::Secret).replace_text("hunter2");
    state.edited();
    assert_eq!(state.spare().text, "");

    let _ = state.open(MountRequest::Password(smb()), 1);
    state.edit_mut(PromptField::Secret).replace_text("hunter2");
    state.edited();
    let form = state.password_form().expect("a password prompt");
    assert_eq!(
        form.secret.text, "hunter2",
        "the form's own field keeps its text"
    );
}
