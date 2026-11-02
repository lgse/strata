// SPDX-License-Identifier: MIT
use super::*;

/// Records each request it receives into its file argument and accepts it.
const RECORDING_PROVIDER: &str = "import json,sys\nfrom pathlib import Path\nfor line in sys.stdin:\n r=json.loads(line)\n Path(sys.argv[1]).write_text(json.dumps(r))\n print(json.dumps({'version':1,'id':r['id'],'message':'Accepted'}),flush=True)";

/// Installs `example` (named Example Cloud) and the unnamed `beta`, each
/// recording requests into its entry of `records`.
fn install_recording_providers(records: &[PathBuf; 2]) {
    with_hub(|hub| {
        hub.providers = ["example", "beta"]
            .into_iter()
            .zip(records)
            .enumerate()
            .map(|(index, (id, record))| {
                let name = (index == 0).then_some("Example Cloud");
                let mut registration = scripted_registration(name, RECORDING_PROVIDER, &[record]);
                registration.manifest.id = id.into();
                provider(id, name.unwrap_or(id), protocol::start(registration))
            })
            .collect();
    });
}

fn label(model: &gio::MenuModel, index: i32) -> String {
    string_attribute(model, index, "label").expect("label")
}

fn received_request(record: &Path) -> serde_json::Value {
    let deadline = Deadline::start();
    loop {
        if let Some(received) = std::fs::read(record)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            return received;
        }
        deadline.check("provider did not receive its action");
        nap();
    }
}

fn assert_each_provider_got_its_context(records: &[PathBuf; 2]) {
    for (record, context) in records
        .iter()
        .zip(["example-incarnation", "beta-incarnation"])
    {
        let received = received_request(record);
        assert_eq!(
            (
                &received["action"],
                &received["paths"],
                &received["context"]
            ),
            (
                &serde_json::json!("keep"),
                &serde_json::json!(["/file"]),
                &serde_json::json!(context)
            ),
            "(action, paths, context)"
        );
    }
}

fn keep_offline_with_context(context: &str) -> MenuAction {
    let mut keep = action("keep", "Keep offline", None);
    keep.context = Some(context.into());
    keep
}

#[test]
fn provider_sections_scope_duplicate_labels_and_retire_empty_groups() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::menu_sections::provider_sections_scope_duplicate_labels_and_retire_empty_groups",
        || {
            install_hub();
            let scratch = tempfile::tempdir().expect("provider fixture");
            let records = [scratch.path().join("example"), scratch.path().join("beta")];
            install_recording_providers(&records);
            let model = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let mut renderer = file_renderer(&model, None, &group);
            let more = action("status", "Show availability", None);
            renderer.update(vec![
                (1, keep_offline_with_context("beta-incarnation")),
                (0, keep_offline_with_context("example-incarnation")),
                (1, more.clone()),
            ]);
            let section = |index| model.item_link(index, "section").expect("provider section");
            let first = section(0);
            let second = section(1);
            assert_eq!(
                [
                    label(model.upcast_ref(), 0),
                    label(model.upcast_ref(), 1),
                    label(&first, 0),
                    label(&second, 0),
                    label(&second, 1),
                ],
                [
                    "Example Cloud",
                    "beta",
                    "Keep offline",
                    "Keep offline",
                    "Show availability"
                ]
            );
            let old_example = group
                .lookup_action("action-0-keep")
                .expect("example action");
            group.activate_action("action-0-keep", None);
            group.activate_action("action-1-keep", None);
            assert_each_provider_got_its_context(&records);

            renderer.update(vec![(1, more)]);
            assert_eq!(section(0), second);
            assert_eq!(model.n_items(), 1);
            assert!(!old_example.is_enabled());
            let remaining = ["action-0-keep", "action-1-keep", "action-1-status"]
                .map(|name| group.lookup_action(name).is_some());
            assert_eq!(
                remaining,
                [false, false, true],
                "registered (example keep, beta keep, beta status)"
            );
            renderer.update(Vec::new());
            assert_eq!(model.n_items(), 0);
            assert!(group.list_actions().is_empty());
            clear_hub();
        },
    );
}

#[test]
fn nested_refresh_retains_navigation_model_and_withdraws_old_leaf_actions() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::menu_sections::nested_refresh_retains_navigation_model_and_withdraws_old_leaf_actions",
        || {
            install_hub();
            let model = gio::Menu::new();
            let group = gio::SimpleActionGroup::new();
            let mut renderer = file_renderer(&model, None, &group);
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
            assert_eq!(
                ["action-0-keep", "action-0-release"]
                    .map(|name| group.lookup_action(name).is_some()),
                [false, true],
                "registered (old keep, new release)"
            );
            renderer.update(Vec::new());
            assert!(group.list_actions().is_empty());
            clear_hub();
        },
    );
}

#[test]
fn retired_menu_does_not_remove_replacement_menu_actions() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::menu_sections::retired_menu_does_not_remove_replacement_menu_actions",
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
            clear_hub();
        },
    );
}

fn descendant_with_class(root: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    let mut found = None;
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) {
            found = Some(widget.clone());
        }
        stack.extend(std::iter::successors(
            widget.first_child(),
            gtk::Widget::next_sibling,
        ));
    }
    found
}

