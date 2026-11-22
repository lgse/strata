// SPDX-License-Identifier: MIT

//! File-type icon resolution for Rune views (`entry_icon` and `icon_for_name` in
//! `src/ui/browser/entry.rs`): Catppuccin VSCode icons for code/config files and Strata's
//! Lucide category icons for documents, media, archives, keys, executables, and folders.

use std::{collections::HashMap, sync::OnceLock};

use strata_core::model::FileEntry;

pub(crate) mod catppuccin;

#[cfg(test)]
mod tests;

pub(crate) const BOX: &str = "strata-box";
pub(crate) const COG: &str = "strata-cog";
pub(crate) const DISC: &str = "strata-disc";
pub(crate) const DOCUMENTS: &str = "strata-file-text";
pub(crate) const FILE_ARCHIVE: &str = "strata-file-archive";
pub(crate) const FILE_AUDIO: &str = "strata-audio-lines";
pub(crate) const FILE_CODE: &str = "strata-file-code";
pub(crate) const FILE_SPREADSHEET: &str = "strata-file-spreadsheet";
pub(crate) const FILE_TERMINAL: &str = "strata-file-terminal";
pub(crate) const FILE_TYPE: &str = "strata-file-type";
pub(crate) const FOLDER: &str = "strata-folder";
pub(crate) const FOLDER_LOCK: &str = "strata-folder-lock";
pub(crate) const KEY_ROUND: &str = "strata-key-round";
pub(crate) const PICTURES: &str = "strata-image";
pub(crate) const PRESENTATION: &str = "strata-presentation";
pub(crate) const VIDEOS: &str = "strata-video";
pub(crate) const X: &str = "strata-x";

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
    ("id_rsa", KEY_ROUND),
    ("id_ed25519", KEY_ROUND),
    ("authorized_keys", KEY_ROUND),
    ("known_hosts", KEY_ROUND),
    (".rprofile", catppuccin::R),
    (".renviron", catppuccin::R),
];

static FILENAME_AFFIX_PATTERNS: &[(&str, &str, &str)] = &[
    ("Dockerfile.", "", catppuccin::DOCKER),
    (".env.", "", catppuccin::ENV),
    ("tsconfig.", ".json", catppuccin::TYPESCRIPT_CONFIG),
    ("", "_history", FILE_TERMINAL),
    ("", ".lock", COG),
];

