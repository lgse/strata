// SPDX-License-Identifier: MIT
use super::{MenuAction, MenuKey, MenuResult, Pending, Placement, request, with_hub};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

struct Context {
    group: gio::SimpleActionGroup,
    owner: glib::WeakRef<gtk::Widget>,
    key: MenuKey,
    alive: Rc<Cell<u64>>,
    epoch: u64,
}

struct Leaf {
    action: gio::SimpleAction,
    context: Rc<RefCell<Option<String>>>,
}

struct Node {
    provider: usize,
    data: MenuAction,
    leaf: Option<Leaf>,
    children: Option<(gio::Menu, Vec<Node>)>,
}

impl Node {
    fn retire(&self, context: &Context) {
        if let Some(leaf) = &self.leaf {
            leaf.action.set_enabled(false);
            if context
                .group
                .lookup_action(leaf.action.name().as_str())
                .as_ref()
                == Some(leaf.action.upcast_ref())
            {
                context.group.remove_action(leaf.action.name().as_str());
            }
        }
        if let Some((_, children)) = &self.children {
            for child in children {
                child.retire(context);
            }
        }
    }

    fn new(context: &Context, provider: usize, data: MenuAction) -> Self {
        let mut node = Self {
            provider,
            data,
            leaf: None,
            children: None,
        };
        if let Some(children) = &node.data.children {
            let model = gio::Menu::new();
            let mut nodes = Vec::new();
            reconcile(
                context,
                &model,
                0,
                &mut nodes,
                &children
                    .iter()
                    .cloned()
                    .map(|a| (provider, a))
                    .collect::<Vec<_>>(),
            );
            node.children = Some((model, nodes));
        } else {
            let name = format!("action-{provider}-{}", node.data.id);
            let action = gio::SimpleAction::new(&name, None);
            let token = Rc::new(RefCell::new(node.data.context.clone()));
            let (key, owner, id, alive, epoch) = (
                context.key.clone(),
                context.owner.clone(),
                node.data.id.clone(),
                context.alive.clone(),
                context.epoch,
            );
            let current_token = token.clone();
            action.connect_activate(move |action, _| {
                if !action.is_enabled() || alive.get() != epoch {
                    return;
                }
                with_hub(|hub| {
                    let mut r = request("activate", key.0.clone(), key.1);
                    r.action = Some(id.clone());
                    r.context = current_token.borrow().clone();
                    hub.send(provider, r, Pending::Activate(provider, owner.clone()));
                });
            });
            context.group.add_action(&action);
            node.leaf = Some(Leaf {
                action,
                context: token,
            });
        }
        node
    }

    fn item(&self) -> gio::MenuItem {
        let label = &self.data.label;
        let item = if let Some((model, _)) = &self.children {
            gio::MenuItem::new_submenu(Some(&label.replace('_', "__")), model)
        } else {
            gio::MenuItem::new(
                Some(&label.replace('_', "__")),
                Some(&format!(
                    "provider.action-{}-{}",
                    self.provider, self.data.id
                )),
            )
        };
        if let Some(icon) = with_hub(|hub| {
            self.data
                .icon
                .as_ref()
                .and_then(|i| hub.providers[self.provider].icons.get(i))
                .cloned()
        }) {
            item.set_icon(&icon);
        }
        if self.data.sends_to() {
            item.set_attribute_value("x-strata-send-to-device", Some(&true.to_variant()));
            item.set_attribute_value("x-strata-description", Some(&label.to_variant()));
        }
        item
    }
}

/// `offset` counts leading model items that are not nodes, such as a heading.
fn reconcile(
    context: &Context,
    model: &gio::Menu,
    offset: usize,
    nodes: &mut Vec<Node>,
    items: &MenuResult,
) -> bool {
    let position = |index: usize| (index + offset) as i32;
    let mut changed = false;
    for index in (0..nodes.len()).rev() {
        if !items.iter().any(|(provider, data)| {
            nodes[index].provider == *provider
                && nodes[index].data.id == data.id
                && nodes[index].children.is_some() == data.children.is_some()
        }) {
            nodes.remove(index).retire(context);
            model.remove(position(index));
            changed = true;
        }
    }
    for (index, (provider, data)) in items.iter().enumerate() {
        let matches = |node: &Node| {
            node.provider == *provider
                && node.data.id == data.id
                && node.children.is_some() == data.children.is_some()
        };
        if !nodes.get(index).is_some_and(matches) {
            if let Some(existing) = nodes
                .iter()
                .skip(index)
                .position(matches)
                .map(|p| p + index)
            {
                let node = nodes.remove(existing);
                model.remove(position(existing));
                model.insert_item(position(index), &node.item());
                nodes.insert(index, node);
            } else {
                let node = Node::new(context, *provider, data.clone());
                model.insert_item(position(index), &node.item());
                nodes.insert(index, node);
            }
            changed = true;
        }
        let node = &mut nodes[index];
        if let (Some((children_model, children)), Some(new_children)) =
            (&mut node.children, &data.children)
        {
            changed |= reconcile(
                context,
                children_model,
                0,
                children,
                &new_children
                    .iter()
                    .cloned()
                    .map(|a| (*provider, a))
                    .collect::<Vec<_>>(),
            );
        }
        if let Some(leaf) = &node.leaf {
            leaf.context.replace(data.context.clone());
        }
        let presentation_changed = node.data.label != data.label || node.data.icon != data.icon;
        node.data = data.clone();
        if presentation_changed {
            model.remove(position(index));
            model.insert_item(position(index), &node.item());
            changed = true;
        }
    }
    while nodes.len() > items.len() {
        let node = nodes.pop().expect("excess menu node");
        node.retire(context);
        model.remove(position(nodes.len()));
        changed = true;
    }
    changed
}

