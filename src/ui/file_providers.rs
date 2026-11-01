// SPDX-License-Identifier: MIT
use super::thumbnail::ThumbnailSlot;
use crate::assets::icons;
use crate::services::file_providers::{
    self as protocol, Client, Decoration, Feedback, MenuAction, OutcomeStatus, Placement,
    Registration, Request, Update,
};
use crate::services::integration_catalog::Category;
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

mod freshness;
mod menus;

use freshness::Freshness;

struct Answer<T> {
    at: Instant,
    generation: u64,
    value: T,
}

const TTL: Duration = Duration::from_secs(5);
// Refresh deadlines do not erase presentation; failure still has a hard bound.
const MAX_AGE: Duration = Duration::from_secs(15);
const MAX_CACHED_DECORATIONS: usize = 2048;
const MAX_CACHED_MENUS: usize = 32;
type MenuKey = (Vec<String>, bool);
type MenuResult = Vec<(usize, MenuAction)>;
struct Provider {
    client: Client,
    icons: HashMap<String, gdk::Texture>,
    id: String,
    name: String,
    heading_icon: Option<gdk::Texture>,
    category: Option<Category>,
    cache: HashMap<String, Answer<Decoration>>,
    menus: HashMap<MenuKey, Answer<Vec<MenuAction>>>,
    freshness: Freshness,
    disconnected: bool,
    last_busy: Option<Instant>,
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
/// A provider a reload dropped while it still owed the answer to an activation.
/// The transport's per-request deadline guarantees each waiting action settles.
struct Retiring {
    client: Client,
    id: String,
    name: String,
    waiting: HashMap<u64, glib::WeakRef<gtk::Widget>>,
}
struct Hub {
    discovery: mpsc::Receiver<Vec<Registration>>,
    discovered: bool,
    providers: Vec<Provider>,
    retiring: Vec<Retiring>,
    slots: HashMap<usize, Slot>,
    pending: HashMap<u64, Pending>,
    serial: u64,
    /// Whether the tick timer is running; it stops once discovery finds nothing.
    ticking: bool,
}
thread_local! { static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) }; }
fn spawn_discovery() -> mpsc::Receiver<Vec<Registration>> {
    let (tx, discovery) = mpsc::channel();
    let root = protocol::config_root();
    std::thread::spawn(move || {
        let _ = tx.send(root.map_or_else(Vec::new, |p| protocol::discover(&p)));
    });
    discovery
}
fn with_hub<T>(f: impl FnOnce(&mut Hub) -> T) -> T {
    HUB.with(|cell| {
        if cell.borrow().is_none() {
            cell.replace(Some(Hub {
                discovery: spawn_discovery(),
                discovered: false,
                providers: Vec::new(),
                retiring: Vec::new(),
                slots: HashMap::new(),
                pending: HashMap::new(),
                serial: 0,
                ticking: true,
            }));
            glib::timeout_add_local(Duration::from_millis(100), tick_hub);
        }
        f(cell
            .borrow_mut()
            .as_mut()
            .expect("initialized provider hub"))
    })
}
fn tick_hub() -> glib::ControlFlow {
    let idle = with_hub(|hub| {
        hub.tick();
        if hub.has_no_providers() && hub.retiring.is_empty() {
            hub.ticking = false;
        }
        !hub.ticking
    });
    if idle {
        glib::ControlFlow::Break
    } else {
        glib::ControlFlow::Continue
    }
}

/// The themed icon standing for an integration without artwork of its own.
pub(crate) fn category_icon(category: Option<Category>) -> &'static str {
    match category {
        Some(Category::Cloud) => icons::CLOUD,
        Some(Category::Devices) => icons::SMARTPHONE,
        Some(Category::VersionControl) => icons::GIT_BRANCH,
        Some(Category::Other) | None => icons::BLOCKS,
    }
}

/// Stops every provider and discovers the registrations again, so edits
/// under the providers directory take effect without a restart. A provider
/// still owing an activation outcome runs until it answers.
pub(crate) fn reload() {
    reload_with(spawn_discovery());
}