/// Fallback extension groups checked when `catppuccin::icon_for_lowercased_extension` has no
/// upstream association, matching `icon_for_name` in `src/ui/browser/entry.rs`.
static EXTENSION_GROUPS: &[(&[&str], &str)] = &[
    (
        &[
            "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "heic", "heif", "jxl",
            "tif", "tiff", "3fr", "arw", "cr2", "cr3", "dcr", "dng", "erf", "kdc", "mef", "mos",
            "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw",
            "x3f",
        ],
        PICTURES,
    ),
    (
        &[
            "mp4", "mkv", "webm", "mov", "avi", "m4v", "mpeg", "mpg", "ogv", "3gp", "3g2", "flv",
            "wmv", "asf", "m2ts", "vob", "h264", "264", "h265", "265", "hevc",
        ],
        VIDEOS,
    ),
    (
        &[
            "mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "wma", "aiff",
        ],
        FILE_AUDIO,
    ),
    (
        &[
            "html",
            "htm",
            "xhtml",
            "mjml",
            "twig",
            "ejs",
            "hbs",
            "handlebars",
            "mustache",
            "pug",
            "jade",
            "haml",
            "slim",
            "smarty",
            "tpl",
        ],
        catppuccin::HTML,
    ),
    (&["css", "scss", "less", "sass"], catppuccin::CSS),
    (
        &[
            "xml",
            "xsd",
            "dtd",
            "rng",
            "xsl",
            "xslt",
            "wsdl",
            "xaml",
            "resx",
            "xcscheme",
            "storyboard",
            "xib",
            "plist",
            "entitlements",
            "rss",
            "atom",
            "mml",
        ],
        catppuccin::XML,
    ),
    (
        &[
            "zip", "7z", "tar", "gz", "tgz", "tbz", "tbz2", "txz", "tzst", "bz2", "xz", "zst",
            "rar", "lz", "lzma", "lzo", "rz", "sz", "cab", "arj", "cpio",
        ],
        FILE_ARCHIVE,
    ),
    (
        &[
            "deb", "rpm", "pkg", "appimage", "msi", "exe", "apk", "jar", "war", "ear", "whl",
            "egg", "gem", "nupkg", "crate", "vsix", "xpi", "o", "obj", "so", "dll", "dylib",
        ],
        BOX,
    ),
    (
        &[
            "pem", "crt", "cer", "key", "der", "csr", "pub", "p12", "pfx", "jks",
        ],
        KEY_ROUND,
    ),
    (
        &[
            "yaml", "yml", "toml", "ini", "conf", "cfg", "config", "env", "lock", "cmake", "mk",
            "mak", "ninja", "m4", "am", "ac", "gyp", "gypi", "bazel", "bzl", "pbxproj", "xcconfig",
            "pro", "pri",
        ],
        catppuccin::CONFIG,
    ),
    (
        &[
            "json", "jsonc", "json5", "jsonl", "ndjson", "proto", "thrift", "avsc", "avdl",
        ],
        catppuccin::JSON,
    ),
    (
        &[
            "db", "sqlite", "sqlite3", "db3", "sql", "psql", "pgsql", "tsql", "plsql", "pls",
            "pks", "pkb", "mdb", "accdb", "prisma", "cypher", "cql", "parquet", "orc", "arrow",
            "feather", "avro",
        ],
        catppuccin::DATABASE,
    ),
    (&["iso", "img", "dmg", "vhd", "vhdx", "vdi", "qcow"], DISC),
    (&["csv", "tsv", "xls", "xlsx", "ods"], FILE_SPREADSHEET),
    (&["rs"], catppuccin::RUST),
    (
        &["py", "pyw", "pyi", "pyx", "pxd", "pxi", "pyc", "pyo"],
        catppuccin::PYTHON,
    ),
    (&["js", "jsx", "cjs", "mjs"], catppuccin::JAVASCRIPT),
    (&["ts", "tsx", "mts", "cts"], catppuccin::TYPESCRIPT),
    (&["go"], catppuccin::GO),
    (&["java", "jsp", "jspx", "class"], catppuccin::JAVA),
    (&["kt", "kts"], catppuccin::KOTLIN),
    (&["swift", "swiftinterface"], catppuccin::SWIFT),
    (&["dart"], catppuccin::DART),
    (&["scala", "sbt"], catppuccin::SCALA),
    (&["hs", "lhs", "hsc", "cabal"], catppuccin::HASKELL),
    (&["lua", "fnl", "tl"], catppuccin::LUA),
    (
        &[
            "cs", "csproj", "sln", "vb", "razor", "cshtml", "props", "targets", "nuspec",
        ],
        catppuccin::CSHARP,
    ),
    (&["fs", "fsx", "fsi"], catppuccin::FSHARP),
    (&["r", "rmd", "rproj"], catppuccin::R),
    (&["jl"], catppuccin::JULIA),
    (&["ex", "exs", "heex", "eex", "leex"], catppuccin::ELIXIR),
    (&["erl", "hrl", "beam"], catppuccin::ERLANG),
    (&["zig", "zon"], catppuccin::ZIG),
    (&["vue"], catppuccin::VUE),
    (&["svelte"], catppuccin::SVELTE),
    (&["astro"], catppuccin::ASTRO),
    (&["elm"], catppuccin::ELM),
    (&["coffee"], catppuccin::COFFEESCRIPT),
    (&["c", "h", "i"], catppuccin::C),
    (
        &[
            "cpp", "hpp", "cc", "cxx", "hh", "hxx", "ii", "inl", "ipp", "tcc", "tpp",
        ],
        catppuccin::CPP,
    ),
    (
        &["rb", "erb", "rake", "gemspec", "podspec", "builder", "ru"],
        catppuccin::RUBY,
    ),
    (&["php", "phtml", "ctp"], catppuccin::PHP),
    (&["pl", "pm", "pod"], catppuccin::PERL),
    (
        &[
            "ml", "mli", "mll", "mly", "opam", "dune", "re", "res", "resi",
        ],
        catppuccin::OCAML,
    ),
    (&["clj", "cljs", "cljc", "edn", "bb"], catppuccin::CLOJURE),
    (&["groovy", "gvy", "gradle"], catppuccin::GROOVY),
    (
        &["tf", "tfvars", "hcl", "tofu", "nomad"],
        catppuccin::TERRAFORM,
    ),
    (&["graphql", "gql", "gqls"], catppuccin::GRAPHQL),
    (&["nim", "nims", "nimble", "nimcfg"], catppuccin::NIM),
    (&["cr", "ecr"], catppuccin::CRYSTAL),
    (&["rkt"], catppuccin::RACKET),
    (&["nix"], catppuccin::NIX),
    (&["ipynb"], catppuccin::JUPYTER),
    (&["sol"], catppuccin::SOLIDITY),
    (&["asm", "s", "nasm"], catppuccin::ASSEMBLY),
    (&["sv", "svh", "vhdl"], catppuccin::VERILOG),
    (
        &[
            "qml", "qrc", "ui", "m", "v", "mm", "tcl", "tk", "exp", "awk", "sed", "el", "elc",
            "lisp", "lsp", "asd", "scm", "ss", "pas", "pp", "lpr", "dpr", "adb", "ads", "ada", "f",
            "for", "f90", "f95", "cob", "cbl", "bas", "purs", "idr", "lidr", "agda", "lean",
            "rego",
        ],
        FILE_CODE,
    ),
    (
        &["sh", "bash", "zsh", "fish", "ksh", "csh", "nu", "elv"],
        catppuccin::BASH,
    ),
    (&["ps1", "psm1", "psd1"], catppuccin::POWERSHELL),
    (&["bat", "cmd"], catppuccin::BATCH),
    (&["ppt", "pptx", "pps", "ppsx", "odp"], PRESENTATION),
    (
        &["ttf", "otf", "woff", "woff2", "eot", "ttc", "otc"],
        FILE_TYPE,
    ),
];

