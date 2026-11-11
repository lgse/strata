//! Unit tests: no bus. A test that claims or forwards needs a private session bus, so those
//! are session tests (`tests/session/p8.rs`, `mise run test-platform p8::`); one here would
//! reach the desktop's own bus through `$XDG_RUNTIME_DIR/bus`.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::unbounded_channel;
use zbus::OwnedVariant;

use super::event::{CommandLine, Event, Out, RevealMethod};
use super::forward::primary_gone;
use super::interfaces::{FileManager1, Pack, action};
use super::launch::{Activation, Launch, PlatformData};
use super::primary::Inbox;
use super::{AppId, Reveal};

#[test]
fn app_ids_follow_gapplication_and_map_to_its_object_path() {
    let path = |id: &str| AppId::new(id).map(|a| a.object_path());
    assert_eq!(
        path("io.github.lgse.Strata").as_deref(),
        Ok("/io/github/lgse/Strata")
    );
    assert_eq!(
        path("org.example.my-app").as_deref(),
        Ok("/org/example/my_app")
    );
    assert_eq!(path("org._1x.App_2").as_deref(), Ok("/org/_1x/App_2"));
    let long = format!("org.{}", "a".repeat(252));
    for bad in [
        "",
        "single",
        ".org.example",
        "org..example",
        "org.example.",
        "org.1example",
        "org.exa mple",
        "org.exämple",
        long.as_str(),
    ] {
        assert!(AppId::new(bad).is_err(), "{bad:?} was accepted");
    }
}

fn launch() -> Launch {
    Launch {
        args: vec![
            OsString::from("/usr/bin/demo"),
            OsString::from("folder with spaces"),
            OsString::from_vec(vec![b'n', 0xff, 0xfe, b'.', b't', b'x', b't']),
            OsString::new(),
        ],
        cwd: Some(PathBuf::from(OsString::from_vec(b"/tmp/caf\xe9".to_vec()))),
        activation: Activation {
            xdg_activation_token: Some("wayland-token".into()),
            desktop_startup_id: Some("x11-id_TIME42".into()),
        },
    }
}

#[test]
fn a_launch_survives_the_wire_with_non_utf8_bytes() {
    let sent = launch();
    let wire = sent.wire_args();
    // GLib reads a byte string without its NUL as empty.
    assert!(wire.iter().all(|a| a.last() == Some(&0)));
    assert_eq!(Launch::from_wire(wire, &sent.platform_data()), sent);
}

#[test]
fn a_launch_from_a_caller_without_nuls_or_platform_data_still_reads() {
    let got = Launch::from_wire(vec![b"demo".to_vec(), b"a".to_vec()], &PlatformData::new());
    assert_eq!(got.args, vec![OsString::from("demo"), OsString::from("a")]);
    assert_eq!(got.cwd, None);
    assert!(got.activation.is_empty());
}

#[test]
fn mistyped_or_empty_platform_data_is_ignored() {
    let mut data = PlatformData::new();
    data.insert("activation-token".into(), OwnedVariant::from(7u32));
    data.insert(
        "desktop-startup-id".into(),
        OwnedVariant::from(zbus::Str::from("")),
    );
    data.insert("cwd".into(), OwnedVariant::from(true));
    let got = Launch::from_wire(Vec::new(), &data);
    assert!(got.activation.is_empty());
    assert_eq!(got.cwd, None);
}

#[test]
fn the_token_follows_the_windowing_system_then_falls_back() {
    let both = launch().activation;
    assert_eq!(both.token_for(true), Some("wayland-token"));
    assert_eq!(both.token_for(false), Some("x11-id_TIME42"));
    let only_x11 = Activation {
        xdg_activation_token: None,
        desktop_startup_id: Some("x11".into()),
    };
    assert_eq!(only_x11.token_for(true), Some("x11"));
    assert_eq!(Activation::default().token_for(false), None);
}

fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Out>) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(out) = rx.try_recv() {
        seen.push(match out {
            Out::Print(t) => format!("out {t}"),
            Out::PrintErr(t) => format!("err {t}"),
            Out::Exit(s) => format!("exit {s}"),
        });
    }
    seen
}

#[test]
fn a_forwarded_launch_hears_its_lines_then_one_exit_status() {
    let (tx, mut rx) = unbounded_channel();
    let mut cmd = CommandLine::remote(launch(), tx.clone());
    assert!(cmd.is_remote());
    cmd.print("opened\n");
    cmd.print_err("usage: demo FILE\n");
    cmd.set_exit_status(2);
    drop(cmd);
    assert_eq!(
        drain(&mut rx),
        ["out opened\n", "err usage: demo FILE\n", "exit 2"]
    );

    CommandLine::remote(launch(), tx.clone()).exit(3);
    drop(CommandLine::remote(launch(), tx));
    assert_eq!(drain(&mut rx), ["exit 3", "exit 0"]);
}

#[test]
fn a_launch_whose_answer_nobody_waits_for_does_not_panic() {
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let cmd = CommandLine::remote(launch(), tx);
    cmd.print("nobody reads this\n");
    cmd.exit(1);
}

