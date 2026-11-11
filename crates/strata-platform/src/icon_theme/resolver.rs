//! The lookup itself: a theme chain over a search path, each theme's directories listed once
//! and checked for changes on demand.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::index::{self, DirParams, ThemeIndex};
use super::matching;

/// The theme the spec makes every chain end in.
pub const FALLBACK_THEME: &str = "hicolor";

/// How many names of themes that no root has stay remembered, so a lookup in one doesn't read
/// the roots again. Each is checked on every revalidation, so an app that tries arbitrary
/// names (or a theme that inherits many missing ones) mustn't add one for good.
const UNKNOWN_THEMES: usize = 8;

/// Where icons are looked up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPath {
    /// The roots that hold themes (`<root>/<theme>/index.theme`) and unthemed icons, in order:
    /// a theme directory in an earlier root overrides the same directory in a later one.
    pub icon_dirs: Vec<PathBuf>,
    /// Further roots for unthemed icons only, searched after `icon_dirs`.
    pub pixmap_dirs: Vec<PathBuf>,
}

impl SearchPath {
    /// GTK 4's search path: `$XDG_DATA_HOME/icons` (by default `~/.local/share/icons`),
    /// `~/.icons`, then `icons` under each of `$XDG_DATA_DIRS` (by default `/usr/local/share`
    /// and `/usr/share`), with `pixmaps` under each of those for unthemed icons, and
    /// `/usr/share/pixmaps` last when they leave it out (the spec names it).
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// [`from_env`](Self::from_env) over `var`, which reads an environment variable.
    fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Self {
        let path = |name| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = path("HOME");
        let data_home = path("XDG_DATA_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        // Paths are bytes on Linux: a directory name that isn't UTF-8 is still one.
        let dirs = var("XDG_DATA_DIRS")
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let data_dirs: Vec<PathBuf> = std::env::split_paths(&dirs)
            .filter(|d| !d.as_os_str().is_empty())
            .collect();
        let mut icon_dirs: Vec<PathBuf> = data_home.map(|d| d.join("icons")).into_iter().collect();
        icon_dirs.extend(home.map(|h| h.join(".icons")));
        icon_dirs.extend(data_dirs.iter().map(|d| d.join("icons")));
        let mut pixmap_dirs: Vec<PathBuf> = data_dirs.iter().map(|d| d.join("pixmaps")).collect();
        let system = PathBuf::from("/usr/share/pixmaps");
        if !pixmap_dirs.contains(&system) {
            pixmap_dirs.push(system);
        }
        Self {
            icon_dirs,
            pixmap_dirs,
        }
    }
}

/// A file a lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The PNG or SVG file.
    pub path: PathBuf,
    /// The name of the request's that it was found under.
    pub name: String,
    /// The theme it came from, or `None` for an unthemed icon.
    pub theme: Option<String>,
}

impl Resolved {
    /// Whether it was found under a `-symbolic` name. Symbolic icons draw as their files
    /// paint them, not in a foreground colour (see the module docs).
    pub fn is_symbolic(&self) -> bool {
        self.name.ends_with("-symbolic")
    }
}

/// Finds icon files by name, size and scale, following the Icon Theme Specification the way
/// GTK 4's `GtkIconTheme` does (the module docs give the rules).
///
/// It reads the file system: a theme's `index.theme` and directory listings the first time a
/// lookup reaches the theme, and the timestamps of every directory it has read on
/// [`revalidate`](Self::revalidate). Call it off the UI thread; [`IconLoader`] does.
///
/// [`IconLoader`]: super::IconLoader
#[derive(Debug)]
pub struct IconResolver {
    search: SearchPath,
    themes: HashMap<String, Theme>,
    /// The names in `themes` that no root had when read, oldest first.
    unknown: VecDeque<String>,
    unthemed: Option<Unthemed>,
}

impl IconResolver {
    pub fn new(search: SearchPath) -> Self {
        Self {
            search,
            themes: HashMap::new(),
            unknown: VecDeque::new(),
            unthemed: None,
        }
    }

    pub fn search_path(&self) -> &SearchPath {
        &self.search
    }

