// SPDX-License-Identifier: MIT
use super::protocol::{FRAME_LIMIT, checked_reply};
use super::transport::take_frames;
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::{
    thread,
    time::{Duration, Instant},
};

fn registration(root: &Path, command: Vec<String>) -> PathBuf {
    let dir = root.join("example");
    fs::create_dir_all(&dir).expect("fixture dir");
    fs::write(
        dir.join("provider.json"),
        serde_json::json!({"version":1,"id":"example","command":command,"icons":{}}).to_string(),
    )
    .expect("manifest");
    dir
}
fn python() -> String {
    "/usr/bin/python3".into()
}
#[test]
fn discovery_refuses_symlinks_writable_registration_and_relative_programs() {
    let t = tempfile::tempdir().expect("temp");
    let d = registration(t.path(), vec![python(), "-c".into(), "pass".into()]);
    assert_eq!(discover(t.path()).len(), 1);
    fs::set_permissions(d.join("provider.json"), fs::Permissions::from_mode(0o666)).expect("chmod");
    assert!(discover(t.path()).is_empty());
    fs::set_permissions(d.join("provider.json"), fs::Permissions::from_mode(0o600)).expect("chmod");
    let source = d.join("provider.json");
    let other = d.join("other.json");
    fs::rename(&source, &other).expect("move fixture");
    symlink(&other, &source).expect("symlink");
    assert!(discover(t.path()).is_empty());
    let second = tempfile::tempdir().expect("temp");
    registration(second.path(), vec!["python3".into()]);
    assert!(discover(second.path()).is_empty());
}
#[test]
fn registration_accepts_optional_names_and_rejects_invalid_display_text() {
    let root = tempfile::tempdir().expect("registration fixture");
    registration(root.path(), vec![python()]);
    let manifest_path = root.path().join("example/provider.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    for (name, accepted) in [
        (None, true),
        (Some(serde_json::Value::Null), true),
        (Some(serde_json::json!("Example Cloud")), true),
        (Some(serde_json::json!("a".repeat(64))), true),
        (Some(serde_json::json!("☁".repeat(21))), true),
        (Some(serde_json::json!("")), false),
        (Some(serde_json::json!("   ")), false),
        (Some(serde_json::json!("a".repeat(65))), false),
        (Some(serde_json::json!("☁".repeat(22))), false),
        (Some(serde_json::json!("Cloud\nProvider")), false),
        (Some(serde_json::json!("Cloud\rProvider")), false),
        (Some(serde_json::json!("Cloud\tProvider")), false),
        (Some(serde_json::json!("Cloud\u{1b}Provider")), false),
        (Some(serde_json::json!("Cloud\0Provider")), false),
        (Some(serde_json::json!(12)), false),
    ] {
        manifest
            .as_object_mut()
            .expect("manifest object")
            .remove("name");
        if let Some(name) = &name {
            manifest["name"] = name.clone();
        }
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        assert_eq!(!discover(root.path()).is_empty(), accepted, "name {name:?}");
    }
}

