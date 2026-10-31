// SPDX-License-Identifier: MIT

#[path = "fs_observe.rs"]
mod observe;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    fixture::expand_directory,
    scenario::{Image, Thumbnail},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PixelAt {
    at: [u32; 2],
    rgb: [u8; 3],
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Mtime {
    Epoch(i64),
    Local { local: [i32; 6] },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TarGz {
    mode: String,
    mtime: i64,
    members: Vec<(String, TarText)>,
    flip_from_end: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum TarText {
    Text(String),
    Repeat { repeat: String, count: usize },
}

impl TarText {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Text(text) => text.as_bytes().to_vec(),
            Self::Repeat { repeat, count } => repeat.repeat(*count).into_bytes(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultApp {
    #[serde(rename = "for")]
    file: String,
    is: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Launches {
    count: Option<usize>,
    within: Option<String>,
    first_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceLines {
    suffix: String,
    prefix: String,
    with: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FsStep {
    pub(crate) fs: FsCheck,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FsCheck {
    pub(crate) path: String,
    exists: Option<bool>,
    text: Option<String>,
    names: Option<Vec<String>>,
    is: Option<String>,
    empty: Option<bool>,
    unchanged: Option<String>,
    copy_of: Option<String>,
    size_of: Option<String>,
    write: Option<String>,
    remember: Option<String>,
    mkdir: Option<bool>,
    parents: Option<bool>,
    chmod: Option<String>,
    populate: Option<Value>,
    image: Option<Image>,
    ffmpeg: Option<Vec<String>>,
    volume: Option<String>,
    prefix: Option<String>,
    #[serde(default, rename = "with")]
    volume_files: BTreeMap<String, String>,
    symlink: Option<String>,
    program: Option<String>,
    has: Option<Has>,
    names_without: Option<String>,
    can_trash: Option<bool>,
    remove_volume: Option<bool>,
    remember_names: Option<String>,
    names_from: Option<String>,
    #[serde(default)]
    minus: Vec<String>,
    #[serde(default)]
    plus: Vec<String>,
    remove: Option<bool>,
    rename: Option<String>,
    keep_bytes: Option<String>,
    bytes_of: Option<String>,
    lines: Option<LineCount>,
    zip: Option<Vec<(String, ZipEntry)>>,
    truncate: Option<u64>,
    from_repository: Option<String>,
    cached_in: Option<String>,
    all_cached_in: Option<String>,
    except_suffix: Option<String>,
    thumbnail: Option<Thumbnail>,
    pixel: Option<PixelAt>,
    alpha_range: Option<[u8; 2]>,
    keep_mtime: Option<String>,
    mtime_of: Option<String>,
    executable: Option<bool>,
    mtime: Option<Mtime>,
    hex: Option<String>,
    base64: Option<String>,
    append: Option<String>,
    raw_photo: Option<bool>,
    tar_gz: Option<TarGz>,
    zip_members: Option<Vec<(String, String)>>,
    tar_members: Option<Vec<(String, String)>>,
    is_zip: Option<bool>,
    default_app: Option<DefaultApp>,
    opened: Option<Vec<String>>,
    toml: Option<Map<String, Value>>,
    toml_like: Option<String>,
    json: Option<Map<String, Value>>,
    json_set: Option<Map<String, Value>>,
    launches: Option<Launches>,
    replace_lines: Option<ReplaceLines>,
    pdf_image: Option<Image>,
    magick: Option<Vec<String>>,
    #[serde(default)]
    percent_path: bool,
    random: Option<u64>,
    size: Option<u64>,
    glob_none: Option<String>,
    lines_equal: Option<Vec<String>>,
    toml_prefix: Option<Map<String, Value>>,
    #[serde(default)]
    toml_keys: Vec<String>,
    png: Option<PngCheck>,
    zip_member: Option<(String, String)>,
    last_line: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PngCheck {
    pub(crate) size: [u32; 2],
    pub(crate) at: [u32; 2],
    pub(crate) min: [u8; 3],
    pub(crate) max: [u8; 3],
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ZipEntry {
    Text(String),
    Image {
        image: Image,
    },
    Member {
        text: String,
        date_time: [u16; 6],
        mode: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum LineCount {
    Exactly(usize),
    AtLeast { at_least: usize },
}

impl LineCount {
    fn matches(self, lines: usize) -> bool {
        match self {
            Self::Exactly(count) => lines == count,
            Self::AtLeast { at_least } => lines >= at_least,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Has {
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    lacks: Vec<String>,
}

pub(crate) enum FsKind<'a> {
    Exists(bool),
    Symlink(&'a str),
    Program(&'a str),
    Has(&'a Has),
    NamesWithout(&'a str),
    CanTrash(bool),
    RemoveVolume,
    Text(&'a str),
    Names(&'a [String]),
    Is(&'a str),
    Empty(bool),
    Unchanged(&'a str),
    CopyOf(&'a str),
    SizeOf(&'a str),
    Write(&'a str),
    Remember(&'a str),
    Mkdir { parents: bool },
    Chmod(u32),
    Populate(&'a Value),
    Image(&'a Image),
    Ffmpeg(&'a [String]),
    Volume(&'a str, &'a str, &'a BTreeMap<String, String>),
    RememberNames(&'a str),
    NamesFrom(&'a str),
    Remove,
    Rename(&'a str),
    Lines(LineCount),
    KeepBytes(&'a str),
    BytesOf(&'a str),
    Zip(&'a [(String, ZipEntry)]),
    Truncate(u64),
    FromRepository(&'a str),
    AllCached(&'a str, Option<&'a str>),
    Thumbnail(&'a Thumbnail),
    Pixel(&'a PixelAt),
    AlphaRange([u8; 2]),
    KeepMtime(&'a str),
    MtimeOf(&'a str),
    Executable(bool),
    Mtime(Mtime),
    Hex(&'a str),
    Base64(&'a str),
    Append(&'a str),
    RawPhoto,
    TarGz(&'a TarGz),
    ZipMembers(&'a [(String, String)]),
    TarMembers(&'a [(String, String)]),
    IsZip(bool),
    DefaultApp(&'a DefaultApp),
    Opened(&'a [String]),
    Toml(&'a Map<String, Value>),
    TomlLike(&'a str),
    Json(&'a Map<String, Value>),
    JsonSet(&'a Map<String, Value>),
    Launches(&'a Launches),
    ReplaceLines(&'a ReplaceLines),
    PdfImage(&'a Image),
    Magick(&'a [String]),
    Random(u64),
    Size(u64),
    GlobNone(&'a str),
    LinesEqual(&'a [String]),
    TomlPrefix(&'a Map<String, Value>),
    Png(&'a PngCheck),
    ZipMember(&'a str, &'a str),
    LastLine(&'a str),
}

#[derive(Default)]
pub(crate) struct FsState {
    pub(crate) listings: BTreeMap<String, String>,
    pub(crate) bytes: BTreeMap<String, Vec<u8>>,
    pub(crate) names: BTreeMap<String, Vec<String>>,
    pub(crate) volumes: Vec<tempfile::TempDir>,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) fatal: Option<String>,
    pub(crate) mtimes: BTreeMap<String, u128>,
}

/// Resolves links in the parent but not the last component, so neither `..` nor a
/// linked directory reaches outside.
pub(crate) fn contained(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let outside = || format!("{} lies outside the run's directories", path.display());
    let name = path
        .file_name()
        .filter(|name| *name != "." && *name != "..")
        .ok_or_else(outside)?;
    let parent = path
        .parent()
        .ok_or_else(outside)?
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let resolved = parent.join(name);
    roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| resolved.starts_with(&root) && resolved != root)
        .then_some(())
        .ok_or_else(outside)
}

impl FsCheck {
    pub(crate) fn kind(&self) -> Result<FsKind<'_>, String> {
        let mut kinds = Vec::new();
        if let Some(exists) = self.exists {
            kinds.push(FsKind::Exists(exists));
        }
        if let Some(text) = &self.text {
            kinds.push(FsKind::Text(text));
        }
        if let Some(names) = &self.names {
            kinds.push(FsKind::Names(names));
        }
        if let Some(kind) = &self.is {
            if !matches!(kind.as_str(), "file" | "dir" | "symlink") {
                return Err(format!("is takes file, dir, or symlink, not {kind:?}"));
            }
            kinds.push(FsKind::Is(kind));
        }
        if let Some(empty) = self.empty {
            kinds.push(FsKind::Empty(empty));
        }
        if let Some(name) = &self.unchanged {
            kinds.push(FsKind::Unchanged(name));
        }
        if let Some(other) = &self.copy_of {
            kinds.push(FsKind::CopyOf(other));
        }
        if let Some(other) = &self.size_of {
            kinds.push(FsKind::SizeOf(other));
        }
        if let Some(mode) = &self.chmod {
            let mode = u32::from_str_radix(mode, 8)
                .ok()
                .filter(|mode| *mode <= 0o7777)
                .ok_or_else(|| {
                    format!("chmod takes an octal mode such as \"755\", not {mode:?}")
                })?;
            kinds.push(FsKind::Chmod(mode));
        }
        if let Some(name) = &self.remember_names {
            kinds.push(FsKind::RememberNames(name));
        }
        if let Some(name) = &self.names_from {
            kinds.push(FsKind::NamesFrom(name));
        } else if !self.minus.is_empty() || !self.plus.is_empty() {
            return Err("minus and plus belong to names_from".to_owned());
        }
        if let Some(directory) = &self.volume {
            if !Path::new(directory).is_absolute() {
                return Err(format!(
                    "volume makes its directory in an absolute path, not {directory:?}"
                ));
            }
            if let Some(name) = self.volume_files.keys().find(|name| {
                let name = Path::new(name);
                name.is_absolute() || name.components().count() != 1
            }) {
                return Err(format!(
                    "with names files in the volume itself, not {name:?}"
                ));
            }
            kinds.push(FsKind::Volume(
                directory,
                self.prefix.as_deref().unwrap_or("strata-drop-volume-"),
                &self.volume_files,
            ));
        } else if self.prefix.is_some() || !self.volume_files.is_empty() {
            return Err("prefix and with belong to volume".to_owned());
        }
        if let Some(target) = &self.symlink {
            kinds.push(FsKind::Symlink(target));
        }
        if let Some(program) = &self.program {
            kinds.push(FsKind::Program(program));
        }
        if let Some(has) = &self.has {
            if has.contains.is_empty() && has.lacks.is_empty() {
                return Err("has needs contains or lacks".to_owned());
            }
            kinds.push(FsKind::Has(has));
        }
        if let Some(part) = &self.names_without {
            kinds.push(FsKind::NamesWithout(part));
        }
        if let Some(can_trash) = self.can_trash {
            kinds.push(FsKind::CanTrash(can_trash));
        }
        match self.remove {
            Some(true) => kinds.push(FsKind::Remove),
            Some(false) => return Err("remove is true or left out".to_owned()),
            None => {}
        }
        if let Some(name) = &self.rename {
            kinds.push(FsKind::Rename(name));
        }
        if let Some(count) = self.lines {
            kinds.push(FsKind::Lines(count));
        }
        if let Some(name) = &self.keep_bytes {
            kinds.push(FsKind::KeepBytes(name));
        }
        if let Some(name) = &self.bytes_of {
            kinds.push(FsKind::BytesOf(name));
        }
        match self.remove_volume {
            Some(true) => kinds.push(FsKind::RemoveVolume),
            Some(false) => return Err("remove_volume is true or left out".to_owned()),
            None => {}
        }
        if let Some(text) = &self.write {
            kinds.push(FsKind::Write(text));
        }
        if let Some(name) = &self.remember {
            kinds.push(FsKind::Remember(name));
        }
        match (self.mkdir, self.parents) {
            (Some(true), parents) => kinds.push(FsKind::Mkdir {
                parents: parents == Some(true),
            }),
            (Some(false), _) => return Err("mkdir is true or left out".to_owned()),
            (None, Some(_)) => return Err("parents belongs to mkdir".to_owned()),
            (None, None) => {}
        }
        if let Some(tree) = &self.populate {
            expand_directory(tree)?;
            kinds.push(FsKind::Populate(tree));
        }
        if let Some(image) = &self.image {
            kinds.push(FsKind::Image(image));
        }
        if let Some(arguments) = &self.ffmpeg {
            kinds.push(FsKind::Ffmpeg(arguments));
        }
        if let Some(entries) = &self.zip {
            kinds.push(FsKind::Zip(entries));
        }
        if let Some(size) = self.truncate {
            kinds.push(FsKind::Truncate(size));
        }
        if let Some(name) = &self.from_repository {
            if Path::new(name).is_absolute() || name.split('/').any(|part| part == "..") {
                return Err(format!(
                    "from_repository names a file inside the repository, not {name:?}"
                ));
            }
            kinds.push(FsKind::FromRepository(name));
        }
        if let Some(cache) = &self.all_cached_in {
            kinds.push(FsKind::AllCached(cache, self.except_suffix.as_deref()));
        } else if self.except_suffix.is_some() {
            return Err("except_suffix belongs to all_cached_in".to_owned());
        }
        if let Some(thumbnail) = &self.thumbnail {
            kinds.push(FsKind::Thumbnail(thumbnail));
        }
        if let Some(pixel) = &self.pixel {
            kinds.push(FsKind::Pixel(pixel));
        }
        if let Some(range) = self.alpha_range {
            kinds.push(FsKind::AlphaRange(range));
        }
        if let Some(name) = &self.keep_mtime {
            kinds.push(FsKind::KeepMtime(name));
        }
        if let Some(name) = &self.mtime_of {
            kinds.push(FsKind::MtimeOf(name));
        }
        if let Some(executable) = self.executable {
            kinds.push(FsKind::Executable(executable));
        }
        if let Some(mtime) = self.mtime {
            kinds.push(FsKind::Mtime(mtime));
        }
        if let Some(text) = &self.hex {
            hex::decode(text).map_err(|error| format!("hex: {error}"))?;
            kinds.push(FsKind::Hex(text));
        }
        if let Some(text) = &self.base64 {
            kinds.push(FsKind::Base64(text));
        }
        if let Some(text) = &self.append {
            kinds.push(FsKind::Append(text));
        }
        match self.raw_photo {
            Some(true) => kinds.push(FsKind::RawPhoto),
            Some(false) => return Err("raw_photo is true or left out".to_owned()),
            None => {}
        }
        if let Some(archive) = &self.tar_gz {
            u32::from_str_radix(&archive.mode, 8)
                .map_err(|_| format!("tar_gz mode is octal, not {:?}", archive.mode))?;
            kinds.push(FsKind::TarGz(archive));
        }
        if let Some(members) = &self.zip_members {
            kinds.push(FsKind::ZipMembers(members));
        }
        if let Some(members) = &self.tar_members {
            kinds.push(FsKind::TarMembers(members));
        }
        if let Some(is_zip) = self.is_zip {
            kinds.push(FsKind::IsZip(is_zip));
        }
        if let Some(default) = &self.default_app {
            kinds.push(FsKind::DefaultApp(default));
        }
        if let Some(paths) = &self.opened {
            kinds.push(FsKind::Opened(paths));
        }
        if let Some(values) = &self.toml {
            kinds.push(FsKind::Toml(values));
        }
        if let Some(name) = &self.toml_like {
            kinds.push(FsKind::TomlLike(name));
        }
        if let Some(values) = &self.json {
            kinds.push(FsKind::Json(values));
        }
        if let Some(values) = &self.json_set {
            kinds.push(FsKind::JsonSet(values));
        }
        if let Some(launches) = &self.launches {
            let full = launches.count.is_some() && launches.within.is_some();
            let first = launches.first_dir.is_some()
                && launches.count.is_none()
                && launches.within.is_none();
            if full == first {
                return Err("launches takes count with within, or first_dir".to_owned());
            }
            kinds.push(FsKind::Launches(launches));
        }
        if let Some(replace) = &self.replace_lines {
            kinds.push(FsKind::ReplaceLines(replace));
        }
        if let Some(image) = &self.pdf_image {
            kinds.push(FsKind::PdfImage(image));
        }
        if let Some(arguments) = &self.magick {
            kinds.push(FsKind::Magick(arguments));
        }
        if let Some(size) = self.random {
            kinds.push(FsKind::Random(size));
        }
        if let Some(size) = self.size {
            kinds.push(FsKind::Size(size));
        }
        if let Some(pattern) = &self.glob_none {
            if pattern.contains('/') {
                return Err("glob_none matches names in the directory, not paths".to_owned());
            }
            kinds.push(FsKind::GlobNone(pattern));
        }
        if let Some(lines) = &self.lines_equal {
            kinds.push(FsKind::LinesEqual(lines));
        }
        if let Some(values) = &self.toml_prefix {
            if values.values().any(|value| !value.is_array()) {
                return Err("toml_prefix gives each key a list".to_owned());
            }
            kinds.push(FsKind::TomlPrefix(values));
        }
        if let Some(check) = &self.png {
            kinds.push(FsKind::Png(check));
        }
        if let Some((member, file)) = &self.zip_member {
            kinds.push(FsKind::ZipMember(member, file));
        }
        if let Some(line) = &self.last_line {
            kinds.push(FsKind::LastLine(line));
        }
        if !self.toml_keys.is_empty() && self.toml_like.is_none() {
            return Err("toml_keys belongs to toml_like".to_owned());
        }
        if self.cached_in.is_some()
            && !matches!(
                kinds.as_slice(),
                [FsKind::Exists(_)
                    | FsKind::Is(_)
                    | FsKind::Pixel(_)
                    | FsKind::AlphaRange(_)
                    | FsKind::KeepMtime(_)
                    | FsKind::MtimeOf(_)]
            )
        {
            return Err(
                "cached_in reads a thumbnail for exists, is, pixel, alpha_range, keep_mtime, or \
                 mtime_of"
                    .to_owned(),
            );
        }
        match kinds.len() {
            1 => Ok(kinds.remove(0)),
            _ => Err(
                "fs needs a path and exactly one of exists, text, names, is, empty, \
                 unchanged, copy_of, size_of, names_from, has, names_without, can_trash, \
                 lines, bytes_of, write, remember, remember_names, keep_bytes, mkdir, chmod, \
                 populate, image, ffmpeg, zip, truncate, from_repository, volume, \
                 remove_volume, symlink, program, remove, rename, all_cached_in, thumbnail, \
                 pixel, alpha_range, keep_mtime, mtime_of, executable, mtime, hex, base64, append, \
                 raw_photo, tar_gz, zip_members, tar_members, is_zip, default_app, opened, \
                 toml, toml_like, json, json_set, launches, replace_lines, pdf_image, magick, \
                 random, size, glob_none, lines_equal, toml_prefix, png, zip_member, or \
                 last_line"
                    .to_owned(),
            ),
        }
    }

    pub(crate) fn acts(&self) -> bool {
        matches!(
            self.kind(),
            Ok(FsKind::Write(_)
                | FsKind::Remember(_)
                | FsKind::Mkdir { .. }
                | FsKind::Chmod(_)
                | FsKind::Populate(_)
                | FsKind::Image(_)
                | FsKind::Ffmpeg(_)
                | FsKind::Volume(..)
                | FsKind::Symlink(_)
                | FsKind::Program(_)
                | FsKind::RemoveVolume
                | FsKind::RememberNames(_)
                | FsKind::Remove
                | FsKind::Rename(_)
                | FsKind::KeepBytes(_)
                | FsKind::Zip(_)
                | FsKind::Truncate(_)
                | FsKind::FromRepository(_)
                | FsKind::Thumbnail(_)
                | FsKind::KeepMtime(_)
                | FsKind::Hex(_)
                | FsKind::Base64(_)
                | FsKind::Append(_)
                | FsKind::RawPhoto
                | FsKind::TarGz(_)
                | FsKind::ReplaceLines(_)
                | FsKind::PdfImage(_)
                | FsKind::Magick(_)
                | FsKind::Random(_))
        )
    }
}
