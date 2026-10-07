// SPDX-License-Identifier: MIT
use super::*;
use std::{cell::Cell, rc::Rc};

#[test]
fn scoped_revisions_make_progress_under_continuous_unrelated_activity() {
    let mut freshness = Freshness::default();
    let selected = vec!["/account-a/folder/file".into()];
    for revision in 1..=100 {
        freshness.invalidate(Some(&["/account-b".into()]), Some(revision));
        assert!(freshness.accept(&selected, Some(0)));
        assert!(freshness.current(&selected, 0));
    }
    freshness.invalidate(Some(&["/account-a/folder".into()]), Some(101));
    assert!(!freshness.accept(&selected, Some(100)));
    assert!(freshness.accept(&selected, Some(101)));
    assert!(!freshness.current(&selected, 0));
    assert!(freshness.current(&selected, freshness.generation()));
    assert!(freshness.current(&["/account-a/folder-other".into()], 0));
    assert!(!freshness.current(&["/account-a".into()], 0));
}

#[test]
fn legacy_snapshot_replies_survive_refresh_hints_without_claiming_freshness() {
    let mut freshness = Freshness::default();
    let selected = vec!["/file".into()];
    for _ in 0..100 {
        let generation = freshness.generation();
        freshness.invalidate(None, None);
        assert!(freshness.accept(&selected, None));
        assert!(!freshness.current(&selected, generation));
    }
    freshness.invalidate(None, Some(101));
    assert!(!freshness.accept(&selected, None));
    assert!(!freshness.accept(&selected, Some(100)));
    assert!(freshness.accept(&selected, Some(101)));
}

fn install_hub() {
    let (_, discovery) = mpsc::channel();
    let registration = Registration {
        manifest: protocol::Manifest {
            version: 1,
            id: "example".into(),
            name: Some("Example Cloud".into()),
            command: vec![
                "/usr/bin/python3".into(),
                "-u".into(),
                "-c".into(),
                "import time;time.sleep(60)".into(),
            ],
            icons: Default::default(),
        },
        icons: Default::default(),
    };
    HUB.with(|cell| {
        cell.replace(Some(Hub {
            discovery,
            providers: vec![Provider {
                client: protocol::start(registration),
                icons: Default::default(),
                id: "example".into(),
                name: "Example Cloud".into(),
                cache: Default::default(),
                menus: Default::default(),
                freshness: Freshness::default(),
                disconnected: false,
                last_busy: None,
            }],
            slots: Default::default(),
            pending: Default::default(),
            serial: 0,
        }))
    });
}

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
    HUB.with(|cell| cell.replace(None));
}

