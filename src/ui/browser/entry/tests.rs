// SPDX-License-Identifier: MIT

use super::{catppuccin, icon_for_name};
use crate::assets::icons;

#[test]
fn file_categories_and_case_insensitive_extensions() {
    for (name, icon) in [
        ("song.mp3", icons::FILE_AUDIO),
        ("take.WAV", icons::FILE_AUDIO),
        ("photo.png", icons::PICTURES),
        ("movie.mp4", icons::VIDEOS),
        ("video.mpeg", icons::VIDEOS),
        ("video.mpg", icons::VIDEOS),
        ("video.ogv", icons::VIDEOS),
        ("video.3gp", icons::VIDEOS),
        ("video.3g2", icons::VIDEOS),
        ("video.flv", icons::VIDEOS),
        ("video.wmv", icons::VIDEOS),
        ("video.asf", icons::VIDEOS),
        ("video.m2ts", icons::VIDEOS),
        ("video.vob", icons::VIDEOS),
        ("page.html", catppuccin::HTML),
        ("mailing.mjml", catppuccin::MJML),
        ("view.twig", catppuccin::TWIG),
        ("style.scss", catppuccin::SASS),
        ("feed.xml", catppuccin::XML),
        ("schema.xsd", catppuccin::XML),
        ("backup.zip", icons::FILE_ARCHIVE),
        ("backup.tar.gz", icons::FILE_ARCHIVE),
        ("backup.tgz", icons::FILE_ARCHIVE),
        ("backup.lz", icons::FILE_ARCHIVE),
        ("backup.lzma", icons::FILE_ARCHIVE),
        ("backup.lzo", icons::FILE_ARCHIVE),
        ("backup.rz", icons::FILE_ARCHIVE),
        ("backup.sz", icons::FILE_ARCHIVE),
        ("backup.cab", icons::FILE_ARCHIVE),
        ("backup.arj", icons::FILE_ARCHIVE),
        ("backup.cpio", icons::FILE_ARCHIVE),
        ("installer.AppImage", icons::BOX),
        ("app.apk", catppuccin::ANDROID),
        ("server.key", icons::KEY_ROUND),
        ("certificate.p12", icons::KEY_ROUND),
        ("config.toml", catppuccin::TOML),
        (".env", catppuccin::ENV),
        ("data.json", catppuccin::JSON),
        ("data.jsonc", catppuccin::JSON),
        ("data.jsonl", catppuccin::JSON),
        ("schema.proto", catppuccin::PROTO),
        ("query.sql", catppuccin::DATABASE),
        ("dump.parquet", catppuccin::DATABASE),
        ("app.nim", catppuccin::NIM),
        ("app.cr", catppuccin::CRYSTAL),
        ("app.rkt", catppuccin::RACKET),
        ("app.fs", catppuccin::FSHARP),
        ("view.qml", icons::FILE_CODE),
        ("flake.nix", catppuccin::NIX),
        ("note.ipynb", catppuccin::JUPYTER),
        ("data.sqlite3", catppuccin::DATABASE),
        ("disk.iso", icons::DISC),
        ("data.tsv", icons::FILE_SPREADSHEET),
        ("sheet.xlsx", icons::FILE_SPREADSHEET),
        ("main.rs", catppuccin::RUST),
        ("main.hpp", catppuccin::CPP_HEADER),
        ("main.c", catppuccin::C),
        ("app.py", catppuccin::PYTHON),
        ("app.pyw", catppuccin::PYTHON),
        ("app.js", catppuccin::JAVASCRIPT),
        ("app.jsx", catppuccin::JAVASCRIPT_REACT),
        ("app.mjs", catppuccin::JAVASCRIPT),
        ("app.ts", catppuccin::TYPESCRIPT),
        ("app.tsx", catppuccin::TYPESCRIPT_REACT),
        ("app.mts", catppuccin::TYPESCRIPT),
        ("main.go", catppuccin::GO),
        ("Main.java", catppuccin::JAVA),
        ("Main.kt", catppuccin::KOTLIN),
        ("build.kts", catppuccin::KOTLIN),
        ("app.swift", catppuccin::SWIFT),
        ("app.dart", catppuccin::DART),
        ("app.scala", catppuccin::SCALA),
        ("app.hs", catppuccin::HASKELL),
        ("app.lua", catppuccin::LUA),
        ("app.cs", catppuccin::CSHARP),
        ("app.r", catppuccin::R),
        ("app.jl", catppuccin::JULIA),
        ("app.ex", catppuccin::ELIXIR),
        ("app.exs", catppuccin::ELIXIR),
        ("app.erl", catppuccin::ERLANG),
        ("app.zig", catppuccin::ZIG),
        ("app.vue", catppuccin::VUE),
        ("app.svelte", catppuccin::SVELTE),
        ("app.astro", catppuccin::ASTRO),
        ("app.elm", catppuccin::ELM),
        ("app.rb", catppuccin::RUBY),
        ("view.erb", catppuccin::RUBY),
        ("index.php", catppuccin::PHP),
        ("app.pl", catppuccin::PERL),
        ("app.ml", catppuccin::OCAML),
        ("app.clj", catppuccin::CLOJURE),
        ("app.groovy", catppuccin::GROOVY),
        ("main.tf", catppuccin::TERRAFORM),
        ("schema.graphql", catppuccin::GRAPHQL),
        ("token.sol", catppuccin::SOLIDITY),
        ("notes.m", catppuccin::MATLAB),
        ("notes.v", catppuccin::V),
        ("boot.s", catppuccin::ASSEMBLY),
        ("app.tcl", icons::FILE_CODE),
        ("app.jsp", catppuccin::JAVA),
        ("app.jar", catppuccin::JAVA_JAR),
        ("lib.o", icons::BOX),
        ("pkg.crate", icons::BOX),
        ("build.cmake", catppuccin::CMAKE),
        ("settings.cfg", catppuccin::PROPERTIES),
        ("Rakefile", catppuccin::RUBY),
        ("Vagrantfile", catppuccin::RUBY),
        ("Justfile", catppuccin::JUST),
        (".env.local", catppuccin::ENV),
        (".rprofile", catppuccin::R),
        ("run.ps1", catppuccin::POWERSHELL),
        ("run.bash", catppuccin::BASH),
        ("report.DOCX", icons::DOCUMENTS),
        ("report.odt", icons::DOCUMENTS),
        ("deck.pptx", icons::PRESENTATION),
        ("deck.odp", icons::PRESENTATION),
        ("README.md", catppuccin::README),
        ("font.woff2", icons::FILE_TYPE),
        ("doc.pdf", icons::DOCUMENTS),
    ] {
        assert_eq!(icon_for_name(name), icon, "file: {name}");
    }
}