    /// The best file for the first of `names` that `theme`'s chain has, at `size` logical
    /// pixels and integer `scale` (clamped to 1..=4096 and 1..=8), or `None`.
    ///
    /// Themes are searched outside names: a later name in the theme beats an earlier name in
    /// a theme it inherits. Leading `-symbolic` names are first searched through the whole
    /// chain. Unthemed icons (the roots' own files, then pixmaps) come last.
    pub fn resolve(
        &mut self,
        theme: &str,
        names: &[impl AsRef<str>],
        size: u32,
        scale: u32,
    ) -> Option<Resolved> {
        let size = size.clamp(1, 4096) as i32;
        let scale = scale.clamp(1, 8) as i32;
        let names: Vec<&str> = names
            .iter()
            .map(AsRef::as_ref)
            .filter(|n| !n.is_empty())
            .collect();
        let chain = self.chain(theme);
        let found = |theme: &String, name: &str| {
            let path = self.themes.get(theme)?.lookup(name, size, scale)?;
            Some(Resolved {
                path,
                name: name.to_owned(),
                theme: Some(theme.clone()),
            })
        };
        // GTK prefers a symbolic icon from an inherited theme over a full-colour one in the
        // current theme when the request leads with symbolic names.
        let symbolic = names.iter().take_while(|n| n.ends_with("-symbolic"));
        for theme in &chain {
            if let Some(hit) = symbolic.clone().find_map(|n| found(theme, n)) {
                return Some(hit);
            }
        }
        for theme in &chain {
            if let Some(hit) = names.iter().find_map(|n| found(theme, n)) {
                return Some(hit);
            }
        }
        let unthemed = self
            .unthemed
            .get_or_insert_with(|| Unthemed::load(&self.search));
        names.iter().find_map(|n| {
            let path = unthemed.lookup(n)?;
            Some(Resolved {
                path,
                name: (*n).to_owned(),
                theme: None,
            })
        })
    }

    /// Checks the timestamps of every directory and `index.theme` read so far, and reads again
    /// each theme (and the unthemed roots) where one changed, appeared or went away. Returns
    /// whether anything did.
    pub fn revalidate(&mut self) -> bool {
        let mut changed = false;
        let stale: Vec<String> = self
            .themes
            .iter()
            .filter(|(_, t)| t.watched.changed())
            .map(|(n, _)| n.clone())
            .collect();
        for name in stale {
            let theme = Theme::load(&name, &self.search.icon_dirs);
            self.keep(name, theme);
            changed = true;
        }
        if self.unthemed.as_ref().is_some_and(|u| u.watched.changed()) {
            self.unthemed = Some(Unthemed::load(&self.search));
            changed = true;
        }
        changed
    }

    /// The themes a lookup in `theme` searches: `theme`, what it inherits depth-first in
    /// order, then hicolor. A theme without an `index.theme` is left out with what it
    /// inherits, as GTK does; so is a theme name that isn't one path component.
    fn chain(&mut self, theme: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut visited = Vec::new();
        if valid_theme_name(theme) {
            self.visit(theme, &mut chain, &mut visited);
        }
        self.visit(FALLBACK_THEME, &mut chain, &mut visited);
        chain
    }

    /// Stores a theme just read. A missing one joins the unknown names, and the oldest unknown
    /// name beyond [`UNKNOWN_THEMES`] that is still missing is forgotten.
    fn keep(&mut self, name: String, theme: Theme) {
        if !theme.found && !self.unknown.contains(&name) {
            self.unknown.push_back(name.clone());
            if self.unknown.len() > UNKNOWN_THEMES
                && let Some(oldest) = self.unknown.pop_front()
                && self.themes.get(&oldest).is_some_and(|t| !t.found)
            {
                self.themes.remove(&oldest);
            }
        }
        self.themes.insert(name, theme);
    }

    fn visit(&mut self, name: &str, chain: &mut Vec<String>, visited: &mut Vec<String>) {
        if visited.iter().any(|v| v == name) || !valid_theme_name(name) {
            return;
        }
        visited.push(name.to_owned());
        if !self.themes.contains_key(name) {
            let theme = Theme::load(name, &self.search.icon_dirs);
            self.keep(name.to_owned(), theme);
        }
        let theme = &self.themes[name];
        if !theme.found {
            return;
        }
        let inherits = theme.inherits.clone();
        chain.push(name.to_owned());
        for parent in inherits {
            self.visit(&parent, chain, visited);
        }
    }
}

fn valid_theme_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// A file's best format in one directory: PNG beats SVG, as in GTK. XPM is left out (Rune
/// can't decode it), so a name with only an XPM falls through to the next directory or name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Suffix {
    Svg,
    Png,
}

impl Suffix {
    fn split(file: &str) -> Option<(&str, Suffix)> {
        // GTK's pre-rendered symbolic PNG files encode their colours in the channels.
        if file.ends_with(".symbolic.png") {
            return None;
        }
        if let Some(name) = file.strip_suffix(".png") {
            Some((name, Suffix::Png))
        } else {
            file.strip_suffix(".svg").map(|name| (name, Suffix::Svg))
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Suffix::Png => "png",
            Suffix::Svg => "svg",
        }
    }
}

/// The modification times of what a load read (`None`: missing), to notice changes.
#[derive(Debug, Default)]
struct Watched(Vec<(PathBuf, Option<SystemTime>)>);

impl Watched {
    /// Records `path`'s time now, before it is read, so a change during the read shows later.
    fn add(&mut self, path: PathBuf) {
        let stamp = stamp(&path);
        self.0.push((path, stamp));
    }

    fn changed(&self) -> bool {
        self.0.iter().any(|(path, was)| stamp(path) != *was)
    }
}

fn stamp(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Lists `dir`, calling `found` with each PNG or SVG file's name and format.
fn list(dir: &Path, mut found: impl FnMut(&str, Suffix)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        if let Some((name, suffix)) = file.to_str().and_then(Suffix::split) {
            found(name, suffix);
        }
    }
}

