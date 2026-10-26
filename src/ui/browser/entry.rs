// SPDX-License-Identifier: MIT

use crate::adapters::directory_summary::{DirectorySummary, summarize_directory};
use crate::adapters::gio_file_for_location;
use crate::assets::icons;
use crate::model::{EntryKind, FileEntry, MetadataValue};
use crate::services::{
    PathMatcher, PathQuery, PreviewContent, content_family, filter_name_matches, fold_for_search,
    has_plain_text_extension, is_extensionless_dotfile,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

thread_local! {
    static FILTER_TERMS: RefCell<Option<(String, PathMatcher)>> = const { RefCell::new(None) };
}

pub(in crate::ui) fn format_file_size(bytes: u64) -> String {
    crate::i18n::file_size(bytes)
}

pub(in crate::ui) fn metadata_needs_fill(entry: &FileEntry) -> bool {
    entry.modified_unix_seconds == crate::model::MetadataValue::Unknown
        || (!entry.is_directory() && entry.size == crate::model::MetadataValue::Unknown)
}

pub(super) fn entry_responds_to_preview_click(entry: &FileEntry, previews_enabled: bool) -> bool {
    previews_enabled
        && !entry.is_directory()
        && crate::ui::preview::entry_supports_quick_preview(entry)
}

pub(super) fn entry_supports_printing(entry: &FileEntry) -> bool {
    if !matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink) {
        return false;
    }

    let native_path = entry.location.native_path();
    let (content_type, uncertain) =
        crate::services::guess_content_type(Path::new(&entry.native_name));
    // An uncertain name guess defers to the loader, which resolves the file's content type.
    matches!(content_family(&content_type), PreviewContent::Text { .. })
        || (uncertain && matches!(content_family(&content_type), PreviewContent::Unsupported))
        || (native_path.is_some()
            && matches!(
                content_family(&content_type),
                PreviewContent::Image | PreviewContent::Pdf { .. }
            ))
        || gio::content_type_is_a(&content_type, "text/plain")
        || has_plain_text_extension(&entry.native_name)
        || is_extensionless_dotfile(&entry.native_name)
}

pub(in crate::ui) fn entry_model_value(entry: &FileEntry) -> String {
    let kind = if entry.is_broken_symbolic_link() {
        'x'
    } else if entry.is_directory() {
        'd'
    } else if entry.is_symbolic_link() {
        's'
    } else if entry.kind == EntryKind::Other {
        'o'
    } else {
        'f'
    };
    let hidden = if entry.is_hidden { 'h' } else { 'v' };
    let entry_type = crate::services::entry_type(entry);
    let group = entry_type.description();
    let name = entry.display_name.as_str();
    let mut value = String::with_capacity(group.len() + name.len() + 3);
    value.push(kind);
    value.push(hidden);
    value.push_str(group);
    value.push('\t');
    value.push_str(name);
    value
}

pub(super) fn model_display_name(value: &str) -> &str {
    value.split_once('\t').map_or(value, |(_, name)| name)
}

pub(super) fn model_is_directory(value: &str) -> bool {
    value.starts_with("d")
}

pub(in crate::ui) fn model_is_hidden(value: &str) -> bool {
    value.as_bytes().get(1) == Some(&b'h')
}

pub(in crate::ui) const FOLDER_TYPE_GROUP: &str = crate::services::FOLDER_TYPE_NAME;
pub(in crate::ui) const OTHER_TYPE_GROUP: &str = crate::services::OTHER_TYPE_NAME;

/// Empty for a value without a group, such as an empty model value.
pub(in crate::ui) fn model_type_group(value: &str) -> &str {
    value
        .get(2..)
        .and_then(|rest| rest.split_once('\t'))
        .map_or("", |(group, _)| group)
}

pub(in crate::ui) fn entry_filter(
    show_hidden: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
) -> gtk::CustomFilter {
    gtk::CustomFilter::new(move |item| {
        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
            return false;
        };
        let value = item.string();
        entry_matches(&value, show_hidden.get(), &filter_query.borrow())
    })
}

pub(in crate::ui) fn entry_icon(entry: &FileEntry) -> &'static str {
    if entry.is_broken_symbolic_link() {
        return crate::assets::icons::X;
    }
    if entry.is_directory() {
        return crate::assets::icons::FOLDER;
    }
    icon_for_name(&entry.display_name)
}

/// `query` must already be folded through `fold_for_search` by the caller.
pub(super) fn entry_matches(value: &str, show_hidden: bool, query: &str) -> bool {
    (show_hidden || !model_is_hidden(value))
        && (query.trim().is_empty() || {
            let name = fold_for_search(model_display_name(value));
            if crate::ui::tenxer_mode::chrome_suppressed() {
                with_filter_terms(query, |terms| terms.score(&name, 0).is_some())
            } else {
                filter_name_matches(&name, query)
            }
        })
}