#[test]
fn provider_frames_restrict_icons_size_and_action_ids() {
    let m = Manifest {
        version: 1,
        id: "example".into(),
        name: None,
        command: vec![python()],
        icons: BTreeMap::new(),
    };
    for bad in [
        serde_json::json!({"version":1,"id":1,"actions":[{"id":"../../run","label":"No","icon":null}]}),
        serde_json::json!({"version":1,"id":1,"decorations":[{"path":"/file","badge":"/tmp/icon"}]}),
        serde_json::json!({"version":1,"event":"invalidate","id":1}),
        serde_json::json!({"version":2,"id":1}),
    ] {
        assert!(checked_reply(&serde_json::to_vec(&bad).expect("json"), &m).is_err());
    }
    assert!(checked_reply(br#"{"version":1,"event":"invalidate"}"#, &m).is_ok());
}
#[test]
fn child_receives_literal_paths_and_invalidation_without_shell_expansion() {
    let t = tempfile::tempdir().expect("temp");
    let script = r#"import sys,json
for line in sys.stdin:
 r=json.loads(line)
 print(json.dumps({'version':1,'event':'invalidate'}),flush=True)
 print(json.dumps({'version':1,'id':r['id'],'decorations':[{'path':p,'badge':None} for p in r['paths']]}),flush=True)
"#;
    registration(
        t.path(),
        vec![python(), "-u".into(), "-c".into(), script.into()],
    );
    let c = start(discover(t.path()).remove(0));
    let path = "/tmp/a\n'$(touch should-not-exist)' 🐈";
    c.requests
        .send(Request {
            version: 1,
            id: 71,
            method: "query".into(),
            paths: vec![path.into()],
            background: false,
            action: None,
            context: None,
        })
        .expect("send");
    assert!(matches!(
        c.updates.recv_timeout(Duration::from_secs(3)),
        Ok(Update::Reply(reply)) if reply.event.is_some()
    ));
    let Update::Reply(reply) = c
        .updates
        .recv_timeout(Duration::from_secs(3))
        .expect("reply")
    else {
        panic!("offline")
    };
    assert_eq!(reply.id, Some(71));
    assert_eq!(reply.decorations.expect("decorations")[0].path, path);
}
#[test]
fn excessive_reply_disconnects_provider_instead_of_allocating_without_bound() {
    let t = tempfile::tempdir().expect("temp");
    registration(
        t.path(),
        vec![
            python(),
            "-u".into(),
            "-c".into(),
            "import sys;sys.stdin.readline();sys.stdout.write('x'*1100000);sys.stdout.flush()"
                .into(),
        ],
    );
    let c = start(discover(t.path()).remove(0));
    c.requests
        .send(Request {
            version: 1,
            id: 1,
            method: "query".into(),
            paths: vec!["/file".into()],
            background: false,
            action: None,
            context: None,
        })
        .expect("send");
    assert!(matches!(
        c.updates.recv_timeout(Duration::from_secs(5)),
        Ok(Update::Offline { .. })
    ));
}

#[test]
fn unresponsive_child_is_terminated_without_replaying_an_activation() {
    let t = tempfile::tempdir().expect("temp");
    let record = t.path().join("accepted");
    let script = r#"import os,sys,time
from pathlib import Path
sys.stdin.readline()
p=Path(sys.argv[1]);p.write_text(str(os.getpid()))
time.sleep(60)
"#;
    registration(
        t.path(),
        vec![
            python(),
            "-u".into(),
            "-c".into(),
            script.into(),
            record.to_string_lossy().into_owned(),
        ],
    );
    let c = start(discover(t.path()).remove(0));
    c.requests
        .send(Request {
            version: 1,
            id: 1,
            method: "activate".into(),
            paths: vec!["/file".into()],
            background: false,
            action: Some("keep".into()),
            context: None,
        })
        .expect("send");
    assert!(matches!(
        c.updates.recv_timeout(Duration::from_secs(13)),
        Ok(Update::Offline { .. })
    ));
    let pid = fs::read_to_string(&record).expect("child received action");
    assert!(
        !Path::new("/proc").join(pid.trim()).exists(),
        "timed-out provider was reaped"
    );
    // No second request means no restart and no automatic activation replay.
    assert!(c.updates.recv_timeout(Duration::from_millis(100)).is_err());
}

fn request(id: u64, method: &str) -> Request {
    Request {
        version: 1,
        id,
        method: method.into(),
        paths: vec!["/file".into()],
        background: false,
        action: (method == "activate").then(|| "keep".into()),
        context: None,
    }
}

fn manifest() -> Manifest {
    Manifest {
        version: 1,
        id: "example".into(),
        name: None,
        command: vec![python()],
        icons: BTreeMap::new(),
    }
}

#[test]
fn menu_trees_validate_whole_tree_bounds_and_leaf_contexts() {
    let leaf = serde_json::json!({"id":"keep","label":"Keep","context":"object-incarnation"});
    let tree = serde_json::json!({"version":1,"id":1,"actions":[
        {"id":"share","label":"Share"}, {"id":"offline","label":"Offline","children":[leaf.clone()]}
    ]});
    let parsed =
        checked_reply(&serde_json::to_vec(&tree).expect("json"), &manifest()).expect("mixed tree");
    assert_eq!(
        parsed.actions.expect("actions")[1]
            .children
            .as_ref()
            .expect("children")[0]
            .context
            .as_deref(),
        Some("object-incarnation")
    );
    let mut invalid = tree.clone();
    invalid["actions"][0]["id"] = "keep".into();
    assert!(checked_reply(&serde_json::to_vec(&invalid).expect("json"), &manifest()).is_err());
    invalid = tree.clone();
    invalid["actions"][1]["context"] = "no-submenu-activation".into();
    assert!(checked_reply(&serde_json::to_vec(&invalid).expect("json"), &manifest()).is_err());
    invalid = tree.clone();
    invalid["actions"][1]["children"] = serde_json::json!([]);
    assert!(checked_reply(&serde_json::to_vec(&invalid).expect("json"), &manifest()).is_err());
    let mut nested = leaf;
    for depth in 1..=4 {
        nested = serde_json::json!({"id":format!("branch-{depth}"),"label":"Branch","children":[nested]});
    }
    let too_deep = serde_json::json!({"version":1,"id":1,"actions":[nested]});
    assert!(checked_reply(&serde_json::to_vec(&too_deep).expect("json"), &manifest()).is_err());
    let branches: Vec<_> = (0..4).map(|branch| serde_json::json!({"id":format!("branch-{branch}"),"label":"Branch",
        "children":(0..16).map(|leaf| serde_json::json!({"id":format!("leaf-{branch}-{leaf}"),"label":"Leaf"})).collect::<Vec<_>>()})).collect();
    assert!(
        checked_reply(
            &serde_json::to_vec(&serde_json::json!({"version":1,"id":1,"actions":branches}))
                .expect("json"),
            &manifest()
        )
        .is_err()
    );
}

#[test]
fn responses_require_method_fields_and_validate_partial_outcomes() {
    for (method, frame) in [
        (
            "query",
            serde_json::json!({"version":1,"id":1,"decorations":[]}),
        ),
        ("menu", serde_json::json!({"version":1,"id":1,"actions":[]})),
        (
            "activate",
            serde_json::json!({"version":1,"id":1,"message":"Accepted one item","outcome":{"status":"accepted","accepted":1,"total":1}}),
        ),
    ] {
        let reply =
            checked_reply(&serde_json::to_vec(&frame).expect("json"), &manifest()).expect("reply");
        assert!(reply.matches(&request(1, method)));
        for other in ["query", "menu", "activate"]
            .into_iter()
            .filter(|other| *other != method)
        {
            assert!(!reply.matches(&request(1, other)));
        }
    }
    let missing = checked_reply(br#"{"version":1,"id":1,"future":true}"#, &manifest())
        .expect("unknown additive fields");
    assert!(!missing.matches(&request(1, "menu")));
    let unsupported = checked_reply(
        br#"{"version":1,"id":1,"error":"unsupported-method","message":"Unsupported request"}"#,
        &manifest(),
    )
    .expect("error");
    assert!(unsupported.matches(&request(1, "menu")));
    for text in ["Accepted\0hidden", "Unavailable\u{1b}[31m", "Status\u{7}"] {
        for frame in [
            serde_json::json!({"version":1,"id":1,"message":text}),
            serde_json::json!({"version":1,"id":1,"decorations":[{"path":"/file","description":text}]}),
        ] {
            assert!(
                checked_reply(&serde_json::to_vec(&frame).expect("json"), &manifest()).is_err(),
                "control data was accepted for GTK presentation"
            );
        }
    }
    let formatted = checked_reply(
        &serde_json::to_vec(&serde_json::json!({
            "version":1,"id":1,"message":"Accepted\nDetails:\tjob 17\r\n"
        }))
        .expect("json"),
        &manifest(),
    )
    .expect("plain-text formatting");
    assert!(formatted.matches(&request(1, "activate")));
    for (accepted, valid) in [(0, false), (1, true), (2, false), (3, false)] {
        let frame = serde_json::json!({"version":1,"id":1,"message":"Partial result","outcome":{"status":"partial","accepted":accepted,"total":2}});
        assert_eq!(
            checked_reply(&serde_json::to_vec(&frame).expect("json"), &manifest()).is_ok(),
            valid
        );
    }
}

#[test]
fn framing_limits_each_line_across_socket_boundaries() {
    let frame = vec![b'x'; FRAME_LIMIT - 99];
    let mut buffer = frame[..frame.len() - 100].to_vec();
    assert!(take_frames(&mut buffer).expect("partial frame").is_empty());
    buffer.extend_from_slice(&frame[frame.len() - 100..]);
    buffer.push(b'\n');
    buffer.extend_from_slice(&[b'y'; 1034]);
    assert_eq!(
        take_frames(&mut buffer).expect("individually bounded frames"),
        vec![frame]
    );
    assert_eq!(buffer.len(), 1034);
    buffer.push(b'\n');
    assert_eq!(
        take_frames(&mut buffer).expect("second frame")[0].len(),
        1034
    );
    for chunk in [1, 7, 8192] {
        let wire = b"first\nsecond\nthird\nremaining";
        let mut buffer = Vec::new();
        let mut frames = Vec::new();
        for bytes in wire.chunks(chunk) {
            buffer.extend_from_slice(bytes);
            frames.extend(take_frames(&mut buffer).expect("frames"));
        }
        assert_eq!(
            frames,
            vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()]
        );
        assert_eq!(buffer, b"remaining");
    }
    assert!(take_frames(&mut vec![b'x'; FRAME_LIMIT + 1]).is_err());
    let mut oversized = vec![b'x'; FRAME_LIMIT + 1];
    oversized.push(b'\n');
    assert!(take_frames(&mut oversized).is_err());
}

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "provider fixture timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn request_saturation_keeps_child_alive_and_prioritizes_activation() {
    let t = tempfile::tempdir().expect("temp");
    let record = t.path().join("record");
    let release = t.path().join("release");
    let script = r#"import sys,json,time
from pathlib import Path
record,release=map(Path,sys.argv[1:])
for line in sys.stdin:
 r=json.loads(line)
 with record.open('a') as f: f.write(str(r['id'])+'\n')
 if r['id']==1:
  while not release.exists(): time.sleep(.01)
 result={'version':1,'id':r['id']}
 if r['method']=='activate': result['message']='Accepted'
 else: result['decorations']=[]
 print(json.dumps(result),flush=True)
"#;
    registration(
        t.path(),
        vec![
            python(),
            "-u".into(),
            "-c".into(),
            script.into(),
            record.to_str().expect("path").into(),
            release.to_str().expect("path").into(),
        ],
    );
    let client = start(discover(t.path()).remove(0));
    client.requests.send(request(1, "query")).expect("first");
    wait_for(|| record.exists());
    for id in 2..=17 {
        client
            .requests
            .try_send(request(id, "query"))
            .expect("refresh queue");
    }
    assert!(matches!(
        client.requests.try_send(request(18, "query")),
        Err(std::sync::mpsc::TrySendError::Full(_))
    ));
    client
        .requests
        .try_send(request(99, "activate"))
        .expect("reserved action queue");
    fs::write(&release, "go").expect("release");
    for expected in [1, 99] {
        let Update::Reply(reply) = client
            .updates
            .recv_timeout(Duration::from_secs(3))
            .expect("reply")
        else {
            panic!("healthy provider disconnected");
        };
        assert_eq!(reply.id, Some(expected));
    }
    wait_for(|| fs::read_to_string(&record).is_ok_and(|s| s.lines().count() >= 3));
    assert_eq!(
        fs::read_to_string(&record)
            .expect("record")
            .lines()
            .take(3)
            .collect::<Vec<_>>(),
        ["1", "99", "2"]
    );
}

#[test]
fn update_saturation_and_event_flood_preserve_pending_activation() {
    let t = tempfile::tempdir().expect("temp");
    let record = t.path().join("record");
    let script = r#"import sys,json
from pathlib import Path
record=Path(sys.argv[1])
for line in sys.stdin:
 r=json.loads(line)
 for i in range(64): print(json.dumps({'version':1,'event':'invalidate'}),flush=True)
 result={'version':1,'id':r['id']}
 if r['method']=='activate': result['message']='Accepted once'
 else: result['decorations']=[]
 print(json.dumps(result),flush=True)
 with record.open('a') as f: f.write(str(r['id'])+'\n')
"#;
    registration(
        t.path(),
        vec![
            python(),
            "-u".into(),
            "-c".into(),
            script.into(),
            record.to_str().expect("path").into(),
        ],
    );
    let client = start(discover(t.path()).remove(0));
    for id in 1..=32 {
        client.requests.send(request(id, "query")).expect("enqueue");
    }
    wait_for(|| fs::read_to_string(&record).is_ok_and(|s| s.lines().count() == 32));
    for id in 33..=40 {
        client
            .requests
            .try_send(request(id, "query"))
            .expect("queued refresh");
    }
    client
        .requests
        .try_send(request(99, "activate"))
        .expect("queued activation");
    thread::sleep(Duration::from_millis(150));
    let (updates, closed) = client.updates.drain();
    assert!(!closed);
    assert_eq!(
        updates
            .iter()
            .filter(|u| matches!(u, Update::Reply(r) if r.id.is_some()))
            .count(),
        32
    );
    assert!(updates.iter().all(|u| !matches!(u, Update::Offline { .. })));
    loop {
        let Update::Reply(reply) = client
            .updates
            .recv_timeout(Duration::from_secs(3))
            .expect("activation result")
        else {
            panic!("healthy provider disconnected");
        };
        if reply.id == Some(99) {
            assert_eq!(reply.message, "Accepted once");
            break;
        }
    }
    wait_for(|| fs::read_to_string(&record).is_ok_and(|s| s.lines().any(|line| line == "99")));
    assert_eq!(
        fs::read_to_string(&record)
            .expect("record")
            .lines()
            .filter(|line| *line == "99")
            .count(),
        1
    );
}

#[test]
fn escaped_large_selections_split_queries_without_killing_the_provider() {
    let scratch = tempfile::tempdir().expect("provider fixture");
    let script = "import sys,json\nfor line in sys.stdin:\n r=json.loads(line)\n print(json.dumps({'version':1,'id':r['id'],'decorations':[{'path':p,'description':'Seen'} for p in r['paths']]}),flush=True)";
    registration(
        scratch.path(),
        vec![python(), "-u".into(), "-c".into(), script.into()],
    );
    let client = start(discover(scratch.path()).remove(0));
    let mut paths = (0..200)
        .map(|index| format!("/{index}-{}", "\u{1}".repeat(3000)))
        .collect::<Vec<_>>();
    let mut too_large = request(1, "menu");
    too_large.paths = paths.clone();
    assert!(!selection_fits(&paths));
    assert!(matches!(
        client.requests.try_send(too_large),
        Err(std::sync::mpsc::TrySendError::Full(_))
    ));
    let mut received = Vec::new();
    for id in 2.. {
        let batch = query_batch(paths.clone());
        assert!(!batch.is_empty());
        let mut query = request(id, "query");
        query.paths = batch.clone();
        assert!(serde_json::to_vec(&query).expect("host frame").len() < FRAME_LIMIT);
        client
            .requests
            .try_send(query)
            .expect("bounded request admitted");
        let Update::Reply(reply) = client
            .updates
            .recv_timeout(Duration::from_secs(3))
            .expect("query response")
        else {
            panic!("host frame budget disconnected a healthy provider")
        };
        assert_eq!(reply.id, Some(id));
        assert_eq!(
            reply
                .decorations
                .expect("query field")
                .iter()
                .map(|d| &d.path)
                .collect::<Vec<_>>(),
            batch.iter().collect::<Vec<_>>()
        );
        received.extend(paths.drain(..batch.len()));
        if paths.is_empty() {
            break;
        }
    }
    assert_eq!(received.len(), 200);
}
