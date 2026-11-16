// SPDX-License-Identifier: MIT

use std::path::Path;

use super::{NoticeProblem, SaveSubject, save_notice_summary, save_notice_text};

#[test]
fn formats_write_and_startup_read_notices_using_first_line_without_period() {
    let path = Path::new("/tmp/config/strata/settings.toml");
    let (title, detail) = save_notice_text(
        SaveSubject::Settings,
        NoticeProblem::WriteFailed,
        path,
        "Is a directory.\nextra detail",
    );
    assert_eq!(title, "Settings can't be saved");
    assert!(
        detail.contains("/tmp/config/strata/settings.toml"),
        "{detail}"
    );
    assert!(
        detail.contains(": Is a directory. It tries again"),
        "{detail}"
    );
    assert!(!detail.contains("extra detail"), "{detail}");
    assert_eq!(
        save_notice_summary(),
        "Changes last only until Strata closes"
    );

    let (read_title, read_detail) = save_notice_text(
        SaveSubject::FolderViews,
        NoticeProblem::UnreadableAtStartup,
        Path::new("/tmp/state/strata/folder-views.toml"),
        "Permission denied.",
    );
    assert_eq!(read_title, "Folder settings file can't be read");
    assert!(
        read_detail.contains("won't save over it") && read_detail.contains("Permission denied"),
        "{read_detail}"
    );
}