fn reload_with(discovery: mpsc::Receiver<Vec<Registration>>) {
    let start_timer = with_hub(|hub| {
        hub.reload(discovery);
        !std::mem::replace(&mut hub.ticking, true)
    });
    if start_timer {
        glib::timeout_add_local(Duration::from_millis(100), tick_hub);
    }
}

pub(super) fn bind(slot: &ThumbnailSlot, path: Option<&Path>) {
    let path = path
        .and_then(Path::to_str)
        .filter(|p| Path::new(p).is_absolute() && p.len() <= 16384)
        .map(str::to_owned);
    if slot.provider_path() != path {
        forget(slot.as_ptr() as usize);
        slot.set_decoration(None, None);
        slot.set_provider_path(path);
    }
    if slot.is_mapped() {
        remap(slot);
    }
}

fn admit(slot: &ThumbnailSlot, path: String) -> bool {
    with_hub(|hub| {
        if hub.has_no_providers() {
            return true;
        }
        let id = slot.as_ptr() as usize;
        if hub.slots.get(&id).is_some_and(|s| s.path == path) {
            return true;
        }
        if hub.slots.len() >= 1024 {
            hub.slots.retain(|_, tracked| {
                if let Some(widget) = tracked.widget.upgrade() {
                    if widget.is_mapped() {
                        return true;
                    }
                    widget.set_decoration(None, None);
                }
                false
            });
        }
        if hub.slots.len() >= 1024 {
            return false;
        }
        hub.slots.insert(
            id,
            Slot {
                widget: slot.downgrade(),
                path,
            },
        );
        true
    })
}

pub(super) fn remap(slot: &ThumbnailSlot) {
    let Some(path) = slot.provider_path() else {
        return;
    };
    if admit(slot, path) || !slot.begin_provider_retry() {
        return;
    }
    let weak = slot.downgrade();
    glib::timeout_add_local(Duration::from_millis(250), move || {
        let Some(slot) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if slot.is_mapped() && slot.provider_path().is_some_and(|path| !admit(&slot, path)) {
            return glib::ControlFlow::Continue;
        }
        slot.finish_provider_retry();
        glib::ControlFlow::Break
    });
}

pub(super) fn unmap(slot: &ThumbnailSlot) {
    forget(slot.as_ptr() as usize);
    slot.set_decoration(None, None);
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

/// A cached badge of one provider for one path, ready to show.
struct Badge<'a> {
    priority: u8,
    provider: &'a str,
    texture: &'a gdk::Texture,
    description: &'a str,
}

impl Pending {
    fn generation(&self) -> u64 {
        match self {
            Pending::Query(_, generation, _) | Pending::Menu(_, generation, _) => *generation,
            Pending::Activate(..) => 0,
        }
    }

    fn activation_owner(&self, index: usize) -> Option<&glib::WeakRef<gtk::Widget>> {
        match self {
            Pending::Activate(i, owner) if *i == index => Some(owner),
            _ => None,
        }
    }

    fn provider(&self) -> usize {
        match self {
            Pending::Query(i, _, _) | Pending::Menu(i, _, _) | Pending::Activate(i, _) => *i,
        }
    }
}

impl Provider {
    fn start(registration: Registration) -> Self {
        let icons: HashMap<String, gdk::Texture> = registration
            .icons
            .iter()
            .filter_map(|(k, v)| {
                gdk::Texture::from_bytes(&glib::Bytes::from(v))
                    .ok()
                    .map(|t| (k.clone(), t))
            })
            .collect();
        let id = registration.manifest.id.clone();
        let name = registration
            .manifest
            .name
            .clone()
            .unwrap_or_else(|| id.clone());
        let heading_icon = registration
            .manifest
            .icon
            .as_ref()
            .and_then(|icon| icons.get(icon))
            .cloned();
        let category = registration
            .manifest
            .category
            .as_deref()
            .and_then(Category::from_id);
        Provider {
            client: protocol::start(registration),
            id,
            name,
            heading_icon,
            category,
            icons,
            cache: HashMap::new(),
            menus: HashMap::new(),
            freshness: Freshness::default(),
            disconnected: false,
            last_busy: None,
        }
    }

