// SPDX-License-Identifier: MIT
use super::*;

fn admit_tracked(slot: &ThumbnailSlot) -> bool {
    admit(slot, slot.provider_path().expect("path"))
}

#[test]
fn tracking_cap_eviction_clears_badges_and_allows_readmission() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::tracking::tracking_cap_eviction_clears_badges_and_allows_readmission",
        || {
            install_hub();
            let texture = black_pixel();
            let mut retired = Vec::new();
            for index in 0..1024 {
                let slot = ThumbnailSlot::new(16);
                slot.set_provider_path(Some(format!("/file-{index}")));
                slot.set_decoration(Some(texture.upcast_ref()), Some("Kept"));
                assert!(admit_tracked(&slot));
                retired.push(slot);
            }
            let current = ThumbnailSlot::new(16);
            current.set_provider_path(Some("/current".into()));
            assert!(admit_tracked(&current));
            for slot in &retired {
                assert!(slot.provider_path().is_some());
                assert!(
                    slot.decoration_description().is_none(),
                    "evicted badge retained its status"
                );
            }
            assert!(admit_tracked(&retired[0]));
            assert!(tracked(&retired[0]));
            unmap(&retired[0]);
            assert!(!tracked(&retired[0]));
            assert!(admit_tracked(&retired[0]));
            clear_hub();
        },
    );
}

fn settle_slots(condition: &dyn Fn() -> bool) {
    let deadline = Deadline::start();
    while !condition() {
        deadline.check("slot lifecycle did not settle");
        drain_main_context();
        nap();
    }
}

fn bound_slot(path: &str) -> ThumbnailSlot {
    let slot = ThumbnailSlot::new(16);
    bind(&slot, Some(Path::new(path)));
    slot
}

/// Tracking another window's slot clears the first window's slots; returning
/// re-admits them without restoring old status.
fn a_second_window_takes_over_tracking(first: &gtk::Window, slots: &[ThumbnailSlot]) {
    first.set_visible(false);
    with_hub(|hub| assert!(hub.slots.is_empty()));
    let other_slot = bound_slot("/other-window");
    let second = gtk::Window::builder().child(&other_slot).build();
    second.present();
    settle_slots(&|| tracked(&other_slot));
    second.destroy();
    slots[1024].set_visible(false);
    slots[0].set_visible(true);
    first.present();
    settle_slots(&|| slots[0].is_mapped());
    assert!(tracked(&slots[0]));
    assert!(slots[0].decoration_description().is_none());
}

#[test]
fn mapped_slots_refused_at_cap_retry_and_window_switches_clear_old_status() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::tracking::mapped_slots_refused_at_cap_retry_and_window_switches_clear_old_status",
        || {
            install_hub();
            let container = gtk::Fixed::new();
            let first = gtk::Window::builder().child(&container).build();
            let slots: Vec<_> = (0..1025)
                .map(|index| {
                    let slot = bound_slot(&format!("/file-{index}"));
                    container.put(&slot, 0.0, 0.0);
                    slot
                })
                .collect();
            first.present();
            settle_slots(&|| slots.iter().all(|slot| slot.is_mapped()));
            assert!(!tracked(&slots[1024]));
            slots[0].set_decoration(None, Some("Old status"));
            slots[0].set_visible(false);
            settle_slots(&|| tracked(&slots[1024]));
            assert!(slots[0].decoration_description().is_none());
            a_second_window_takes_over_tracking(&first, &slots);
            first.destroy();
            clear_hub();
        },
    );
}

/// Logs each query's paths into its file argument, invalidates, then answers.
const INVALIDATING_PROVIDER: &str = "import sys,json\nfor line in sys.stdin:\n r=json.loads(line)\n with open(sys.argv[1],'a') as f: f.write(json.dumps(r['paths'])+'\\n')\n print(json.dumps({'version':1,'event':'invalidate'}),flush=True)\n print(json.dumps({'version':1,'id':r['id'],'decorations':[]}),flush=True)";

fn queried_paths(record: &Path) -> std::collections::HashSet<String> {
    std::fs::read_to_string(record)
        .unwrap_or_default()
        .lines()
        .flat_map(|line| serde_json::from_str::<Vec<String>>(line).expect("recorded request"))
        .collect()
}

/// 200 slots in the first window and 5 in a second, all bound to provider paths.
fn two_windows_of_slots() -> (Vec<gtk::Window>, Vec<ThumbnailSlot>) {
    let containers = [gtk::Fixed::new(), gtk::Fixed::new()];
    let windows = containers
        .iter()
        .map(|container| gtk::Window::builder().child(container).build())
        .collect::<Vec<_>>();
    let slots = (0..205)
        .map(|index| {
            let slot = bound_slot(&format!("/file-{index:03}"));
            containers[usize::from(index >= 200)].put(&slot, 0.0, 0.0);
            slot
        })
        .collect::<Vec<_>>();
    (windows, slots)
}

#[test]
fn continuously_dirty_rows_do_not_starve_other_windows_or_later_batches() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::tracking::continuously_dirty_rows_do_not_starve_other_windows_or_later_batches",
        || {
            install_hub();
            let scratch = tempfile::tempdir().expect("provider fixture");
            let record = scratch.path().join("queries");
            with_hub(|hub| {
                hub.providers[0].client = protocol::start(scripted_registration(
                    None,
                    INVALIDATING_PROVIDER,
                    &[&record],
                ))
            });
            let (windows, slots) = two_windows_of_slots();
            for window in &windows {
                window.present();
            }
            let deadline = Deadline::start();
            while queried_paths(&record).len() != slots.len() {
                deadline.check("continuously invalidated first batch starved another window");
                drain_main_context();
                with_hub(Hub::tick);
                nap();
            }
            for window in windows {
                window.destroy();
            }
            clear_hub();
        },
    );
}

#[test]
fn hub_timer_stops_and_tracking_ends_once_discovery_finds_no_providers() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::tracking::hub_timer_stops_and_tracking_ends_once_discovery_finds_no_providers",
        || {
            let (sender, discovery) = mpsc::channel();
            HUB.with(|cell| cell.replace(Some(hub_with(discovery, false))));
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
            clear_hub();
        },
    );
}

#[test]
fn reload_replaces_providers_and_restarts_a_stopped_timer() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::tracking::reload_replaces_providers_and_restarts_a_stopped_timer",
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
                assert_eq!(
                    (
                        hub.providers.is_empty(),
                        hub.discovered,
                        hub.ticking,
                        hub.slots.len()
                    ),
                    (true, false, true, 1),
                    "(running providers are dropped, a new discovery is awaited, \
                     a stopped timer is started again, tracked rows stay tracked)"
                );
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
            clear_hub();
        },
    );
}