#[test]
fn the_first_launch_leaves_its_own_status_for_main() {
    let status = Arc::new(AtomicI32::new(0));
    let cmd = CommandLine::local(launch(), status.clone());
    assert!(!cmd.is_remote());
    cmd.exit(4);
    assert_eq!(status.load(Ordering::Relaxed), 4);
}

fn tag(event: &Event) -> String {
    match event {
        Event::CommandLine(c) => format!("launch {:?}", c.arguments()),
        Event::Activate(_) => "activate".into(),
        Event::Open { uris, .. } => format!("open {uris:?}"),
        Event::Action { name, .. } => format!("action {name}"),
        Event::Reveal(r) => format!("reveal {:?} {:?}", r.method, r.uris),
    }
}

#[test]
fn events_before_the_app_attaches_arrive_first_and_in_order() {
    let inbox = Inbox::default();
    inbox.deliver(Event::Activate(Activation::default()));
    inbox.deliver(Event::Open {
        uris: vec!["file:///a".into()],
        hint: String::new(),
        activation: Activation::default(),
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    inbox.attach(Box::new(move |e| {
        sink.lock().unwrap_or_else(|p| p.into_inner()).push(tag(&e))
    }));
    inbox.deliver(Event::Action {
        name: "quit".into(),
        parameter: None,
        activation: Activation::default(),
    });
    let seen = seen.lock().unwrap_or_else(|p| p.into_inner()).clone();
    assert_eq!(seen, ["activate", "open [\"file:///a\"]", "action quit"]);
}

#[test]
fn file_manager1_calls_arrive_as_reveals_in_the_callers_order() {
    let inbox = Arc::new(Inbox::default());
    let fm = FileManager1 {
        inbox: inbox.clone(),
    };
    let uris = || vec!["file:///b/2".to_owned(), "file:///a/1".to_owned()];
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        fm.show_folders(uris(), String::new()).await;
        fm.show_items(uris(), "token".into()).await;
        fm.show_item_properties(uris(), String::new()).await;
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    inbox.attach(Box::new(move |e| {
        if let Event::Reveal(r) = e {
            sink.lock().unwrap_or_else(|p| p.into_inner()).push(r);
        }
    }));
    let reveal = |method, startup_id: &str| Reveal {
        method,
        uris: uris(),
        startup_id: startup_id.into(),
    };
    assert_eq!(
        *seen.lock().unwrap_or_else(|p| p.into_inner()),
        [
            reveal(RevealMethod::Folders, ""),
            reveal(RevealMethod::Items, "token"),
            reveal(RevealMethod::ItemProperties, ""),
        ]
    );
}

#[test]
fn a_launch_tries_again_only_when_the_primary_left() {
    let gone = [
        ("org.freedesktop.DBus.Error.ServiceUnknown", ""),
        ("org.freedesktop.DBus.Error.NameHasNoOwner", ""),
        ("org.freedesktop.DBus.Error.UnknownObject", ""),
        (
            "org.freedesktop.DBus.Error.NoReply",
            "Message recipient disconnected from message bus without replying",
        ),
        // dbus-broker's wording.
        (
            "org.freedesktop.DBus.Error.NoReply",
            "Remote peer disconnected",
        ),
    ];
    for (name, text) in gone {
        assert!(primary_gone(name, text), "{name}");
    }
    // The primary may still act on a call that only timed out, or that it refused.
    let stay = [
        (
            "org.freedesktop.DBus.Error.NoReply",
            "Did not receive a reply",
        ),
        ("org.freedesktop.DBus.Error.NotSupported", ""),
        ("org.freedesktop.DBus.Error.Failed", "disconnected"),
    ];
    for (name, text) in stay {
        assert!(!primary_gone(name, text), "{name}: {text}");
    }
}

#[test]
fn actions_are_read_as_glib_reads_them() {
    let accepted = ["go".to_owned()];
    let data = PlatformData::new();
    let unknown = action(&accepted, "quit".into(), Vec::new(), &data, Pack::Tuple);
    assert!(
        matches!(&unknown, Err(zbus::fdo::Error::InvalidArgs(m)) if m.contains("quit")),
        "{unknown:?}"
    );
    let parameter =
        |values: Vec<OwnedVariant>, pack| match action(&accepted, "go".into(), values, &data, pack)
        {
            Ok(Event::Action { parameter, .. }) => {
                parameter.map(|p| p.value_signature().to_string())
            }
            other => panic!("{other:?}"),
        };
    let s = || OwnedVariant::from(zbus::Str::from("s".to_owned()));
    let two = || vec![s(), OwnedVariant::from(7u32)];
    assert_eq!(parameter(Vec::new(), Pack::Tuple), None);
    assert_eq!(parameter(vec![s()], Pack::Tuple).as_deref(), Some("s"));
    // org.freedesktop.Application packs several into one tuple, in order; org.gtk.Actions
    // takes the first.
    assert_eq!(parameter(two(), Pack::Tuple).as_deref(), Some("(su)"));
    assert_eq!(parameter(two(), Pack::First).as_deref(), Some("s"));
}
