// SPDX-License-Identifier: MIT

//! Opt-in integration tests against disposable OpenSSH, FTP/FTPS, and
//! WebDAV/DAVS servers. `scripts/remote-fixtures.py test` starts the servers,
//! isolates GVfs on a private session bus, and runs these ignored tests.

use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc, time::Duration};

use gtk::gio::prelude::*;
use serde::Deserialize;

use super::super::*;
use crate::services::remote::{
    MountQuestionKind, RemoteFailure, RemoteProtocol, load_failure_message, plaintext_destination,
};

#[derive(Deserialize)]
struct Fixtures {
    sftp: SftpFixture,
    ftp: Endpoint,
    ftps: Endpoint,
    dav: Endpoint,
    davs: Endpoint,
}

#[derive(Deserialize)]
struct SftpFixture {
    host: String,
    port: u16,
    user: String,
    password: String,
    key_user: String,
    passphrase_user: String,
    passphrase: String,
    known_hosts: PathBuf,
    path: String,
    closed_port: u16,
    /// A valid host key that the server doesn't hold.
    impostor_key: String,
}

#[derive(Deserialize)]
struct Endpoint {
    host: String,
    port: u16,
    user: String,
    password: String,
    path: String,
    entry: String,
}

fn fixtures() -> Fixtures {
    let path = std::env::var_os("STRATA_REMOTE_FIXTURES").expect(
        "STRATA_REMOTE_FIXTURES is unset; run these through scripts/remote-fixtures.py test",
    );
    let contents = std::fs::read(&path).expect("fixture description should be readable");
    serde_json::from_slice(&contents).expect("fixture description should parse")
}

#[derive(Clone, Debug)]
enum Answer {
    Credentials(&'static str, String),
    Choose(usize),
    Cancel,
}

#[derive(Debug, Eq, PartialEq)]
enum Asked {
    Password { retry: bool, passphrase: bool },
    Question(MountQuestionKind),
}

#[derive(Default)]
struct ScriptedPrompter {
    answers: RefCell<VecDeque<Answer>>,
    asked: RefCell<Vec<Asked>>,
}

impl ScriptedPrompter {
    fn new(answers: impl IntoIterator<Item = Answer>) -> Rc<Self> {
        Rc::new(Self {
            answers: RefCell::new(answers.into_iter().collect()),
            asked: RefCell::default(),
        })
    }

    fn next(&self) -> Answer {
        self.answers
            .borrow_mut()
            .pop_front()
            .unwrap_or(Answer::Cancel)
    }
}

impl MountPrompter for ScriptedPrompter {
    fn ask_password(&self, request: PasswordRequest, reply: PasswordReply) {
        self.asked.borrow_mut().push(Asked::Password {
            retry: request.retry,
            passphrase: request.is_passphrase(),
        });
        match self.next() {
            Answer::Credentials(user, password) => {
                let user = if user.is_empty() {
                    request.default_user.as_str()
                } else {
                    user
                };
                reply.submit(MountCredentials::password(user, &password));
            }
            Answer::Choose(_) | Answer::Cancel => reply.cancel(),
        }
    }

    fn ask_question(&self, question: MountQuestion, reply: QuestionReply) {
        self.asked.borrow_mut().push(Asked::Question(question.kind));
        match self.next() {
            Answer::Choose(index) => reply.choose(index),
            Answer::Credentials(..) | Answer::Cancel => reply.cancel(),
        }
    }
}

/// Runs on the global default context, which GVfs's asynchronous client uses
/// for part of its D-Bus plumbing regardless of the thread-default context.
fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    glib::MainContext::default().block_on(async {
        glib::future_with_timeout(Duration::from_secs(60), future)
            .await
            .expect("remote fixture operation timed out")
    })
}

struct Attempt {
    resolution: MountResolution,
    asked: Vec<Asked>,
}

fn mount(uri: &str, answers: impl IntoIterator<Item = Answer>) -> Attempt {
    let prompter = ScriptedPrompter::new(answers);
    let session = MountSession::new(gio::MountOperation::new(), None, prompter.clone());
    let location = Location::uri(uri);
    let result = run(session.mount(&location, MountStrategy::EnclosingVolume));
    let resolution = session.resolve(&result, RemoteErrorContext::Mount);
    Attempt {
        resolution,
        asked: std::mem::take(&mut *prompter.asked.borrow_mut()),
    }
}

