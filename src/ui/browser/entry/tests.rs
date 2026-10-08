// SPDX-License-Identifier: MIT

use super::icon_for_name;
use crate::assets::icons;

#[test]
fn file_categories_and_case_insensitive_extensions() {
    for (name, icon) in [
        ("song.mp3", icons::FILE_AUDIO),
        ("take.WAV", icons::FILE_AUDIO),
        ("photo.png", icons::PICTURES),
        ("movie.mp4", icons::VIDEOS),
        ("page.html", icons::LANG_HTML),
        ("mailing.mjml", icons::LANG_HTML),
        ("view.twig", icons::LANG_HTML),
        ("style.scss", icons::LANG_CSS),
        ("feed.xml", icons::GLOBE),
        ("schema.xsd", icons::GLOBE),
        ("backup.zip", icons::FILE_ARCHIVE),
        ("backup.tar.gz", icons::FILE_ARCHIVE),
        ("backup.tgz", icons::FILE_ARCHIVE),
        ("installer.AppImage", icons::BOX),
        ("app.apk", icons::BOX),
        ("server.key", icons::KEY_ROUND),
        ("certificate.p12", icons::KEY_ROUND),
        ("config.toml", icons::COG),
        (".env", icons::COG),
        ("data.json", icons::FILE_BRACES),
        ("data.jsonc", icons::FILE_BRACES),
        ("data.jsonl", icons::FILE_BRACES),
        ("schema.proto", icons::FILE_BRACES),
        ("query.sql", icons::DATABASE),
        ("dump.parquet", icons::DATABASE),
        ("app.nim", icons::LANG_NIM),
        ("app.cr", icons::LANG_CRYSTAL),
        ("app.rkt", icons::LANG_RACKET),
        ("app.fs", icons::LANG_FSHARP),
        ("view.qml", icons::LANG_QT),
        ("flake.nix", icons::LANG_NIXOS),
        ("note.ipynb", icons::LANG_JUPYTER),
        ("data.sqlite3", icons::DATABASE),
        ("disk.iso", icons::DISC),
        ("data.tsv", icons::FILE_SPREADSHEET),
        ("sheet.xlsx", icons::FILE_SPREADSHEET),
        ("main.rs", icons::LANG_RUST),
        ("main.hpp", icons::LANG_CPP),
        ("main.c", icons::LANG_C),
        ("app.py", icons::LANG_PYTHON),
        ("app.pyw", icons::LANG_PYTHON),
        ("app.js", icons::LANG_JS),
        ("app.jsx", icons::LANG_JS),
        ("app.mjs", icons::LANG_JS),
        ("app.ts", icons::LANG_TS),
        ("app.tsx", icons::LANG_TS),
        ("app.mts", icons::LANG_TS),
        ("main.go", icons::LANG_GO),
        ("Main.java", icons::LANG_JAVA),
        ("Main.kt", icons::LANG_KOTLIN),
        ("build.kts", icons::LANG_KOTLIN),
        ("app.swift", icons::LANG_SWIFT),
        ("app.dart", icons::LANG_DART),
        ("app.scala", icons::LANG_SCALA),
        ("app.hs", icons::LANG_HASKELL),
        ("app.lua", icons::LANG_LUA),
        ("app.cs", icons::LANG_CSHARP),
        ("app.r", icons::LANG_R),
        ("app.jl", icons::LANG_JULIA),
        ("app.ex", icons::LANG_ELIXIR),
        ("app.exs", icons::LANG_ELIXIR),
        ("app.erl", icons::LANG_ERLANG),
        ("app.zig", icons::LANG_ZIG),
        ("app.vue", icons::LANG_VUE),
        ("app.svelte", icons::LANG_SVELTE),
        ("app.astro", icons::LANG_ASTRO),
        ("app.elm", icons::LANG_ELM),
        ("app.rb", icons::LANG_RUBY),
        ("view.erb", icons::LANG_RUBY),
        ("index.php", icons::LANG_PHP),
        ("app.pl", icons::LANG_PERL),
        ("app.ml", icons::LANG_OCAML),
        ("app.clj", icons::LANG_CLOJURE),
        ("app.groovy", icons::LANG_GROOVY),
        ("main.tf", icons::LANG_TERRAFORM),
        ("schema.graphql", icons::LANG_GRAPHQL),
        ("token.sol", icons::LANG_SOLIDITY),
        ("notes.m", icons::FILE_CODE),
        ("notes.v", icons::FILE_CODE),
        ("boot.s", icons::FILE_CODE),
        ("app.tcl", icons::FILE_CODE),
        ("app.jsp", icons::LANG_JAVA),
        ("app.jar", icons::BOX),
        ("lib.o", icons::BOX),
        ("pkg.crate", icons::BOX),
        ("build.cmake", icons::COG),
        ("settings.cfg", icons::COG),
        ("Rakefile", icons::LANG_RUBY),
        ("Vagrantfile", icons::LANG_RUBY),
        ("Justfile", icons::COG),
        (".env.local", icons::COG),
        (".rprofile", icons::LANG_R),
        ("run.ps1", icons::FILE_TERMINAL),
        ("run.bash", icons::FILE_TERMINAL),
        ("report.DOCX", icons::DOCUMENTS),
        ("report.odt", icons::DOCUMENTS),
        ("deck.pptx", icons::PRESENTATION),
        ("deck.odp", icons::PRESENTATION),
        ("README.md", icons::DOCUMENTS),
        ("font.woff2", icons::FILE_TYPE),
        ("doc.pdf", icons::DOCUMENTS),
    ] {
        assert_eq!(icon_for_name(name), icon, "file: {name}");
    }
}