    /// Whether a reply may be stored: errors always settle the request.
    fn accepts(&self, paths: &[String], reply: &protocol::Reply) -> bool {
        reply.error.is_some() || self.freshness.accept(paths, reply.revision)
    }

    fn evict_oldest_decorations(&mut self, paths: &[String]) {
        let new_count = paths
            .iter()
            .filter(|path| !self.cache.contains_key(*path))
            .count();
        let excess = (self.cache.len() + new_count).saturating_sub(MAX_CACHED_DECORATIONS);
        if excess == 0 {
            return;
        }
        let mut oldest: Vec<_> = self
            .cache
            .iter()
            .map(|(path, answer)| (answer.at, path.clone()))
            .collect();
        oldest.sort_unstable();
        for (_, path) in oldest.into_iter().take(excess) {
            self.cache.remove(&path);
        }
    }

    fn store_query(&mut self, paths: Vec<String>, generation: u64, decorations: Vec<Decoration>) {
        self.evict_oldest_decorations(&paths);
        for path in &paths {
            self.cache.insert(
                path.clone(),
                Answer {
                    at: Instant::now(),
                    generation,
                    value: Decoration {
                        path: path.clone(),
                        badge: None,
                        description: String::new(),
                        priority: 0,
                    },
                },
            );
        }
        for decoration in decorations {
            if paths.contains(&decoration.path) {
                self.cache.insert(
                    decoration.path.clone(),
                    Answer {
                        at: Instant::now(),
                        generation,
                        value: decoration,
                    },
                );
            }
        }
    }

    fn evict_oldest_menu(&mut self) {
        if let Some(oldest) = self
            .menus
            .iter()
            .min_by_key(|(_, answer)| answer.at)
            .map(|(key, _)| key.clone())
        {
            self.menus.remove(&oldest);
        }
    }

    fn store_menu(&mut self, key: MenuKey, generation: u64, actions: Vec<MenuAction>) {
        let menus_full = self.menus.len() >= MAX_CACHED_MENUS && !self.menus.contains_key(&key);
        if menus_full {
            self.evict_oldest_menu();
        }
        self.menus.insert(
            key,
            Answer {
                at: Instant::now(),
                generation,
                value: actions,
            },
        );
    }

    fn has_fresh_decoration(&self, path: &String) -> bool {
        self.cache.get(path).is_some_and(|answer| {
            self.freshness
                .current(std::slice::from_ref(path), answer.generation)
                && answer.at.elapsed() < TTL
        })
    }

    fn cached_menu(&self, key: &MenuKey) -> Option<&Answer<Vec<MenuAction>>> {
        self.menus
            .get(key)
            .filter(|answer| answer.at.elapsed() < MAX_AGE)
    }

    fn has_fresh_menu(&self, key: &MenuKey) -> bool {
        self.menus.get(key).is_some_and(|answer| {
            self.freshness.current(&key.0, answer.generation) && answer.at.elapsed() < TTL
        })
    }

    fn badge(&self, path: &str) -> Option<Badge<'_>> {
        let answer = self
            .cache
            .get(path)
            .filter(|answer| answer.at.elapsed() < MAX_AGE)?;
        let texture = answer
            .value
            .badge
            .as_ref()
            .and_then(|badge| self.icons.get(badge))?;
        Some(Badge {
            priority: answer.value.priority,
            provider: &self.id,
            texture,
            description: answer.value.description.as_str(),
        })
    }
}

impl Retiring {
    fn settle(&mut self, answered: &mut Vec<String>) {
        let (updates, closed) = self.client.updates.drain();
        for update in updates {
            self.apply_update(update, answered);
        }
        if closed {
            for (_, owner) in self.waiting.drain() {
                notify_disconnected(&owner, &self.id, false);
            }
        }
    }