fn filename_icon_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| FILENAME_ICONS.iter().copied().collect())
}

fn extension_icon_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        EXTENSION_GROUPS
            .iter()
            .flat_map(|(exts, icon)| exts.iter().map(move |ext| (*ext, *icon)))
            .collect()
    })
}

fn exact_filename_icon(lowered: &str) -> Option<&'static str> {
    catppuccin::icon_for_lowercased_name(lowered)
        .or_else(|| filename_icon_map().get(lowered).copied())
}

fn affix_pattern_icon(name: &str) -> Option<&'static str> {
    let bytes = name.as_bytes();
    let first = *bytes.first()?;
    let last = *bytes.last()?;
    if !matches!(first, b'D' | b'.' | b't') && !matches!(last, b'y' | b'k') {
        return None;
    }
    FILENAME_AFFIX_PATTERNS
        .iter()
        .find(|(prefix, suffix, _)| name.starts_with(prefix) && name.ends_with(suffix))
        .map(|(_, _, icon)| *icon)
}

fn fallback_extension_icon(extension: &str) -> Option<&'static str> {
    extension_icon_map().get(extension).copied()
}

/// Catalog icon key for `entry`, matching `entry_icon` in `src/ui/browser/entry.rs`.
pub(crate) fn entry_icon(entry: &FileEntry) -> &'static str {
    if entry.is_broken_symbolic_link() {
        return X;
    }
    if entry.is_locked_folder() {
        return FOLDER_LOCK;
    }
    if entry.is_directory() {
        return FOLDER;
    }
    icon_for_name(&entry.display_name)
}

/// Catalog icon key for a file named `name`, matching `icon_for_name` in
/// `src/ui/browser/entry.rs`.
pub(crate) fn icon_for_name(name: &str) -> &'static str {
    if !name.bytes().any(|b| b.is_ascii_uppercase()) {
        return icon_for_lowercased_name(name, name);
    }
    if name.len() <= 256 {
        let mut buf = [0_u8; 256];
        for (dst, &byte) in buf.iter_mut().zip(name.as_bytes()) {
            *dst = byte.to_ascii_lowercase();
        }
        if let Ok(lowered) = std::str::from_utf8(&buf[..name.len()]) {
            return icon_for_lowercased_name(name, lowered);
        }
    }
    let lowered = name.to_ascii_lowercase();
    icon_for_lowercased_name(name, &lowered)
}

fn icon_for_lowercased_name(original: &str, lowered: &str) -> &'static str {
    if let Some(icon) = exact_filename_icon(lowered) {
        return icon;
    }
    if let Some(icon) = affix_pattern_icon(original) {
        return icon;
    }
    if let Some(icon) = catppuccin::icon_for_lowercased_extension(lowered) {
        return icon;
    }
    lowered
        .rsplit_once('.')
        .and_then(|(_, extension)| fallback_extension_icon(extension))
        .unwrap_or(DOCUMENTS)
}
