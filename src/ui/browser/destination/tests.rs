// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn destination_paths_expand_home_and_relative_input() {
    let base = std::path::Path::new("/work/current");
    let home = std::path::Path::new("/home/example");

    assert_eq!(resolve_destination_path("~", base, home), home);
    assert_eq!(
        resolve_destination_path("~/Documents", base, home),
        home.join("Documents")
    );
    assert_eq!(
        resolve_destination_path("../Archive", base, home),
        base.join("../Archive")
    );
    assert_eq!(
        resolve_destination_path("/tmp/export", base, home),
        std::path::Path::new("/tmp/export")
    );
}

#[test]
fn path_suggestions_list_only_matching_folders() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("strata-path-suggestions-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Documents"))?;
    std::fs::create_dir_all(root.join("Downloads"))?;
    std::fs::create_dir_all(root.join("relative/Documents"))?;
    std::fs::write(root.join("Document.txt"), b"not a folder")?;
    let home = root.join("home");

    let suggestions = path_suggestions(&format!("{}/Doc", root.display()), &root, &home);
    assert_eq!(suggestions, vec![root.join("Documents")]);

    let relative = path_suggestions("relative/Doc", &root, &home);
    assert_eq!(relative, vec![root.join("relative/Documents")]);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
