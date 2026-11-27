// SPDX-License-Identifier: MIT

use std::{ffi::OsString, os::unix::ffi::OsStringExt};

use super::{Command, is_browser_worker, is_preview_helper, parse};

fn arguments(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

#[test]
fn arguments_name_paths_the_portal_or_ask_for_help() {
    let open = |paths: &[&str]| Ok(Command::Open(arguments(paths)));
    for (list, expected) in [
        (&[][..], open(&[])),
        (&["/srv/files"], open(&["/srv/files"])),
        (&["--", "--help"], open(&["--help"])),
        (&["-"], open(&["-"])),
        (&["--help"], Ok(Command::Help)),
        (&["/srv/files", "-h"], Ok(Command::Help)),
        (&["--version"], Ok(Command::Version)),
        (&["-V"], Ok(Command::Version)),
        (&["--", "--version"], open(&["--version"])),
        (&["--qa"], Err("unknown option --qa".to_owned())),
        (&["--portal"], Ok(Command::Portal)),
        (&["--", "--portal"], open(&["--portal"])),
        (&["--portal", "--help"], Ok(Command::Help)),
        (
            &["--portal", "/srv/files"],
            Err("--portal cannot be combined with a directory".to_owned()),
        ),
        (
            &["/srv/files", "--portal"],
            Err("--portal cannot be combined with a directory".to_owned()),
        ),
        (
            &["--portal", "--portal"],
            Err("--portal cannot be combined with a directory".to_owned()),
        ),
        (
            &["/srv/files", "/srv/more"],
            open(&["/srv/files", "/srv/more"]),
        ),
        (
            &["/srv/files", "--", "-x", "--"],
            open(&["/srv/files", "-x", "--"]),
        ),
    ] {
        assert_eq!(parse(arguments(list)), expected, "{list:?}");
    }
}

#[test]
fn an_argument_that_is_not_utf8_is_a_path() {
    let name = OsString::from_vec(b"-caf\xe9".to_vec());

    assert_eq!(parse([name.clone()]), Ok(Command::Open(vec![name])));
}

#[test]
fn only_a_first_argument_makes_a_thumbnail_worker() {
    for (list, worker) in [
        (&["strata", "--browser-worker"][..], true),
        (&["strata", "--", "--browser-worker"], false),
        (&["strata", "/srv/files", "--browser-worker"], false),
        (&["strata"], false),
    ] {
        assert_eq!(is_browser_worker(&arguments(list)), worker, "{list:?}");
    }
    for (list, helper) in [
        (&["strata", "--preview-helper"][..], true),
        (&["strata", "--", "--preview-helper"], false),
        (&["strata", "/srv/files", "--preview-helper"], false),
        (&["strata"], false),
    ] {
        assert_eq!(is_preview_helper(&arguments(list)), helper, "{list:?}");
    }
}
