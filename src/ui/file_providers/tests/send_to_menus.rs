// SPDX-License-Identifier: MIT
use super::*;

fn install_device_provider() {
    install_hub();
    with_hub(|hub| {
        hub.providers[0].category = Some(crate::services::integration_catalog::Category::Devices)
    });
}

struct SendToMenus {
    group: gio::SimpleActionGroup,
    model: gio::Menu,
    send_to: gio::Menu,
}

impl SendToMenus {
    fn new() -> Self {
        Self {
            group: gio::SimpleActionGroup::new(),
            model: gio::Menu::new(),
            send_to: gio::Menu::new(),
        }
    }

    fn renderer(&self) -> menus::Renderer {
        file_renderer(&self.model, Some(&self.send_to), &self.group)
    }

    fn registered(&self, names: &[&str]) -> Vec<bool> {
        names
            .iter()
            .map(|name| self.group.lookup_action(name).is_some())
            .collect()
    }

    fn send_to_label(&self) -> Option<String> {
        string_attribute(self.send_to.upcast_ref(), 0, "label")
    }
}

fn pixel_with_context() -> MenuAction {
    let mut pixel = placed("pixel", "Pixel", Placement::SendTo, None);
    pixel.context = Some("pixel-incarnation".into());
    pixel
}

fn tablet_with_downloads() -> MenuAction {
    placed(
        "tablet",
        "Tablet",
        Placement::SendTo,
        Some(vec![placed(
            "downloads",
            "Downloads",
            Placement::SendTo,
            None,
        )]),
    )
}

/// The provider's group keeps its ordinary action and Send to… gets one provider
/// submenu, drawn with the category icon, listing the device destinations.
fn devices_fill_the_provider_submenu(menus: &SendToMenus) {
    assert_eq!(labels(menus.model.upcast_ref()), ["Example Cloud"]);
    assert_eq!(
        labels(&menus.model.item_link(0, "section").expect("group")),
        ["Copy link"]
    );
    assert_eq!(labels(menus.send_to.upcast_ref()), ["Example Cloud"]);
    assert_eq!(
        first_icon_name(menus.send_to.upcast_ref(), 0, "provider submenu icon").as_deref(),
        Some(crate::assets::icons::SMARTPHONE),
        "a provider without artwork is drawn with its category icon"
    );
    let devices = menus
        .send_to
        .item_link(0, "submenu")
        .expect("provider submenu inside Send to…");
    assert_eq!(labels(&devices), ["Pixel", "Tablet"]);
    assert_eq!(send_to_device(&devices, 0), Some(true));
    assert_eq!(
        string_attribute(&devices, 0, "x-strata-description").as_deref(),
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
    assert_eq!(
        menus.registered(&["action-0-pixel", "action-0-downloads", "action-0-tablet"]),
        [true, true, false],
        "registered (pixel leaf, downloads leaf, tablet branch)"
    );
}

fn withdrawn_devices_remove_their_submenu(menus: &SendToMenus, renderer: &mut menus::Renderer) {
    let old_pixel = menus
        .group
        .lookup_action("action-0-pixel")
        .expect("device leaf");
    assert!(renderer.update(vec![(0, action("share", "Copy link", None))]));
    assert_eq!(
        menus.send_to.n_items(),
        0,
        "withdrawn devices remove their submenu"
    );
    assert!(!old_pixel.is_enabled());
    assert_eq!(
        menus.registered(&["action-0-pixel", "action-0-share"]),
        [false, true],
        "registered (withdrawn pixel, share)"
    );
    assert_eq!(menus.model.n_items(), 1);
}

/// Without a Send to… menu, a device is an ordinary action in the provider's group.
fn devices_fall_back_to_the_group(menus: &SendToMenus, pixel: MenuAction) {
    let mut without_send_to = menus::Renderer::new(
        (menus.model.clone(), None),
        menus.group.clone(),
        glib::WeakRef::new(),
        glib::WeakRef::new(),
        (vec!["/file".into()], true),
        Rc::new(Cell::new(1)),
        1,
    );
    without_send_to.update(vec![(0, pixel)]);
    let section = menus.model.item_link(0, "section").expect("fallback group");
    assert_eq!(labels(&section), ["Pixel"]);
    assert!(
        section
            .item_attribute_value(0, "x-strata-send-to-device", None)
            .is_none(),
        "without Send to… the device is an ordinary group action"
    );
    assert_eq!(menus.send_to.n_items(), 0);
}

#[test]
fn send_to_placement_fills_provider_submenus_or_falls_back_to_the_group() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::send_to_menus::send_to_placement_fills_provider_submenus_or_falls_back_to_the_group",
        || {
            install_device_provider();
            let menus = SendToMenus::new();
            let mut renderer = menus.renderer();
            let pixel = pixel_with_context();
            assert!(renderer.update(vec![
                (0, action("share", "Copy link", None)),
                (0, pixel.clone()),
                (0, tablet_with_downloads()),
            ]));
            devices_fill_the_provider_submenu(&menus);
            withdrawn_devices_remove_their_submenu(&menus, &mut renderer);

            assert!(renderer.update(vec![(0, pixel.clone())]));
            assert_eq!(
                menus.model.n_items(),
                0,
                "a provider with only devices has no group"
            );
            assert_eq!(labels(menus.send_to.upcast_ref()), ["Example Cloud"]);
            drop(renderer);
            assert!(menus.group.list_actions().is_empty());
            menus.send_to.remove_all();

            devices_fall_back_to_the_group(&menus, pixel);
            clear_hub();
        },
    );
}

