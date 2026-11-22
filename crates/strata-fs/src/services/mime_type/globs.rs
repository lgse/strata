// SPDX-License-Identifier: MIT

//! Parses freedesktop `mime/globs2` rules so `mime_type` can safely key simple
//! extension lookups by extension without diverging from `gio::content_type_guess`
//! on literal filenames, prefix globs, compound suffixes, or case-sensitive globs.

use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// Maximum ASCII extension length eligible for stack-buffered extension keying.
pub(super) const MAX_EXT_LEN: usize = 32;

/// Key strategy for caching a filename's MIME description.
pub(super) enum CacheKey<'a> {
    /// Cache by lowercased extension (ASCII bytes stored in `buf[..len]`).
    ExtLower { buf: [u8; MAX_EXT_LEN], len: usize },
    /// Cache by exact-case extension (for `:cs` simple globs like `*.c` / `*.C`).
    ExtExact(&'a str),
    /// Fall back to caching by full filename.
    FullName,
}

/// Parsed non-simple glob patterns from the system and user `mime/globs2` files.
pub(super) struct GlobRules {
    loaded: bool,
    /// Lowercased extensions that have a case-sensitive simple glob (`:cs`).
    case_sensitive_exts: HashSet<String>,
    /// Lowercased final extensions that end a compound glob (e.g. `gz` from `*.tar.gz`).
    compound_tails: HashSet<String>,
    /// Lowercased literal whole-filename globs (e.g. `makefile`, `thumbs.db`).
    literal_names: HashSet<String>,
    /// Lowercased final extensions that appear in `literal_names` (prefilter).
    literal_exts: HashSet<String>,
    /// Lowercased literal prefixes for `prefix*` globs (e.g. `makefile.`, `readme`).
    prefixes: Vec<String>,
    /// First lowercased ASCII byte of each prefix in `prefixes` (prefilter).
    prefix_first_bytes: [bool; 256],
    /// Lowercased non-dot literal suffixes for `*suffix` globs (e.g. `~`, `%`, `,v`).
    special_suffixes: Vec<String>,
    /// Remaining wildcard globs (typically ~4 patterns, e.g. `*.so.[0-9]*`, `*.[1-9]`).
    complex_globs: Vec<String>,
}

