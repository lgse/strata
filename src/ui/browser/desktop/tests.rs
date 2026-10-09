// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn link_opening_reads_only_safe_web_shortcuts_and_bounds_input() {
    let directory = tempfile::tempdir().expect("shortcut directory");
    let path = directory.path().join("website.desktop");
    let special = directory.path().join("pipe.desktop");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &special,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .expect("FIFO");
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                assert_eq!(
                    read_web_link(&gio::File::for_path(&special))
                        .await
                        .expect("special file is not read"),
                    None
                );
                let link = crate::services::WebLink::parse("https://example.org/path?q=1#part")
                    .expect("link");
                std::fs::write(&path, link.desktop_entry()).expect("saved shortcut");
                assert_eq!(
                    read_web_link(&gio::File::for_path(&path))
                        .await
                        .expect("read link"),
                    Some(link)
                );
                for invalid in [
                    "[Desktop Entry]\nType=Link\nURL=javascript:alert(1)\n".to_owned(),
                    "[Desktop Entry]\nType=Link\nURL=https://example.org/\nExec=touch /tmp/owned\n"
                        .to_owned(),
                    "[Desktop Entry]\nType=Link\nURL=https://example.org/\n".to_owned()
                        + &"#".repeat(64 * 1024),
                ] {
                    std::fs::write(&path, invalid).expect("invalid shortcut");
                    assert!(read_web_link(&gio::File::for_path(&path)).await.is_err());
                }
                std::fs::write(
                    &path,
                    "[Desktop Entry]\nType=Application\nName=Example\nExec=example\n",
                )
                .expect("application launcher");
                assert_eq!(
                    read_web_link(&gio::File::for_path(&path))
                        .await
                        .expect("ordinary desktop file"),
                    None
                );
                assert_eq!(
                    read_web_link(&gio::File::for_path(directory.path().join("ordinary.txt")))
                        .await
                        .expect("not a shortcut"),
                    None
                );
            })
        })
        .expect("private context");
}
