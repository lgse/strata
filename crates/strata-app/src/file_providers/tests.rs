// SPDX-License-Identifier: MIT

use std::{collections::HashMap, time::Instant};

use rune::Pixels;
use strata_components::{
    context_menu::{ContextMenuState, Item, MenuModel, MenuTarget, Section},
    contract::{EntryId, EntrySource, Point},
};
use strata_fs::services::file_providers::{
    self as protocol, Decoration, Feedback, Manifest, MenuAction, OutcomeStatus, Registration,
    Reply,
};

use super::{
    FileProviders,
    freshness::Freshness,
    hub::{
        Answer, Effect, Hub, Pending, Provider, activation_effect, activation_summary,
        disconnected_message,
    },
    menus::{OpenMenu, build_sections_and_leaves, splice_provider_sections},
};

fn dummy_provider(id: &str, name: &str) -> Provider {
    let registration = Registration {
        manifest: Manifest {
            version: 1,
            id: id.to_owned(),
            name: Some(name.to_owned()),
            icon: None,
            command: vec!["/bin/true".to_owned()],
            icons: Default::default(),
            category: None,
        },
        icons: Default::default(),
    };
    let mut provider = Provider::start(registration);
    provider.icons.insert("badge".to_owned(), Pixels::new());
    provider
}

fn action(
    id: &str,
    label: &str,
    context: Option<&str>,
    children: Option<Vec<MenuAction>>,
) -> MenuAction {
    MenuAction {
        id: id.to_owned(),
        label: label.to_owned(),
        icon: None,
        context: context.map(str::to_owned),
        children,
        placement: None,
    }
}

fn nested_fixture(token: &str) -> Vec<(usize, MenuAction)> {
    vec![
        (0, action("inspect", "Inspect test state", None, None)),
        (
            0,
            action(
                "availability",
                "Offline availability",
                None,
                Some(vec![
                    action("keep", "Keep test pin", Some(token), None),
                    action(
                        "details",
                        "Details",
                        None,
                        Some(vec![action("info", "Inspect nested state", None, None)]),
                    ),
                ]),
            ),
        ),
    ]
}

fn open_fixture_menu(providers: &[Provider]) -> (OpenMenu, ContextMenuState) {
    let key = (vec!["/work/todo.txt".to_owned()], false);
    let mut open_menu = OpenMenu::default();
    let sections = open_menu.begin(
        Some(key),
        build_sections_and_leaves(providers, nested_fixture("old-token")),
    );
    let mut model = MenuModel::new()
        .section(Section::new([Item::new("open", "Open")]))
        .section(Section::new([Item::new("cut", "Cut")]));
    splice_provider_sections(&mut model, 1, sections);
    let mut state = ContextMenuState::new();
    state.open(
        MenuTarget::Entry(EntryId::new(1)),
        Point::new(10.0, 10.0),
        model,
    );
    (open_menu, state)
}

#[test]
fn scoped_and_global_invalidations_gate_freshness_and_revisions() {
    let mut freshness = Freshness::default();
    let file_a = vec!["/work/sub/a.txt".to_owned()];
    let file_b = vec!["/other/b.txt".to_owned()];

    assert!(freshness.current(&file_a, 0));
    freshness.invalidate(Some(&["/work".to_owned()]), Some(3));
    assert!(!freshness.current(&file_a, 0));
    assert!(
        freshness.current(&file_b, 0),
        "unrelated roots remain current"
    );
    assert!(!freshness.accept(&file_a, Some(2)));
    assert!(freshness.accept(&file_a, Some(3)));
    assert!(freshness.accept(&file_b, None));

    let epoch = freshness.generation();
    freshness.invalidate(None, Some(5));
    assert!(!freshness.current(&file_b, epoch));
    assert!(!freshness.accept(&file_b, Some(4)));
    assert!(freshness.accept(&file_b, Some(5)));
}