pub(super) fn with_filter_terms<R>(query: &str, apply: impl FnOnce(&mut PathMatcher) -> R) -> R {
    FILTER_TERMS.with_borrow_mut(|cached| {
        if cached.as_ref().is_none_or(|(cached, _)| cached != query) {
            *cached = Some((query.to_owned(), PathMatcher::new(&PathQuery::parse(query))));
        }
        let (_, matcher) = cached.as_mut().expect("filter terms cached above");
        apply(matcher)
    })
}

static FILENAME_ICONS: &[(&str, &str)] = &[
    ("npm-shrinkwrap.json", catppuccin::NPM_LOCK),
    ("rakefile", catppuccin::RUBY),
    ("vagrantfile", catppuccin::RUBY),
    ("podfile", catppuccin::RUBY),
    ("dune", catppuccin::OCAML),
    ("dune-project", catppuccin::OCAML),
    ("build.gradle", catppuccin::GRADLE),
    (".inputrc", catppuccin::CONFIG),
    (".pypirc", catppuccin::PYTHON_CONFIG),
    (".xcompose", catppuccin::CONFIG),
    (".vimrc", catppuccin::VIM),
    (".gvimrc", catppuccin::VIM),
    (".viminfo", catppuccin::VIM),
    (".bashrc", catppuccin::BASH),
    (".bash_profile", catppuccin::BASH),
    (".bash_login", catppuccin::BASH),
    (".bash_logout", catppuccin::BASH),
    (".zshrc", catppuccin::BASH),
    (".zprofile", catppuccin::BASH),
    (".zlogin", catppuccin::BASH),
    (".zlogout", catppuccin::BASH),
    (".profile", catppuccin::BASH),
    (".login", catppuccin::BASH),
    (".logout", catppuccin::BASH),
    (".kshrc", catppuccin::BASH),
    (".cshrc", catppuccin::BASH),
    (".tcshrc", catppuccin::BASH),
    ("id_rsa", icons::KEY_ROUND),
    ("id_ed25519", icons::KEY_ROUND),
    ("authorized_keys", icons::KEY_ROUND),
    ("known_hosts", icons::KEY_ROUND),
    (".rprofile", catppuccin::R),
    (".renviron", catppuccin::R),
];

static FILENAME_AFFIX_PATTERNS: &[(&str, &str, &str)] = &[
    ("Dockerfile.", "", catppuccin::DOCKER),
    (".env.", "", catppuccin::ENV),
    ("tsconfig.", ".json", catppuccin::TYPESCRIPT_CONFIG),
    ("", "_history", icons::FILE_TERMINAL),
    ("", ".lock", icons::COG),
];

fn exact_filename_icon(lowered: &str) -> Option<&'static str> {
    catppuccin::icon_for_lowercased_name(lowered).or_else(|| {
        FILENAME_ICONS
            .iter()
            .find(|(name, _)| *name == lowered)
            .map(|(_, icon)| *icon)
    })
}

fn affix_pattern_icon(name: &str) -> Option<&'static str> {
    FILENAME_AFFIX_PATTERNS
        .iter()
        .find(|(prefix, suffix, _)| name.starts_with(prefix) && name.ends_with(suffix))
        .map(|(_, _, icon)| *icon)
}

