// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn install_fixture(replies: &[&[u8]], delay: Duration, timeout: Duration) -> (bool, Vec<String>) {
    let root = tempfile::tempdir().expect("temporary socket directory");
    let socket = root.path().join("placement.sock");
    let listener =
        Async::new(UnixListener::bind(&socket).expect("bind fixture")).expect("async listener");
    async_io::block_on(future::zip(
        install_centering_rule(&socket, timeout),
        async {
            let mut received = Vec::new();
            let commands = centering_commands();
            for (index, reply) in replies.iter().enumerate() {
                let (mut stream, _) = listener.accept().await.expect("accept command");
                let mut command = vec![0; commands[index].len()];
                stream.read_exact(&mut command).await.expect("read command");
                received.push(String::from_utf8(command).expect("UTF-8 command"));
                Timer::after(delay).await;
                let _ = stream.write_all(reply).await;
            }
            received
        },
    ))
}

#[test]
fn lua_centering_rule_is_named_and_matches_only_chooser_windows() {
    let (installed, received) = install_fixture(&[b"ok"], Duration::ZERO, Duration::from_secs(2));
    assert!(installed);
    assert_ne!(crate::portal::CHOOSER_APPLICATION_ID, crate::APPLICATION_ID);
    assert_eq!(
        received,
        vec![
            "/eval hl.window_rule({name='strata-file-chooser-center',match={class='^io[.]github[.]lgse[.]Strata[.]FileChooser$'},center=true})"
        ]
    );
    let (installed_again, repeated) =
        install_fixture(&[b"ok"], Duration::ZERO, Duration::from_secs(2));
    assert!(installed_again);
    assert_eq!(received, repeated);
}

#[test]
fn legacy_centering_probes_support_then_batches_the_filter_and_effect() {
    for unsupported in [
        b"unknown request".as_slice(),
        b"eval is only supported with the lua config manager",
    ] {
        let (installed, received) = install_fixture(
            &[unsupported, b"ok", b"ok\n\n\nok"],
            Duration::ZERO,
            Duration::from_secs(2),
        );
        assert!(installed);
        assert_eq!(received, centering_commands());
        assert_eq!(
            received[1],
            "/keyword windowrule[strata-file-chooser-center]:match:class ^io[.]github[.]lgse[.]Strata[.]FileChooser$"
        );
        assert_eq!(
            received[2],
            "[[BATCH]] keyword windowrule[strata-file-chooser-center]:match:class ^io[.]github[.]lgse[.]Strata[.]FileChooser$;keyword windowrule[strata-file-chooser-center]:center on"
        );
    }
}

#[test]
fn placement_errors_stop_unsupported_installation_and_report_failed_batches() {
    for replies in [
        vec![b"Lua syntax error".as_slice()],
        vec![b"unknown request", b"invalid field"],
        vec![b"unknown request", b"ok", b"invalid effect"],
        vec![b"unknown request", b"ok", b"invalid field\n\n\nok"],
        vec![b"unknown request", b"ok", b"ok\n\n\ninvalid effect"],
    ] {
        let (installed, received) =
            install_fixture(&replies, Duration::ZERO, Duration::from_secs(2));
        assert!(!installed);
        assert_eq!(received.len(), replies.len());
    }
}

#[test]
fn placement_ipc_has_a_total_deadline_and_reply_limit() {
    let (installed, _) = install_fixture(
        &[b"ok"],
        Duration::from_millis(60),
        Duration::from_millis(30),
    );
    assert!(!installed);
    let oversized = vec![b' '; 8193];
    let (installed, _) = install_fixture(&[&oversized], Duration::ZERO, Duration::from_secs(2));
    assert!(!installed);
    let root = tempfile::tempdir().expect("temporary directory");
    assert!(!async_io::block_on(install_centering_rule(
        &root.path().join("missing"),
        QUERY_TIMEOUT
    )));
}