#[test]
fn badges_sort_by_priority_descending_then_provider_id_ascending() {
    let mut state = FileProviders {
        later: crate::slots::Inbox::new().later(),
        started: true,
        timer: None,
        hub: Hub::new(),
        open_menu: OpenMenu::default(),
        presented: HashMap::new(),
        next_version: 0,
    };
    let mut example = dummy_provider("example", "Example Cloud");
    let mut beta = dummy_provider("beta", "beta");
    let path = "/work/todo.txt".to_owned();
    for (prov, prio) in [(&mut example, 0), (&mut beta, 2)] {
        prov.cache.insert(
            path.clone(),
            Answer {
                at: Instant::now(),
                generation: 0,
                value: Decoration {
                    path: path.clone(),
                    badge: Some("badge".to_owned()),
                    description: "Available offline".to_owned(),
                    priority: prio,
                },
            },
        );
    }
    state.hub.providers = vec![example, beta];

    let entry_id = EntryId::new(7);
    assert!(state.update_presentations(&[(entry_id, path.clone())]));
    let shown = state
        .presented
        .get(&entry_id)
        .expect("presented decoration");
    assert_eq!(
        shown.badge_key.as_ref().map(|(p, _)| p.as_str()),
        Some("beta"),
        "higher priority provider wins the drawn badge"
    );
    assert_eq!(
        shown.description,
        "beta: Available offline; example: Available offline"
    );
    let version = shown.version;
    assert!(!state.update_presentations(&[(entry_id, path)]));
    assert_eq!(state.presented[&entry_id].version, version);
}

#[test]
fn open_menu_replaces_context_token_in_place_without_rebuilding() {
    let providers = vec![dummy_provider("example", "Example Cloud")];
    let (mut open_menu, mut menu_state) = open_fixture_menu(&providers);
    let built = build_sections_and_leaves(&providers, nested_fixture("new-token"));
    assert!(!open_menu.refresh(&mut menu_state, 1, built));
    let activated = open_menu
        .take_activation("provider:0:keep")
        .expect("leaf activation");
    assert_eq!(activated.context.as_deref(), Some("new-token"));
}

#[test]
fn open_menu_retains_nested_submenus_across_sibling_removal_and_renames() {
    let providers = vec![dummy_provider("example", "Example Cloud")];
    let (mut open_menu, mut menu_state) = open_fixture_menu(&providers);
    assert!(menu_state.open_submenu(&[2, 1]));

    let shifted_and_renamed = vec![(
        0,
        action(
            "availability",
            "Cloud availability",
            None,
            Some(vec![
                action("keep", "Keep test pin", Some("latest-token"), None),
                action(
                    "details",
                    "Updated details",
                    None,
                    Some(vec![action("info", "Inspect refreshed state", None, None)]),
                ),
            ]),
        ),
    )];
    let built = build_sections_and_leaves(&providers, shifted_and_renamed);
    assert!(open_menu.refresh(&mut menu_state, 1, built));
    assert_eq!(menu_state.submenu(), [1, 1]);
    assert_eq!(
        menu_state
            .model()
            .and_then(|m| m.item_at(&[1, 1, 0]))
            .map(|i| i.label.as_str()),
        Some("Inspect refreshed state")
    );
}

#[test]
fn partial_outcomes_and_disconnects_report_expected_feedback() {
    let mut partial = Reply {
        version: 1,
        id: Some(4),
        message: "One test pin accepted; another was rejected".to_owned(),
        outcome: Some(
            serde_json::from_value(serde_json::json!({
                "status": "partial",
                "accepted": 1,
                "total": 2,
                "job": "fixture-job-17"
            }))
            .expect("valid outcome"),
        ),
        feedback: Some(Feedback::Notice),
        ..Reply::default()
    };
    assert_eq!(partial.feedback(), Feedback::Dialog);
    assert_eq!(
        activation_summary("example", partial.clone()),
        "example: Partially accepted (1/2)\nJob: fixture-job-17\n\nOne test pin accepted; another was rejected"
    );

    partial.outcome.as_mut().expect("outcome").status = OutcomeStatus::Accepted;
    partial.outcome.as_mut().expect("outcome").accepted = Some(2);
    assert_eq!(
        activation_effect("example", "Example Cloud", partial),
        Some(Effect::Notice {
            heading: "Example Cloud".to_owned(),
            message: "One test pin accepted; another was rejected".to_owned(),
        })
    );
    assert_eq!(
        disconnected_message("example", true),
        "example: The provider disconnected before this action was sent."
    );

    let mut hub = Hub::new();
    hub.providers
        .push(dummy_provider("example", "Example Cloud"));
    hub.pending.insert(9, Pending::Activate(0));
    let mut effects = Vec::new();
    hub.apply_update(
        0,
        protocol::Update::Offline {
            unsent: Vec::new(),
            in_flight: Some(9),
        },
        &mut effects,
    );
    assert_eq!(
        effects,
        vec![Effect::Dialog(
            "example: The provider disconnected. The action may already have been accepted; check its state before retrying.".to_owned()
        )]
    );
    assert!(hub.pending.is_empty());
}

