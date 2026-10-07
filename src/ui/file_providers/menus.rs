// SPDX-License-Identifier: MIT
use super::{MenuAction, MenuKey, MenuResult, Pending, request, with_hub};
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
        item
    }
}

fn reconcile(
    context: &Context,
    model: &gio::Menu,
    nodes: &mut Vec<Node>,
    items: &MenuResult,
) -> bool {
    let mut changed = false;
    for index in (0..nodes.len()).rev() {
        if !items.iter().any(|(provider, data)| {
            nodes[index].provider == *provider
                && nodes[index].data.id == data.id
                && nodes[index].children.is_some() == data.children.is_some()
        }) {
            nodes.remove(index).retire(context);
            model.remove(index as i32);
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
            if let Some(position) = nodes
                .iter()
                .skip(index)
                .position(matches)
                .map(|p| p + index)
            {
                let node = nodes.remove(position);
                model.remove(position as i32);
                model.insert_item(index as i32, &node.item());
                nodes.insert(index, node);
            } else {
                let node = Node::new(context, *provider, data.clone());
                model.insert_item(index as i32, &node.item());
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
            model.remove(index as i32);
            model.insert_item(index as i32, &node.item());
            changed = true;
        }
    }
    while nodes.len() > items.len() {
        let node = nodes.pop().expect("excess menu node");
        node.retire(context);
        model.remove(nodes.len() as i32);
        changed = true;
    }
    changed
}

struct ProviderSection {
    provider: usize,
    model: gio::Menu,
    nodes: Vec<Node>,
}

impl ProviderSection {
    fn retire(&self, context: &Context) {
        for node in &self.nodes {
            node.retire(context);
        }
    }
}

pub(super) struct Renderer {
    context: Context,
    model: gio::Menu,
    sections: Vec<ProviderSection>,
}

impl Renderer {
    pub(super) fn new(
        model: gio::Menu,
        group: gio::SimpleActionGroup,
        owner: glib::WeakRef<gtk::Widget>,
        key: MenuKey,
        alive: Rc<Cell<u64>>,
        epoch: u64,
    ) -> Self {
        Self {
            context: Context {
                group,
                owner,
                key,
                alive,
                epoch,
            },
            model,
            sections: Vec::new(),
        }
    }

    pub(super) fn update(&mut self, result: MenuResult) -> bool {
        let mut groups: BTreeMap<usize, MenuResult> = BTreeMap::new();
        for (provider, action) in result {
            groups.entry(provider).or_default().push((provider, action));
        }
        let mut changed = false;
        for index in (0..self.sections.len()).rev() {
            if !groups.contains_key(&self.sections[index].provider) {
                self.sections.remove(index).retire(&self.context);
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
                        nodes: Vec::new(),
                    },
                );
            }
            let section = &mut self.sections[index];
            changed |= reconcile(&self.context, &section.model, &mut section.nodes, actions);
            if new_section {
                let name = with_hub(|hub| hub.providers[*provider].name.clone());
                self.model
                    .insert_section(index as i32, Some(&name), &section.model);
                changed = true;
            }
        }
        changed
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        for section in &self.sections {
            section.retire(&self.context);
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