fn heading_label(popover: &gtk::PopoverMenu, with_icon: bool) -> gtk::Label {
    if !with_icon {
        return descendant_with_class(popover.upcast_ref(), "separator")
            .and_downcast::<gtk::Label>()
            .expect("provider heading label");
    }
    let heading = descendant_with_class(popover.upcast_ref(), "provider-heading")
        .expect("provider heading row");
    let image = heading.first_child().expect("heading icon");
    assert!(image.downcast_ref::<gtk::Image>().is_some());
    image
        .next_sibling()
        .and_downcast::<gtk::Label>()
        .expect("heading label")
}

/// A provider menu model drawn by a popover that holds the provider actions.
struct RenderedMenu {
    model: gio::Menu,
    popover: gtk::PopoverMenu,
    group: gio::SimpleActionGroup,
}

impl RenderedMenu {
    fn new() -> Self {
        let model = gio::Menu::new();
        let popover = gtk::PopoverMenu::from_model(Some(&model));
        let group = gio::SimpleActionGroup::new();
        popover.insert_action_group("provider", Some(&group));
        Self {
            model,
            popover,
            group,
        }
    }

    /// A renderer for `/file` in its own live `epoch`.
    fn renderer(&self, epoch: u64) -> menus::Renderer {
        menus::Renderer::new(
            (self.model.clone(), None),
            self.group.clone(),
            glib::WeakRef::new(),
            self.popover.downgrade(),
            (vec!["/file".into()], false),
            Rc::new(Cell::new(epoch)),
            epoch,
        )
    }
}

fn heading_labels(root: &gtk::Widget, name: &str) -> usize {
    let own = root
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.has_css_class("separator") && label.label() == name);
    usize::from(own)
        + std::iter::successors(root.first_child(), gtk::Widget::next_sibling)
            .map(|child| heading_labels(&child, name))
            .sum::<usize>()
}

static HEADING_SLOT_FAULTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Records GTK's complaints about a reused custom slot ID or a role set on a
/// realized accessible, and writes every message as usual. Install it once,
/// in an isolated GTK test's own process.
fn record_heading_slot_faults() {
    glib::log_set_writer_func(|level, fields| {
        let field = |key: &str| {
            fields
                .iter()
                .find(|field| field.key() == key)
                .and_then(glib::LogField::value_str)
                .unwrap_or_default()
                .to_owned()
        };
        let message = field("MESSAGE");
        let relevant = message.contains("Duplicate custom ID")
            || message.contains("gtk_at_context_set_accessible_role");
        if field("GLIB_DOMAIN") == "Gtk"
            && matches!(level, glib::LogLevel::Warning | glib::LogLevel::Critical)
            && relevant
        {
            HEADING_SLOT_FAULTS
                .lock()
                .expect("heading slot faults")
                .push(message);
        }
        glib::log_writer_default(level, fields)
    });
}

#[test]
fn reopened_menu_renders_a_fresh_provider_heading_row() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::menu_sections::reopened_menu_renders_a_fresh_provider_heading_row",
        || {
            record_heading_slot_faults();
            install_hub();
            let menu = RenderedMenu::new();
            for open in 1..=3 {
                // Each rebuild clears the model the popover keeps rendering.
                menu.model.remove_all();
                let mut renderer = menu.renderer(open);
                renderer.update(vec![(0, action("keep", "Keep offline", None))]);
                assert_eq!(
                    string_attribute(menu.model.upcast_ref(), 0, "label"),
                    None,
                    "no GTK section title on open {open}"
                );
                assert_eq!(
                    heading_labels(menu.popover.upcast_ref(), "Example Cloud"),
                    1,
                    "one heading row on open {open}"
                );
            }
            assert_eq!(
                *HEADING_SLOT_FAULTS.lock().expect("heading slot faults"),
                Vec::<String>::new(),
                "GTK slot warnings across reopened menus"
            );
            clear_hub();
        },
    );
}

#[test]
fn provider_heading_row_keeps_the_name_and_action_positions() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::menu_sections::provider_heading_row_keeps_the_name_and_action_positions",
        || {
            install_hub();
            for with_icon in [true, false] {
                with_hub(|hub| {
                    hub.providers[0].heading_icon = with_icon.then(|| black_pixel().upcast());
                });
                let menu = RenderedMenu::new();
                let mut renderer = menu.renderer(1);
                renderer.update(vec![
                    (0, action("keep", "Keep offline", None)),
                    (0, action("share", "Share", None)),
                ]);
                let section = menu
                    .model
                    .item_link(0, "section")
                    .expect("provider section");
                assert_eq!(labels(&section), ["Keep offline", "Share"]);
                renderer.update(vec![
                    (0, action("share", "Share", None)),
                    (0, action("link", "Copy link", None)),
                ]);
                assert_eq!(labels(&section), ["Share", "Copy link"]);
                assert!(menu.group.lookup_action("action-0-keep").is_none());

                // GTK can hold a section title outside the widget tree (#2011).
                assert_eq!(
                    string_attribute(menu.model.upcast_ref(), 0, "label"),
                    None,
                    "no GTK section title (icon: {with_icon})"
                );
                assert_eq!(
                    heading_label(&menu.popover, with_icon).label(),
                    "Example Cloud",
                    "heading (icon: {with_icon})"
                );
            }
        },
    );
}