pub(in crate::ui) fn icon_for_name(name: &str) -> &'static str {
    let lowered = name.to_ascii_lowercase();
    if let Some(icon) = exact_filename_icon(&lowered) {
        return icon;
    }
    if let Some(icon) = affix_pattern_icon(name) {
        return icon;
    }
    if let Some(icon) = catppuccin::icon_for_lowercased_extension(&lowered) {
        return icon;
    }
    let extension = lowered.rsplit_once('.').map(|(_, extension)| extension);
    match extension {
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "heic" | "heif"
            | "jxl" | "tif" | "tiff" | "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf"
            | "kdc" | "mef" | "mos" | "mrw" | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2"
            | "rwl" | "sr2" | "srf" | "srw" | "x3f",
        ) => crate::assets::icons::PICTURES,
        Some(
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" | "3gp" | "3g2"
            | "flv" | "wmv" | "asf" | "m2ts" | "vob",
        ) => crate::assets::icons::VIDEOS,
        Some("mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" | "aiff") => {
            crate::assets::icons::FILE_AUDIO
        }
        Some("html" | "htm" | "xhtml" | "mjml") => catppuccin::HTML,
        Some("css" | "scss" | "less" | "sass") => catppuccin::CSS,
        Some(
            "xml" | "xsd" | "dtd" | "rng" | "xsl" | "xslt" | "wsdl" | "xaml" | "resx" | "xcscheme"
            | "storyboard" | "xib" | "plist" | "entitlements" | "rss" | "atom" | "mml",
        ) => catppuccin::XML,
        Some(
            "twig" | "ejs" | "hbs" | "handlebars" | "mustache" | "pug" | "jade" | "haml" | "slim"
            | "smarty" | "tpl",
        ) => catppuccin::HTML,
        Some(
            "zip" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "rar" | "lz" | "lzma"
            | "lzo" | "rz" | "sz" | "cab" | "arj" | "cpio",
        ) => crate::assets::icons::FILE_ARCHIVE,
        Some(
            "deb" | "rpm" | "pkg" | "appimage" | "msi" | "exe" | "apk" | "jar" | "war" | "ear"
            | "whl" | "egg" | "gem" | "nupkg" | "crate" | "vsix" | "xpi",
        ) => crate::assets::icons::BOX,
        Some("o" | "obj" | "so" | "dll" | "dylib") => crate::assets::icons::BOX,
        Some("pem" | "crt" | "cer" | "key" | "der" | "csr" | "pub" | "p12" | "pfx" | "jks") => {
            crate::assets::icons::KEY_ROUND
        }
        Some(
            "yaml" | "yml" | "toml" | "ini" | "conf" | "cfg" | "config" | "env" | "lock" | "cmake"
            | "mk" | "mak" | "ninja" | "m4" | "am" | "ac" | "gyp" | "gypi" | "bazel" | "bzl"
            | "pbxproj" | "xcconfig" | "pro" | "pri",
        ) => catppuccin::CONFIG,
        Some(
            "json" | "jsonc" | "json5" | "jsonl" | "ndjson" | "proto" | "thrift" | "avsc" | "avdl",
        ) => catppuccin::JSON,
        Some(
            "db" | "sqlite" | "sqlite3" | "db3" | "sql" | "psql" | "pgsql" | "tsql" | "plsql"
            | "pls" | "pks" | "pkb" | "mdb" | "accdb" | "prisma" | "cypher" | "cql" | "parquet"
            | "orc" | "arrow" | "feather" | "avro",
        ) => catppuccin::DATABASE,
        Some("iso" | "img" | "dmg" | "vhd" | "vhdx" | "vdi" | "qcow") => crate::assets::icons::DISC,
        Some("csv" | "tsv" | "xls" | "xlsx" | "ods") => icons::FILE_SPREADSHEET,
        Some("rs") => catppuccin::RUST,
        Some("py" | "pyw" | "pyi" | "pyx" | "pxd" | "pxi" | "pyc" | "pyo") => catppuccin::PYTHON,
        Some("js" | "jsx" | "cjs" | "mjs") => catppuccin::JAVASCRIPT,
        Some("ts" | "tsx" | "mts" | "cts") => catppuccin::TYPESCRIPT,
        Some("go") => catppuccin::GO,
        Some("java" | "jsp" | "jspx" | "class") => catppuccin::JAVA,
        Some("kt" | "kts") => catppuccin::KOTLIN,
        Some("swift" | "swiftinterface") => catppuccin::SWIFT,
        Some("dart") => catppuccin::DART,
        Some("scala" | "sbt") => catppuccin::SCALA,
        Some("hs" | "lhs" | "hsc" | "cabal") => catppuccin::HASKELL,
        Some("lua" | "fnl" | "tl") => catppuccin::LUA,
        Some(
            "cs" | "csproj" | "sln" | "vb" | "razor" | "cshtml" | "props" | "targets" | "nuspec",
        ) => catppuccin::CSHARP,
        Some("fs" | "fsx" | "fsi") => catppuccin::FSHARP,
        Some("r" | "rmd" | "rproj") => catppuccin::R,
        Some("jl") => catppuccin::JULIA,
        Some("ex" | "exs" | "heex" | "eex" | "leex") => catppuccin::ELIXIR,
        Some("erl" | "hrl" | "beam") => catppuccin::ERLANG,
        Some("zig" | "zon") => catppuccin::ZIG,
        Some("vue") => catppuccin::VUE,
        Some("svelte") => catppuccin::SVELTE,
        Some("astro") => catppuccin::ASTRO,
        Some("elm") => catppuccin::ELM,
        Some("coffee") => catppuccin::COFFEESCRIPT,
        Some("c" | "h" | "i") => catppuccin::C,
        Some(
            "cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx" | "ii" | "inl" | "ipp" | "tcc" | "tpp",
        ) => catppuccin::CPP,
        Some("rb" | "erb" | "rake" | "gemspec" | "podspec" | "builder" | "ru") => catppuccin::RUBY,
        Some("php" | "phtml" | "ctp") => catppuccin::PHP,
        Some("pl" | "pm" | "pod") => catppuccin::PERL,
        Some("ml" | "mli" | "mll" | "mly" | "opam" | "dune") => catppuccin::OCAML,
        Some("re" | "res" | "resi") => catppuccin::OCAML,
        Some("clj" | "cljs" | "cljc" | "edn" | "bb") => catppuccin::CLOJURE,
        Some("groovy" | "gvy" | "gradle") => catppuccin::GROOVY,
        Some("tf" | "tfvars" | "hcl" | "tofu" | "nomad") => catppuccin::TERRAFORM,
        Some("graphql" | "gql" | "gqls") => catppuccin::GRAPHQL,
        Some("nim" | "nims" | "nimble" | "nimcfg") => catppuccin::NIM,
        Some("cr" | "ecr") => catppuccin::CRYSTAL,
        Some("rkt") => catppuccin::RACKET,
        Some("qml" | "qrc" | "ui") => crate::assets::icons::FILE_CODE,
        Some("nix") => catppuccin::NIX,
        Some("ipynb") => catppuccin::JUPYTER,
        Some("sol") => catppuccin::SOLIDITY,
        Some("asm" | "s" | "nasm") => catppuccin::ASSEMBLY,
        Some("sv" | "svh" | "vhdl") => catppuccin::VERILOG,
        Some(
            "m" | "v" | "mm" | "tcl" | "tk" | "exp" | "awk" | "sed" | "el" | "elc" | "lisp" | "lsp"
            | "asd" | "scm" | "ss" | "pas" | "pp" | "lpr" | "dpr" | "adb" | "ads" | "ada" | "f"
            | "for" | "f90" | "f95" | "cob" | "cbl" | "bas" | "purs" | "idr" | "lidr" | "agda"
            | "lean" | "rego",
        ) => crate::assets::icons::FILE_CODE,
        Some("sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "nu" | "elv") => catppuccin::BASH,
        Some("ps1" | "psm1" | "psd1") => catppuccin::POWERSHELL,
        Some("bat" | "cmd") => catppuccin::BATCH,
        Some("ppt" | "pptx" | "pps" | "ppsx" | "odp") => icons::PRESENTATION,
        Some("ttf" | "otf" | "woff" | "woff2" | "eot" | "ttc" | "otc") => icons::FILE_TYPE,
        _ => icons::DOCUMENTS,
    }
}

