// SPDX-License-Identifier: MIT
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

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
fn provider_frames_restrict_icons_size_and_action_ids() {
    let m = Manifest {
        version: 1,
        id: "example".into(),
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
        })
        .expect("send");
    assert!(matches!(
        c.updates.recv_timeout(Duration::from_secs(3)),
        Ok(Update::Reply(Reply { event: Some(_), .. }))
    ));
    let Update::Reply(reply) = c
        .updates
        .recv_timeout(Duration::from_secs(3))
        .expect("reply")
    else {
        panic!("offline")
    };
    assert_eq!(reply.id, Some(71));
    assert_eq!(reply.decorations[0].path, path);
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
        })
        .expect("send");
    assert!(matches!(
        c.updates.recv_timeout(Duration::from_secs(5)),
        Ok(Update::Offline)
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
        })
        .expect("send");
    assert!(matches!(
        c.updates.recv_timeout(Duration::from_secs(13)),
        Ok(Update::Offline)
    ));
    let pid = fs::read_to_string(&record).expect("child received action");
    assert!(
        !Path::new("/proc").join(pid.trim()).exists(),
        "timed-out provider was reaped"
    );
    // No second request means no restart and no automatic activation replay.
    assert!(c.updates.recv_timeout(Duration::from_millis(100)).is_err());
}
