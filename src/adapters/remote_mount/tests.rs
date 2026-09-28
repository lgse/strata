// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use super::*;
use crate::services::remote::{MountQuestionKind, RemoteFailure};

mod fixtures;

#[derive(Default)]
struct RecordingPrompter {
    passwords: RefCell<Vec<(PasswordRequest, PasswordReply)>>,
    questions: RefCell<Vec<(MountQuestion, QuestionReply)>>,
}

impl MountPrompter for RecordingPrompter {
    fn ask_password(&self, request: PasswordRequest, reply: PasswordReply) {
        self.passwords.borrow_mut().push((request, reply));
    }

    fn ask_question(&self, question: MountQuestion, reply: QuestionReply) {
        self.questions.borrow_mut().push((question, reply));
    }
}

struct Harness {
    prompter: Rc<RecordingPrompter>,
    session: MountSession,
    replies: Rc<RefCell<Vec<gio::MountOperationResult>>>,
}

impl Harness {
    fn new(supplied: Option<MountCredentials>) -> Self {
        let prompter = Rc::new(RecordingPrompter::default());
        let session = MountSession::new(gio::MountOperation::new(), supplied, prompter.clone());
        let replies = Rc::new(RefCell::new(Vec::new()));
        let recorded = replies.clone();
        session
            .operation()
            .connect_reply(move |_, result| recorded.borrow_mut().push(result));
        Self {
            prompter,
            session,
            replies,
        }
    }

    fn ask_password(&self) {
        self.session.operation().emit_by_name::<()>(
            "ask-password",
            &[
                &"Authentication Required\nEnter password for “alice” on “host”:",
                &"alice",
                &"",
                &gio::AskPasswordFlags::NEED_PASSWORD,
            ],
        );
    }

    fn ask_question(&self, message: &str, choices: &[&str]) {
        self.session
            .operation()
            .emit_by_name::<()>("ask-question", &[&message, &glib::StrV::from(choices)]);
    }

    fn show_processes(&self) {
        let processes =
            glib::Value::from_type(glib::Type::from_name("GArray").expect("GArray is registered"));
        let values = [
            self.session.operation().to_value(),
            "Volume is busy".to_value(),
            processes,
            glib::StrV::from(["Unmount Anyway", "Cancel"].as_slice()).to_value(),
        ];
        self.session
            .operation()
            .emit_by_name_with_values("show-processes", &values[1..]);
    }

    fn replies(&self) -> Vec<gio::MountOperationResult> {
        self.replies.borrow().clone()
    }

    fn take_password(&self) -> (PasswordRequest, PasswordReply) {
        self.prompter.passwords.borrow_mut().remove(0)
    }

    fn take_question(&self) -> (MountQuestion, QuestionReply) {
        self.prompter.questions.borrow_mut().remove(0)
    }
}

fn rejected() -> Result<(), glib::Error> {
    Err(glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Host key verification failed",
    ))
}

#[test]
fn supplied_credentials_answer_once_and_a_second_prompt_is_a_retry() {
    let supplied = MountCredentials::password("alice", "from-uri");
    let harness = Harness::new(Some(supplied.clone()));

    harness.ask_password();
    assert!(harness.prompter.passwords.borrow().is_empty());
    assert_eq!(harness.replies(), [gio::MountOperationResult::Handled]);
    assert_eq!(
        harness.session.operation().password().as_deref(),
        Some("from-uri")
    );

    harness.ask_password();
    let (request, reply) = harness.take_password();
    assert!(request.retry, "the supplied password was rejected");
    let corrected = MountCredentials::password("alice", "corrected");
    reply.submit(corrected.clone());
    assert_eq!(
        harness.replies(),
        [
            gio::MountOperationResult::Handled,
            gio::MountOperationResult::Handled
        ]
    );
    assert_eq!(harness.session.attempted_credentials(), Some(corrected));
    assert!(!harness.session.declined());
}

#[test]
fn the_first_interactive_prompt_is_not_a_retry() {
    let harness = Harness::new(None);
    harness.ask_password();
    let (request, reply) = harness.take_password();
    assert!(!request.retry);
    assert!(!request.is_passphrase());
    assert_eq!(harness.session.last_password_request(), Some(request));
    reply.submit(MountCredentials::password("alice", "pw"));
    harness.ask_password();
    assert!(harness.take_password().0.retry);
}

#[test]
fn cancelled_or_abandoned_prompts_abort_and_resolve_as_cancelled() {
    let harness = Harness::new(None);
    harness.ask_password();
    harness.take_password().1.cancel();
    assert_eq!(harness.replies(), [gio::MountOperationResult::Aborted]);
    assert_eq!(
        harness.session.resolve(
            &Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                "Login dialog cancelled"
            )),
            RemoteErrorContext::Mount
        ),
        MountResolution::Cancelled
    );

    let abandoned = Harness::new(None);
    abandoned.ask_question(
        "Identity Verification Failed\nkey",
        &["Log In Anyway", "Cancel Login"],
    );
    drop(abandoned.take_question());
    assert_eq!(abandoned.replies(), [gio::MountOperationResult::Aborted]);
    assert!(abandoned.session.declined());
}

#[test]
fn trust_questions_require_an_explicit_choice() {
    let harness = Harness::new(None);
    harness.ask_question(
        "Identity Verification Failed\nVerifying the identity of “host” failed.",
        &["Log In Anyway", "Cancel Login"],
    );
    assert!(
        harness.replies().is_empty(),
        "nothing is answered by default"
    );
    let (question, reply) = harness.take_question();
    assert_eq!(question.kind, MountQuestionKind::HostIdentity);
    assert_eq!(question.choices, ["Log In Anyway", "Cancel Login"]);
    reply.choose(1);
    assert_eq!(harness.session.operation().choice(), 1);
    assert_eq!(harness.replies(), [gio::MountOperationResult::Handled]);
    assert_eq!(
        harness
            .session
            .resolve(&rejected(), RemoteErrorContext::Mount),
        MountResolution::Cancelled,
        "declining the key is the user's decision, not an error"
    );

    let accepted = Harness::new(None);
    accepted.ask_question(
        "Identity Verification Failed\nThe certificate has expired.",
        &["Yes", "No"],
    );
    let (question, reply) = accepted.take_question();
    assert_eq!(question.kind, MountQuestionKind::Certificate);
    reply.choose(0);
    assert_eq!(accepted.session.operation().choice(), 0);
    assert!(!accepted.session.declined());
    assert_eq!(
        accepted
            .session
            .resolve(&rejected(), RemoteErrorContext::Mount),
        MountResolution::Failed(RemoteFailure::HostKeyRejected)
    );
}

#[test]
fn repeated_busy_notices_share_one_open_prompt() {
    let harness = Harness::new(None);
    harness.show_processes();
    harness.show_processes();
    assert_eq!(harness.prompter.questions.borrow().len(), 1);
    let (question, reply) = harness.take_question();
    assert_eq!(question.kind, MountQuestionKind::Busy);
    assert!(question.is_risky_choice(0));
    reply.cancel();
    assert_eq!(harness.replies(), [gio::MountOperationResult::Aborted]);

    harness.show_processes();
    assert_eq!(
        harness.prompter.questions.borrow().len(),
        1,
        "a later notice may ask again once the first prompt is answered"
    );
}