/// Directories with the same size parameters, merged into one class as GTK does. Each name
/// keeps the first directory (in `Directories` order, then root order) that has it.
#[derive(Debug)]
struct SizeClass {
    params: DirParams,
    files: HashMap<Box<str>, (u32, Suffix)>,
}

#[derive(Debug)]
struct Theme {
    /// Whether any root has the theme's `index.theme` (hicolor always counts).
    found: bool,
    inherits: Vec<String>,
    classes: Vec<SizeClass>,
    dirs: Vec<PathBuf>,
    watched: Watched,
}

impl Theme {
    fn load(name: &str, roots: &[PathBuf]) -> Theme {
        let mut watched = Watched::default();
        let mut index: Option<ThemeIndex> = None;
        // Only roots that have the theme are listed: most have none of a given theme, and
        // hicolor alone names hundreds of directories. A root that gains the theme later
        // changes its watched `<root>/<theme>`, and the theme is read again.
        let mut present: Vec<&PathBuf> = Vec::new();
        for root in roots {
            let dir = root.join(name);
            let file = dir.join("index.theme");
            if dir.is_dir() {
                present.push(root);
            }
            watched.add(dir);
            watched.add(file.clone());
            if index.is_none() {
                index = fs::read_to_string(&file)
                    .ok()
                    .and_then(|text| index::parse(&text));
            }
        }
        let found = index.is_some() || name == FALLBACK_THEME;
        let index = match index {
            Some(index) => index,
            None if found => index::derived_hicolor(subdirs(roots, name)),
            None => ThemeIndex::default(),
        };
        let mut theme = Theme {
            found,
            inherits: index.inherits,
            classes: Vec::new(),
            dirs: Vec::new(),
            watched,
        };
        for spec in index.dirs {
            let class = match theme.classes.iter().position(|c| c.params == spec.params) {
                Some(i) => i,
                None => {
                    theme.classes.push(SizeClass {
                        params: spec.params,
                        files: HashMap::new(),
                    });
                    theme.classes.len() - 1
                }
            };
            for root in &present {
                let dir = root.join(name).join(&spec.path);
                theme.watched.add(dir.clone());
                let mut here: HashMap<Box<str>, Suffix> = HashMap::new();
                list(&dir, |name, suffix| {
                    let best = here.entry(name.into()).or_insert(suffix);
                    *best = (*best).max(suffix);
                });
                if here.is_empty() {
                    continue;
                }
                let at = theme.dirs.len() as u32;
                theme.dirs.push(dir);
                let files = &mut theme.classes[class].files;
                for (name, suffix) in here {
                    files.entry(name).or_insert((at, suffix));
                }
            }
        }
        theme
    }

    fn lookup(&self, name: &str, size: i32, scale: i32) -> Option<PathBuf> {
        let mut best: Option<(&DirParams, i32, (u32, Suffix))> = None;
        for class in &self.classes {
            let Some(&file) = class.files.get(name) else {
                continue;
            };
            let d = matching::difference(&class.params, size, scale);
            let wins = best.is_none_or(|(params, bd, _)| {
                matching::better(&class.params, d, params, bd, size, scale)
            });
            if wins {
                best = Some((&class.params, d, file));
            }
        }
        let (_, _, (dir, suffix)) = best?;
        Some(self.dirs[dir as usize].join(format!("{name}.{}", suffix.ext())))
    }
}

/// `<root>/<theme>/<a>/<b>` directory paths (`a/b`) in every root, for a hicolor without an
/// `index.theme`.
fn subdirs(roots: &[PathBuf], theme: &str) -> Vec<String> {
    let names = |dir: &Path| -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    };
    let mut out = Vec::new();
    for root in roots {
        let base = root.join(theme);
        for class in names(&base) {
            for context in names(&base.join(&class)) {
                out.push(format!("{class}/{context}"));
            }
        }
    }
    out
}

/// Icons directly in a root: the first PNG and the first SVG of each name, in root order. A
/// PNG beats an SVG, as in GTK.
#[derive(Debug)]
struct Unthemed {
    files: HashMap<Box<str>, (Option<PathBuf>, Option<PathBuf>)>,
    watched: Watched,
}

impl Unthemed {
    fn load(search: &SearchPath) -> Unthemed {
        let mut watched = Watched::default();
        let mut files: HashMap<Box<str>, (Option<PathBuf>, Option<PathBuf>)> = HashMap::new();
        for root in search.icon_dirs.iter().chain(&search.pixmap_dirs) {
            watched.add(root.clone());
            list(root, |name, suffix| {
                let (png, svg) = files.entry(name.into()).or_default();
                let slot = if suffix == Suffix::Png { png } else { svg };
                if slot.is_none() {
                    *slot = Some(root.join(format!("{name}.{}", suffix.ext())));
                }
            });
        }
        Unthemed { files, watched }
    }

    fn lookup(&self, name: &str) -> Option<PathBuf> {
        let (png, svg) = self.files.get(name)?;
        png.clone().or_else(|| svg.clone())
    }
}
