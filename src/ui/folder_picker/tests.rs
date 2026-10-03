// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::{PickerScope, Refused, scope, typed_path, wants_hidden};

const HOME: &str = "/home/user";
const CURRENT: &str = "/work/project";

fn picked(base: &str, query: &str) -> Result<Option<PickerScope>, &'static str> {
    Ok(Some(PickerScope {
        base: PathBuf::from(base),
        query: query.to_owned(),
    }))
}

#[test]
fn text_searches_the_open_folder_until_it_names_another() {
    for (text, expected) in [
        ("", Ok(None)),
        ("   ", Ok(None)),
        ("proj", picked(CURRENT, "proj")),
        ("  src ui ", picked(CURRENT, "src ui")),
        ("src/ui", picked(CURRENT, "src/ui")),
        (".config", picked(CURRENT, ".config")),
        ("~", picked(HOME, "")),
        ("~/", picked(HOME, "")),
        ("~/dev/str", picked("/home/user/dev", "str")),
        ("~/dev/ foo bar", picked("/home/user/dev", "foo bar")),
        (
            "~/My Documents/rep",
            picked("/home/user/My Documents", "rep"),
        ),
        ("/", picked("/", "")),
        ("/tmp", picked("/", "tmp")),
        ("/usr/lib/", picked("/usr/lib", "")),
        ("/..", picked("/", "")),
        ("..", picked("/work", "")),
        ("../sib", picked("/work", "sib")),
        ("./a/../b/x", picked("/work/project/b", "x")),
        ("smb://host/share", Err("Only local folders can be chosen")),
        ("~bob/x", Err("Only ~ and ~/ are supported")),
    ] {
        assert_eq!(
            scope(text, Some(Path::new(CURRENT)), Path::new(HOME)),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn uris_are_never_searched_even_with_slashes_or_credentials() {
    for text in [
        "sftp://user:secret@host.invalid/srv/",
        "smb://host.invalid/share/Do",
        "smb:",
        "dav:host/share",
        "//host.invalid/share/",
        "\\\\host\\share",
        "user@host.invalid:/srv/",
    ] {
        assert_eq!(
            scope(text, Some(Path::new(CURRENT)), Path::new(HOME)),
            Err("Only local folders can be chosen"),
            "{text:?}"
        );
    }
}

#[test]
fn only_typed_full_paths_work_without_a_local_folder() {
    for (text, expected) in [
        ("proj", Err("Type a full path here")),
        ("../sib", Err("Type a full path here")),
        ("/tmp/x", picked("/tmp", "x")),
        ("~/x", picked(HOME, "x")),
    ] {
        assert_eq!(scope(text, None, Path::new(HOME)), expected, "{text:?}");
    }
}

#[test]
fn a_leading_dot_term_includes_hidden_folders() {
    for (query, hidden) in [
        (".config", true),
        ("dev !.git", true),
        ("'.cache", true),
        ("dev/.local share", true),
        ("dev config", false),
        ("v1.2", false),
    ] {
        assert_eq!(wants_hidden(query), hidden, "{query:?}");
    }
}

#[test]
fn refused_folders_cover_sent_trees_and_a_move_source() {
    let refused = Refused {
        trees: vec![PathBuf::from("/work/dest")],
        folder: Some(PathBuf::from("/work")),
    };
    for (path, expected) in [
        ("/work", true),
        ("/work/dest", true),
        ("/work/dest/inner", true),
        ("/work/dest2", false),
        ("/work/other", false),
    ] {
        assert_eq!(refused.refuses(Path::new(path)), expected, "{path}");
    }
}

#[test]
fn tab_writes_a_folder_back_as_a_path_that_searches_inside_it() {
    for (folder, text) in [
        ("/work/project", "./"),
        ("/work/project/src/ui", "./src/ui/"),
        ("/home/user", "~/"),
        ("/home/user/My Documents", "~/My Documents/"),
        ("/work", "/work/"),
        ("/", "/"),
    ] {
        let typed = typed_path(Path::new(folder), Some(Path::new(CURRENT)), Path::new(HOME));
        assert_eq!(typed, text, "{folder}");
        let searched = scope(&typed, Some(Path::new(CURRENT)), Path::new(HOME));
        assert_eq!(searched, picked(folder, ""), "{folder}");
    }
}
