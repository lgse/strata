// SPDX-License-Identifier: MIT

use std::{
    io::{Read as _, Write as _},
    os::unix::net::UnixListener,
    path::Path,
};

use serde_json::json;

use super::{admit, headless, private};

fn output(name: &str, active: bool) -> serde_json::Value {
    json!({ "name": name, "active": active, "rect": { "x": 0, "y": 0, "width": 1440, "height": 900 } })
}

#[test]
fn only_a_sway_whose_every_output_is_headless_is_taken() {
    let private_sway = json!([output("HEADLESS-1", true)]);
    assert_eq!(headless(&private_sway), Ok((1440, 900)));

    for desktop in [
        json!([output("DP-1", true)]),
        json!([output("HEADLESS-1", true), output("eDP-1", false)]),
    ] {
        let refused = headless(&desktop).expect_err("a desktop's outputs");
        assert!(refused.contains("not a headless one"), "{refused}");
    }
    assert!(headless(&json!([])).is_err(), "no output");
}

#[test]
fn a_variable_into_the_desktops_runtime_directory_is_refused() {
    let session = |bus: &str| {
        [
            ("XDG_RUNTIME_DIR", Some("/tmp/strata-session-1".to_owned())),
            ("WAYLAND_DISPLAY", Some("wayland-1".to_owned())),
            ("DBUS_SESSION_BUS_ADDRESS", Some(bus.to_owned())),
        ]
    };
    assert_eq!(
        private(&session("unix:path=/tmp/strata-session-1/bus")),
        Ok(())
    );
    assert_eq!(private(&[("DBUS_SESSION_BUS_ADDRESS", None)]), Ok(()));

    let refused = private(&session("unix:path=/run/user/1000/bus")).expect_err("the desktop's bus");
    assert!(
        refused.starts_with("DBUS_SESSION_BUS_ADDRESS="),
        "{refused}"
    );
}

/// A sway IPC socket at `socket` that answers one request with `outputs`.
fn answering(socket: &Path, outputs: serde_json::Value) -> std::thread::JoinHandle<()> {
    let listener = UnixListener::bind(socket).expect("the socket");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("a client");
        let mut header = [0; 14];
        stream.read_exact(&mut header).expect("the request");
        let length = u32::from_ne_bytes(header[6..10].try_into().expect("four bytes"));
        let mut payload = vec![0; length as usize];
        stream.read_exact(&mut payload).expect("the payload");
        let reply = outputs.to_string();
        let mut message = b"i3-ipc".to_vec();
        message.extend(
            u32::try_from(reply.len())
                .expect("a short reply")
                .to_ne_bytes(),
        );
        message.extend(header[10..14].iter());
        message.extend(reply.as_bytes());
        stream.write_all(&message).expect("the reply");
    })
}

/// The whole admission, against a sway that answers as a desktop's and as a headless one.
#[test]
fn a_session_is_admitted_only_in_a_headless_sway() {
    let directory = tempfile::tempdir().expect("a directory");
    let socket = directory.path().join("sway-ipc.sock");
    let session = |name: &str| match name {
        "SWAYSOCK" => Some(socket.display().to_string()),
        "XDG_RUNTIME_DIR" => Some(directory.path().display().to_string()),
        _ => None,
    };

    let desktop = answering(&socket, json!([output("DP-1", true)]));
    let refused = admit(session).err().expect("a desktop's sway");
    desktop.join().expect("the desktop's sway");
    assert!(refused.contains("not a headless one"), "{refused}");

    std::fs::remove_file(&socket).expect("the old socket");
    let private_sway = answering(&socket, json!([output("HEADLESS-1", true)]));
    let admitted = admit(session).expect("a private headless sway");
    private_sway.join().expect("the private sway");
    assert_eq!(admitted.screen, (1440, 900));

    let no_sway = |name: &str| (name != "SWAYSOCK").then(|| "/tmp/strata".to_owned());
    assert!(admit(no_sway).is_err(), "no SWAYSOCK");
}