    fn apply_update(&mut self, update: Update, answered: &mut Vec<String>) {
        match update {
            Update::Reply(reply) => {
                if let Some(owner) = reply.id.and_then(|id| self.waiting.remove(&id)) {
                    present_activation(&owner, &self.id, &self.name, *reply);
                    answered.push(self.id.clone());
                }
            }
            Update::Offline { unsent, in_flight } => {
                for id in unsent.iter().copied().chain(in_flight) {
                    if let Some(owner) = self.waiting.remove(&id) {
                        notify_disconnected(&owner, &self.id, unsent.contains(&id));
                    }
                }
            }
        }
    }
}

impl Hub {
    fn has_no_providers(&self) -> bool {
        self.discovered && self.providers.is_empty()
    }

    /// Drops the running providers, which ends their processes, and waits
    /// for a new discovery result. Tracked rows lose their badges until the
    /// new providers answer. A provider with an unanswered activation retires
    /// instead, so the outcome is still shown.
    fn reload(&mut self, discovery: mpsc::Receiver<Vec<Registration>>) {
        self.discovery = discovery;
        self.discovered = false;
        for (index, provider) in std::mem::take(&mut self.providers).into_iter().enumerate() {
            self.retire(index, provider);
        }
        self.pending.clear();
        self.clear_decorations();
    }

    fn retire(&mut self, index: usize, provider: Provider) {
        let waiting = self.waiting_activations(index);
        if !waiting.is_empty() {
            self.retiring.push(Retiring {
                client: provider.client,
                id: provider.id,
                name: provider.name,
                waiting,
            });
        }
    }

    fn clear_decorations(&self) {
        for slot in self.slots.values() {
            if let Some(widget) = slot.widget.upgrade() {
                widget.set_decoration(None, None);
            }
        }
    }

    fn waiting_activations(&self, index: usize) -> HashMap<u64, glib::WeakRef<gtk::Widget>> {
        self.pending
            .iter()
            .filter_map(|(id, pending)| {
                pending
                    .activation_owner(index)
                    .map(|owner| (*id, owner.clone()))
            })
            .collect()
    }

