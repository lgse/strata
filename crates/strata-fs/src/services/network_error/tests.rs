// SPDX-License-Identifier: MIT

use std::io;

use super::NetworkError;

#[test]
fn network_failures_never_show_the_errno_suffix() {
    for (error, expected) in [
        (
            ureq::Error::Io(io::Error::from_raw_os_error(libc::ECONNREFUSED)),
            "The connection was refused",
        ),
        (
            ureq::Error::Io(io::Error::from_raw_os_error(libc::ENETUNREACH)),
            "The network is unreachable",
        ),
        (
            ureq::Error::Io(io::Error::other(
                "failed to lookup address information: Name or service not known",
            )),
            "Could not find the server",
        ),
        (ureq::Error::HostNotFound, "Could not find the server"),
        (ureq::Error::StatusCode(404), "The server returned HTTP 404"),
    ] {
        assert_eq!(NetworkError::from_ureq(&error).message(), expected);
    }
}

#[test]
fn a_ureq_error_wrapped_by_a_body_reader_is_unwrapped() {
    let error = ureq::Error::ConnectionFailed.into_io();
    assert_eq!(
        NetworkError::from_io(&error).message(),
        "Could not connect to the server"
    );
}

#[test]
fn unknown_system_text_drops_the_errno_suffix() {
    let error = io::Error::from_raw_os_error(libc::EPROTO);
    let NetworkError::Detail { text } = NetworkError::from_io(&error) else {
        panic!("an unmapped error should keep its system text");
    };
    assert!(!text.contains("os error"), "{text}");
}
