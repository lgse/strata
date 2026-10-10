// SPDX-License-Identifier: MIT

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::Ordering,
};

use super::{
    LinkPolicy, MAX_LINK_HOPS, NEXT_TEMP_FILE, atomic_write, atomic_write_config, atomic_write_with,
};

#[test]
fn write_failure_preserves_destination_and_removes_temporary_file() -> io::Result<()> {
    for (policy, linked) in [
        (LinkPolicy::Reject, false),
        (LinkPolicy::FollowUserOwned, true),
    ] {
        let directory = test_directory()?;
        let destination = directory.join("settings.toml");
        let target = if linked {
            fs::create_dir(directory.join("dotfiles"))?;
            let target = directory.join("dotfiles/settings.toml");
            symlink(&target, &destination)?;
            target
        } else {
            destination.clone()
        };
        fs::write(&target, b"valid")?;

        let result = atomic_write_with(&destination, policy, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        });

        assert!(result.is_err(), "{policy:?}");
        assert_eq!(fs::read(&target)?, b"valid", "{policy:?}");
        assert_eq!(fs::read_dir(&directory)?.count(), 1 + usize::from(linked));
        if linked {
            assert_eq!(fs::read_link(&destination)?, target);
            assert_eq!(fs::read_dir(directory.join("dotfiles"))?.count(), 1);
        }
        fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[test]
fn symlink_destination_is_rejected_without_touching_target() -> io::Result<()> {
    let directory = test_directory()?;
    let target = directory.join("target");
    let destination = directory.join("settings.toml");
    fs::write(&target, b"valid")?;
    symlink(&target, &destination)?;

    let result = atomic_write(&destination, b"replacement");

    assert!(result.is_err());
    assert_eq!(fs::read(&target)?, b"valid");
    assert!(fs::symlink_metadata(&destination)?.file_type().is_symlink());
    fs::remove_dir_all(directory)
}

#[test]
fn long_destination_name_writes_with_private_permissions() -> io::Result<()> {
    let directory = test_directory()?;
    let destination = directory.join(format!("{}.toml", "a".repeat(240)));

    atomic_write(&destination, b"valid")?;
    assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o600);

    fs::set_permissions(&destination, fs::Permissions::from_mode(0o644))?;
    atomic_write(&destination, b"replacement")?;
    assert_eq!(
        fs::metadata(&destination)?.mode() & 0o777,
        0o600,
        "strict writes do not keep an existing destination's mode"
    );
    fs::remove_dir_all(directory)
}

#[test]
fn non_regular_destination_is_rejected() -> io::Result<()> {
    let directory = test_directory()?;
    let destination = directory.join("settings.toml");
    fs::create_dir(&destination)?;

    assert!(atomic_write(&destination, b"replacement").is_err());
    assert!(destination.is_dir());
    fs::remove_dir_all(directory)
}

#[test]
fn config_write_follows_a_user_owned_symlink_and_keeps_the_link() -> io::Result<()> {
    let directory = test_directory()?;
    let dotfiles = directory.join("dotfiles");
    let config = directory.join("config");
    fs::create_dir(&dotfiles)?;
    fs::create_dir(&config)?;
    let target = dotfiles.join("settings.toml");
    let link = config.join("settings.toml");
    fs::write(&target, b"valid")?;
    symlink(&target, &link)?;

    atomic_write_config(&link, b"replacement")?;

    assert_eq!(fs::read(&target)?, b"replacement");
    assert!(fs::symlink_metadata(&link)?.file_type().is_symlink());
    assert_eq!(fs::read_link(&link)?, target);
    assert_eq!(fs::read_dir(&dotfiles)?.count(), 1);
    assert_eq!(fs::read_dir(&config)?.count(), 1);
    fs::remove_dir_all(directory)
}