impl GlobRules {
    pub(super) fn global() -> &'static Self {
        static RULES: OnceLock<GlobRules> = OnceLock::new();
        RULES.get_or_init(Self::load)
    }

    fn load() -> Self {
        let mut rules = Self::empty();
        for path in globs2_paths() {
            if let Ok(contents) = fs::read_to_string(&path) {
                rules.loaded = true;
                rules.parse_contents(&contents);
            }
        }
        rules
    }

    fn empty() -> Self {
        Self {
            loaded: false,
            case_sensitive_exts: HashSet::new(),
            compound_tails: HashSet::new(),
            literal_names: HashSet::new(),
            literal_exts: HashSet::new(),
            prefixes: Vec::new(),
            prefix_first_bytes: [false; 256],
            special_suffixes: Vec::new(),
            complex_globs: Vec::new(),
        }
    }

    fn parse_contents(&mut self, contents: &str) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split(':');
            let Some(_weight) = parts.next() else {
                continue;
            };
            let Some(_mime) = parts.next() else {
                continue;
            };
            let Some(pattern) = parts.next() else {
                continue;
            };
            let flags = parts.next().unwrap_or("");
            self.record_pattern(pattern, flags.split(',').any(|flag| flag == "cs"));
        }
    }

    fn record_pattern(&mut self, pattern: &str, case_sensitive: bool) {
        if let Some(ext) = simple_extension(pattern) {
            if case_sensitive {
                self.case_sensitive_exts.insert(ext.to_ascii_lowercase());
            }
            return;
        }
        if let Some(tail) = compound_extension_tail(pattern) {
            self.compound_tails.insert(tail.to_ascii_lowercase());
            return;
        }
        if !has_glob_meta(pattern) {
            self.record_literal(pattern);
            return;
        }
        if let Some(prefix) = pure_prefix(pattern) {
            self.record_prefix(prefix);
            return;
        }
        if let Some(suffix) = pure_suffix(pattern) {
            self.special_suffixes.push(suffix.to_ascii_lowercase());
            return;
        }
        self.complex_globs.push(pattern.to_ascii_lowercase());
    }

    fn record_literal(&mut self, pattern: &str) {
        let lower = pattern.to_ascii_lowercase();
        if let Some((_, ext)) = lower.rsplit_once('.')
            && !ext.is_empty()
        {
            self.literal_exts.insert(ext.to_owned());
        }
        self.literal_names.insert(lower);
    }

    fn record_prefix(&mut self, prefix: &str) {
        let lower = prefix.to_ascii_lowercase();
        if let Some(&first) = lower.as_bytes().first() {
            self.prefix_first_bytes[usize::from(first)] = true;
        }
        self.prefixes.push(lower);
    }

    pub(super) fn classify<'a>(&self, name: &'a str) -> CacheKey<'a> {
        let Some((stem, ext)) = self.split_cacheable_ext(name) else {
            return CacheKey::FullName;
        };
        let buf = lower_ext_buf(ext);
        let ext_lower = std::str::from_utf8(&buf[..ext.len()]).unwrap_or("");
        if self.needs_full_name(name, stem, ext_lower) {
            return CacheKey::FullName;
        }
        if self.case_sensitive_exts.contains(ext_lower) {
            CacheKey::ExtExact(ext)
        } else {
            CacheKey::ExtLower {
                buf,
                len: ext.len(),
            }
        }
    }

    fn split_cacheable_ext<'a>(&self, name: &'a str) -> Option<(&'a str, &'a str)> {
        if !self.loaded {
            return None;
        }
        let (stem, ext) = name.rsplit_once('.')?;
        let valid = !stem.is_empty() && !ext.is_empty() && ext.len() <= MAX_EXT_LEN;
        (valid && ext.is_ascii()).then_some((stem, ext))
    }

    fn needs_full_name(&self, name: &str, stem: &str, ext_lower: &str) -> bool {
        if stem.contains('.') && self.compound_tails.contains(ext_lower) {
            return true;
        }
        if self.matches_special_suffix(name) || self.matches_prefix(name) {
            return true;
        }
        if self.literal_exts.contains(ext_lower) && self.matches_literal_name(name) {
            return true;
        }
        self.matches_complex_glob(name)
    }

    fn matches_special_suffix(&self, name: &str) -> bool {
        self.special_suffixes
            .iter()
            .any(|suffix| ends_with_ignore_ascii_case(name, suffix))
    }

    fn matches_prefix(&self, name: &str) -> bool {
        let Some(&first) = name.as_bytes().first() else {
            return false;
        };
        if !self.prefix_first_bytes[usize::from(first.to_ascii_lowercase())] {
            return false;
        }
        self.prefixes
            .iter()
            .any(|prefix| starts_with_ignore_ascii_case(name, prefix))
    }

    fn matches_literal_name(&self, name: &str) -> bool {
        if name.len() <= 256 && name.is_ascii() {
            let mut buf = [0_u8; 256];
            for (dst, &byte) in buf.iter_mut().zip(name.as_bytes()) {
                *dst = byte.to_ascii_lowercase();
            }
            let lower = std::str::from_utf8(&buf[..name.len()]).unwrap_or("");
            self.literal_names.contains(lower)
        } else {
            self.literal_names.contains(&name.to_ascii_lowercase())
        }
    }

    fn matches_complex_glob(&self, name: &str) -> bool {
        self.complex_globs
            .iter()
            .any(|pattern| glob_matches_ci(pattern.as_bytes(), name.as_bytes()))
    }
}