struct ProviderSection {
    provider: usize,
    model: gio::Menu,
    heading: bool,
    /// Shown in Send to… as its single leaf instead of a submenu.
    entry: bool,
    nodes: Vec<Node>,
}

impl ProviderSection {
    fn retire(&self, context: &Context) {
        for node in &self.nodes {
            node.retire(context);
        }
    }
}

/// Provider sections of one menu model, in provider order. In Send to…, each
/// provider is a submenu instead, like a removable drive, or the single entry
/// it asked for.
struct Sections {
    model: gio::Menu,
    sections: Vec<ProviderSection>,
    submenus: bool,
}

impl Sections {
    fn update(
        &mut self,
        context: &Context,
        groups: &BTreeMap<usize, MenuResult>,
        popover: Option<gtk::PopoverMenu>,
    ) -> bool {
        let mut changed = false;
        let submenus = self.submenus;
        let entry = |actions: &MenuResult| submenus && single_entry(actions);
        for index in (0..self.sections.len()).rev() {
            let section = &self.sections[index];
            if !groups
                .get(&section.provider)
                .is_some_and(|actions| entry(actions) == section.entry)
            {
                self.sections.remove(index).retire(context);
                self.model.remove(index as i32);
                changed = true;
            }
        }
        for (index, (provider, actions)) in groups.iter().enumerate() {
            let new_section = self
                .sections
                .get(index)
                .is_none_or(|s| s.provider != *provider);
            if new_section {
                self.sections.insert(
                    index,
                    ProviderSection {
                        provider: *provider,
                        model: gio::Menu::new(),
                        heading: false,
                        entry: entry(actions),
                        nodes: Vec::new(),
                    },
                );
            }
            let section = &mut self.sections[index];
            let reconciled = reconcile(
                context,
                &section.model,
                usize::from(section.heading),
                &mut section.nodes,
                actions,
            );
            changed |= reconciled;
            if section.entry {
                if reconciled {
                    if !new_section {
                        self.model.remove(index as i32);
                    }
                    self.model
                        .insert_item(index as i32, &send_to_entry(section));
                }
            } else if new_section {
                if self.submenus {
                    self.model
                        .insert_item(index as i32, &destination_submenu(section));
                } else {
                    insert_section(&self.model, index, section, popover.clone());
                }
                changed = true;
            }
        }
        changed
    }

    fn retire(&self, context: &Context) {
        for section in &self.sections {
            section.retire(context);
        }
    }
}

pub(super) struct Renderer {
    context: Context,
    popover: glib::WeakRef<gtk::PopoverMenu>,
    sections: Sections,
    send_to: Option<Sections>,
}

impl Renderer {
    /// `models` holds the section model and, when the menu has one, Send to….
    pub(super) fn new(
        models: (gio::Menu, Option<gio::Menu>),
        group: gio::SimpleActionGroup,
        owner: glib::WeakRef<gtk::Widget>,
        popover: glib::WeakRef<gtk::PopoverMenu>,
        key: MenuKey,
        alive: Rc<Cell<u64>>,
        epoch: u64,
    ) -> Self {
        let (model, send_to) = models;
        Self {
            context: Context {
                group,
                owner,
                key,
                alive,
                epoch,
            },
            popover,
            sections: Sections {
                model,
                sections: Vec::new(),
                submenus: false,
            },
            send_to: send_to.map(|model| Sections {
                model,
                sections: Vec::new(),
                submenus: true,
            }),
        }
    }

    pub(super) fn update(&mut self, result: MenuResult) -> bool {
        let mut groups: BTreeMap<usize, MenuResult> = BTreeMap::new();
        let mut send_to_groups: BTreeMap<usize, MenuResult> = BTreeMap::new();
        for (provider, mut action) in result {
            placed(&mut action, true, self.send_to.is_some());
            let target = match action.placement {
                Some(Placement::SendTo | Placement::SendToEntry) => &mut send_to_groups,
                Some(Placement::Unrecognized) | None => &mut groups,
            };
            target.entry(provider).or_default().push((provider, action));
        }
        let mut changed = self
            .sections
            .update(&self.context, &groups, self.popover.upgrade());
        if let Some(send_to) = &mut self.send_to {
            changed |= send_to.update(&self.context, &send_to_groups, None);
        }
        changed
    }
}