#[test]
fn exact_names_override_extensions_without_matching_backups() {
    for (name, icon) in [
        ("package-lock.json", catppuccin::NPM_LOCK),
        ("pnpm-lock.yaml", catppuccin::PNPM_LOCK),
        ("CMakeLists.txt", catppuccin::CMAKE),
        ("Cargo.lock", catppuccin::CARGO_LOCK),
        (".BASHRC", catppuccin::BASH),
        (".zprofile", catppuccin::BASH),
        (".gitignore", catppuccin::GIT),
        (".XCompose", catppuccin::CONFIG),
        ("Makefile", catppuccin::MAKEFILE),
        ("Dockerfile", catppuccin::DOCKER),
        ("id_ed25519", icons::KEY_ROUND),
        ("known_hosts", icons::KEY_ROUND),
        ("README", catppuccin::README),
        ("LICENSE", catppuccin::LICENSE),
        (".bashrc.omarchy-upgrade.done.bak", icons::DOCUMENTS),
        (".bash_history.bak", icons::DOCUMENTS),
        ("id_rsa.bak", icons::DOCUMENTS),
        ("Dockerfile.bak", catppuccin::DOCKER),
    ] {
        assert_eq!(icon_for_name(name), icon, "file: {name}");
    }
}

#[test]
fn filename_patterns_override_suffixes_and_respect_casing() {
    for (name, icon) in [
        ("Dockerfile.txt", catppuccin::DOCKER),
        ("Dockerfile.dev", catppuccin::DOCKER),
        ("tsconfig.json", catppuccin::TYPESCRIPT_CONFIG),
        ("tsconfig.base.json", catppuccin::TYPESCRIPT_CONFIG),
        ("tsconfig.base.json.bak", icons::DOCUMENTS),
        ("other-tsconfig.json", catppuccin::JSON),
        (".bash_history", icons::FILE_TERMINAL),
        (".BASH_HISTORY", icons::DOCUMENTS),
        ("yarn.lock", catppuccin::YARN_LOCK),
        ("poetry.lock", catppuccin::POETRY_LOCK),
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

#[test]
fn catppuccin_associations_prefer_longest_suffix_and_exact_names() {
    for (name, icon) in [
        ("types.d.ts", catppuccin::TYPESCRIPT_DEF),
        ("app.test.ts", catppuccin::TYPESCRIPT_TEST),
        ("docker-compose.override.yml", catppuccin::DOCKER_COMPOSE),
        ("compose.yaml", catppuccin::DOCKER_COMPOSE),
        ("deploy.yml", catppuccin::YAML),
        ("package.json", catppuccin::PACKAGE_JSON),
        ("Cargo.toml", catppuccin::CARGO),
        ("rustfmt.toml", catppuccin::RUST_CONFIG),
        ("pyproject.toml", catppuccin::PYTHON_CONFIG),
        ("go.mod", catppuccin::GO_MOD),
        ("vite.config.ts", catppuccin::VITE),
        (".eslintrc.json", catppuccin::ESLINT),
        ("CHANGELOG.md", catppuccin::CHANGELOG),
        ("notes.md", catppuccin::MARKDOWN),
        ("ci.yml", catppuccin::YAML),
        ("archive.lock", icons::COG),
        ("photo.svg", icons::PICTURES),
        ("notes.txt", icons::DOCUMENTS),
        ("report.csv", icons::FILE_SPREADSHEET),
        ("server.pem", icons::KEY_ROUND),
    ] {
        assert_eq!(icon_for_name(name), icon, "file: {name}");
    }
}

#[test]
fn entry_kind_summary_distinguishes_folders_files_and_symlinks() {
    let make_entry = |name: &str, kind: crate::model::EntryKind| {
        let mut entry = crate::test_support::operations::entry(crate::model::Location::local(name));
        entry.kind = kind;
        entry
    };

    let dir = make_entry("realdir", crate::model::EntryKind::Directory);
    let dir2 = make_entry("realdir2", crate::model::EntryKind::Directory);
    let file = make_entry("file.txt", crate::model::EntryKind::File);
    let file2 = make_entry("file2.txt", crate::model::EntryKind::File);
    let dir_link = make_entry("0dirlink", crate::model::EntryKind::DirectorySymbolicLink);
    let file_link = make_entry("filelink", crate::model::EntryKind::FileSymbolicLink);

    assert_eq!(
        super::entry_kind_summary(std::slice::from_ref(&dir)),
        "1 folder"
    );
    assert_eq!(super::entry_kind_summary(&[dir.clone(), dir2]), "2 folders");
    assert_eq!(
        super::entry_kind_summary(std::slice::from_ref(&file)),
        "1 file"
    );
    assert_eq!(super::entry_kind_summary(&[file.clone(), file2]), "2 files");
    assert_eq!(
        super::entry_kind_summary(std::slice::from_ref(&dir_link)),
        "1 item"
    );
    assert_eq!(
        super::entry_kind_summary(std::slice::from_ref(&file_link)),
        "1 file"
    );
    assert_eq!(super::entry_kind_summary(&[dir, file.clone()]), "2 items");
    assert_eq!(
        super::entry_kind_summary(&[dir_link.clone(), file]),
        "2 items"
    );
    assert_eq!(super::entry_kind_summary(&[dir_link, file_link]), "2 items");
}

#[test]
fn aggregate_directory_summary_does_not_recurse_into_directory_symlinks() {
    let fixture = tempfile::tempdir().expect("fixture");
    let real_dir = fixture.path().join("realdir");
    std::fs::create_dir(&real_dir).expect("realdir");
    for i in 1..=5 {
        std::fs::write(real_dir.join(format!("file{i}")), b"x").expect("file");
    }
    let symlink = fixture.path().join("0dirlink");
    std::os::unix::fs::symlink(&real_dir, &symlink).expect("symlink");

    let context = glib::MainContext::new();
    let dir_link_entry = context
        .block_on(crate::adapters::query_file_entry(
            crate::model::Location::local(&symlink),
        ))
        .expect("query symlink entry");
    assert_eq!(
        dir_link_entry.kind,
        crate::model::EntryKind::DirectorySymbolicLink
    );

    let real_dir_entry = context
        .block_on(crate::adapters::query_file_entry(
            crate::model::Location::local(&real_dir),
        ))
        .expect("query realdir entry");
    assert_eq!(real_dir_entry.kind, crate::model::EntryKind::Directory);

    let link_summary = context.block_on(super::aggregate_directory_summary(
        std::slice::from_ref(&dir_link_entry),
        |_, _| {},
    ));
    assert_eq!(link_summary.item_count, 1);
    assert_eq!(link_summary.total_size, 0);
    assert_eq!(link_summary.visible_file_count, 1);
    assert_eq!(link_summary.visible_folder_count, 0);
    assert!(!link_summary.truncated());

    let dir_summary = context.block_on(super::aggregate_directory_summary(
        std::slice::from_ref(&real_dir_entry),
        |_, _| {},
    ));
    assert_eq!(dir_summary.item_count, 5);
    assert_eq!(dir_summary.total_size, 5);
    assert_eq!(dir_summary.visible_file_count, 5);
    assert_eq!(dir_summary.visible_folder_count, 0);
    assert!(!dir_summary.truncated());

    let combined_summary = context.block_on(super::aggregate_directory_summary(
        &[
            dir_link_entry,
            crate::test_support::operations::entry(crate::model::Location::local(
                fixture.path().join("other.txt"),
            )),
        ],
        |_, _| {},
    ));
    assert_eq!(combined_summary.item_count, 2);
    assert_eq!(combined_summary.total_size, 100);
    assert!(!combined_summary.truncated());
}