fn globs2_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        dirs.push(Path::new(&home).join(".local/share"));
    }
    let system = env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty());
    match system {
        Some(value) => dirs.extend(env::split_paths(&value)),
        None => {
            dirs.push(PathBuf::from("/usr/local/share"));
            dirs.push(PathBuf::from("/usr/share"));
        }
    }
    dirs.into_iter()
        .map(|dir| dir.join("mime/globs2"))
        .collect()
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.bytes().any(|b| matches!(b, b'*' | b'?' | b'['))
}

fn simple_extension(pattern: &str) -> Option<&str> {
    let ext = pattern.strip_prefix("*.")?;
    (!ext.is_empty() && !ext.contains('.') && !has_glob_meta(ext)).then_some(ext)
}

fn compound_extension_tail(pattern: &str) -> Option<&str> {
    let rest = pattern.strip_prefix("*.")?;
    if has_glob_meta(rest) {
        return None;
    }
    let (_, tail) = rest.rsplit_once('.')?;
    (!tail.is_empty()).then_some(tail)
}

fn pure_prefix(pattern: &str) -> Option<&str> {
    let prefix = pattern.strip_suffix('*')?;
    (!prefix.is_empty() && !has_glob_meta(prefix)).then_some(prefix)
}

fn pure_suffix(pattern: &str) -> Option<&str> {
    let suffix = pattern.strip_prefix('*')?;
    (!suffix.is_empty() && !has_glob_meta(suffix)).then_some(suffix)
}

fn starts_with_ignore_ascii_case(text: &str, prefix_lower: &str) -> bool {
    text.len() >= prefix_lower.len()
        && text.as_bytes()[..prefix_lower.len()].eq_ignore_ascii_case(prefix_lower.as_bytes())
}

fn ends_with_ignore_ascii_case(text: &str, suffix_lower: &str) -> bool {
    text.len() >= suffix_lower.len()
        && text.as_bytes()[text.len() - suffix_lower.len()..]
            .eq_ignore_ascii_case(suffix_lower.as_bytes())
}

/// ASCII case-insensitive glob matcher supporting `*`, `?`, and `[...]` classes.
fn glob_matches_ci(pattern: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < text.len() {
        if pi < pattern.len() && pattern[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some(next_pi) = match_atom(pattern, pi, text[ti]) {
            pi = next_pi;
            ti += 1;
        } else if let Some((star_pi, star_ti)) = star {
            let next_ti = star_ti + 1;
            star = Some((star_pi, next_ti));
            pi = star_pi + 1;
            ti = next_ti;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}

fn match_atom(pattern: &[u8], pi: usize, byte: u8) -> Option<usize> {
    let p = *pattern.get(pi)?;
    if p == b'?' {
        return Some(pi + 1);
    }
    if p == b'[' {
        return match_class(pattern, pi + 1, byte.to_ascii_lowercase());
    }
    (p == byte.to_ascii_lowercase()).then_some(pi + 1)
}

fn lower_ext_buf(ext: &str) -> [u8; MAX_EXT_LEN] {
    let mut buf = [0_u8; MAX_EXT_LEN];
    for (dst, &byte) in buf.iter_mut().zip(ext.as_bytes()) {
        *dst = byte.to_ascii_lowercase();
    }
    buf
}

fn match_class(pattern: &[u8], start: usize, byte_lower: u8) -> Option<usize> {
    let negated = pattern.get(start).is_some_and(|&b| b == b'!' || b == b'^');
    let mut i = start + usize::from(negated);
    let mut matched = false;
    while let Some(&ch) = pattern.get(i).filter(|&&b| b != b']') {
        if let Some((end, hit)) = match_class_range(pattern, i, byte_lower) {
            matched |= hit;
            i = end;
        } else {
            matched |= ch == byte_lower;
            i += 1;
        }
    }
    let closed = pattern.get(i) == Some(&b']');
    (closed && matched != negated).then_some(i + 1)
}

fn match_class_range(pattern: &[u8], i: usize, byte_lower: u8) -> Option<(usize, bool)> {
    let [lo, b'-', hi] = *pattern.get(i..i + 3)? else {
        return None;
    };
    (hi != b']').then(|| (i + 3, (lo..=hi).contains(&byte_lower)))
}