/// A lone entry activates directly from Send to…, drawn like the provider submenu.
fn a_lone_entry_activates_directly(menus: &SendToMenus) {
    let send_to = menus.send_to.upcast_ref::<gio::MenuModel>();
    assert_eq!(
        (menus.model.n_items(), menus.send_to.n_items()),
        (0, 1),
        "(group items, Send to… items)"
    );
    assert_eq!(menus.send_to_label().as_deref(), Some("Open"));
    assert_eq!(
        string_attribute(send_to, 0, "action").as_deref(),
        Some("provider.action-0-open"),
        "the entry activates directly"
    );
    assert!(send_to.item_link(0, "submenu").is_none());
    assert_eq!(send_to_device(send_to, 0), Some(true));
    assert_eq!(
        first_icon_name(send_to, 0, "entry icon").as_deref(),
        Some(crate::assets::icons::SMARTPHONE),
        "an entry without its own icon is drawn like the provider's submenu"
    );
}

fn a_relabelled_entry_keeps_its_action(menus: &SendToMenus, renderer: &mut menus::Renderer) {
    let leaf = menus
        .group
        .lookup_action("action-0-open")
        .expect("entry leaf");
    assert!(renderer.update(vec![(
        0,
        placed("open", "LocalSend", Placement::SendToEntry, None)
    )]));
    assert_eq!(menus.send_to_label().as_deref(), Some("LocalSend"));
    assert_eq!(
        menus.group.lookup_action("action-0-open"),
        Some(leaf.clone())
    );
    assert!(leaf.is_enabled());
}

fn several_destinations_use_the_submenu(
    menus: &SendToMenus,
    renderer: &mut menus::Renderer,
    open: &MenuAction,
    pixel: &MenuAction,
) {
    assert!(renderer.update(vec![(0, open.clone()), (0, pixel.clone())]));
    assert_eq!(menus.send_to.n_items(), 1);
    assert_eq!(
        menus.send_to_label().as_deref(),
        Some("Example Cloud"),
        "several destinations fall back to the provider submenu"
    );
    let submenu = menus
        .send_to
        .item_link(0, "submenu")
        .expect("provider submenu");
    assert_eq!(
        string_attribute(&submenu, 0, "label").as_deref(),
        Some("Open")
    );
    assert_eq!(
        string_attribute(&submenu, 1, "label").as_deref(),
        Some("Pixel")
    );
}

#[test]
fn send_to_entry_is_a_single_item_only_while_it_is_the_providers_only_destination() {
    crate::test_support::gtk_test(
        "ui::file_providers::tests::send_to_menus::send_to_entry_is_a_single_item_only_while_it_is_the_providers_only_destination",
        || {
            install_device_provider();
            let menus = SendToMenus::new();
            let mut renderer = menus.renderer();
            let open = placed("open", "Open", Placement::SendToEntry, None);
            assert!(renderer.update(vec![(0, open.clone())]));
            a_lone_entry_activates_directly(&menus);
            a_relabelled_entry_keeps_its_action(&menus, &mut renderer);

            let pixel = placed("pixel", "Pixel", Placement::SendTo, None);
            several_destinations_use_the_submenu(&menus, &mut renderer, &open, &pixel);
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
                    menus.send_to_label().as_deref(),
                    Some("Example Cloud"),
                    "one destination or branch stays in the submenu"
                );
                assert!(menus.send_to.item_link(0, "submenu").is_some());
            }

            assert!(renderer.update(vec![(0, open)]));
            assert_eq!(menus.send_to_label().as_deref(), Some("Open"));
            assert!(menus.send_to.item_link(0, "submenu").is_none());
            assert!(renderer.update(Vec::new()));
            assert_eq!(menus.send_to.n_items(), 0);
            assert!(menus.group.list_actions().is_empty());
            clear_hub();
        },
    );
}
