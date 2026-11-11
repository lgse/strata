// SPDX-License-Identifier: MIT

use std::{ffi::OsString, os::unix::ffi::OsStringExt};

use super::{Command, parse};

fn arguments(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

#[test]
fn arguments_name_one_directory_or_ask_for_help() {
    let open = |directory: &str| Ok(Command::Open(Some(directory.into())));
    for (list, expected) in [
        (&[][..], Ok(Command::Open(None))),
        (&["/srv/files"], open("/srv/files")),
        (&["--", "--help"], open("--help")),
        (&["-"], open("-")),
        (&["--help"], Ok(Command::Help)),
        (&["/srv/files", "-h"], Ok(Command::Help)),
        (&["--qa"], Err("unknown option --qa".to_owned())),
        (
            &["/srv/files", "/srv/more"],
            Err("only one directory can be opened".to_owned()),
        ),
    ] {
        assert_eq!(parse(arguments(list)), expected, "{list:?}");
    }
}

#[test]
fn an_argument_that_is_not_utf8_is_a_directory() {
    let name = OsString::from_vec(b"-caf\xe9".to_vec());

    assert_eq!(parse([name.clone()]), Ok(Command::Open(Some(name))));
}