fn single_entry(actions: &MenuResult) -> bool {
    matches!(
        actions.as_slice(),
        [(_, action)] if action.placement == Some(Placement::SendToEntry) && action.children.is_none()
    )
}

/// Placement only applies to root actions, and only where Send to… exists.
fn placed(action: &mut MenuAction, root: bool, send_to_available: bool) {
    if !root || !send_to_available {
        action.placement = None;
    }
    for child in action.children.iter_mut().flatten() {
        placed(child, false, send_to_available);
    }
}

/// GTK section titles cannot hold an image, so a provider icon replaces the
/// title with a custom heading row styled like one.
fn insert_section(
    model: &gio::Menu,
    index: usize,
    section: &mut ProviderSection,
    popover: Option<gtk::PopoverMenu>,
) {
    let (id, name, icon) = with_hub(|hub| {
        let provider = &hub.providers[section.provider];
        (
            provider.id.clone(),
            provider.name.clone(),
            provider.heading_icon.clone(),
        )
    });
    if let (Some(icon), Some(popover)) = (icon, popover) {
        let custom = format!("provider-heading-{id}");
        let item = gio::MenuItem::new(None, None);
        item.set_attribute_value("custom", Some(&custom.to_variant()));
        section.model.insert_item(0, &item);
        model.insert_section(index as i32, None, &section.model);
        if popover.add_child(&heading(&icon, &name), &custom) {
            section.heading = true;
            return;
        }
        section.model.remove(0);
        model.remove(index as i32);
    }
    model.insert_section(index as i32, Some(&name), &section.model);
}

/// Named and drawn with the provider's artwork, or its category icon.
fn destination_submenu(section: &ProviderSection) -> gio::MenuItem {
    let name = with_hub(|hub| hub.providers[section.provider].name.clone());
    let item = gio::MenuItem::new_submenu(Some(&name.replace('_', "__")), &section.model);
    set_provider_icon(&item, section.provider);
    item.set_attribute_value("x-strata-send-to-device", Some(&true.to_variant()));
    item.set_attribute_value("x-strata-description", Some(&name.to_variant()));
    item
}

/// The leaf as the provider labelled it, drawn like a submenu when it has no icon.
fn send_to_entry(section: &ProviderSection) -> gio::MenuItem {
    let node = &section.nodes[0];
    let item = node.item();
    if node.data.icon.is_none() {
        set_provider_icon(&item, section.provider);
    }
    item
}

fn set_provider_icon(item: &gio::MenuItem, provider: usize) {
    let (artwork, category) = with_hub(|hub| {
        let provider = &hub.providers[provider];
        (provider.heading_icon.clone(), provider.category)
    });
    match artwork {
        Some(artwork) => item.set_icon(&artwork),
        None => item.set_icon(&gio::ThemedIcon::new(super::category_icon(category))),
    }
}

fn heading(icon: &gtk::gdk::Texture, name: &str) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("provider-heading");
    let image = gtk::Image::from_paintable(Some(icon));
    image.set_pixel_size(14);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.append(&image);
    let label = gtk::Label::new(Some(name));
    label.add_css_class("separator");
    label.set_xalign(0.0);
    row.append(&label);
    row.upcast()
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.sections.retire(&self.context);
        if let Some(send_to) = &self.send_to {
            send_to.retire(&self.context);
        }
    }
}

fn submenu_buttons(root: &gtk::Widget) -> Vec<(gtk::Widget, gtk::PopoverMenu)> {
    fn visit(widget: &gtk::Widget, buttons: &mut Vec<(gtk::Widget, gtk::PopoverMenu)>) {
        if widget.find_property("popover").is_some()
            && let Some(popover) = widget
                .property::<Option<gtk::Popover>>("popover")
                .and_then(|popover| popover.downcast::<gtk::PopoverMenu>().ok())
        {
            buttons.push((widget.clone(), popover));
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            visit(&widget, buttons);
        }
    }
    let mut buttons = Vec::new();
    visit(root, &mut buttons);
    buttons
}

pub(super) fn open_submenus(root: &gtk::PopoverMenu) -> Vec<gio::MenuModel> {
    submenu_buttons(root.upcast_ref())
        .into_iter()
        .filter(|(_, popover)| popover.is_visible())
        .filter_map(|(_, popover)| popover.menu_model())
        .collect()
}

pub(super) fn restore_submenus(root: &gtk::PopoverMenu, open: &[gio::MenuModel]) {
    // GTK recreates native submenu owners when a branch is renamed or moved.
    // Retained model identity also disambiguates duplicate labels; removed models stay closed.
    for model in open {
        if let Some((button, popover)) = submenu_buttons(root.upcast_ref())
            .into_iter()
            .find(|(_, popover)| popover.menu_model().as_ref() == Some(model))
            && !popover.is_visible()
        {
            button.activate();
        }
    }
}
