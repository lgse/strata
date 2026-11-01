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

fn registration() -> Registration {
    Registration {
        manifest: protocol::Manifest {
            version: 1,
            id: "example".into(),
            name: Some("Example Cloud".into()),
            icon: None,
            command: vec![
                "/usr/bin/python3".into(),
                "-u".into(),
                "-c".into(),
                "import time;time.sleep(60)".into(),
            ],
            icons: Default::default(),
            category: None,
        },
        icons: Default::default(),
    }
}
fn install_hub() {
    let (_, discovery) = mpsc::channel();
    HUB.with(|cell| {
        cell.replace(Some(Hub {
            discovery,
            discovered: true,
            providers: vec![Provider {
                client: protocol::start(registration()),
                icons: Default::default(),
                id: "example".into(),
                name: "Example Cloud".into(),
                heading_icon: None,
                category: None,
                cache: Default::default(),
                menus: Default::default(),
                freshness: Freshness::default(),
                disconnected: false,
                last_busy: None,
            }],
            retiring: Vec::new(),
            slots: Default::default(),
            pending: Default::default(),
            serial: 0,
            ticking: true,
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
                icon: None,
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
                category: None,
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
        placement: None,
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
                            heading_icon: None,
                            category: None,
                            client: protocol::start(Registration {
                                manifest: protocol::Manifest {
                                    version: 1,
                                    id: id.into(),
                                    name,
                                    icon: None,
                                    command: vec![
                                        "/usr/bin/python3".into(),
                                        "-u".into(),
                                        "-c".into(),
                                        script.into(),
                                        records[index].to_str().expect("fixture path").into(),
                                    ],
                                    icons: Default::default(),
                                    category: None,
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
                (model.clone(), None),
                group.clone(),
                glib::WeakRef::new(),
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
                (model.clone(), None),
                group.clone(),
                glib::WeakRef::new(),
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
fn send_to_placement_fills_provider_submenus_or_falls_back_to_the_group() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::send_to_placement_fills_provider_submenus_or_falls_back_to_the_group",
        || {
            install_hub();
            with_hub(|hub| {
                hub.providers[0].category =
                    Some(crate::services::integration_catalog::Category::Devices)
            });
            let placed = |id: &str, label: &str, children| {
                let mut device = action(id, label, children);
                device.placement = Some(Placement::SendTo);
                device
            };
            let labels = |model: &gio::MenuModel| {
                (0..model.n_items())
                    .filter_map(|index| {
                        model
                            .item_attribute_value(index, "label", None)
                            .and_then(|value| value.get::<String>())
                    })
                    .collect::<Vec<_>>()
            };
            let group = gio::SimpleActionGroup::new();
            let model = gio::Menu::new();
            let send_to = gio::Menu::new();
            let mut renderer = menus::Renderer::new(
                (model.clone(), Some(send_to.clone())),
                group.clone(),
                glib::WeakRef::new(),
                glib::WeakRef::new(),
                (vec!["/file".into()], false),
                Rc::new(Cell::new(1)),
                1,
            );
            let mut pixel = placed("pixel", "Pixel", None);
            pixel.context = Some("pixel-incarnation".into());
            let tablet = placed(
                "tablet",
                "Tablet",
                Some(vec![placed("downloads", "Downloads", None)]),
            );
            assert!(renderer.update(vec![
                (0, action("share", "Copy link", None)),
                (0, pixel.clone()),
                (0, tablet),
            ]));
            assert_eq!(labels(model.upcast_ref()), ["Example Cloud"]);
            assert_eq!(
                labels(&model.item_link(0, "section").expect("group")),
                ["Copy link"]
            );
            assert_eq!(labels(send_to.upcast_ref()), ["Example Cloud"]);
            let icon = send_to
                .item_attribute_value(0, "icon", None)
                .and_then(|icon| gio::Icon::deserialize(&icon))
                .and_downcast::<gio::ThemedIcon>()
                .expect("provider submenu icon");
            assert_eq!(
                icon.names().first().map(|name| name.as_str()),
                Some(crate::assets::icons::SMARTPHONE),
                "a provider without artwork is drawn with its category icon"
            );
            let devices = send_to
                .item_link(0, "submenu")
                .expect("provider submenu inside Send to…");
            assert_eq!(labels(&devices), ["Pixel", "Tablet"]);
            assert_eq!(
                devices
                    .item_attribute_value(0, "x-strata-send-to-device", None)
                    .and_then(|value| value.get::<bool>()),
                Some(true)
            );
            assert_eq!(
                devices
                    .item_attribute_value(0, "x-strata-description", None)
                    .and_then(|value| value.get::<String>())
                    .as_deref(),
                Some("Pixel")
            );
            let downloads = devices.item_link(1, "submenu").expect("device submenu");
            assert_eq!(labels(&downloads), ["Downloads"]);
            assert!(
                downloads
                    .item_attribute_value(0, "x-strata-send-to-device", None)
                    .is_none(),
                "placement below the root is ignored"
            );
            assert!(group.lookup_action("action-0-pixel").is_some());
            assert!(group.lookup_action("action-0-downloads").is_some());
            assert!(group.lookup_action("action-0-tablet").is_none());

            let old_pixel = group.lookup_action("action-0-pixel").expect("device leaf");
            assert!(renderer.update(vec![(0, action("share", "Copy link", None))]));
            assert_eq!(
                send_to.n_items(),
                0,
                "withdrawn devices remove their submenu"
            );
            assert!(!old_pixel.is_enabled());
            assert!(group.lookup_action("action-0-pixel").is_none());
            assert!(group.lookup_action("action-0-share").is_some());
            assert_eq!(model.n_items(), 1);

            assert!(renderer.update(vec![(0, pixel.clone())]));
            assert_eq!(
                model.n_items(),
                0,
                "a provider with only devices has no group"
            );
            assert_eq!(labels(send_to.upcast_ref()), ["Example Cloud"]);
            drop(renderer);
            assert!(group.list_actions().is_empty());
            send_to.remove_all();

            let mut without_send_to = menus::Renderer::new(
                (model.clone(), None),
                group.clone(),
                glib::WeakRef::new(),
                glib::WeakRef::new(),
                (vec!["/file".into()], true),
                Rc::new(Cell::new(1)),
                1,
            );
            without_send_to.update(vec![(0, pixel)]);
            let section = model.item_link(0, "section").expect("fallback group");
            assert_eq!(labels(&section), ["Pixel"]);
            assert!(
                section
                    .item_attribute_value(0, "x-strata-send-to-device", None)
                    .is_none(),
                "without Send to… the device is an ordinary group action"
            );
            assert_eq!(send_to.n_items(), 0);
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn send_to_entry_is_a_single_item_only_while_it_is_the_providers_only_destination() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::send_to_entry_is_a_single_item_only_while_it_is_the_providers_only_destination",
        || {
            install_hub();
            with_hub(|hub| {
                hub.providers[0].category =
                    Some(crate::services::integration_catalog::Category::Devices)
            });
            let placed = |id: &str, label: &str, placement, children| {
                let mut action = action(id, label, children);
                action.placement = Some(placement);
                action
            };
            let string = |model: &gio::MenuModel, index: i32, attribute: &str| {
                model
                    .item_attribute_value(index, attribute, None)
                    .and_then(|value| value.get::<String>())
            };
            let group = gio::SimpleActionGroup::new();
            let model = gio::Menu::new();
            let send_to = gio::Menu::new();
            let mut renderer = menus::Renderer::new(
                (model.clone(), Some(send_to.clone())),
                group.clone(),
                glib::WeakRef::new(),
                glib::WeakRef::new(),
                (vec!["/file".into()], false),
                Rc::new(Cell::new(1)),
                1,
            );
            let open = placed("open", "Open", Placement::SendToEntry, None);
            assert!(renderer.update(vec![(0, open.clone())]));
            assert_eq!(model.n_items(), 0);
            assert_eq!(send_to.n_items(), 1);
            assert_eq!(
                string(send_to.upcast_ref(), 0, "label").as_deref(),
                Some("Open")
            );
            assert_eq!(
                string(send_to.upcast_ref(), 0, "action").as_deref(),
                Some("provider.action-0-open"),
                "the entry activates directly"
            );
            assert!(send_to.item_link(0, "submenu").is_none());
            assert_eq!(
                send_to
                    .item_attribute_value(0, "x-strata-send-to-device", None)
                    .and_then(|value| value.get::<bool>()),
                Some(true)
            );
            let icon = send_to
                .item_attribute_value(0, "icon", None)
                .and_then(|icon| gio::Icon::deserialize(&icon))
                .and_downcast::<gio::ThemedIcon>()
                .expect("entry icon");
            assert_eq!(
                icon.names().first().map(|name| name.as_str()),
                Some(crate::assets::icons::SMARTPHONE),
                "an entry without its own icon is drawn like the provider's submenu"
            );
            let leaf = group.lookup_action("action-0-open").expect("entry leaf");

            assert!(renderer.update(vec![(
                0,
                placed("open", "LocalSend", Placement::SendToEntry, None)
            )]));
            assert_eq!(
                string(send_to.upcast_ref(), 0, "label").as_deref(),
                Some("LocalSend")
            );
            assert_eq!(group.lookup_action("action-0-open"), Some(leaf.clone()));
            assert!(leaf.is_enabled());

            let pixel = placed("pixel", "Pixel", Placement::SendTo, None);
            assert!(renderer.update(vec![(0, open.clone()), (0, pixel.clone())]));
            assert_eq!(send_to.n_items(), 1);
            assert_eq!(
                string(send_to.upcast_ref(), 0, "label").as_deref(),
                Some("Example Cloud"),
                "several destinations fall back to the provider submenu"
            );
            let submenu = send_to.item_link(0, "submenu").expect("provider submenu");
            assert_eq!(string(&submenu, 0, "label").as_deref(), Some("Open"));
            assert_eq!(string(&submenu, 1, "label").as_deref(), Some("Pixel"));

            for single in [
                pixel,
                placed(
                    "open",
                    "Open",
                    Placement::SendToEntry,
                    Some(vec![action("there", "There", None)]),
                ),
            ] {
                renderer.update(vec![(0, single)]);
                assert_eq!(
                    string(send_to.upcast_ref(), 0, "label").as_deref(),
                    Some("Example Cloud"),
                    "one destination or branch stays in the submenu"
                );
                assert!(send_to.item_link(0, "submenu").is_some());
            }

            assert!(renderer.update(vec![(0, open)]));
            assert_eq!(
                string(send_to.upcast_ref(), 0, "label").as_deref(),
                Some("Open")
            );
            assert!(send_to.item_link(0, "submenu").is_none());
            assert!(renderer.update(Vec::new()));
            assert_eq!(send_to.n_items(), 0);
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
                    (model.clone(), None),
                    group.clone(),
                    glib::WeakRef::new(),
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
                        icon: None,
                        command: vec![
                            "/usr/bin/python3".into(),
                            "-u".into(),
                            "-c".into(),
                            script.into(),
                            record.to_str().expect("fixture path").into(),
                        ],
                        icons: Default::default(),
                        category: None,
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

fn start_scripted_provider(hub: &mut Hub, script: &str) {
    hub.providers[0].client = protocol::start(Registration {
        manifest: protocol::Manifest {
            version: 1,
            id: "example".into(),
            name: Some("Example Cloud".into()),
            icon: None,
            command: vec![
                "/usr/bin/python3".into(),
                "-u".into(),
                "-c".into(),
                script.into(),
            ],
            icons: Default::default(),
            category: None,
        },
        icons: Default::default(),
    });
}

fn overlay_layers(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    let mut layers = Vec::new();
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if overlay.child().as_ref() != Some(&widget) {
            layers.push(widget);
        }
    }
    layers
}

#[test]
fn activation_feedback_routes_success_to_notice_and_problems_to_dialog() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::activation_feedback_routes_success_to_notice_and_problems_to_dialog",
        || {
            install_hub();
            let script = r#"import json,sys
replies={
 'notice':{'outcome':{'status':'accepted','accepted':1,'total':1},'feedback':'notice'},
 'quiet':{'outcome':{'status':'accepted','accepted':1,'total':1},'feedback':'none'},
 'legacy':{},
 'rejected':{'outcome':{'status':'rejected','accepted':0,'total':1},'feedback':'notice'},
}
for line in sys.stdin:
 r=json.loads(line)
 print(json.dumps({'version':1,'id':r['id'],'message':'Link copied',**replies[r['action']]}),flush=True)
"#;
            with_hub(|hub| start_scripted_provider(hub, script));
            for (action, notice, dialog) in [
                ("notice", true, false),
                ("quiet", false, false),
                ("legacy", false, true),
                ("rejected", false, true),
            ] {
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&gtk::Label::new(None)));
                let owner: gtk::Widget = overlay.clone().upcast();
                with_hub(|hub| {
                    let mut activation = request("activate", vec!["/file".into()], false);
                    activation.action = Some(action.into());
                    hub.send(0, activation, Pending::Activate(0, owner.downgrade()));
                });
                let deadline = Instant::now() + Duration::from_secs(5);
                while with_hub(|hub| !hub.pending.is_empty()) {
                    assert!(Instant::now() < deadline, "{action} reply was not handled");
                    with_hub(Hub::tick);
                    std::thread::sleep(Duration::from_millis(5));
                }
                let (docks, dialogs): (Vec<_>, Vec<_>) = overlay_layers(&overlay)
                    .into_iter()
                    .partition(|layer| layer.has_css_class("file-operation-dock"));
                let notices = docks
                    .iter()
                    .filter_map(|dock| dock.first_child()?.first_child()?.first_child())
                    .filter(|row| row.has_css_class("overlay-notice"))
                    .count();
                assert_eq!(notices, usize::from(notice), "{action} notice");
                assert_eq!(dialogs.len(), usize::from(dialog), "{action} dialog");
            }
        },
    );
}

#[test]
fn a_reload_still_shows_the_outcome_of_a_pending_activation() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::a_reload_still_shows_the_outcome_of_a_pending_activation",
        || {
            for (action, notice, dialog) in [("reply", true, false), ("exit", false, true)] {
                install_hub();
                let fixture = tempfile::tempdir().expect("fixture");
                let release = fixture.path().join("release");
                let script = format!(
                    r#"import json,os,sys,time
for line in sys.stdin:
 r=json.loads(line)
 while not os.path.exists({release:?}): time.sleep(0.01)
 if r['action']=='exit': sys.exit()
 print(json.dumps({{'version':1,'id':r['id'],'message':'Link copied','outcome':{{'status':'accepted','accepted':1,'total':1}},'feedback':'notice'}}),flush=True)
"#
                );
                with_hub(|hub| start_scripted_provider(hub, &script));
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&gtk::Label::new(None)));
                let owner: gtk::Widget = overlay.clone().upcast();
                with_hub(|hub| {
                    let mut activation = request("activate", vec!["/file".into()], false);
                    activation.action = Some(action.into());
                    hub.send(0, activation, Pending::Activate(0, owner.downgrade()));
                });
                let (sender, discovery) = mpsc::channel();
                with_hub(|hub| hub.reload(discovery));
                sender.send(Vec::new()).expect("discovery result");
                std::fs::write(&release, "go").expect("release provider");

                let deadline = Instant::now() + Duration::from_secs(5);
                while tick_hub() == glib::ControlFlow::Continue {
                    assert!(
                        Instant::now() < deadline,
                        "{action}: the pending activation was not settled after a reload"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                let (docks, dialogs): (Vec<_>, Vec<_>) = overlay_layers(&overlay)
                    .into_iter()
                    .partition(|layer| layer.has_css_class("file-operation-dock"));
                let notices = docks
                    .iter()
                    .filter_map(|dock| dock.first_child()?.first_child()?.first_child())
                    .filter(|row| row.has_css_class("overlay-notice"))
                    .count();
                assert_eq!(notices, usize::from(notice), "{action} notice");
                assert_eq!(dialogs.len(), usize::from(dialog), "{action} dialog");
                HUB.with(|cell| cell.replace(None));
            }
        },
    );
}

#[test]
fn provider_icon_heading_keeps_the_name_and_action_positions() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::provider_icon_heading_keeps_the_name_and_action_positions",
        || {
            install_hub();
            let texture = gdk::MemoryTexture::new(
                1,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_static(&[0, 0, 0, 255]),
                4,
            );
            with_hub(|hub| hub.providers[0].heading_icon = Some(texture.upcast()));
            let model = gio::Menu::new();
            let popover = gtk::PopoverMenu::from_model(Some(&model));
            let group = gio::SimpleActionGroup::new();
            popover.insert_action_group("provider", Some(&group));
            let mut renderer = menus::Renderer::new(
                (model.clone(), None),
                group.clone(),
                glib::WeakRef::new(),
                popover.downgrade(),
                (vec!["/file".into()], false),
                Rc::new(Cell::new(1)),
                1,
            );
            let labels = |section: &gio::MenuModel| {
                (0..section.n_items())
                    .filter_map(|index| {
                        section
                            .item_attribute_value(index, "label", None)
                            .and_then(|value| value.get::<String>())
                    })
                    .collect::<Vec<_>>()
            };
            renderer.update(vec![
                (0, action("keep", "Keep offline", None)),
                (0, action("share", "Share", None)),
            ]);
            let section = model.item_link(0, "section").expect("provider section");
            assert_eq!(labels(&section), ["Keep offline", "Share"]);
            renderer.update(vec![
                (0, action("share", "Share", None)),
                (0, action("link", "Copy link", None)),
            ]);
            assert_eq!(labels(&section), ["Share", "Copy link"]);
            assert!(group.lookup_action("action-0-keep").is_none());

            let mut heading = None;
            let mut stack = vec![popover.clone().upcast::<gtk::Widget>()];
            while let Some(widget) = stack.pop() {
                if widget.has_css_class("provider-heading") {
                    heading = Some(widget.clone());
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    child = next.next_sibling();
                    stack.push(next);
                }
            }
            let heading = heading.expect("provider heading row");
            let image = heading.first_child().expect("heading icon");
            assert!(image.downcast_ref::<gtk::Image>().is_some());
            let label = image.next_sibling().and_downcast::<gtk::Label>();
            assert_eq!(label.expect("heading label").label(), "Example Cloud");
        },
    );
}

#[test]
fn hub_timer_stops_and_tracking_ends_once_discovery_finds_no_providers() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::hub_timer_stops_and_tracking_ends_once_discovery_finds_no_providers",
        || {
            let (sender, discovery) = mpsc::channel();
            HUB.with(|cell| {
                cell.replace(Some(Hub {
                    discovery,
                    discovered: false,
                    providers: Vec::new(),
                    retiring: Vec::new(),
                    slots: Default::default(),
                    pending: Default::default(),
                    serial: 0,
                    ticking: true,
                }))
            });
            let early = ThumbnailSlot::new(16);
            early.set_provider_path(Some("/early".into()));
            assert!(admit(&early, "/early".into()));
            assert_eq!(tick_hub(), glib::ControlFlow::Continue);
            with_hub(|hub| assert_eq!(hub.slots.len(), 1));

            sender.send(Vec::new()).expect("discovery result");
            assert_eq!(tick_hub(), glib::ControlFlow::Break);
            let late = ThumbnailSlot::new(16);
            late.set_provider_path(Some("/late".into()));
            assert!(admit(&late, "/late".into()));
            with_hub(|hub| assert!(hub.slots.is_empty()));

            install_hub();
            assert_eq!(tick_hub(), glib::ControlFlow::Continue);
            HUB.with(|cell| cell.replace(None));
        },
    );
}

