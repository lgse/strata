// SPDX-License-Identifier: MIT
use super::thumbnail::ThumbnailSlot;
use crate::services::file_providers::{
    self as protocol, Client, Decoration, MenuAction, Registration, Request, Update,
};
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(5);
// Refresh deadlines do not erase presentation; failure still has a hard bound.
const MAX_AGE: Duration = Duration::from_secs(15);
type MenuKey = (Vec<String>, bool);
type MenuResult = Vec<(usize, MenuAction)>;
struct Provider {
    client: Client,
    icons: HashMap<String, gdk::Texture>,
    cache: HashMap<String, (Instant, Decoration)>,
    menus: HashMap<MenuKey, (Instant, Vec<MenuAction>)>,
    epoch: u64,
    refresh_after: Instant,
    disconnected: bool,
}
enum Pending {
    Query(usize, u64, Vec<String>),
    Menu(usize, u64, MenuKey),
    Activate(usize, glib::WeakRef<gtk::Widget>),
}
struct Slot {
    widget: glib::WeakRef<ThumbnailSlot>,
    path: String,
}
struct Hub {
    discovery: mpsc::Receiver<Vec<Registration>>,
    providers: Vec<Provider>,
    slots: HashMap<usize, Slot>,
    pending: HashMap<u64, Pending>,
    serial: u64,
}
thread_local! { static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) }; }
fn with_hub<T>(f: impl FnOnce(&mut Hub) -> T) -> T {
    HUB.with(|cell| {
        if cell.borrow().is_none() {
            let (tx, discovery) = mpsc::channel();
            let root = protocol::config_root();
            std::thread::spawn(move || {
                let _ = tx.send(root.map_or_else(Vec::new, |p| protocol::discover(&p)));
            });
            cell.replace(Some(Hub {
                discovery,
                providers: Vec::new(),
                slots: HashMap::new(),
                pending: HashMap::new(),
                serial: 0,
            }));
            glib::timeout_add_local(Duration::from_millis(100), || {
                with_hub(Hub::tick);
                glib::ControlFlow::Continue
            });
        }
        f(cell
            .borrow_mut()
            .as_mut()
            .expect("initialized provider hub"))
    })
}
pub(super) fn bind(slot: &ThumbnailSlot, path: Option<&Path>) {
    let path = path
        .and_then(Path::to_str)
        .filter(|s| s.len() <= 16384)
        .map(str::to_owned);
    with_hub(|hub| {
        let id = slot.as_ptr() as usize;
        if hub
            .slots
            .get(&id)
            .is_some_and(|s| Some(&s.path) == path.as_ref())
        {
            return;
        }
        hub.slots.remove(&id);
        slot.set_decoration(None, None);
        if let Some(path) = path {
            if hub.slots.len() >= 1024 {
                hub.slots
                    .retain(|_, s| s.widget.upgrade().is_some_and(|w| w.is_mapped()));
            }
            if hub.slots.len() < 1024 {
                hub.slots.insert(
                    id,
                    Slot {
                        widget: slot.downgrade(),
                        path,
                    },
                );
            }
        }
    });
}
pub(super) fn forget(id: usize) {
    HUB.with(|h| {
        if let Ok(mut state) = h.try_borrow_mut()
            && let Some(hub) = state.as_mut()
        {
            hub.slots.remove(&id);
        }
    });
}
impl Hub {
    fn send(&mut self, provider: usize, mut r: Request, pending: Pending) {
        self.serial += 1;
        r.id = self.serial;
        if self.providers[provider].client.requests.try_send(r).is_ok() {
            self.pending.insert(self.serial, pending);
        } else if let Pending::Activate(_, owner) = pending {
            notify(
                &owner,
                "The file provider is busy or unavailable. This action was not sent.",
            );
        }
    }
    fn tick(&mut self) {
        if let Ok(registrations) = self.discovery.try_recv() {
            for r in registrations {
                let icons = r
                    .icons
                    .iter()
                    .filter_map(|(k, v)| {
                        gdk::Texture::from_bytes(&glib::Bytes::from(v))
                            .ok()
                            .map(|t| (k.clone(), t))
                    })
                    .collect();
                self.providers.push(Provider {
                    client: protocol::start(r),
                    icons,
                    cache: HashMap::new(),
                    menus: HashMap::new(),
                    epoch: 0,
                    refresh_after: Instant::now(),
                    disconnected: false,
                });
            }
        }
        for index in 0..self.providers.len() {
            if self.providers[index].disconnected {
                continue;
            }
            for _ in 0..32 {
                let update = match self.providers[index].client.updates.try_recv() {
                    Ok(update) => update,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.providers[index].disconnected = true;
                        Update::Offline
                    }
                };
                match update {
                    Update::Offline => {
                        self.invalidate(index);
                        self.providers[index].cache.clear();
                        self.providers[index].menus.clear();
                        let ids: Vec<_> = self
                            .pending
                            .iter()
                            .filter_map(|(id, p)| match p {
                                Pending::Query(i, _, _)
                                | Pending::Menu(i, _, _)
                                | Pending::Activate(i, _)
                                    if *i == index =>
                                {
                                    Some(*id)
                                }
                                _ => None,
                            })
                            .collect();
                        for id in ids {
                            if let Some(Pending::Activate(_, owner)) = self.pending.remove(&id) {
                                notify(
                                    &owner,
                                    "The file provider is unavailable. Check availability before retrying; an action may already have been accepted.",
                                );
                            }
                        }
                    }
                    Update::Reply(reply) if reply.event.is_some() => self.invalidate(index),
                    Update::Reply(reply) => {
                        let Some(pending) = reply.id.and_then(|id| self.pending.remove(&id)) else {
                            continue;
                        };
                        let provider = &mut self.providers[index];
                        match pending {
                            Pending::Query(i, e, paths) if i == index && e == provider.epoch => {
                                let new_count = paths
                                    .iter()
                                    .filter(|path| !provider.cache.contains_key(*path))
                                    .count();
                                let excess =
                                    (provider.cache.len() + new_count).saturating_sub(2048);
                                if excess > 0 {
                                    let mut oldest: Vec<_> = provider
                                        .cache
                                        .iter()
                                        .map(|(path, (at, _))| (*at, path.clone()))
                                        .collect();
                                    oldest.sort_unstable();
                                    for (_, path) in oldest.into_iter().take(excess) {
                                        provider.cache.remove(&path);
                                    }
                                }
                                // Missing entries are a negative answer, never an old badge.
                                for path in &paths {
                                    provider.cache.insert(
                                        path.clone(),
                                        (
                                            Instant::now(),
                                            Decoration {
                                                path: path.clone(),
                                                badge: None,
                                                description: String::new(),
                                            },
                                        ),
                                    );
                                }
                                for d in reply.decorations {
                                    if paths.contains(&d.path) {
                                        provider.cache.insert(d.path.clone(), (Instant::now(), d));
                                    }
                                }
                            }
                            Pending::Menu(i, e, key) if i == index && e == provider.epoch => {
                                if provider.menus.len() >= 32
                                    && !provider.menus.contains_key(&key)
                                    && let Some(oldest) = provider
                                        .menus
                                        .iter()
                                        .min_by_key(|(_, (at, _))| *at)
                                        .map(|(key, _)| key.clone())
                                {
                                    provider.menus.remove(&oldest);
                                }
                                provider.menus.insert(key, (Instant::now(), reply.actions));
                            }
                            Pending::Activate(i, owner) if i == index => {
                                notify(
                                    &owner,
                                    if reply.message.is_empty() {
                                        "The file provider did not report an outcome."
                                    } else {
                                        &reply.message
                                    },
                                );
                                self.invalidate(index);
                            }
                            _ => (),
                        }
                    }
                }
                if self.providers[index].disconnected {
                    break;
                }
            }
        }
        self.slots.retain(|_, s| s.widget.upgrade().is_some());
        let visible: Vec<_> = self
            .slots
            .values()
            .filter(|s| s.widget.upgrade().is_some_and(|w| w.is_mapped()))
            .map(|s| s.path.clone())
            .collect();
        for index in 0..self.providers.len() {
            let provider = &self.providers[index];
            if provider.disconnected {
                continue;
            }
            let mut missing: Vec<_> = visible.iter().filter(|path| !provider.cache.get(*path).is_some_and(|(at,_)| *at >= provider.refresh_after && at.elapsed() < TTL)
                && !self.pending.values().any(|p| matches!(p,Pending::Query(i,_,paths) if *i == index && paths.contains(path)))).cloned().collect();
            missing.sort();
            missing.dedup();
            missing.truncate(protocol::PATH_LIMIT);
            if !missing.is_empty() {
                let epoch = provider.epoch;
                self.send(
                    index,
                    request("query", missing.clone(), false),
                    Pending::Query(index, epoch, missing),
                );
            }
        }
        for s in self.slots.values() {
            if let Some(widget) = s.widget.upgrade() {
                let decoration = self.providers.iter().find_map(|p| {
                    p.cache
                        .get(&s.path)
                        .filter(|(at, _)| at.elapsed() < MAX_AGE)
                        .and_then(|(_, d)| {
                            d.badge
                                .as_ref()
                                .and_then(|b| p.icons.get(b))
                                .map(|t| (t, d.description.as_str()))
                        })
                });
                widget.set_decoration(decoration.map(|(t, _)| t), decoration.map(|(_, d)| d));
            }
        }
    }
    fn invalidate(&mut self, index: usize) {
        let p = &mut self.providers[index];
        p.epoch += 1;
        p.refresh_after = Instant::now();
    }
    fn menu(&mut self, key: &MenuKey) -> MenuResult {
        let mut out = Vec::new();
        for i in 0..self.providers.len() {
            let p = &self.providers[i];
            if p.disconnected {
                continue;
            }
            if let Some((_, actions)) = p.menus.get(key).filter(|(at, _)| at.elapsed() < MAX_AGE) {
                out.extend(actions.iter().cloned().map(|a| (i, a)));
            }
            if !p
                .menus
                .get(key)
                .is_some_and(|(at, _)| *at >= p.refresh_after && at.elapsed() < TTL)
                && !self
                    .pending
                    .values()
                    .any(|r| matches!(r, Pending::Menu(j,_,k) if *j == i && k == key))
            {
                let epoch = p.epoch;
                self.send(
                    i,
                    request("menu", key.0.clone(), key.1),
                    Pending::Menu(i, epoch, key.clone()),
                );
            }
        }
        out
    }
}
fn request(method: &str, paths: Vec<String>, background: bool) -> Request {
    Request {
        version: 1,
        id: 0,
        method: method.into(),
        paths,
        background,
        action: None,
    }
}
fn notify(owner: &glib::WeakRef<gtk::Widget>, message: &str) {
    let Some(owner) = owner.upgrade() else {
        return;
    };
    super::modal::show_information_dialog(&owner, "File availability", message);
}
/// The caller owns the epoch: rebuilding/closing a menu cancels its old watcher.
pub(super) fn watch_menu(
    model: gio::Menu,
    group: gio::SimpleActionGroup,
    owner: &gtk::Widget,
    paths: Vec<PathBuf>,
    background: bool,
    lifetime: (std::rc::Rc<std::cell::Cell<u64>>, u64),
    changed: impl Fn() + 'static,
) {
    if paths.is_empty() || paths.len() > protocol::PATH_LIMIT {
        return;
    }
    let Some(paths) = paths
        .iter()
        .map(|p| p.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let key = (paths, background);
    let (alive, epoch) = lifetime;
    let owner = owner.downgrade();
    let mut previous = Vec::new();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if alive.get() != epoch || owner.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let result = with_hub(|h| h.menu(&key));
        if result != previous {
            let prefix = previous
                .iter()
                .zip(&result)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = previous[prefix..]
                .iter()
                .rev()
                .zip(result[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            for n in (prefix..previous.len() - suffix).rev() {
                let (provider, action) = &previous[n];
                model.remove(n as i32);
                group.remove_action(&format!("action-{provider}-{}", action.id));
            }
            for (n, (provider, a)) in result
                .iter()
                .enumerate()
                .take(result.len() - suffix)
                .skip(prefix)
            {
                let name = format!("action-{provider}-{}", a.id);
                let item = gio::MenuItem::new(
                    Some(&a.label.replace('_', "__")),
                    Some(&format!("provider.{name}")),
                );
                let icon = with_hub(|h| {
                    a.icon
                        .as_ref()
                        .and_then(|i| h.providers[*provider].icons.get(i))
                        .cloned()
                });
                if let Some(icon) = icon {
                    item.set_icon(&icon);
                }
                let action = gio::SimpleAction::new(&name, None);
                let (key, owner, id, provider) =
                    (key.clone(), owner.clone(), a.id.clone(), *provider);
                action.connect_activate(move |_, _| {
                    with_hub(|h| {
                        let mut r = request("activate", key.0.clone(), key.1);
                        r.action = Some(id.clone());
                        h.send(provider, r, Pending::Activate(provider, owner.clone()));
                    });
                });
                group.add_action(&action);
                model.insert_item(n as i32, &item);
            }
            previous = result;
            changed();
        }
        glib::ControlFlow::Continue
    });
}