#[test]
fn config_write_resolves_relative_and_chained_links() -> io::Result<()> {
    let directory = test_directory()?;
    for name in ["a", "b", "c"] {
        fs::create_dir(directory.join(name))?;
    }
    let target = directory.join("c/real.toml");
    let link = directory.join("a/settings.toml");
    let hop = directory.join("b/hop");
    fs::write(&target, b"valid")?;
    symlink("../c/real.toml", &hop)?;
    symlink("../b/hop", &link)?;

    atomic_write_config(&link, b"replacement")?;

    assert_eq!(fs::read(&target)?, b"replacement");
    assert_eq!(fs::read_link(&link)?, PathBuf::from("../b/hop"));
    assert_eq!(fs::read_link(&hop)?, PathBuf::from("../c/real.toml"));
    assert_eq!(fs::read_dir(directory.join("a"))?.count(), 1);
    assert_eq!(fs::read_dir(directory.join("b"))?.count(), 1);
    assert_eq!(fs::read_dir(directory.join("c"))?.count(), 1);
    fs::remove_dir_all(directory)
}

#[test]
fn config_write_rejects_unsafe_links() -> io::Result<()> {
    struct Case {
        name: &'static str,
        target: &'static str,
        kind: io::ErrorKind,
        message: Option<&'static str>,
    }
    let cases = [
        Case {
            name: "dangling",
            target: "missing.toml",
            kind: io::ErrorKind::NotFound,
            message: None,
        },
        Case {
            name: "directory_target",
            target: "folder",
            kind: io::ErrorKind::InvalidInput,
            message: Some("non-regular"),
        },
        Case {
            name: "loop",
            target: "other",
            kind: io::ErrorKind::InvalidInput,
            message: Some("too many levels"),
        },
    ];
    let mut failures = Vec::new();
    for case in cases {
        let directory = test_directory()?;
        let link = directory.join("settings.toml");
        match case.name {
            "directory_target" => fs::create_dir(directory.join("folder"))?,
            "loop" => symlink("settings.toml", directory.join("other"))?,
            _ => {}
        }
        symlink(case.target, &link)?;
        let entries = fs::read_dir(&directory)?.count();

        match atomic_write_config(&link, b"replacement") {
            Ok(()) => failures.push(format!("{}: write succeeded", case.name)),
            Err(error) => {
                if error.kind() != case.kind {
                    failures.push(format!(
                        "{}: expected {:?}, got {:?} ({error})",
                        case.name,
                        case.kind,
                        error.kind()
                    ));
                }
                if let Some(message) = case.message
                    && !error.to_string().contains(message)
                {
                    failures.push(format!(
                        "{}: message {error:?} lacks {message:?}",
                        case.name
                    ));
                }
                let shown = crate::services::io_error_message(&error);
                if case.name == "dangling"
                    && shown
                        != format!(
                            "The symlink “{}” points to a missing target “{}”",
                            link.display(),
                            directory.join("missing.toml").display()
                        )
                {
                    failures.push(format!("{}: shown as {shown:?}", case.name));
                }
            }
        }
        assert_eq!(
            fs::read_link(&link)?,
            PathBuf::from(case.target),
            "{}",
            case.name
        );
        assert_eq!(fs::read_dir(&directory)?.count(), entries, "{}", case.name);
        if case.name == "dangling" {
            assert!(!directory.join("missing.toml").exists());
        }
        if case.name == "directory_target" {
            assert_eq!(fs::read_dir(directory.join("folder"))?.count(), 0);
        }
        fs::remove_dir_all(directory)?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

#[test]
fn config_write_keeps_the_destination_mode() -> io::Result<()> {
    #[derive(Debug, Clone, Copy)]
    enum Kind {
        Regular,
        Linked,
        Missing,
    }
    let cases = [
        (Kind::Regular, Some(0o644)),
        (Kind::Regular, Some(0o600)),
        (Kind::Linked, Some(0o644)),
        (Kind::Linked, Some(0o600)),
        (Kind::Missing, None),
    ];
    let mut failures = Vec::new();
    for (kind, mode) in cases {
        let directory = test_directory()?;
        let destination = directory.join("settings.toml");
        let resolved = match kind {
            Kind::Linked => directory.join("target.toml"),
            Kind::Regular | Kind::Missing => destination.clone(),
        };
        if let Some(mode) = mode {
            fs::write(&resolved, b"valid")?;
            fs::set_permissions(&resolved, fs::Permissions::from_mode(mode))?;
        }
        if let Kind::Linked = kind {
            symlink(&resolved, &destination)?;
        }
        let expected = mode.unwrap_or(0o600);

        match atomic_write_config(&destination, b"replacement") {
            Err(error) => failures.push(format!("{kind:?} {expected:o}: {error}")),
            Ok(()) => {
                let actual = fs::metadata(&resolved)?.mode() & 0o777;
                if actual != expected {
                    failures.push(format!("{kind:?} {expected:o}: ended {actual:o}"));
                }
                assert_eq!(fs::read(&resolved)?, b"replacement", "{kind:?}");
            }
        }
        fs::remove_dir_all(directory)?;
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}

#[test]
fn config_write_follows_at_most_the_link_hop_limit() -> io::Result<()> {
    for (hops, accepted) in [(MAX_LINK_HOPS, true), (MAX_LINK_HOPS + 1, false)] {
        let directory = test_directory()?;
        let target = directory.join("real.toml");
        fs::write(&target, b"valid")?;
        for hop in (0..hops).rev() {
            let next = if hop + 1 == hops {
                PathBuf::from("real.toml")
            } else {
                PathBuf::from(format!("link-{}", hop + 1))
            };
            symlink(next, directory.join(format!("link-{hop}")))?;
        }

        let result = atomic_write_config(&directory.join("link-0"), b"replacement");

        if accepted {
            assert!(result.is_ok(), "{hops} hops: {result:?}");
            assert_eq!(fs::read(&target)?, b"replacement");
        } else {
            let error = result.expect_err("one hop too many is refused");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("too many levels"), "{error}");
            assert_eq!(fs::read(&target)?, b"valid");
        }
        assert_eq!(fs::read_dir(&directory)?.count(), hops + 1);
        fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[test]
fn config_write_refuses_links_and_targets_owned_by_another_user() -> io::Result<()> {
    if rustix::process::geteuid().is_root() {
        eprintln!("skipping: root owns the system entries this test uses as foreign files");
        return Ok(());
    }
    // procfs entries are root-owned and cannot be replaced, even if the check failed.
    for (target, refused) in [("/proc/version", "file"), ("/proc/self", "symlink")] {
        let directory = test_directory()?;
        let link = directory.join("settings.toml");
        symlink(target, &link)?;

        let error =
            atomic_write_config(&link, b"replacement").expect_err("foreign ownership is refused");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{target}");
        assert!(
            error
                .to_string()
                .contains(&format!("{refused} {target} owned by another user")),
            "{error}"
        );
        assert_eq!(fs::read_dir(&directory)?.count(), 1, "{target}");
        fs::remove_dir_all(directory)?;
    }
    Ok(())
}

#[test]
fn config_write_errors_name_the_symlink_target() -> io::Result<()> {
    if rustix::process::geteuid().is_root() {
        eprintln!("skipping: root can write into a read-only directory");
        return Ok(());
    }
    let directory = test_directory()?;
    let dotfiles = directory.join("dotfiles");
    fs::create_dir(&dotfiles)?;
    let target = dotfiles.join("settings.toml");
    let link = directory.join("settings.toml");
    fs::write(&target, b"valid")?;
    symlink(&target, &link)?;
    fs::set_permissions(&dotfiles, fs::Permissions::from_mode(0o555))?;

    let result = atomic_write_config(&link, b"replacement");

    fs::set_permissions(&dotfiles, fs::Permissions::from_mode(0o755))?;
    let error = result.expect_err("the target directory is read-only");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        error
            .to_string()
            .starts_with(&format!("symlink target {}: ", target.display())),
        "{error}"
    );
    assert_eq!(fs::read(&target)?, b"valid");
    fs::remove_dir_all(directory)
}

fn test_directory() -> io::Result<PathBuf> {
    loop {
        let path = std::env::temp_dir().join(format!(
            "strata-storage-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}
