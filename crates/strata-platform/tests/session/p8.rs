//! P8's session tests: launches of one app id meeting on a private session bus.
//!
//! The bus tests run `single-instance-probe` (the shim without Rune) on a bus alone; the window
//! tests run `single-instance-demo` under headless sway, on Wayland and X11. The interop tests
//! (with `glib-bridge`) put a real GLib `GApplication` (`gapplication-peer`) on either side.
//!
//! Every process gets the session's environment (`Session::command`), whose
//! `DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR` are the session's own: [`command`] checks
//! that before each launch. The tests' own D-Bus client connects to `s.bus().address()` and
//! never to `Connection::session()`, which with no address set would reach the desktop's bus
//! through `$XDG_RUNTIME_DIR/bus`.
//!
//! Feature map: `browser/navigation/startup-arguments` ("Each argument opens its own window …
//! when Strata is already running", "A path whose name is not valid UTF-8 opens normally",
//! "Launching with stdout or stderr connected to a closed pipe still opens the window"),
//! `devices/volumes/udiskie-unlock` ("With Strata already running, either usage error prints to
//! the invoking terminal's stderr … and the command exits with status 1"), and
//! `integration/file-manager-interface` (the export, `InvalidArgs` and `UnknownMethod`, not
//! taking the name from another owner, D-Bus activation). What a window opens and selects is
//! Strata's.
//!
//! Work package: P8, `docs/work/phase-1-leaves/P8-single-instance.md`.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener;
use std::pin::Pin;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use strata_platform::single_instance::zbus::export::futures_core::Stream;
use strata_platform::single_instance::zbus::{
    self, Connection, Message, MessageStream, OwnedVariant, connection,
};

use crate::support::{Display, Options, Proc, Session, example, timeout};

const PROBE: &str = "single-instance-probe";
const PROBE_ID: &str = "org.example.SingleInstanceProbe";
const PROBE_PATH: &str = "/org/example/SingleInstanceProbe";
const DEMO: &str = "single-instance-demo";

fn bus_only() -> Session {
    Session::with(Options {
        compositor: false,
        ..Options::default()
    })
}

/// `program` in the session's environment, checked to name the session's own bus.
fn command(s: &Session, program: &str, display: Display) -> Command {
    let cmd = s.command(example(program), display);
    let env: HashMap<&OsStr, Option<&OsStr>> = cmd.get_envs().collect();
    let address = env
        .get(OsStr::new("DBUS_SESSION_BUS_ADDRESS"))
        .copied()
        .flatten();
    assert_eq!(
        address,
        Some(OsStr::new(s.bus().address())),
        "a launch must name the session's bus"
    );
    assert_eq!(
        env.get(OsStr::new("XDG_RUNTIME_DIR")).copied().flatten(),
        Some(s.runtime_dir().as_os_str()),
        "a launch must keep the session's runtime directory"
    );
    cmd
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs `cmd` to its end within the session timeout, killing it after, with its output.
fn run(mut cmd: Command) -> Output {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawning a launch");
    let deadline = Instant::now() + timeout();
    while child.try_wait().expect("waiting for a launch").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("a launch's output")
}

fn claim_line(p: &Proc) -> Value {
    p.wait_json("its claim", |v| v["ev"] == "claim")
}

fn start_primary(s: &Session, name: &str, cmd: Command) -> Proc {
    let p = s.spawn(name, cmd);
    assert_eq!(claim_line(&p)["result"], "primary");
    p
}

/// The tests' own client, on the session's bus only.
fn client(s: &Session) -> (tokio::runtime::Runtime, Connection) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let conn = rt
        .block_on(connection::Builder::address(s.bus().address()).build())
        .expect("connecting to the session's bus");
    (rt, conn)
}