#[test]
fn exact_names_override_extensions_without_matching_backups() {
    for (name, icon) in [
        ("package-lock.json", icons::COG),
        ("pnpm-lock.yaml", icons::COG),
        ("CMakeLists.txt", icons::COG),
        ("Cargo.lock", icons::COG),
        (".BASHRC", icons::FILE_TERMINAL),
        (".zprofile", icons::FILE_TERMINAL),
        (".gitignore", icons::COG),
        (".XCompose", icons::COG),
        ("Makefile", icons::COG),
        ("Dockerfile", icons::COG),
        ("id_ed25519", icons::KEY_ROUND),
        ("known_hosts", icons::KEY_ROUND),
        ("README", icons::DOCUMENTS),
        ("LICENSE", icons::DOCUMENTS),
        (".bashrc.omarchy-upgrade.done.bak", icons::DOCUMENTS),
        (".bash_history.bak", icons::DOCUMENTS),
        ("id_rsa.bak", icons::DOCUMENTS),
        ("Dockerfile.bak", icons::COG),
    ] {
        assert_eq!(icon_for_name(name), icon, "file: {name}");
    }
}

#[test]
fn filename_patterns_override_suffixes_and_respect_casing() {
    for (name, icon) in [
        ("Dockerfile.txt", icons::COG),
        ("Dockerfile.dev", icons::COG),
        ("tsconfig.json", icons::COG),
        ("tsconfig.base.json", icons::COG),
        ("tsconfig.base.json.bak", icons::DOCUMENTS),
        ("other-tsconfig.json", icons::FILE_BRACES),
        (".bash_history", icons::FILE_TERMINAL),
        (".BASH_HISTORY", icons::DOCUMENTS),
        ("yarn.lock", icons::COG),
        ("poetry.lock", icons::COG),
    ] {
        assert_eq!(icon_for_name(name), icon, "file: {name}");
    }
}

#[test]
fn unknown_files_preserve_the_document_fallback() {
    for name in ["notes.xyz", "randomfile", "archive.???", "", "trailing."] {
        assert_eq!(icon_for_name(name), icons::DOCUMENTS, "file: {name}");
    }
}
