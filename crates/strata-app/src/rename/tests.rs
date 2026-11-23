// SPDX-License-Identifier: MIT

use strata_components::{
    contract::{EntryKind, LinkTarget},
    rename::{Initial, initial_selection},
};
use strata_fs::services::rename_stem_end;

#[test]
fn the_editors_first_selection_is_stratas_stem_rule() {
    for name in [
        "todo.txt",
        "readme.md",
        "archive",
        "synthetic-quarterly-report-with-a-very-long-descriptive-basename-2026.txt",
        "archive.tar.gz",
        ".bashrc",
        ".config.toml",
        "..hidden.txt",
        "README",
        "trailing.",
        "a.b.c",
        "résumé.pdf",
        "日本語.txt",
        "māo",
    ] {
        let (start, end) = initial_selection(name, EntryKind::File, Initial::Stem);
        let characters = name[..end].chars().count();
        let stem = usize::try_from(rename_stem_end(name)).expect("an offset");
        assert_eq!((start, characters), (0, stem), "{name}");

        for folder in [EntryKind::Folder, EntryKind::Symlink(LinkTarget::Folder)] {
            let whole = initial_selection(name, folder, Initial::Stem);
            assert_eq!(whole, (0, name.len()), "{name} as {folder:?}");
        }
    }
}

mod reveal;