#[test]
fn disconnect_does_not_retire_requests_admitted_after_the_failed_session() {
    install_hub();
    let scratch = tempfile::tempdir().expect("provider fixture");
    let ready = scratch.path().join("ready");
    let release = scratch.path().join("release");
    let record = scratch.path().join("activations");
    let script = r#"import json,sys,time
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
    with_hub(|hub| {
        hub.providers[0].client = protocol::start(Registration {
            manifest: protocol::Manifest {
                version: 1,
                id: "example".into(),
                name: None,
                command: vec![
                    "/usr/bin/python3".into(),
                    "-u".into(),
                    "-c".into(),
                    script.into(),
                    ready.to_str().expect("fixture path").into(),
                    release.to_str().expect("fixture path").into(),
                    record.to_str().expect("fixture path").into(),
                ],
                icons: Default::default(),
            },
            icons: Default::default(),
        });
        for _ in 0..10 {
            hub.invalidate(0);
        }
        let mut activation = request("activate", vec!["/file".into()], false);
        activation.action = Some("keep".into());
        let owner = glib::WeakRef::new();
        hub.send(0, activation.clone(), Pending::Activate(0, owner.clone()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "provider did not receive activation"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        hub.send(0, activation.clone(), Pending::Activate(0, owner.clone()));
        std::fs::write(&release, "go").expect("release provider");
        let Update::Offline { unsent, in_flight } = hub.providers[0]
            .client
            .updates
            .recv_timeout(Duration::from_secs(5))
            .expect("disconnect update")
        else {
            panic!("expected failed session")
        };
        assert_eq!(in_flight, Some(1));
        assert_eq!(unsent, [2]);
        hub.send(0, activation, Pending::Activate(0, owner));
        let key = (vec!["/file".into()], false);
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
        let deadline = Instant::now() + Duration::from_secs(5);
        while hub.pending.contains_key(&3) || hub.pending.contains_key(&4) {
            assert!(Instant::now() < deadline, "new request outcome was lost");
            hub.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            std::fs::read_to_string(&record).expect("activation record"),
            "3\n"
        );
        assert!(hub.providers[0].menus.contains_key(&key));
        hub.invalidate(0);
        assert_eq!(hub.menu(&key).len(), 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !std::fs::read_to_string(&ready)
            .is_ok_and(|requests| requests.lines().any(|id| id == "5"))
        {
            assert!(
                Instant::now() < deadline,
                "previous session generation suppressed the new menu invalidation"
            );
            hub.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    HUB.with(|cell| cell.replace(None));
}

fn action(id: &str, label: &str, children: Option<Vec<MenuAction>>) -> MenuAction {
    MenuAction {
        id: id.into(),
        label: label.into(),
        icon: None,
        context: None,
        children,
    }
}

#[test]
fn provider_sections_scope_duplicate_labels_and_retire_empty_groups() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::provider_sections_scope_duplicate_labels_and_retire_empty_groups",
        || {
            install_hub();
            let scratch = tempfile::tempdir().expect("provider fixture");
            let records = [scratch.path().join("example"), scratch.path().join("beta")];
            let script = "import json,sys\nfrom pathlib import Path\nfor line in sys.stdin:\n r=json.loads(line)\n Path(sys.argv[1]).write_text(json.dumps(r))\n print(json.dumps({'version':1,'id':r['id'],'message':'Accepted'}),flush=True)";
            with_hub(|hub| {
                hub.providers = ["example", "beta"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| {
                        let name = (index == 0).then(|| "Example Cloud".to_owned());
                        Provider {
                            id: id.into(),
                            name: name.clone().unwrap_or_else(|| id.into()),
                            client: protocol::start(Registration {
                                manifest: protocol::Manifest {
                                    version: 1,
                                    id: id.into(),
                                    name,
                                    command: vec![
                                        "/usr/bin/python3".into(),
                                        "-u".into(),
                                        "-c".into(),
                                        script.into(),
                                        records[index].to_str().expect("fixture path").into(),
                                    ],
                                    icons: Default::default(),
                                },
                                icons: Default::default(),
                            }),
                            icons: Default::default(),
                            cache: Default::default(),
                            menus: Default::default(),
                            freshness: Freshness::default(),
                            disconnected: false,
                            last_busy: None,
                        }
                    })
                    .collect();
            });
            let model = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let mut renderer = menus::Renderer::new(
                model.clone(),
                group.clone(),
                glib::WeakRef::new(),
                (vec!["/file".into()], false),
                Rc::new(Cell::new(1)),
                1,
            );
            let mut example = action("keep", "Keep offline", None);
            example.context = Some("example-incarnation".into());
            let mut beta = example.clone();
            beta.context = Some("beta-incarnation".into());
            let more = action("status", "Show availability", None);
            renderer.update(vec![(1, beta), (0, example), (1, more.clone())]);
            let section = |index| model.item_link(index, "section").expect("provider section");
            let label = |model: &gio::MenuModel, index| {
                model
                    .item_attribute_value(index, "label", None)
                    .and_then(|value| value.get::<String>())
                    .expect("label")
            };
            let first = section(0);
            let second = section(1);
            assert_eq!(label(model.upcast_ref(), 0), "Example Cloud");
            assert_eq!(label(model.upcast_ref(), 1), "beta");
            assert_eq!(label(&first, 0), "Keep offline");
            assert_eq!(label(&second, 0), "Keep offline");
            assert_eq!(label(&second, 1), "Show availability");
            let old_example = group
                .lookup_action("action-0-keep")
                .expect("example action");
            group.activate_action("action-0-keep", None);
            group.activate_action("action-1-keep", None);
            for (index, record) in records.iter().enumerate() {
                let deadline = Instant::now() + Duration::from_secs(5);
                let received: serde_json::Value = loop {
                    if let Some(received) = std::fs::read(record)
                        .ok()
                        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    {
                        break received;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "provider did not receive its action"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                };
                assert_eq!(received["action"], "keep");
                assert_eq!(received["paths"], serde_json::json!(["/file"]));
                assert_eq!(
                    received["context"],
                    ["example-incarnation", "beta-incarnation"][index]
                );
            }
            renderer.update(vec![(1, more)]);
            assert_eq!(section(0), second);
            assert_eq!(model.n_items(), 1);
            assert!(!old_example.is_enabled());
            assert!(group.lookup_action("action-0-keep").is_none());
            assert!(group.lookup_action("action-1-keep").is_none());
            assert!(group.lookup_action("action-1-status").is_some());
            renderer.update(Vec::new());
            assert_eq!(model.n_items(), 0);
            assert!(group.list_actions().is_empty());
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn nested_refresh_retains_navigation_model_and_withdraws_old_leaf_actions() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::nested_refresh_retains_navigation_model_and_withdraws_old_leaf_actions",
        || {
            install_hub();
            let model = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let mut renderer = menus::Renderer::new(
                model.clone(),
                group.clone(),
                glib::WeakRef::new(),
                (vec!["/file".into()], false),
                Rc::new(Cell::new(1)),
                1,
            );
            let keep = action("keep", "Keep offline", None);
            let availability = action("availability", "Availability", Some(vec![keep]));
            renderer.update(vec![
                (0, action("share", "Share", None)),
                (0, availability.clone()),
            ]);
            let section = model.item_link(0, "section").expect("provider section");
            let submenu = section.item_link(1, "submenu").expect("submenu");
            assert!(group.lookup_action("action-0-availability").is_none());
            let old_keep = group.lookup_action("action-0-keep").expect("leaf");
            renderer.update(vec![(
                0,
                action(
                    "availability",
                    "Availability",
                    Some(vec![action("release", "Release", None)]),
                ),
            )]);
            assert_eq!(model.item_link(0, "section"), Some(section.clone()));
            assert_eq!(section.item_link(0, "submenu"), Some(submenu));
            assert!(!old_keep.is_enabled());
            assert!(group.lookup_action("action-0-keep").is_none());
            assert!(group.lookup_action("action-0-release").is_some());
            renderer.update(Vec::new());
            assert!(group.list_actions().is_empty());
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn retired_menu_does_not_remove_replacement_menu_actions() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::retired_menu_does_not_remove_replacement_menu_actions",
        || {
            install_hub();
            let group = gio::SimpleActionGroup::new();
            let model = gio::Menu::new();
            let alive = Rc::new(Cell::new(1));
            let make = |epoch| {
                menus::Renderer::new(
                    model.clone(),
                    group.clone(),
                    glib::WeakRef::new(),
                    (vec!["/file".into()], false),
                    alive.clone(),
                    epoch,
                )
            };
            let mut old = make(1);
            old.update(vec![(0, action("keep", "Keep", None))]);
            let old_action = group.lookup_action("action-0-keep").expect("old action");
            alive.set(2);
            model.remove_all();
            group.remove_action("action-0-keep");
            let mut new = make(2);
            new.update(vec![(0, action("keep", "Keep", None))]);
            let new_action = group.lookup_action("action-0-keep").expect("new action");
            drop(old);
            assert!(!old_action.is_enabled());
            assert_eq!(
                group.lookup_action("action-0-keep"),
                Some(new_action.clone())
            );
            assert!(new_action.is_enabled());
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn tracking_cap_eviction_clears_badges_and_allows_readmission() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::tracking_cap_eviction_clears_badges_and_allows_readmission",
        || {
            install_hub();
            let texture = gdk::MemoryTexture::new(
                1,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(&[0u8, 0, 0, 255][..]),
                4,
            );
            let mut retired = Vec::new();
            for index in 0..1024 {
                let slot = ThumbnailSlot::new(16);
                slot.set_provider_path(Some(format!("/file-{index}")));
                slot.set_decoration(Some(texture.upcast_ref()), Some("Kept"));
                assert!(admit(&slot, slot.provider_path().expect("path")));
                retired.push(slot);
            }
            let current = ThumbnailSlot::new(16);
            current.set_provider_path(Some("/current".into()));
            assert!(admit(&current, current.provider_path().expect("path")));
            for slot in &retired {
                assert!(slot.provider_path().is_some());
                assert!(
                    slot.decoration_description().is_none(),
                    "evicted badge retained its status"
                );
            }
            assert!(admit(
                &retired[0],
                retired[0].provider_path().expect("path")
            ));
            with_hub(|hub| assert!(hub.slots.contains_key(&(retired[0].as_ptr() as usize))));
            unmap(&retired[0]);
            with_hub(|hub| assert!(!hub.slots.contains_key(&(retired[0].as_ptr() as usize))));
            assert!(admit(
                &retired[0],
                retired[0].provider_path().expect("path")
            ));
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn mapped_slots_refused_at_cap_retry_and_window_switches_clear_old_status() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::mapped_slots_refused_at_cap_retry_and_window_switches_clear_old_status",
        || {
            install_hub();
            let container = gtk::Fixed::new();
            let first = gtk::Window::builder().child(&container).build();
            let slots: Vec<_> = (0..1025)
                .map(|index| {
                    let slot = ThumbnailSlot::new(16);
                    bind(&slot, Some(Path::new(&format!("/file-{index}"))));
                    container.put(&slot, 0.0, 0.0);
                    slot
                })
                .collect();
            first.present();
            let wait = |condition: &dyn Fn() -> bool| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !condition() {
                    assert!(Instant::now() < deadline, "slot lifecycle did not settle");
                    let main = glib::MainContext::default();
                    while main.pending() {
                        main.iteration(false);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            };
            wait(&|| slots.iter().all(|slot| slot.is_mapped()));
            with_hub(|hub| assert!(!hub.slots.contains_key(&(slots[1024].as_ptr() as usize))));
            slots[0].set_decoration(None, Some("Old status"));
            slots[0].set_visible(false);
            wait(&|| with_hub(|hub| hub.slots.contains_key(&(slots[1024].as_ptr() as usize))));
            assert!(slots[0].decoration_description().is_none());
            first.set_visible(false);
            with_hub(|hub| assert!(hub.slots.is_empty()));
            let other_slot = ThumbnailSlot::new(16);
            bind(&other_slot, Some(Path::new("/other-window")));
            let second = gtk::Window::builder().child(&other_slot).build();
            second.present();
            wait(&|| with_hub(|hub| hub.slots.contains_key(&(other_slot.as_ptr() as usize))));
            second.destroy();
            slots[1024].set_visible(false);
            slots[0].set_visible(true);
            first.present();
            wait(&|| slots[0].is_mapped());
            with_hub(|hub| assert!(hub.slots.contains_key(&(slots[0].as_ptr() as usize))));
            assert!(slots[0].decoration_description().is_none());
            first.destroy();
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn continuously_dirty_rows_do_not_starve_other_windows_or_later_batches() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::continuously_dirty_rows_do_not_starve_other_windows_or_later_batches",
        || {
            install_hub();
            let scratch = tempfile::tempdir().expect("provider fixture");
            let record = scratch.path().join("queries");
            let script = "import sys,json\nfor line in sys.stdin:\n r=json.loads(line)\n with open(sys.argv[1],'a') as f: f.write(json.dumps(r['paths'])+'\\n')\n print(json.dumps({'version':1,'event':'invalidate'}),flush=True)\n print(json.dumps({'version':1,'id':r['id'],'decorations':[]}),flush=True)";
            with_hub(|hub| {
                hub.providers[0].client = protocol::start(Registration {
                    manifest: protocol::Manifest {
                        version: 1,
                        id: "example".into(),
                        name: None,
                        command: vec![
                            "/usr/bin/python3".into(),
                            "-u".into(),
                            "-c".into(),
                            script.into(),
                            record.to_str().expect("fixture path").into(),
                        ],
                        icons: Default::default(),
                    },
                    icons: Default::default(),
                })
            });
            let containers = [gtk::Fixed::new(), gtk::Fixed::new()];
            let windows = containers
                .iter()
                .map(|container| gtk::Window::builder().child(container).build())
                .collect::<Vec<_>>();
            let slots = (0..205)
                .map(|index| {
                    let slot = ThumbnailSlot::new(16);
                    bind(&slot, Some(Path::new(&format!("/file-{index:03}"))));
                    containers[usize::from(index >= 200)].put(&slot, 0.0, 0.0);
                    slot
                })
                .collect::<Vec<_>>();
            for window in &windows {
                window.present();
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let seen = std::fs::read_to_string(&record)
                    .unwrap_or_default()
                    .lines()
                    .flat_map(|line| {
                        serde_json::from_str::<Vec<String>>(line).expect("recorded request")
                    })
                    .collect::<std::collections::HashSet<_>>();
                if seen.len() == slots.len() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "continuously invalidated first batch starved another window"
                );
                let main = glib::MainContext::default();
                while main.pending() {
                    main.iteration(false);
                }
                with_hub(Hub::tick);
                std::thread::sleep(Duration::from_millis(5));
            }
            for window in windows {
                window.destroy();
            }
            HUB.with(|cell| cell.replace(None));
        },
    );
}
