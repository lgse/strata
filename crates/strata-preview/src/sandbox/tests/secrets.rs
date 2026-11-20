// SPDX-License-Identifier: MIT

use std::fs;

use crate::sandbox::{ParseOperation, stage_secret_anon};

#[test]
fn staged_secrets_have_no_directory_entry() {
    use std::os::fd::AsRawFd;

    let secret = stage_secret_anon(b"s3cret").expect("stage secret");
    let number = secret.as_raw_fd();

    let link =
        std::fs::read_link(format!("/proc/self/fd/{number}")).expect("inspect secret descriptor");
    assert!(
        link.to_string_lossy().ends_with(" (deleted)"),
        "secret descriptor must resolve to an unlinked inode, got {link:?}"
    );
    let stat = rustix::fs::fstat(&secret).expect("stat secret");
    assert_eq!(
        stat.st_nlink, 0,
        "anonymous secret inode must have no links"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(format!("/proc/self/fd/{number}"))
            .expect("stat secret path")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "anonymous secret inode must stay mode-0600"
    );

    let round_trip = std::fs::read(format!("/proc/self/fd/{number}")).expect("read secret");
    assert_eq!(round_trip, b"s3cret");
}

#[test]
fn staged_secrets_are_inherited_only_when_explicitly_mapped_to_stdin() {
    use std::os::fd::AsRawFd;
    use std::process::{Command, Stdio};

    let secret = stage_secret_anon(b"synthetic archive password").expect("stage secret");
    let path = format!("/proc/self/fd/{}", secret.as_raw_fd());
    let unrelated = Command::new("/bin/cat")
        .arg(&path)
        .output()
        .expect("spawn unrelated child");
    assert!(!unrelated.status.success());
    assert!(unrelated.stdout.is_empty());

    let intended = Command::new("/bin/cat")
        .arg("/proc/self/fd/0")
        .stdin(Stdio::from(fs::File::from(secret)))
        .output()
        .expect("spawn intended child");
    assert!(intended.status.success());
    assert_eq!(intended.stdout, b"synthetic archive password");
}

#[test]
fn archive_operation_debug_redacts_the_password() {
    let operation = ParseOperation::ArchiveList {
        format: crate::model::ArchiveFormat::Zip,
        password: Some(crate::services::SecretString::new(
            "strata-security-repro-password".to_owned(),
        )),
    };
    let rendered = format!("{operation:?}");
    assert!(
        !rendered.contains("strata-security-repro-password"),
        "Debug must never expose the password, got {rendered:?}"
    );
    assert!(
        rendered.contains("Some([REDACTED])"),
        "a present password must render redacted-but-present, got {rendered:?}"
    );
    let operation = ParseOperation::ArchiveList {
        format: crate::model::ArchiveFormat::Zip,
        password: None,
    };
    let rendered = format!("{operation:?}");
    assert!(
        rendered.contains("password: None"),
        "an absent password must stay distinguishable, got {rendered:?}"
    );
}