fn list(uri: &str) -> Result<Vec<String>, glib::Error> {
    run(async {
        let directory = gio::File::for_uri(uri);
        let enumerator = directory
            .enumerate_children_future(
                "standard::name",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await?;
        let mut names = Vec::new();
        loop {
            let infos = enumerator
                .next_files_future(64, glib::Priority::DEFAULT)
                .await?;
            if infos.is_empty() {
                break;
            }
            names.extend(
                infos
                    .iter()
                    .map(|info| info.name().to_string_lossy().into_owned()),
            );
        }
        // Dropping an open daemon enumerator closes it synchronously and stalls
        // this context's next GVfs request.
        enumerator.close_future(glib::Priority::DEFAULT).await?;
        Ok(names)
    })
}

fn unmount(uri: &str) {
    let session = MountSession::new(gio::MountOperation::new(), None, ScriptedPrompter::new([]));
    run(async {
        let mount = gio::File::for_uri(uri)
            .find_enclosing_mount(None::<&gio::Cancellable>)
            .expect("mounted fixture");
        session.unmount(&mount).await.expect("unmount fixture");
    });
}

fn sftp_uri(fixture: &SftpFixture, user: &str) -> String {
    format!(
        "sftp://{user}@{}:{}{}",
        fixture.host, fixture.port, fixture.path
    )
}

fn forget_host_keys(fixture: &SftpFixture) {
    let _ = std::fs::remove_file(&fixture.known_hosts);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn sftp_unknown_host_keys_need_a_decision_and_declining_cancels() {
    let fixture = fixtures().sftp;
    forget_host_keys(&fixture);
    let attempt = mount(&sftp_uri(&fixture, &fixture.user), [Answer::Choose(1)]);
    assert_eq!(
        attempt.asked,
        [Asked::Question(MountQuestionKind::HostIdentity)]
    );
    assert_eq!(attempt.resolution, MountResolution::Cancelled);
    assert!(
        !fixture.known_hosts.exists(),
        "a declined key must not be trusted"
    );

    let attempt = mount(&sftp_uri(&fixture, &fixture.user), [Answer::Cancel]);
    assert_eq!(attempt.resolution, MountResolution::Cancelled);
    assert!(!fixture.known_hosts.exists());
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn sftp_password_sign_in_retries_after_a_wrong_password_and_browses() {
    let fixture = fixtures().sftp;
    forget_host_keys(&fixture);
    let uri = sftp_uri(&fixture, &fixture.user);
    let attempt = mount(
        &uri,
        [
            Answer::Choose(0),
            Answer::Credentials("", "wrong-password".into()),
            Answer::Credentials("", fixture.password.clone()),
        ],
    );
    assert_eq!(attempt.resolution, MountResolution::Succeeded);
    assert_eq!(
        attempt.asked,
        [
            Asked::Question(MountQuestionKind::HostIdentity),
            Asked::Password {
                retry: false,
                passphrase: false
            },
            Asked::Password {
                retry: true,
                passphrase: false
            },
        ]
    );
    assert!(
        fixture.known_hosts.exists(),
        "an accepted key is remembered"
    );
    let names = list(&uri).expect("browse the mounted share");
    assert!(names.contains(&"hello.txt".to_owned()), "{names:?}");
    assert!(names.contains(&"nested".to_owned()), "{names:?}");
    let nested = list(&format!("{uri}/nested")).expect("descend into a folder");
    assert!(nested.is_empty(), "{nested:?}");

    unmount(&uri);
    let error = list(&uri).expect_err("a disconnected mount can't be listed");
    assert_eq!(
        load_failure_message(&Location::uri(&uri), &error),
        RemoteFailure::Disconnected.guidance(None)
    );
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn sftp_cancelling_sign_in_is_quiet() {
    let fixture = fixtures().sftp;
    forget_host_keys(&fixture);
    let attempt = mount(
        &sftp_uri(&fixture, &fixture.user),
        [Answer::Choose(0), Answer::Cancel],
    );
    assert_eq!(attempt.resolution, MountResolution::Cancelled);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn sftp_key_authentication_needs_no_password() {
    let fixture = fixtures().sftp;
    forget_host_keys(&fixture);
    let uri = sftp_uri(&fixture, &fixture.key_user);
    let attempt = mount(&uri, [Answer::Choose(0)]);
    assert_eq!(attempt.resolution, MountResolution::Succeeded);
    assert_eq!(
        attempt.asked,
        [Asked::Question(MountQuestionKind::HostIdentity)]
    );
    assert!(list(&uri).is_ok());
    unmount(&uri);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn sftp_encrypted_keys_ask_for_their_passphrase() {
    let fixture = fixtures().sftp;
    forget_host_keys(&fixture);
    let uri = sftp_uri(&fixture, &fixture.passphrase_user);
    let attempt = mount(
        &uri,
        [
            Answer::Choose(0),
            Answer::Credentials("", fixture.passphrase.clone()),
        ],
    );
    assert_eq!(attempt.resolution, MountResolution::Succeeded);
    assert!(
        attempt.asked.contains(&Asked::Password {
            retry: false,
            passphrase: true
        }),
        "{:?}",
        attempt.asked
    );
    unmount(&uri);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn sftp_changed_host_keys_are_refused_without_an_override() {
    let fixture = fixtures().sftp;
    let host = if fixture.port == 22 {
        fixture.host.clone()
    } else {
        format!("[{}]:{}", fixture.host, fixture.port)
    };
    std::fs::write(
        &fixture.known_hosts,
        format!("{host} {}\n", fixture.impostor_key),
    )
    .expect("write a stale host key");
    let attempt = mount(&sftp_uri(&fixture, &fixture.user), []);
    assert_eq!(
        attempt.resolution,
        MountResolution::Failed(RemoteFailure::HostKeyRejected)
    );
    assert!(attempt.asked.is_empty(), "{:?}", attempt.asked);
    forget_host_keys(&fixture);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn unreachable_servers_map_to_actionable_failures() {
    let fixture = fixtures().sftp;
    let refused = mount(
        &format!("sftp://{}:{}/", fixture.host, fixture.closed_port),
        [],
    );
    assert_eq!(
        refused.resolution,
        MountResolution::Failed(RemoteFailure::ConnectionRefused)
    );
    let missing = mount("sftp://strata-fixture-host.invalid/", []);
    assert_eq!(
        missing.resolution,
        MountResolution::Failed(RemoteFailure::HostNotFound)
    );
}

fn endpoint_uri(protocol: RemoteProtocol, endpoint: &Endpoint) -> String {
    format!(
        "{}://{}@{}:{}{}",
        protocol.scheme(),
        endpoint.user,
        endpoint.host,
        endpoint.port,
        endpoint.path
    )
}

fn assert_signs_in_and_browses(protocol: RemoteProtocol, endpoint: &Endpoint, trust: bool) {
    let uri = endpoint_uri(protocol, endpoint);
    let mut answers = Vec::new();
    if trust {
        answers.push(Answer::Choose(0));
    }
    answers.push(Answer::Credentials("", "wrong-password".into()));
    answers.push(Answer::Credentials("", endpoint.password.clone()));
    let attempt = mount(&uri, answers);
    assert_eq!(attempt.resolution, MountResolution::Succeeded, "{uri}");
    let retried = attempt
        .asked
        .iter()
        .any(|asked| matches!(asked, Asked::Password { retry: true, .. }));
    assert!(retried, "{:?}", attempt.asked);
    let names = list(&uri).expect("browse the mounted fixture");
    assert!(names.contains(&endpoint.entry), "{names:?}");
    unmount(&uri);
}

fn assert_untrusted_certificate_is_a_question(protocol: RemoteProtocol, endpoint: &Endpoint) {
    let uri = endpoint_uri(protocol, endpoint);
    let attempt = mount(&uri, [Answer::Choose(1)]);
    assert_eq!(
        attempt.asked,
        [Asked::Question(MountQuestionKind::Certificate)],
        "{uri}"
    );
    assert_eq!(attempt.resolution, MountResolution::Cancelled);
    let attempt = mount(&uri, [Answer::Cancel]);
    assert_eq!(attempt.resolution, MountResolution::Cancelled);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn ftp_is_plaintext_and_signs_in_with_a_retry() {
    let fixture = fixtures().ftp;
    let uri = endpoint_uri(RemoteProtocol::Ftp, &fixture);
    assert!(plaintext_destination(&Location::uri(&uri)).is_some());
    assert_signs_in_and_browses(RemoteProtocol::Ftp, &fixture, false);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn ftp_bad_credentials_can_be_abandoned() {
    let fixture = fixtures().ftp;
    let attempt = mount(
        &endpoint_uri(RemoteProtocol::Ftp, &fixture),
        [
            Answer::Credentials("", "wrong-password".into()),
            Answer::Cancel,
        ],
    );
    assert_eq!(attempt.resolution, MountResolution::Cancelled);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn ftps_untrusted_certificates_need_a_decision() {
    let fixture = fixtures().ftps;
    assert!(
        plaintext_destination(&Location::uri(endpoint_uri(RemoteProtocol::Ftps, &fixture)))
            .is_none()
    );
    assert_untrusted_certificate_is_a_question(RemoteProtocol::Ftps, &fixture);
    assert_signs_in_and_browses(RemoteProtocol::Ftps, &fixture, true);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn webdav_is_plaintext_and_signs_in_with_a_retry() {
    let fixture = fixtures().dav;
    let uri = endpoint_uri(RemoteProtocol::Dav, &fixture);
    assert!(plaintext_destination(&Location::uri(&uri)).is_some());
    assert_signs_in_and_browses(RemoteProtocol::Dav, &fixture, false);
}

#[test]
#[ignore = "requires scripts/remote-fixtures.py"]
fn davs_untrusted_certificates_need_a_decision() {
    let fixture = fixtures().davs;
    assert_untrusted_certificate_is_a_question(RemoteProtocol::Davs, &fixture);
    assert_signs_in_and_browses(RemoteProtocol::Davs, &fixture, true);
}