    fn send(&mut self, provider: usize, mut r: Request, pending: Pending) {
        self.serial += 1;
        r.id = self.serial;
        if self.providers[provider].client.requests.try_send(r).is_ok() {
            self.pending.insert(self.serial, pending);
        } else {
            let provider = &mut self.providers[provider];
            if provider
                .last_busy
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(10))
            {
                eprintln!(
                    "Strata provider {}: host queue full or closed; request not sent",
                    provider.id
                );
                provider.last_busy = Some(Instant::now());
            }
            if let Pending::Activate(_, owner) = pending {
                notify(
                    &owner,
                    &format!(
                        "{}: The file provider is busy or unavailable. This action was not sent.",
                        provider.id
                    ),
                );
            }
        }
    }
    fn settle_retiring(&mut self) {
        let mut answered = Vec::new();
        for retiring in &mut self.retiring {
            retiring.settle(&mut answered);
        }
        self.retiring
            .retain(|retiring| !retiring.waiting.is_empty());
        for index in 0..self.providers.len() {
            if answered.contains(&self.providers[index].id) {
                self.invalidate(index);
            }
        }
    }

    fn tick(&mut self) {
        self.settle_retiring();
        self.adopt_discovery();
        for index in 0..self.providers.len() {
            self.drain_provider(index);
        }
        self.slots.retain(|_, s| s.widget.upgrade().is_some());
        self.request_missing_decorations();
        self.present_decorations();
    }

    fn adopt_discovery(&mut self) {
        let Ok(registrations) = self.discovery.try_recv() else {
            return;
        };
        self.discovered = true;
        if registrations.is_empty() {
            self.slots.clear();
        }
        for registration in registrations {
            self.providers.push(Provider::start(registration));
        }
    }

    fn drain_provider(&mut self, index: usize) {
        if self.providers[index].disconnected {
            return;
        }
        let (updates, closed) = self.providers[index].client.updates.drain();
        for update in updates {
            self.apply_update(index, update);
        }
        if closed {
            self.offline(index, &[], None);
            self.providers[index].disconnected = true;
        }
    }

    fn apply_update(&mut self, index: usize, update: Update) {
        match update {
            Update::Offline { unsent, in_flight } => {
                let affected: Vec<_> = unsent.iter().copied().chain(in_flight).collect();
                self.offline(index, &unsent, Some(&affected));
            }
            Update::Reply(reply) if reply.event.is_some() => {
                self.providers[index]
                    .freshness
                    .invalidate(reply.paths.as_deref(), reply.revision);
            }
            Update::Reply(reply) => self.settle_reply(index, reply),
        }
    }

    fn settle_reply(&mut self, index: usize, reply: Box<protocol::Reply>) {
        let Some(pending) = reply.id.and_then(|id| self.pending.remove(&id)) else {
            return;
        };
        let provider = &mut self.providers[index];
        let generation = if reply.revision.is_some() {
            provider.freshness.generation()
        } else {
            pending.generation()
        };
        match pending {
            Pending::Query(i, _, paths) if i == index && provider.accepts(&paths, &reply) => {
                provider.store_query(paths, generation, reply.decorations.unwrap_or_default());
            }
            Pending::Menu(i, _, key) if i == index && provider.accepts(&key.0, &reply) => {
                provider.store_menu(key, generation, reply.actions.unwrap_or_default());
            }
            Pending::Activate(i, owner) if i == index => {
                present_activation(&owner, &provider.id, &provider.name, *reply);
                self.invalidate(index);
            }
            _ => (),
        }
    }

    fn is_querying(&self, index: usize, path: &String) -> bool {
        self.pending.values().any(
            |p| matches!(p, Pending::Query(i, _, paths) if *i == index && paths.contains(path)),
        )
    }

    fn missing_decorations(&self, index: usize, visible: &[String]) -> Vec<String> {
        let provider = &self.providers[index];
        let mut missing: Vec<_> = visible
            .iter()
            .filter(|path| !provider.has_fresh_decoration(path) && !self.is_querying(index, path))
            .cloned()
            .collect();
        missing.sort_by(|a, b| {
            provider
                .cache
                .get(a)
                .map(|answer| answer.at)
                .cmp(&provider.cache.get(b).map(|answer| answer.at))
                .then(a.cmp(b))
        });
        missing.dedup();
        protocol::query_batch(missing)
    }

    fn request_missing_decorations(&mut self) {
        let visible: Vec<_> = self
            .slots
            .values()
            .filter(|s| s.widget.upgrade().is_some_and(|w| w.is_mapped()))
            .map(|s| s.path.clone())
            .collect();
        for index in 0..self.providers.len() {
            if self.providers[index].disconnected {
                continue;
            }
            let missing = self.missing_decorations(index, &visible);
            if !missing.is_empty() {
                let epoch = self.providers[index].freshness.generation();
                self.send(
                    index,
                    request("query", missing.clone(), false),
                    Pending::Query(index, epoch, missing),
                );
            }
        }
    }

    fn badges(&self, path: &str) -> Vec<Badge<'_>> {
        let mut badges: Vec<_> = self
            .providers
            .iter()
            .filter_map(|p| p.badge(path))
            .collect();
        badges.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.provider.cmp(b.provider)));
        badges
    }

    fn present_decorations(&self) {
        for s in self.slots.values() {
            let Some(widget) = s.widget.upgrade() else {
                continue;
            };
            let badges = self.badges(&s.path);
            let description = badges
                .iter()
                .map(|badge| format!("{}: {}", badge.provider, badge.description))
                .collect::<Vec<_>>()
                .join("; ");
            widget.set_decoration(
                badges.first().map(|badge| badge.texture),
                (!badges.is_empty()).then_some(description.as_str()),
            );
        }
    }

    fn offline(&mut self, index: usize, unsent: &[u64], affected: Option<&[u64]>) {
        self.providers[index].freshness = Freshness::default();
        self.providers[index].cache.clear();
        self.providers[index].menus.clear();
        self.rebase_pending(index);
        for id in self.pending_ids(index, affected) {
            if let Some(Pending::Activate(_, owner)) = self.pending.remove(&id) {
                notify_disconnected(&owner, &self.providers[index].id, unsent.contains(&id));
            }
        }
    }

    /// Moves the provider's outstanding queries onto its new generation.
    fn rebase_pending(&mut self, index: usize) {
        let generation = self.providers[index].freshness.generation();
        for pending in self.pending.values_mut() {
            match pending {
                Pending::Query(provider, pending_generation, _)
                | Pending::Menu(provider, pending_generation, _)
                    if *provider == index =>
                {
                    *pending_generation = generation;
                }
                _ => (),
            }
        }
    }

    fn pending_ids(&self, index: usize, affected: Option<&[u64]>) -> Vec<u64> {
        self.pending
            .iter()
            .filter(|(id, pending)| {
                pending.provider() == index && affected.is_none_or(|ids| ids.contains(id))
            })
            .map(|(id, _)| *id)
            .collect()
    }

    fn invalidate(&mut self, index: usize) {
        self.providers[index].freshness.invalidate(None, None);
    }

    fn is_requesting_menu(&self, index: usize, key: &MenuKey) -> bool {
        self.pending
            .values()
            .any(|r| matches!(r, Pending::Menu(j, _, k) if *j == index && k == key))
    }

    fn menu(&mut self, key: &MenuKey) -> MenuResult {
        let mut out = Vec::new();
        for i in 0..self.providers.len() {
            let p = &self.providers[i];
            if p.disconnected {
                continue;
            }
            if let Some(answer) = p.cached_menu(key) {
                out.extend(answer.value.iter().cloned().map(|a| (i, a)));
            }
            if !p.has_fresh_menu(key) && !self.is_requesting_menu(i, key) {
                let epoch = p.freshness.generation();
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
        context: None,
    }
}
fn outcome_status(outcome: &protocol::Outcome) -> String {
    let mut status = match outcome.status {
        OutcomeStatus::Accepted => "Accepted",
        OutcomeStatus::Rejected => "Rejected",
        OutcomeStatus::Partial => "Partially accepted",
        OutcomeStatus::Unknown => "Outcome unknown",
    }
    .to_owned();
    if let Some((accepted, total)) = outcome.accepted.zip(outcome.total) {
        if outcome.status == OutcomeStatus::Unknown {
            status.push_str(&format!(" (at least {accepted}/{total} accepted)"));
        } else {
            status.push_str(&format!(" ({accepted}/{total})"));
        }
    }
    if let Some(job) = &outcome.job {
        status.push_str(&format!("\nJob: {job}"));
    }
    status
}
fn activation_summary(provider: &str, reply: protocol::Reply) -> String {
    let message = match &reply.outcome {
        Some(outcome) => format!("{}\n\n{}", outcome_status(outcome), reply.message),
        None => reply.message,
    };
    format!(
        "{provider}: {}",
        if message.is_empty() {
            "The provider did not report an outcome."
        } else {
            &message
        }
    )
}

fn present_activation(
    owner: &glib::WeakRef<gtk::Widget>,
    provider_id: &str,
    provider_name: &str,
    reply: protocol::Reply,
) {
    match reply.feedback() {
        Feedback::None => (),
        Feedback::Notice => notice(owner, provider_name, &reply.message),
        Feedback::Dialog | Feedback::Unrecognized => {
            notify(owner, &activation_summary(provider_id, reply));
        }
    }
}

fn notify_disconnected(owner: &glib::WeakRef<gtk::Widget>, provider_id: &str, unsent: bool) {
    let message = if unsent {
        "The provider disconnected before this action was sent."
    } else {
        "The provider disconnected. The action may already have been accepted; check its state before retrying."
    };
    notify(owner, &format!("{provider_id}: {message}"));
}

const NOTICE_DURATION: Duration = Duration::from_secs(4);

/// Falls back to the dialog when the menu owner has no overlay to host a notice.
fn notice(owner: &glib::WeakRef<gtk::Widget>, heading: &str, message: &str) {
    let Some(widget) = owner.upgrade() else {
        return;
    };
    let overlay = widget.clone().downcast::<gtk::Overlay>().ok().or_else(|| {
        widget
            .ancestor(gtk::Overlay::static_type())?
            .downcast()
            .ok()
    });
    match overlay {
        Some(overlay) => {
            super::notice::show_notice(&overlay, Some(heading), message, NOTICE_DURATION)
        }
        None => notify(owner, &format!("{heading}: {message}")),
    }
}

fn notify(owner: &glib::WeakRef<gtk::Widget>, message: &str) {
    let Some(owner) = owner.upgrade() else {
        return;
    };
    super::modal::show_information_dialog(&owner, &crate::i18n::tr("File availability"), message);
}

/// The selection as a menu key, or `None` when providers are not asked about it.
fn menu_key(paths: &[PathBuf], background: bool) -> Option<MenuKey> {
    if paths.is_empty() || paths.len() > protocol::PATH_LIMIT {
        return None;
    }
    let paths = paths
        .iter()
        .map(|p| p.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    protocol::selection_fits(&paths).then_some((paths, background))
}

/// One open menu's provider sections, refreshed while the popover stays visible.
struct MenuWatcher<F: Fn(bool)> {
    owner: glib::WeakRef<gtk::Widget>,
    popover: glib::WeakRef<gtk::PopoverMenu>,
    key: MenuKey,
    alive: std::rc::Rc<std::cell::Cell<u64>>,
    epoch: u64,
    renderer: menus::Renderer,
    changed: F,
}

impl<F: Fn(bool)> MenuWatcher<F> {
    fn poll(&mut self) -> glib::ControlFlow {
        let Some(popover) = self.popover.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if self.is_stale(&popover) {
            return glib::ControlFlow::Break;
        }
        let result = with_hub(|h| h.menu(&self.key));
        let navigation = menus::open_submenus(&popover);
        if self.renderer.update(result) {
            (self.changed)(!navigation.is_empty());
            menus::restore_submenus(&popover, &navigation);
        }
        glib::ControlFlow::Continue
    }

    fn is_stale(&self, popover: &gtk::PopoverMenu) -> bool {
        self.alive.get() != self.epoch || self.owner.upgrade().is_none() || !popover.is_visible()
    }
}

/// The caller owns the epoch: rebuilding/closing a menu cancels its old watcher.
pub(super) fn watch_menu(
    models: (gio::Menu, Option<gio::Menu>),
    group: gio::SimpleActionGroup,
    owner: (&gtk::Widget, &gtk::PopoverMenu),
    paths: Vec<PathBuf>,
    background: bool,
    lifetime: (std::rc::Rc<std::cell::Cell<u64>>, u64),
    changed: impl Fn(bool) + 'static,
) {
    let (owner, popover) = owner;
    let Some(key) = menu_key(&paths, background) else {
        return;
    };
    let (alive, epoch) = lifetime;
    let owner = owner.downgrade();
    let popover = popover.downgrade();
    let renderer = menus::Renderer::new(
        models,
        group,
        owner.clone(),
        popover.clone(),
        key.clone(),
        alive.clone(),
        epoch,
    );
    let mut watcher = MenuWatcher {
        owner,
        popover,
        key,
        alive,
        epoch,
        renderer,
        changed,
    };
    glib::timeout_add_local(Duration::from_millis(100), move || watcher.poll());
}

#[cfg(test)]
mod tests;
