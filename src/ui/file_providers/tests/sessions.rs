// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn restart_resets_revision_requirements_and_pending_work() {
    install_hub();
    with_hub(|hub| {
        hub.providers[0].freshness.invalidate(None, Some(100));
        hub.pending
            .insert(9, Pending::Query(0, 1, vec!["/file".into()]));
        hub.offline(0, &[], Some(&[9]));
        assert!(hub.pending.is_empty());
        assert!(
            hub.providers[0]
                .freshness
                .accept(&["/file".into()], Some(0))
        );
        assert!(hub.providers[0].freshness.accept(&["/file".into()], None));
    });
    clear_hub();
}

/// Request 1 holds the session until `release` exists, then the provider exits;
/// menus are logged to `ready` and activations to `record`.
const FAILING_SESSION: &str = r#"import json,sys,time
from pathlib import Path
ready,release,record=map(Path,sys.argv[1:])
for line in sys.stdin:
 r=json.loads(line)
 if r['id']==1:
  ready.touch()
  while not release.exists(): time.sleep(.01)
  sys.exit(0)
 if r['method']=='menu':
  with ready.open('a') as f: f.write(str(r['id'])+'\n')
  result={'version':1,'id':r['id'],'actions':[{'id':'keep','label':'Keep'}]}
 else:
  with record.open('a') as f: f.write(str(r['id'])+'\n')
  result={'version':1,'id':r['id'],'message':'Accepted'}
 print(json.dumps(result),flush=True)
"#;

struct FailingSession {
    ready: PathBuf,
    release: PathBuf,
    record: PathBuf,
    _scratch: tempfile::TempDir,
}

impl FailingSession {
    fn new() -> Self {
        let scratch = tempfile::tempdir().expect("provider fixture");
        Self {
            ready: scratch.path().join("ready"),
            release: scratch.path().join("release"),
            record: scratch.path().join("activations"),
            _scratch: scratch,
        }
    }

    fn start(&self, hub: &mut Hub) {
        hub.providers[0].client = protocol::start(scripted_registration(
            None,
            FAILING_SESSION,
            &[&self.ready, &self.release, &self.record],
        ));
    }

    fn menu_requested(&self, id: &str) -> bool {
        std::fs::read_to_string(&self.ready)
            .is_ok_and(|requests| requests.lines().any(|line| line == id))
    }
}

fn keep_activation() -> Request {
    let mut activation = request("activate", vec!["/file".into()], false);
    activation.action = Some("keep".into());
    activation
}

/// Sends request 1, which the provider holds, queues request 2 unsent, then lets
/// the session fail; returns (unsent, in flight).
fn fail_the_session(
    hub: &mut Hub,
    session: &FailingSession,
    owner: &glib::WeakRef<gtk::Widget>,
) -> (Vec<u64>, Option<u64>) {
    hub.send(0, keep_activation(), Pending::Activate(0, owner.clone()));
    let deadline = Deadline::start();
    while !session.ready.exists() {
        deadline.check("provider did not receive activation");
        nap();
    }
    hub.send(0, keep_activation(), Pending::Activate(0, owner.clone()));
    std::fs::write(&session.release, "go").expect("release provider");
    let Update::Offline { unsent, in_flight } = hub.providers[0]
        .client
        .updates
        .recv_timeout(Duration::from_secs(5))
        .expect("disconnect update")
    else {
        panic!("expected failed session")
    };
    assert_eq!((in_flight, unsent.as_slice()), (Some(1), [2].as_slice()));
    (unsent, in_flight)
}

/// Requests 3 and 4, admitted after the failure, survive retiring 1 and 2 and
/// get their outcomes.
fn later_requests_keep_their_outcomes(
    hub: &mut Hub,
    session: &FailingSession,
    owner: glib::WeakRef<gtk::Widget>,
    key: &MenuKey,
    failed: (Vec<u64>, Option<u64>),
) {
    let (unsent, in_flight) = failed;
    hub.send(0, keep_activation(), Pending::Activate(0, owner));
    let generation = hub.providers[0].freshness.generation();
    hub.send(
        0,
        request("menu", key.0.clone(), key.1),
        Pending::Menu(0, generation, key.clone()),
    );
    let affected: Vec<_> = unsent.iter().copied().chain(in_flight).collect();
    hub.offline(0, &unsent, Some(&affected));
    assert!(
        hub.pending.contains_key(&3),
        "newly admitted activation lost its pending outcome"
    );
    assert!(!hub.pending.contains_key(&1));
    assert!(!hub.pending.contains_key(&2));
    let deadline = Deadline::start();
    while hub.pending.contains_key(&3) || hub.pending.contains_key(&4) {
        deadline.check("new request outcome was lost");
        hub.tick();
        nap();
    }
    assert_eq!(
        std::fs::read_to_string(&session.record).expect("activation record"),
        "3\n"
    );
    assert!(hub.providers[0].menus.contains_key(key));
}

#[test]
fn disconnect_does_not_retire_requests_admitted_after_the_failed_session() {
    install_hub();
    let session = FailingSession::new();
    with_hub(|hub| {
        session.start(hub);
        for _ in 0..10 {
            hub.invalidate(0);
        }
        let owner = glib::WeakRef::new();
        let failed = fail_the_session(hub, &session, &owner);
        let key = (vec!["/file".into()], false);
        later_requests_keep_their_outcomes(hub, &session, owner, &key, failed);
        hub.invalidate(0);
        assert_eq!(hub.menu(&key).len(), 1);
        let deadline = Deadline::start();
        while !session.menu_requested("5") {
            deadline.check("previous session generation suppressed the new menu invalidation");
            hub.tick();
            nap();
        }
    });
    clear_hub();
}
