// SPDX-License-Identifier: MIT
use super::*;
use std::{cell::Cell, rc::Rc};

mod activation_feedback;
mod menu_sections;
mod revisions;
mod send_to_menus;
mod sessions;
mod tracking;

fn python_command(script: &str, args: &[&Path]) -> Vec<String> {
    ["/usr/bin/python3", "-u", "-c", script]
        .into_iter()
        .map(str::to_owned)
        .chain(
            args.iter()
                .map(|arg| arg.to_str().expect("fixture path").to_owned()),
        )
        .collect()
}

fn scripted_registration(name: Option<&str>, script: &str, args: &[&Path]) -> Registration {
    Registration {
        manifest: protocol::Manifest {
            version: 1,
            id: "example".into(),
            name: name.map(str::to_owned),
            icon: None,
            command: python_command(script, args),
            icons: Default::default(),
            category: None,
        },
        icons: Default::default(),
    }
}

fn registration() -> Registration {
    scripted_registration(Some("Example Cloud"), "import time;time.sleep(60)", &[])
}

fn provider(id: &str, name: &str, client: Client) -> Provider {
    Provider {
        client,
        icons: Default::default(),
        id: id.into(),
        name: name.into(),
        heading_icon: None,
        category: None,
        cache: Default::default(),
        menus: Default::default(),
        freshness: Freshness::default(),
        disconnected: false,
        last_busy: None,
    }
}

fn hub_with(discovery: mpsc::Receiver<Vec<Registration>>, discovered: bool) -> Hub {
    Hub {
        discovery,
        discovered,
        providers: Vec::new(),
        retiring: Vec::new(),
        slots: Default::default(),
        pending: Default::default(),
        serial: 0,
        ticking: true,
    }
}

fn install_hub() {
    let (_, discovery) = mpsc::channel();
    let mut hub = hub_with(discovery, true);
    hub.providers = vec![provider(
        "example",
        "Example Cloud",
        protocol::start(registration()),
    )];
    HUB.with(|cell| cell.replace(Some(hub)));
}

fn clear_hub() {
    HUB.with(|cell| cell.replace(None));
}

fn start_scripted_provider(hub: &mut Hub, script: &str) {
    hub.providers[0].client =
        protocol::start(scripted_registration(Some("Example Cloud"), script, &[]));
}

/// Five seconds from now, the limit every wait in these tests allows.
struct Deadline(Instant);

impl Deadline {
    fn start() -> Self {
        Self(Instant::now() + Duration::from_secs(5))
    }

    fn check(&self, message: &str) {
        assert!(Instant::now() < self.0, "{message}");
    }
}

fn nap() {
    std::thread::sleep(Duration::from_millis(5));
}

fn drain_main_context() {
    let main = glib::MainContext::default();
    while main.pending() {
        main.iteration(false);
    }
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

fn placed(
    id: &str,
    label: &str,
    placement: Placement,
    children: Option<Vec<MenuAction>>,
) -> MenuAction {
    let mut action = action(id, label, children);
    action.placement = Some(placement);
    action
}

/// A renderer for `/file` in the first epoch, without owner or popover.
fn file_renderer(
    model: &gio::Menu,
    send_to: Option<&gio::Menu>,
    group: &gio::SimpleActionGroup,
) -> menus::Renderer {
    menus::Renderer::new(
        (model.clone(), send_to.cloned()),
        group.clone(),
        glib::WeakRef::new(),
        glib::WeakRef::new(),
        (vec!["/file".into()], false),
        Rc::new(Cell::new(1)),
        1,
    )
}

fn string_attribute(model: &gio::MenuModel, index: i32, attribute: &str) -> Option<String> {
    model
        .item_attribute_value(index, attribute, None)
        .and_then(|value| value.get::<String>())
}

fn labels(model: &gio::MenuModel) -> Vec<String> {
    (0..model.n_items())
        .filter_map(|index| string_attribute(model, index, "label"))
        .collect()
}

fn send_to_device(model: &gio::MenuModel, index: i32) -> Option<bool> {
    model
        .item_attribute_value(index, "x-strata-send-to-device", None)
        .and_then(|value| value.get::<bool>())
}

fn first_icon_name(model: &gio::MenuModel, index: i32, what: &str) -> Option<String> {
    let icon = model
        .item_attribute_value(index, "icon", None)
        .and_then(|icon| gio::Icon::deserialize(&icon))
        .and_downcast::<gio::ThemedIcon>()
        .expect(what);
    icon.names().first().map(|name| name.to_string())
}

fn tracked(slot: &ThumbnailSlot) -> bool {
    with_hub(|hub| hub.slots.contains_key(&(slot.as_ptr() as usize)))
}

fn black_pixel() -> gdk::MemoryTexture {
    gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_static(&[0, 0, 0, 255]),
        4,
    )
}
