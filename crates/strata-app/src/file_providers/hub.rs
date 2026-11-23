// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

use rune::Pixels;
use strata_fs::services::file_providers::{
    self as protocol, Client, Decoration, Feedback, MenuAction, OutcomeStatus, Registration,
    Request, Update,
};

use super::{
    freshness::Freshness,
    menus::{MenuKey, MenuResult},
};

const TTL: Duration = Duration::from_secs(5);
const MAX_AGE: Duration = Duration::from_secs(15);
const MAX_CACHED_DECORATIONS: usize = 2048;
const MAX_CACHED_MENUS: usize = 32;

pub(super) struct Answer<T> {
    pub(super) at: Instant,
    pub(super) generation: u64,
    pub(super) value: T,
}

pub(super) struct Provider {
    client: Client,
    pub(super) icons: HashMap<String, Pixels>,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) cache: HashMap<String, Answer<Decoration>>,
    menus: HashMap<MenuKey, Answer<Vec<MenuAction>>>,
    freshness: Freshness,
    disconnected: bool,
    last_busy: Option<Instant>,
}

pub(super) enum Pending {
    Query(usize, u64, Vec<String>),
    Menu(usize, u64, MenuKey),
    Activate(usize),
}

pub(super) struct Retiring {
    client: Client,
    id: String,
    name: String,
    waiting: Vec<u64>,
}

pub(super) struct Badge<'a> {
    pub(super) priority: u8,
    pub(super) provider: &'a str,
    pub(super) icon_id: &'a str,
    pub(super) pixels: &'a Pixels,
    pub(super) description: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effect {
    Dialog(String),
    Notice { heading: String, message: String },
}

pub(super) struct Hub {
    pub(super) discovery: Option<mpsc::Receiver<Vec<Registration>>>,
    discovered: bool,
    pub(super) providers: Vec<Provider>,
    pub(super) retiring: Vec<Retiring>,
    pub(super) pending: HashMap<u64, Pending>,
    serial: u64,
}

pub(super) fn spawn_discovery() -> mpsc::Receiver<Vec<Registration>> {
    let (tx, rx) = mpsc::channel();
    let root = protocol::config_root();
    std::thread::spawn(move || {
        let _ = tx.send(root.map_or_else(Vec::new, |p| protocol::discover(&p)));
    });
    rx
}

fn decode_png(bytes: &[u8]) -> Option<Pixels> {
    let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = Pixels::new();
    pixels.set(width, height, rgba.into_raw());
    Some(pixels)
}

impl Pending {
    fn generation(&self) -> u64 {
        match self {
            Self::Query(_, generation, _) | Self::Menu(_, generation, _) => *generation,
            Self::Activate(_) => 0,
        }
    }

    fn provider(&self) -> usize {
        match self {
            Self::Query(i, _, _) | Self::Menu(i, _, _) | Self::Activate(i) => *i,
        }
    }
}