pub(super) fn item_count_label(count: usize) -> String {
    crate::i18n::count("items", count)
}

pub(super) fn entry_kind_summary(entries: &[FileEntry]) -> String {
    let directories = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .count();
    let files = entries.iter().filter(|entry| entry.is_file()).count();
    match (files, directories) {
        (files, 0) if files == entries.len() => crate::i18n::count("files", files),
        (0, directories) if directories == entries.len() => {
            crate::i18n::count("folders", directories)
        }
        _ => item_count_label(entries.len()),
    }
}

pub(super) async fn delete_entry_size(entry: &FileEntry) -> MetadataValue<u64> {
    if entry.kind == EntryKind::FileSymbolicLink {
        crate::adapters::query_file_entry(entry.location.clone())
            .await
            .map_or(MetadataValue::Unavailable, |link| link.size)
    } else {
        entry.size.clone()
    }
}

pub(super) async fn aggregate_directory_summary(
    entries: &[FileEntry],
    mut on_symlink_size: impl FnMut(usize, &MetadataValue<u64>),
) -> DirectorySummary {
    let mut total = DirectorySummary::default();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 && index % 256 == 0 {
            glib::timeout_future(std::time::Duration::ZERO).await;
        }
        if entry.kind == EntryKind::Directory {
            let directory = gio_file_for_location(&entry.location);
            match summarize_directory(&directory).await {
                Ok(summary) => total.include(summary),
                Err(_) => total.issues.unreadable = true,
            }
        } else {
            total.item_count = total.item_count.saturating_add(1);
            total.visible_file_count = total.visible_file_count.saturating_add(1);
            let size = delete_entry_size(entry).await;
            if entry.kind == EntryKind::FileSymbolicLink {
                on_symlink_size(index, &size);
            }
            match size {
                MetadataValue::Known(size) => {
                    total.total_size = total.total_size.saturating_add(size);
                }
                MetadataValue::Unknown | MetadataValue::Unavailable => {
                    if !entry.is_symbolic_link() || entry.kind == EntryKind::FileSymbolicLink {
                        total.issues.unreadable = true;
                    }
                }
            }
        }
    }
    total
}

pub(in crate::ui) mod catppuccin;

#[cfg(test)]
mod tests;
