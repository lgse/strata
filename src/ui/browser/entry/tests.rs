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
        ("page.html", icons::GLOBE),
        ("style.scss", icons::GLOBE),
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
        ("query.sql", icons::DATABASE),
        ("data.sqlite3", icons::DATABASE),
        ("disk.iso", icons::DISC),
        ("data.tsv", icons::FILE_SPREADSHEET),
        ("sheet.xlsx", icons::FILE_SPREADSHEET),
        ("main.rs", icons::FILE_CODE),
        ("main.hpp", icons::FILE_CODE),
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
