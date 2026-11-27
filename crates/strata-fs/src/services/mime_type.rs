// SPDX-License-Identifier: MIT

use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc};

use crate::model::{EntryKind, FileEntry};

mod globs;

use globs::{CacheKey, GlobRules};

pub const FOLDER_TYPE_NAME: &str = "Folder";
pub const BROKEN_LINK_TYPE_NAME: &str = "Broken link";
pub const OTHER_TYPE_NAME: &str = "Other";

const TYPE_CACHE_LIMIT: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryType {
    Folder,
    BrokenLink,
    Known(String),
    Other,
}

impl EntryType {
    pub fn description(&self) -> &str {
        match self {
            Self::Folder => FOLDER_TYPE_NAME,
            Self::BrokenLink => BROKEN_LINK_TYPE_NAME,
            Self::Known(description) => description.as_str(),
            Self::Other => OTHER_TYPE_NAME,
        }
    }
}

pub fn entry_type(entry: &FileEntry) -> EntryType {
    entry_type_for_name(entry.kind, &entry.display_name)
}

pub fn entry_type_for_name(kind: EntryKind, name: &str) -> EntryType {
    match kind {
        EntryKind::Directory | EntryKind::DirectorySymbolicLink => EntryType::Folder,
        EntryKind::SymbolicLink => EntryType::BrokenLink,
        EntryKind::Other => EntryType::Other,
        EntryKind::File | EntryKind::FileSymbolicLink => mime_type_for_name(name),
    }
}

/// Shared interned description for `kind` and `name`, avoiding per-entry `String` allocations.
pub fn entry_type_description_for_name(kind: EntryKind, name: &str) -> Rc<str> {
    match kind {
        EntryKind::Directory | EntryKind::DirectorySymbolicLink => {
            TYPE_CACHE.with_borrow(|cache| cache.folder.clone())
        }
        EntryKind::SymbolicLink => TYPE_CACHE.with_borrow(|cache| cache.broken_link.clone()),
        EntryKind::Other => TYPE_CACHE.with_borrow(|cache| cache.other.clone()),
        EntryKind::File | EntryKind::FileSymbolicLink => mime_description_rc(name),
    }
}

pub fn mime_type_for_name(name: &str) -> EntryType {
    let description = mime_description_rc(name);
    if description.as_ref() == OTHER_TYPE_NAME {
        EntryType::Other
    } else {
        EntryType::Known(description.to_string())
    }
}

#[cfg(test)]
pub fn mime_description_for_name(name: &str) -> String {
    mime_description_rc(name).to_string()
}

pub fn mime_description_rc(name: &str) -> Rc<str> {
    let key = GlobRules::global().classify(name);
    TYPE_CACHE.with_borrow_mut(|cache| match key {
        CacheKey::ExtLower { buf, len } => {
            let ext = std::str::from_utf8(&buf[..len]).unwrap_or("");
            cache.get_or_insert_ext(ext, name)
        }
        CacheKey::ExtExact(ext) => cache.get_or_insert_exact_ext(ext, name),
        CacheKey::FullName => cache.get_or_insert_name(name),
    })
}

fn guess_mime_description(name: &str) -> String {
    let (content_type, _) = super::guess_content_type(Path::new(name));
    if content_type.is_empty() || content_type == "application/octet-stream" {
        return OTHER_TYPE_NAME.to_owned();
    }
    let description = gio::content_type_get_description(&content_type);
    if description.is_empty() {
        return OTHER_TYPE_NAME.to_owned();
    }
    description.to_string()
}

struct TypeCache {
    ext_lower: HashMap<String, Rc<str>>,
    ext_exact: HashMap<String, Rc<str>>,
    by_name: HashMap<String, Rc<str>>,
    interned: HashMap<Rc<str>, Rc<str>>,
    folder: Rc<str>,
    broken_link: Rc<str>,
    other: Rc<str>,
}

#[derive(Clone, Copy)]
enum CacheBucket {
    ExtLower,
    ExtExact,
    ByName,
}

impl TypeCache {
    fn new() -> Self {
        let folder: Rc<str> = Rc::from(FOLDER_TYPE_NAME);
        let broken_link: Rc<str> = Rc::from(BROKEN_LINK_TYPE_NAME);
        let other: Rc<str> = Rc::from(OTHER_TYPE_NAME);
        let mut interned = HashMap::new();
        interned.insert(folder.clone(), folder.clone());
        interned.insert(broken_link.clone(), broken_link.clone());
        interned.insert(other.clone(), other.clone());
        Self {
            ext_lower: HashMap::new(),
            ext_exact: HashMap::new(),
            by_name: HashMap::new(),
            interned,
            folder,
            broken_link,
            other,
        }
    }

    fn intern(&mut self, description: String) -> Rc<str> {
        if let Some(existing) = self.interned.get(description.as_str()) {
            return existing.clone();
        }
        if self.interned.len() >= TYPE_CACHE_LIMIT {
            self.interned.clear();
            self.interned
                .insert(self.folder.clone(), self.folder.clone());
            self.interned
                .insert(self.broken_link.clone(), self.broken_link.clone());
            self.interned.insert(self.other.clone(), self.other.clone());
        }
        let shared: Rc<str> = Rc::from(description);
        self.interned.insert(shared.clone(), shared.clone());
        shared
    }

    fn bucket(&self, which: CacheBucket) -> &HashMap<String, Rc<str>> {
        match which {
            CacheBucket::ExtLower => &self.ext_lower,
            CacheBucket::ExtExact => &self.ext_exact,
            CacheBucket::ByName => &self.by_name,
        }
    }

    fn bucket_mut(&mut self, which: CacheBucket) -> &mut HashMap<String, Rc<str>> {
        match which {
            CacheBucket::ExtLower => &mut self.ext_lower,
            CacheBucket::ExtExact => &mut self.ext_exact,
            CacheBucket::ByName => &mut self.by_name,
        }
    }

    fn cached_description(&mut self, which: CacheBucket, key: &str, name: &str) -> Rc<str> {
        if let Some(description) = self.bucket(which).get(key) {
            return description.clone();
        }
        let description = self.intern(guess_mime_description(name));
        let bucket = self.bucket_mut(which);
        if bucket.len() >= TYPE_CACHE_LIMIT {
            bucket.clear();
        }
        bucket.insert(key.to_owned(), description.clone());
        description
    }

    fn get_or_insert_ext(&mut self, ext: &str, name: &str) -> Rc<str> {
        self.cached_description(CacheBucket::ExtLower, ext, name)
    }

    fn get_or_insert_exact_ext(&mut self, ext: &str, name: &str) -> Rc<str> {
        self.cached_description(CacheBucket::ExtExact, ext, name)
    }

    fn get_or_insert_name(&mut self, name: &str) -> Rc<str> {
        self.cached_description(CacheBucket::ByName, name, name)
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.ext_lower.clear();
        self.ext_exact.clear();
        self.by_name.clear();
    }
}

thread_local! {
    static TYPE_CACHE: RefCell<TypeCache> = RefCell::new(TypeCache::new());
}

#[cfg(test)]
mod tests;