impl Provider {
    pub(super) fn start(registration: Registration) -> Self {
        let icons: HashMap<String, Pixels> = registration
            .icons
            .iter()
            .filter_map(|(k, bytes)| decode_png(bytes).map(|px| (k.clone(), px)))
            .collect();
        let id = registration.manifest.id.clone();
        let name = registration
            .manifest
            .name
            .clone()
            .unwrap_or_else(|| id.clone());
        Self {
            client: protocol::start(registration),
            icons,
            id,
            name,
            cache: HashMap::new(),
            menus: HashMap::new(),
            freshness: Freshness::default(),
            disconnected: false,
            last_busy: None,
        }
    }

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
        let now = Instant::now();
        for path in &paths {
            self.cache.insert(
                path.clone(),
                Answer {
                    at: now,
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
                        at: now,
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
            .map(|(k, _)| k.clone())
        {
            self.menus.remove(&oldest);
        }
    }

    fn needs_menu_eviction(&self, key: &MenuKey) -> bool {
        self.menus.len() >= MAX_CACHED_MENUS && !self.menus.contains_key(key)
    }

    fn store_menu(&mut self, key: MenuKey, generation: u64, actions: Vec<MenuAction>) {
        if self.needs_menu_eviction(&key) {
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
        let (icon_id, pixels) = answer
            .value
            .badge
            .as_ref()
            .and_then(|badge| Some((badge.as_str(), self.icons.get(badge)?)))?;
        Some(Badge {
            priority: answer.value.priority,
            provider: &self.id,
            icon_id,
            pixels,
            description: answer.value.description.as_str(),
        })
    }
}

impl Retiring {
    fn settle(&mut self, answered: &mut Vec<String>, effects: &mut Vec<Effect>) {
        let (updates, closed) = self.client.updates.drain();
        for update in updates {
            self.apply_update(update, answered, effects);
        }
        if closed {
            for _ in self.waiting.drain(..) {
                effects.push(Effect::Dialog(disconnected_message(&self.id, false)));
            }
        }
    }

    fn apply_update(
        &mut self,
        update: Update,
        answered: &mut Vec<String>,
        effects: &mut Vec<Effect>,
    ) {
        match update {
            Update::Reply(reply) => self.settle_reply(*reply, answered, effects),
            Update::Offline { unsent, in_flight } => {
                self.settle_offline(&unsent, in_flight, effects);
            }
        }
    }

    fn settle_reply(
        &mut self,
        reply: protocol::Reply,
        answered: &mut Vec<String>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(pos) = reply
            .id
            .and_then(|id| self.waiting.iter().position(|w| *w == id))
        else {
            return;
        };
        self.waiting.swap_remove(pos);
        effects.extend(activation_effect(&self.id, &self.name, reply));
        answered.push(self.id.clone());
    }

    fn settle_offline(
        &mut self,
        unsent: &[u64],
        in_flight: Option<u64>,
        effects: &mut Vec<Effect>,
    ) {
        for id in unsent.iter().copied().chain(in_flight) {
            if let Some(pos) = self.waiting.iter().position(|w| *w == id) {
                self.waiting.swap_remove(pos);
                effects.push(Effect::Dialog(disconnected_message(
                    &self.id,
                    unsent.contains(&id),
                )));
            }
        }
    }
}

impl Hub {
    pub(super) fn new() -> Self {
        Self {
            discovery: None,
            discovered: false,
            providers: Vec::new(),
            retiring: Vec::new(),
            pending: HashMap::new(),
            serial: 0,
        }
    }

    pub(super) fn idle(&self) -> bool {
        self.discovered && self.providers.is_empty() && self.retiring.is_empty()
    }

    pub(super) fn reload(&mut self, discovery: mpsc::Receiver<Vec<Registration>>) {
        self.discovery = Some(discovery);
        self.discovered = false;
        for (index, provider) in std::mem::take(&mut self.providers).into_iter().enumerate() {
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
        self.pending.clear();
    }

    fn waiting_activations(&self, index: usize) -> Vec<u64> {
        self.pending
            .iter()
            .filter_map(|(id, p)| matches!(p, Pending::Activate(i) if *i == index).then_some(*id))
            .collect()
    }

    pub(super) fn send(
        &mut self,
        provider: usize,
        mut req: Request,
        pending: Pending,
    ) -> Option<Effect> {
        self.serial += 1;
        req.id = self.serial;
        if self.providers[provider]
            .client
            .requests
            .try_send(req)
            .is_ok()
        {
            self.pending.insert(self.serial, pending);
            return None;
        }
        let entry = &mut self.providers[provider];
        if entry
            .last_busy
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(10))
        {
            eprintln!(
                "Strata provider {}: host queue full or closed; request not sent",
                entry.id
            );
            entry.last_busy = Some(Instant::now());
        }
        matches!(pending, Pending::Activate(_)).then(|| {
            Effect::Dialog(format!(
                "{}: The file provider is busy or unavailable. This action was not sent.",
                entry.id
            ))
        })
    }

    pub(super) fn drain(&mut self, effects: &mut Vec<Effect>) {
        self.settle_retiring(effects);
        self.adopt_discovery();
        for index in 0..self.providers.len() {
            self.drain_provider(index, effects);
        }
    }

    fn settle_retiring(&mut self, effects: &mut Vec<Effect>) {
        let mut answered = Vec::new();
        for retiring in &mut self.retiring {
            retiring.settle(&mut answered, effects);
        }
        self.retiring.retain(|r| !r.waiting.is_empty());
        for provider in &mut self.providers {
            if answered.contains(&provider.id) {
                provider.freshness.invalidate(None, None);
            }
        }
    }

    fn adopt_discovery(&mut self) {
        let Some(registrations) = self.discovery.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.discovered = true;
        self.discovery = None;
        for registration in registrations {
            self.providers.push(Provider::start(registration));
        }
    }

    fn drain_provider(&mut self, index: usize, effects: &mut Vec<Effect>) {
        if self.providers[index].disconnected {
            return;
        }
        let (updates, closed) = self.providers[index].client.updates.drain();
        for update in updates {
            self.apply_update(index, update, effects);
        }
        if closed {
            effects.extend(self.offline(index, &[], None));
            self.providers[index].disconnected = true;
        }
    }

    pub(super) fn apply_update(&mut self, index: usize, update: Update, effects: &mut Vec<Effect>) {
        match update {
            Update::Offline { unsent, in_flight } => {
                let affected: Vec<_> = unsent.iter().copied().chain(in_flight).collect();
                effects.extend(self.offline(index, &unsent, Some(&affected)));
            }
            Update::Reply(reply) if reply.event.is_some() => {
                self.providers[index]
                    .freshness
                    .invalidate(reply.paths.as_deref(), reply.revision);
            }
            Update::Reply(reply) => {
                effects.extend(self.settle_reply(index, *reply));
            }
        }
    }

    fn settle_reply(&mut self, index: usize, reply: protocol::Reply) -> Option<Effect> {
        let pending = reply.id.and_then(|id| self.pending.remove(&id))?;
        if pending.provider() != index {
            return None;
        }
        let provider = &mut self.providers[index];
        let generation = if reply.revision.is_some() {
            provider.freshness.generation()
        } else {
            pending.generation()
        };
        match pending {
            Pending::Query(_, _, paths) => {
                if provider.accepts(&paths, &reply) {
                    provider.store_query(paths, generation, reply.decorations.unwrap_or_default());
                }
                None
            }
            Pending::Menu(_, _, key) => {
                if provider.accepts(&key.0, &reply) {
                    provider.store_menu(key, generation, reply.actions.unwrap_or_default());
                }
                None
            }
            Pending::Activate(_) => {
                let effect = activation_effect(&provider.id, &provider.name, reply);
                provider.freshness.invalidate(None, None);
                effect
            }
        }
    }

    fn offline(&mut self, index: usize, unsent: &[u64], affected: Option<&[u64]>) -> Vec<Effect> {
        self.providers[index].freshness = Freshness::default();
        self.providers[index].cache.clear();
        self.providers[index].menus.clear();
        self.rebase_pending(index);
        let ids: Vec<u64> = self
            .pending
            .iter()
            .filter(|(id, p)| p.provider() == index && affected.is_none_or(|a| a.contains(id)))
            .map(|(id, _)| *id)
            .collect();
        let mut effects = Vec::new();
        for id in ids {
            if let Some(Pending::Activate(_)) = self.pending.remove(&id) {
                effects.push(Effect::Dialog(disconnected_message(
                    &self.providers[index].id,
                    unsent.contains(&id),
                )));
            }
        }
        effects
    }

    fn rebase_pending(&mut self, index: usize) {
        let generation = self.providers[index].freshness.generation();
        for pending in self.pending.values_mut() {
            if let Pending::Query(i, g, _) | Pending::Menu(i, g, _) = pending
                && *i == index
            {
                *g = generation;
            }
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

    pub(super) fn request_missing_decorations(
        &mut self,
        visible: &[String],
        effects: &mut Vec<Effect>,
    ) {
        for index in 0..self.providers.len() {
            if self.providers[index].disconnected {
                continue;
            }
            let missing = self.missing_decorations(index, visible);
            if !missing.is_empty() {
                let epoch = self.providers[index].freshness.generation();
                let req = request("query", missing.clone(), false);
                effects.extend(self.send(index, req, Pending::Query(index, epoch, missing)));
            }
        }
    }

    pub(super) fn badges(&self, path: &str) -> Vec<Badge<'_>> {
        let mut badges: Vec<_> = self
            .providers
            .iter()
            .filter_map(|p| p.badge(path))
            .collect();
        badges.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.provider.cmp(b.provider)));
        badges
    }

    fn is_requesting_menu(&self, index: usize, key: &MenuKey) -> bool {
        self.pending
            .values()
            .any(|p| matches!(p, Pending::Menu(j, _, k) if *j == index && k == key))
    }

    pub(super) fn menu(&mut self, key: &MenuKey, effects: &mut Vec<Effect>) -> MenuResult {
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
                let req = request("menu", key.0.clone(), key.1);
                effects.extend(self.send(i, req, Pending::Menu(i, epoch, key.clone())));
            }
        }
        out
    }
}

pub(super) fn request(method: &str, paths: Vec<String>, background: bool) -> Request {
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

fn outcome_status(reply: &protocol::Reply) -> Option<String> {
    let outcome = reply.outcome.as_ref()?;
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
    Some(status)
}

pub(super) fn activation_summary(provider: &str, reply: protocol::Reply) -> String {
    let message = match outcome_status(&reply) {
        Some(status) => format!("{status}\n\n{}", reply.message),
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

pub(super) fn disconnected_message(provider_id: &str, unsent: bool) -> String {
    let message = if unsent {
        "The provider disconnected before this action was sent."
    } else {
        "The provider disconnected. The action may already have been accepted; check its state before retrying."
    };
    format!("{provider_id}: {message}")
}

pub(super) fn activation_effect(
    provider_id: &str,
    provider_name: &str,
    reply: protocol::Reply,
) -> Option<Effect> {
    match reply.feedback() {
        Feedback::None => None,
        Feedback::Notice => Some(Effect::Notice {
            heading: provider_name.to_owned(),
            message: reply.message,
        }),
        Feedback::Dialog | Feedback::Unrecognized => {
            Some(Effect::Dialog(activation_summary(provider_id, reply)))
        }
    }
}