#[test]
fn reload_replaces_providers_and_restarts_a_stopped_timer() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::reload_replaces_providers_and_restarts_a_stopped_timer",
        || {
            install_hub();
            let slot = ThumbnailSlot::new(16);
            slot.set_provider_path(Some("/tracked".into()));
            assert!(admit(&slot, "/tracked".into()));
            slot.set_decoration(None, Some("Available offline"));
            with_hub(|hub| hub.ticking = false);

            let (sender, discovery) = mpsc::channel();
            reload_with(discovery);
            with_hub(|hub| {
                assert!(hub.providers.is_empty(), "running providers are dropped");
                assert!(!hub.discovered, "a new discovery is awaited");
                assert!(hub.ticking, "a stopped timer is started again");
                assert_eq!(hub.slots.len(), 1, "tracked rows stay tracked");
            });
            assert!(
                slot.decoration_description().is_none(),
                "old badges are cleared"
            );
            assert_eq!(tick_hub(), glib::ControlFlow::Continue);

            sender.send(vec![registration()]).expect("discovery result");
            assert_eq!(tick_hub(), glib::ControlFlow::Continue);
            with_hub(|hub| assert_eq!(hub.providers.len(), 1));

            let (sender, discovery) = mpsc::channel();
            reload_with(discovery);
            sender.send(Vec::new()).expect("discovery result");
            assert_eq!(tick_hub(), glib::ControlFlow::Break);
            with_hub(|hub| assert!(!hub.ticking));
            HUB.with(|cell| cell.replace(None));
        },
    );
}