fn call<B>(
    rt: &tokio::runtime::Runtime,
    conn: &Connection,
    dest: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> zbus::Result<zbus::Message>
where
    B: serde::Serialize + zbus::DynamicType,
{
    rt.block_on(async {
        tokio::time::timeout(
            timeout(),
            conn.call_method(Some(dest), path, Some(interface), method, body),
        )
        .await
        .unwrap_or(Err(zbus::Error::Failure("timed out".into())))
    })
}

fn platform(token: &str) -> HashMap<String, OwnedVariant> {
    HashMap::from([(
        "activation-token".to_owned(),
        OwnedVariant::from(zbus::Str::from(token.to_owned())),
    )])
}

fn error_name(result: zbus::Result<zbus::Message>) -> String {
    match result {
        Err(zbus::Error::MethodError(name, _, _)) => name.to_string(),
        Err(zbus::Error::FDO(e)) => format!("{e:?}"),
        other => panic!("expected a D-Bus error, got {other:?}"),
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn second_launch_forwards_argv_cwd_and_token() {
    let s = bus_only();
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.args(["a", "b"]);
    let primary = start_primary(&s, "primary", cmd);
    let own = primary.wait_json("its own launch", |v| v["ev"] == "launch");
    assert_eq!(own["remote"], false);
    assert_eq!(own["args"][1], hex(b"a"));

    let cwd = s.runtime_dir().join("home");
    let odd = OsStr::from_bytes(b"n\xff\xfe.txt");
    let mut second = command(&s, PROBE, Display::None);
    second
        .arg("c")
        .arg(odd)
        .current_dir(&cwd)
        .env("XDG_ACTIVATION_TOKEN", "token-1")
        .env("DESKTOP_STARTUP_ID", "startup-1");
    let out = run(second);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let got = primary.wait_json("the forwarded launch", |v| {
        v["ev"] == "launch" && v["remote"] == true
    });
    let args: Vec<&str> = got["args"]
        .as_array()
        .expect("args")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(&args[1..], [hex(b"c"), hex(odd.as_bytes())]);
    assert_eq!(got["cwd"], hex(cwd.as_os_str().as_bytes()));
    assert_eq!(got["xdg"], "token-1");
    assert_eq!(got["sid"], "startup-1");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn simultaneous_launches_end_with_one_primary() {
    let s = bus_only();
    for round in 0..10 {
        let launches: Vec<Proc> = (0..8)
            .map(|i| {
                let mut cmd = command(&s, PROBE, Display::None);
                cmd.arg(format!("{round}-{i}")).env("PROBE_GATE", "1");
                s.spawn(&format!("launch-{round}-{i}"), cmd)
            })
            .collect();
        for l in &launches {
            l.send_line("go");
        }
        let claims: Vec<Value> = launches.iter().map(claim_line).collect();
        let primaries: Vec<usize> = (0..8)
            .filter(|i| claims[*i]["result"] == "primary")
            .collect();
        assert_eq!(primaries.len(), 1, "round {round}: {claims:?}");
        assert!(
            claims
                .iter()
                .all(|c| c["result"] == "primary"
                    || (c["result"] == "forwarded" && c["status"] == 0)),
            "round {round}: {claims:?}"
        );
        let primary = &launches[primaries[0]];
        let deadline = Instant::now() + timeout();
        loop {
            let remote = primary
                .lines()
                .iter()
                .filter(|l| l.contains("\"ev\":\"launch\"") && l.contains("\"remote\":true"))
                .count();
            if remote == 7 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "round {round}: {remote} of 7 forwarded launches"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        primary.send_line("quit");
        drop(launches);
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn exit_status_stdout_and_stderr_come_back() {
    let s = bus_only();
    let _primary = start_primary(&s, "primary", command(&s, PROBE, Display::None));

    let mut ok = command(&s, PROBE, Display::None);
    ok.arg("--print");
    let out = run(ok);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("hello from the primary\n"),
        "{out:?}"
    );

    let mut bad = command(&s, PROBE, Display::None);
    bad.args(["--print", "--usage-error"]);
    let out = run(bad);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("hello from the primary\n"));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage: single-instance-probe"),
        "{out:?}"
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_launch_with_a_closed_stdout_still_forwards() {
    let s = bus_only();
    let primary = start_primary(&s, "primary", command(&s, PROBE, Display::None));
    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.args(["--print", "closed"])
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(Stdio::null());
    let mut child = cmd.spawn().expect("spawning a launch");
    let deadline = Instant::now() + timeout();
    let status = loop {
        if let Some(status) = child.try_wait().expect("waiting") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the launch with a closed stdout never ended");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
    primary.wait_json("the launch", |v| {
        v["ev"] == "launch" && v["args"][2] == hex(b"closed")
    });
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn without_a_bus_each_launch_runs_alone() {
    let s = bus_only();
    let nowhere = format!(
        "unix:path={}",
        s.runtime_dir().join("no-such-bus").display()
    );
    let launches: Vec<Proc> = (0..2)
        .map(|i| {
            let mut cmd = command(&s, PROBE, Display::None);
            cmd.env("DBUS_SESSION_BUS_ADDRESS", &nowhere);
            s.spawn(&format!("alone-{i}"), cmd)
        })
        .collect();
    for l in &launches {
        let claim = claim_line(l);
        assert_eq!(claim["result"], "standalone");
        assert!(
            claim["reason"]
                .as_str()
                .is_some_and(|r| r.starts_with("no-bus")),
            "{claim}"
        );
        // Its own launch still arrives.
        l.wait_json("its own launch", |v| {
            v["ev"] == "launch" && v["remote"] == false
        });
    }
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_bus_that_never_answers_times_out_to_running_alone() {
    let s = bus_only();
    let path = s.runtime_dir().join("silent-bus");
    let listener = UnixListener::bind(&path).expect("a silent socket");
    std::thread::spawn(move || {
        // Accepts and keeps every connection, and never says a word.
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.extend(stream);
        }
    });
    let started = Instant::now();
    let launches: Vec<Proc> = (0..2)
        .map(|i| {
            let mut cmd = command(&s, PROBE, Display::None);
            cmd.env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", path.display()),
            )
            .env("PROBE_BUS_TIMEOUT_MS", "1000");
            s.spawn(&format!("silent-{i}"), cmd)
        })
        .collect();
    for l in &launches {
        let claim = claim_line(l);
        assert_eq!(claim["result"], "standalone");
        assert_eq!(claim["reason"], "timeout");
    }
    assert!(started.elapsed() < timeout(), "{:?}", started.elapsed());
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_released_or_killed_primary_hands_over_to_the_next_launch() {
    let s = bus_only();
    let first = start_primary(&s, "first", command(&s, PROBE, Display::None));
    first.send_line("release");
    let released = first.wait_json("released", |v| v["ev"] == "released");
    assert_eq!(released["claimed"], false);
    let second = start_primary(&s, "second", command(&s, PROBE, Display::None));
    drop(first);

    let mut forwarded = command(&s, PROBE, Display::None);
    forwarded.arg("to-second");
    assert_eq!(run(forwarded).status.code(), Some(0));
    second.wait_json("the launch", |v| v["ev"] == "launch" && v["remote"] == true);

    // Killed outright: the bus drops its name with its connection.
    drop(second);
    let _third = start_primary(&s, "third", command(&s, PROBE, Display::None));
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn gapplication_and_freedesktop_methods_arrive_as_events() {
    let s = bus_only();
    let primary = start_primary(&s, "primary", command(&s, PROBE, Display::None));
    let (rt, conn) = client(&s);
    let fdo = "org.freedesktop.Application";
    call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        fdo,
        "Activate",
        &(platform("t1"),),
    )
    .expect("Activate");
    let a = primary.wait_json("activate", |v| v["ev"] == "activate");
    assert_eq!(a["xdg"], "t1");
    call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        fdo,
        "Open",
        &(vec!["file:///x"], platform("t2")),
    )
    .expect("Open");
    let o = primary.wait_json("open", |v| v["ev"] == "open");
    assert_eq!(o["uris"][0], "file:///x");
    assert_eq!(o["xdg"], "t2");
    let param = vec![OwnedVariant::from(zbus::Str::from("p".to_owned()))];
    call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        fdo,
        "ActivateAction",
        &("go", param, platform("t3")),
    )
    .expect("ActivateAction");
    let act = primary.wait_json("action", |v| v["ev"] == "action");
    assert_eq!(act["name"], "go");
    assert_eq!(act["xdg"], "t3");
    // An action the app doesn't accept is refused, as a GApplication refuses one.
    let unknown = call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        fdo,
        "ActivateAction",
        &("explode", Vec::<OwnedVariant>::new(), platform("")),
    );
    assert!(error_name(unknown).contains("InvalidArgs"));
    let gtk = "org.gtk.Application";
    call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        gtk,
        "Open",
        &(vec!["file:///y"], "hint", platform("")),
    )
    .expect("org.gtk.Application.Open");
    let o = primary.wait_json("gtk open", |v| {
        v["ev"] == "open" && v["uris"][0] == "file:///y"
    });
    assert_eq!(o["hint"], "hint");
    // A GApplication forwarder lists the actions first.
    let all = call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        "org.gtk.Actions",
        "DescribeAll",
        &(),
    )
    .expect("DescribeAll");
    assert_eq!(all.body().signature().to_string(), "a{s(bgav)}");
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn file_manager1_calls_arrive_as_reveals_and_bad_calls_fail() {
    let s = bus_only();
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.env("PROBE_FILE_MANAGER1", "1");
    let primary = start_primary(&s, "primary", cmd);
    assert_eq!(claim_line(&primary)["file_manager1"], "owned");
    let (rt, conn) = client(&s);
    let (fm, path) = (
        "org.freedesktop.FileManager1",
        "/org/freedesktop/FileManager1",
    );
    let uris = vec!["file:///b/2", "file:///a/1"];
    for (method, kind) in [
        ("ShowFolders", "folders"),
        ("ShowItems", "items"),
        ("ShowItemProperties", "item-properties"),
    ] {
        // Owned before the claim returned, so the first call reaches the primary.
        call(&rt, &conn, fm, path, fm, method, &(uris.clone(), "sid")).expect(method);
        let r = primary.wait_json(method, |v| v["ev"] == "reveal" && v["method"] == kind);
        assert_eq!(r["uris"], serde_json::json!(uris));
        assert_eq!(r["startup_id"], "sid");
    }
    let wrong = call(&rt, &conn, fm, path, fm, "ShowItems", &(uris.clone(),));
    assert!(error_name(wrong).contains("InvalidArgs"));
    let unknown = call(&rt, &conn, fm, path, fm, "ShowEverything", &(uris, "sid"));
    assert!(error_name(unknown).contains("UnknownMethod"));
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn file_manager1_waits_behind_another_owner() {
    let s = bus_only();
    let (rt, stub) = client(&s);
    let fm = "org.freedesktop.FileManager1";
    rt.block_on(stub.request_name(fm))
        .expect("the stub takes the name");
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.env("PROBE_FILE_MANAGER1", "1");
    let primary = start_primary(&s, "primary", cmd);
    assert_eq!(claim_line(&primary)["file_manager1"], "queued");
    let owner = |rt: &tokio::runtime::Runtime| -> String {
        let reply = call(
            rt,
            &stub,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            &(fm,),
        )
        .expect("GetNameOwner");
        reply.body().deserialize::<String>().expect("an owner")
    };
    let stub_name = stub.unique_name().expect("a unique name").to_string();
    assert_eq!(
        owner(&rt),
        stub_name,
        "the primary took the name from its owner"
    );
    rt.block_on(stub.release_name(fm))
        .expect("the stub lets go");
    let deadline = Instant::now() + timeout();
    while owner(&rt) == stub_name {
        assert!(
            Instant::now() < deadline,
            "the queued primary never got the name"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    call(
        &rt,
        &stub,
        fm,
        "/org/freedesktop/FileManager1",
        fm,
        "ShowFolders",
        &(vec!["file:///c"], ""),
    )
    .expect("ShowFolders");
    primary.wait_json("the reveal", |v| {
        v["ev"] == "reveal" && v["uris"][0] == "file:///c"
    });
}

/// A primary that prints, answers and gives the app id up at once: the launch still ends
/// with that answer, its line printed once, and never runs again as a primary of its own. A
/// release that cut the answer off would look to the launch like a primary gone, and it would
/// claim the app id and handle itself a second time. Half the rounds release from the main
/// thread, half from the bus thread (inside the sink).
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_release_right_after_answering_never_hands_the_launch_on() {
    for round in 0..24 {
        let on = if round % 2 == 0 { "main" } else { "bus" };
        let s = bus_only();
        let mut cmd = command(&s, PROBE, Display::None);
        cmd.env("PROBE_RELEASE_ON", on);
        let primary = start_primary(&s, "primary", cmd);
        let mut launch = command(&s, PROBE, Display::None);
        launch.args(["--print", "--release-after"]);
        let out = run(launch);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let context = format!("round {round}, released on {on}: {out:?}");
        assert_eq!(out.status.code(), Some(0), "{context}");
        assert_eq!(
            stdout.matches("hello from the primary").count(),
            1,
            "{context}"
        );
        assert!(stdout.contains("\"result\":\"forwarded\""), "{context}");
        assert!(!stdout.contains("\"result\":\"primary\""), "{context}");
        primary.wait_json("the release", |v| v["ev"] == "released" && v["on"] == on);
    }
}

/// A launch the app still holds when it gives the app id up hears that the primary left, at
/// once rather than after `Options::forward_timeout`, and claims the app id itself.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_launch_held_through_a_release_claims_the_app_id_again() {
    let s = bus_only();
    let primary = start_primary(&s, "primary", command(&s, PROBE, Display::None));
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.arg("--hold");
    let launch = s.spawn("launch", cmd);
    primary.wait_json("the held launch", |v| {
        v["ev"] == "launch" && v["remote"] == true
    });
    let released = Instant::now();
    primary.send_line("release");
    assert_eq!(claim_line(&launch)["result"], "primary");
    assert!(released.elapsed() < timeout(), "{:?}", released.elapsed());
    launch.wait_json("its own launch", |v| {
        v["ev"] == "launch" && v["remote"] == false
    });
}

/// A bus that won't let the primary have `org.freedesktop.FileManager1` (a sandbox without
/// permission to own it) leaves it the primary without that name. The probe asks for
/// `org.freedesktop.DBus` in its place, which every bus refuses.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_refused_file_manager1_leaves_the_primary_claimed() {
    let s = bus_only();
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.env("PROBE_FILE_MANAGER1", "1")
        .env("PROBE_FILE_MANAGER1_NAME", "org.freedesktop.DBus");
    let primary = start_primary(&s, "primary", cmd);
    let claim = claim_line(&primary);
    assert!(
        claim["file_manager1"]
            .as_str()
            .is_some_and(|r| r.starts_with("failed")),
        "{claim}"
    );
    // Still the one instance: a later launch reaches it.
    let mut launch = command(&s, PROBE, Display::None);
    launch.arg("still-single");
    assert_eq!(run(launch).status.code(), Some(0));
    primary.wait_json("the forwarded launch", |v| {
        v["ev"] == "launch" && v["args"][1] == hex(b"still-single")
    });
}

/// A primary that takes a launch and leaves without answering: the launch hears `NoReply`,
/// claims the app id again, becomes the primary and handles itself, once.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_launch_whose_primary_vanishes_unanswered_becomes_the_primary() {
    let s = bus_only();
    let (rt, fake) = client(&s);
    let mut stream = rt.block_on(async { MessageStream::from(&fake) });
    rt.block_on(fake.request_name(PROBE_ID))
        .expect("the fake takes the app id");
    let mut cmd = command(&s, PROBE, Display::None);
    cmd.arg("vanished");
    let launch = s.spawn("launch", cmd);
    let taken = rt.block_on(async {
        tokio::time::timeout(timeout(), async {
            loop {
                let message = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                    .await
                    .expect("the bus stream")
                    .expect("a message");
                if message
                    .header()
                    .member()
                    .is_some_and(|m| m.as_str() == "CommandLine")
                {
                    return message;
                }
            }
        })
        .await
    });
    let taken = taken.expect("the launch's CommandLine call");
    assert_eq!(taken.body().signature().to_string(), "(oaaya{sv})");
    // Gone without a word: the bus answers the launch for it.
    rt.block_on(async move {
        drop(stream);
        drop(fake);
    });
    drop(rt);
    assert_eq!(claim_line(&launch)["result"], "primary");
    launch.wait_json("its own launch", |v| v["ev"] == "launch");
    let mut forwarded = command(&s, PROBE, Display::None);
    forwarded.arg("after");
    assert_eq!(run(forwarded).status.code(), Some(0));
    launch.wait_json("the next launch", |v| {
        v["ev"] == "launch" && v["remote"] == true
    });
    let own = launch
        .lines()
        .iter()
        .filter(|l| l.contains("\"ev\":\"launch\"") && l.contains("\"remote\":false"))
        .count();
    assert_eq!(own, 1, "{:?}", launch.lines());
}

/// Calls sent back to back reach the app in the order they were sent, across interfaces.
#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn pipelined_calls_arrive_in_order() {
    let s = bus_only();
    let primary = start_primary(&s, "primary", command(&s, PROBE, Display::None));
    let (rt, conn) = client(&s);
    const N: usize = 90;
    rt.block_on(async {
        let mut stream = MessageStream::from(&conn);
        for i in 0..N {
            let (interface, method) = [
                ("org.freedesktop.Application", "Activate"),
                ("org.freedesktop.Application", "Open"),
                ("org.gtk.Application", "Open"),
            ][i % 3];
            let builder = Message::method_call(PROBE_PATH, method)
                .destination(PROBE_ID)
                .interface(interface);
            let data = platform(&i.to_string());
            let message = match (interface, method) {
                (_, "Activate") => builder.build(&(data,)),
                ("org.gtk.Application", _) => {
                    builder.build(&(vec![format!("file:///{i}")], "", data))
                }
                _ => builder.build(&(vec![format!("file:///{i}")], data)),
            };
            conn.send(&message.expect("a call")).await.expect("sending");
        }
        let mut answered = 0;
        tokio::time::timeout(timeout(), async {
            while answered < N {
                let message = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                    .await
                    .expect("the bus stream")
                    .expect("a message");
                match message.message_type() {
                    zbus::message::Type::MethodReturn => answered += 1,
                    zbus::message::Type::Error => panic!("a call failed: {message:?}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("every call answered");
    });
    wait_until("every event", || {
        primary
            .lines()
            .iter()
            .filter(|l| l.contains("\"ev\":\"activate\"") || l.contains("\"ev\":\"open\""))
            .count()
            == N
    });
    let order: Vec<usize> = primary
        .lines()
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["ev"] == "activate" || v["ev"] == "open")
        .filter_map(|v| v["xdg"].as_str().and_then(|t| t.parse().ok()))
        .collect();
    assert_eq!(order, (0..N).collect::<Vec<_>>());
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_file_manager1_call_starts_the_service() {
    let s = bus_only();
    s.bus().add_service(
        "org.freedesktop.FileManager1",
        &format!(
            "/usr/bin/env PROBE_SERVICE=1 PROBE_FILE_MANAGER1=1 {}",
            example(PROBE).display()
        ),
    );
    let (rt, conn) = client(&s);
    let fm = "org.freedesktop.FileManager1";
    call(
        &rt,
        &conn,
        fm,
        "/org/freedesktop/FileManager1",
        fm,
        "ShowItems",
        &(vec!["file:///d/e"], ""),
    )
    .expect("ShowItems through D-Bus activation");
    // The service is the app's primary too: a later launch reaches it.
    let mut launch = command(&s, PROBE, Display::None);
    launch.arg("after-activation");
    assert_eq!(run(launch).status.code(), Some(0));
    // The probe exits inside the action, before it can answer.
    let quit = call(
        &rt,
        &conn,
        PROBE_ID,
        PROBE_PATH,
        "org.freedesktop.Application",
        "ActivateAction",
        &(
            "quit",
            Vec::<OwnedVariant>::new(),
            HashMap::<String, OwnedVariant>::new(),
        ),
    );
    let no_reply =
        matches!(&quit, Err(zbus::Error::FDO(e)) if matches!(**e, zbus::fdo::Error::NoReply(_)));
    assert!(quit.is_ok() || no_reply, "{quit:?}");
    let has_owner = |name: &str| {
        call(
            &rt,
            &conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            &(name,),
        )
        .and_then(|r| r.body().deserialize::<bool>())
        .expect("NameHasOwner")
    };
    wait_until("the service gone", || {
        !has_owner(PROBE_ID) && !has_owner(fm)
    });
}

#[cfg(feature = "glib-bridge")]
mod interop {
    //! The shim against a real GLib `GApplication` of the same id, both ways round.

    use super::*;

    const PEER: &str = "gapplication-peer";

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_launch_reaches_a_gapplication_primary() {
        let s = bus_only();
        let mut cmd = command(&s, PEER, Display::None);
        cmd.env("GAPP_ID", PROBE_ID);
        let peer = start_primary(&s, "gapplication", cmd);
        let mut launch = command(&s, PROBE, Display::None);
        launch
            .args(["--exit5", "x"])
            .env("XDG_ACTIVATION_TOKEN", "token-g");
        let out = run(launch);
        assert_eq!(out.status.code(), Some(5), "{out:?}");
        let got = peer.wait_json("the forwarded launch", |v| {
            v["ev"] == "launch" && v["remote"] == true
        });
        assert_eq!(got["args"][2], "x");
        assert_eq!(
            got["cwd"],
            s.runtime_dir().join("home").display().to_string()
        );
        assert!(
            got["platform"]
                .as_str()
                .is_some_and(|p| p.contains("token-g")),
            "{got}"
        );
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_gapplication_launch_reaches_this_primary() {
        let s = bus_only();
        let primary = start_primary(&s, "primary", command(&s, PROBE, Display::None));
        let mut launch = command(&s, PEER, Display::None);
        launch.args(["--print", "y"]).env("GAPP_ID", PROBE_ID);
        let out = run(launch);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("\"result\":\"forwarded\""), "{stdout}");
        assert!(stdout.contains("hello from the primary"), "{stdout}");
        let got = primary.wait_json("the GLib launch", |v| {
            v["ev"] == "launch" && v["remote"] == true
        });
        assert_eq!(got["args"][2], hex(b"y"));

        let mut bad = command(&s, PEER, Display::None);
        bad.arg("--usage-error").env("GAPP_ID", PROBE_ID);
        let out = run(bad);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage: single-instance-probe"));
    }
}

// ---- windows, under sway ----

/// A toplevel in sway's tree, with what these tests read.
#[derive(Debug)]
struct Toplevel {
    id: i64,
    focused: bool,
    urgent: bool,
}

fn windows_of(s: &Session, pid: u32) -> Vec<Toplevel> {
    fn walk(v: &Value, pid: u32, out: &mut Vec<Toplevel>) {
        if v["pid"].as_u64() == Some(u64::from(pid))
            && v["nodes"].as_array().is_some_and(Vec::is_empty)
        {
            out.push(Toplevel {
                id: v["id"].as_i64().unwrap_or(-1),
                focused: v["focused"].as_bool().unwrap_or(false),
                urgent: v["urgent"].as_bool().unwrap_or(false),
            });
        }
        for key in ["nodes", "floating_nodes"] {
            for n in v[key].as_array().into_iter().flatten() {
                walk(n, pid, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&s.tree(), pid, &mut out);
    out
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout();
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn second_window_focused(display: Display) {
    let s = Session::start();
    let mut cmd = command(&s, DEMO, display);
    cmd.args(["a", "b"]);
    let primary = start_primary(&s, "demo", cmd);
    primary.wait_json("the first window", |v| {
        v["ev"] == "ready" && v["key"] == "launch-1"
    });
    let first = s.wait_window(primary.pid());

    let mut second = command(&s, DEMO, display);
    second.arg("c");
    // On Wayland the launch carries a launcher's token, made by a focused peer, so the
    // primary gives it to the new window's app before `open_window`. sway focuses a new window
    // with or without one, so this shows the token reaching that path, not the compositor
    // using it; the WAYLAND_DEBUG probe (P-a) showed the window created with it.
    let peer = (display == Display::Wayland).then(|| {
        let peer = s.peer(Display::Wayland);
        peer.map("launcher");
        s.focus(&s.wait_window(peer.pid()));
        peer
    });
    let token = peer.as_ref().map(|p| p.activation_token());
    if let Some(token) = &token {
        second.env("XDG_ACTIVATION_TOKEN", token);
    }
    let out = run(second);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("opened in the running instance"),
        "{out:?}"
    );
    let launch = primary.wait_json("the forwarded launch", |v| {
        v["ev"] == "launch" && v["remote"] == true
    });
    assert_eq!(launch["token"].as_str(), token.as_deref());
    primary.wait_json("the second window", |v| {
        v["ev"] == "ready" && v["key"] == "launch-2"
    });
    let pid = primary.pid();
    wait_until(
        "two windows of the first process, the new one focused",
        || {
            let windows = windows_of(&s, pid);
            windows.len() == 2 && windows.iter().any(|w| w.id != first.id && w.focused)
        },
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn second_launch_opens_a_focused_window_in_the_primary_wayland() {
    second_window_focused(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn second_launch_opens_a_focused_window_in_the_primary_x11() {
    second_window_focused(Display::X11);
}

/// A bare launch of the running app brings its window forward. On Wayland only a token lets
/// it take focus; without one sway marks it urgent. sway focuses an X11 window that asks
/// (`focus_on_window_activation focus`, W3's config) with or without a startup id, so the X11
/// case can't show what mutter and KWin do with the timestamp-less request (U-P8-1,
/// `docs/decisions/S5-upstream.md#u-p8-1`).
fn bare_launch_raises(display: Display) {
    let s = Session::start();
    let primary = start_primary(&s, "demo", command(&s, DEMO, display));
    primary.wait_json("the window", |v| {
        v["ev"] == "ready" && v["key"] == "launch-1"
    });
    let window = s.wait_window(primary.pid());
    let peer = s.peer(Display::Wayland);
    peer.map("peer");
    let peer_window = s.wait_window(peer.pid());

    s.focus(&peer_window);
    let out = run(command(&s, DEMO, display));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    if display == Display::Wayland {
        let pid = primary.pid();
        wait_until("the window marked urgent", || {
            windows_of(&s, pid)
                .iter()
                .any(|w| w.id == window.id && w.urgent)
        });
        assert!(
            !windows_of(&s, primary.pid())[0].focused,
            "focused without a token"
        );

        s.focus(&peer_window);
        let token = peer.activation_token();
        let mut with_token = command(&s, DEMO, display);
        with_token.env("XDG_ACTIVATION_TOKEN", &token);
        let out = run(with_token);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
    }
    let pid = primary.pid();
    wait_until("the window focused", || {
        windows_of(&s, pid).iter().any(|w| w.focused)
    });
    assert_eq!(
        windows_of(&s, pid).len(),
        1,
        "a bare launch opened a window"
    );
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_bare_launch_raises_the_window_with_its_token_wayland() {
    bare_launch_raises(Display::Wayland);
}

#[test]
#[ignore = "needs a private session: mise run test-platform"]
fn a_bare_launch_raises_the_window_x11() {
    bare_launch_raises(Display::X11);
}