#[test]
fn reload_retires_providers_with_waiting_activations_and_clears_presentations() {
    crate::test_support::hosted(
        "file_providers::tests",
        "reload_retires_providers_with_waiting_activations_and_clears_presentations",
        |mut host| {
            let entry_id = EntryId::new(1);
            host.runtime().update(|app, _| {
                app.file_providers.presented.insert(
                    entry_id,
                    super::Presented {
                        version: 1,
                        badge_key: Some(("example".to_owned(), "badge".to_owned())),
                        badge: Some(Pixels::new()),
                        description: "example: Available offline".to_owned(),
                    },
                );
                app.file_providers
                    .hub
                    .providers
                    .push(dummy_provider("example", "Example Cloud"));
                app.file_providers
                    .hub
                    .pending
                    .insert(42, Pending::Activate(0));
                app.file_providers.ensure_started();
                let first_token = app
                    .file_providers
                    .timer
                    .clone()
                    .expect("initial timer armed");
                app.file_providers.stop_timer();
                assert!(!first_token.get(), "stopped timer token is cancelled");
                super::reload(app);
                let second_token = app
                    .file_providers
                    .timer
                    .clone()
                    .expect("reload rearms timer");
                assert!(second_token.get());
                assert!(!std::rc::Rc::ptr_eq(&first_token, &second_token));
            });
            assert!(host.app().file_providers.hub.providers.is_empty());
            assert_eq!(host.app().file_providers.hub.retiring.len(), 1);
            assert_eq!(
                host.app().file_providers.presented[&entry_id].description,
                ""
            );
            assert!(
                host.app().file_providers.presented[&entry_id]
                    .badge
                    .is_none()
            );
        },
    );
}

#[test]
fn provider_descriptions_appear_on_entry_accesskit_nodes_and_clear_on_withdrawal() {
    crate::test_support::hosted(
        "file_providers::tests",
        "provider_descriptions_appear_on_entry_accesskit_nodes_and_clear_on_withdrawal",
        |mut host| {
            let folder = crate::test_support::fixture();
            let todo_path = folder.path().join("todo.txt");
            let todo_loc = strata_core::model::Location::local(&todo_path);
            host.app()
                .open(strata_core::model::Location::local(folder.path()));
            assert!(host.run_until(crate::test_support::PATIENCE, |h| {
                h.app().columns.listing(0).is_some_and(|c| !c.is_empty())
            }));
            while host.turn() {}
            let entry_id = crate::shell::known_id(host.app(), &todo_loc).expect("todo.txt id");
            let todo_desc = |host: &mut crate::host::offscreen::Offscreen| {
                let tree = host.runtime().accessibility_tree();
                tree.nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some("todo.txt"))
                    .and_then(|(_, n)| n.description().map(str::to_owned))
            };
            let base = todo_desc(&mut host).expect("base description");
            assert!(base.starts_with("File"), "unexpected base: {base}");

            host.runtime().update(|app, _| {
                app.file_providers.next_version += 1;
                let version = app.file_providers.next_version;
                app.file_providers.presented.insert(
                    entry_id,
                    super::Presented {
                        version,
                        badge_key: Some(("example".to_owned(), "badge".to_owned())),
                        badge: Some(Pixels::new()),
                        description: "example: Available offline".to_owned(),
                    },
                );
            });
            host.turn();
            assert_eq!(
                todo_desc(&mut host).as_deref(),
                Some(format!("{base}, example: Available offline").as_str())
            );

            host.runtime().update(|app, _| {
                app.file_providers.clear_all_presentations();
            });
            host.turn();
            assert_eq!(todo_desc(&mut host).as_deref(), Some(base.as_str()));
        },
    );
}
