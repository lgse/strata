// SPDX-License-Identifier: MIT

use std::{
    error::Error,
    fs::{self, File},
    os::{fd::OwnedFd, unix::fs::MetadataExt},
    process::{Child, Command, Stdio},
};

use rustix::fs::{Mode, OFlags};

use super::*;

const NFS_LEFTOVER: &str = ".nfs000000000000002a00000001";
const FUSE_LEFTOVER: &str = ".fuse_hidden0000002a00000001";

fn open_folder(path: &Path) -> Result<OwnedFd, Box<dyn Error>> {
    Ok(rustix::fs::open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?)
}

fn leftovers(names: &[&str]) -> Vec<OsString> {
    names.iter().map(OsString::from).collect()
}

#[derive(Clone, Copy, Debug)]
enum Aside {
    RenamedToALeftover,
    UnlinkedBesideALeftover,
    UnlinkedUnlisted,
    OpenInASiblingFolder,
    OtherFileOpenBesideALeftover,
}

#[test]
fn a_file_this_process_keeps_open_below_a_folder_is_named_as_strata() -> Result<(), Box<dyn Error>>
{
    use Aside::*;
    for aside in [
        RenamedToALeftover,
        UnlinkedBesideALeftover,
        UnlinkedUnlisted,
        OpenInASiblingFolder,
        OtherFileOpenBesideALeftover,
    ] {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("folder");
        let sibling = root.path().join("sibling");
        fs::create_dir(&folder)?;
        fs::create_dir(&sibling)?;
        let file = folder.join("open.txt");
        fs::write(&file, b"open")?;
        let (held, listed) = match aside {
            RenamedToALeftover => {
                let held = File::open(&file)?;
                fs::rename(&file, folder.join(NFS_LEFTOVER))?;
                (held, leftovers(&[NFS_LEFTOVER]))
            }
            UnlinkedBesideALeftover => {
                let held = File::open(&file)?;
                fs::hard_link(&file, folder.join(FUSE_LEFTOVER))?;
                fs::remove_file(&file)?;
                (held, leftovers(&[FUSE_LEFTOVER]))
            }
            UnlinkedUnlisted => {
                let held = File::open(&file)?;
                fs::remove_file(&file)?;
                (held, Vec::new())
            }
            OpenInASiblingFolder => {
                fs::rename(&file, folder.join(NFS_LEFTOVER))?;
                fs::hard_link(folder.join(NFS_LEFTOVER), sibling.join("open.txt"))?;
                (
                    File::open(sibling.join("open.txt"))?,
                    leftovers(&[NFS_LEFTOVER]),
                )
            }
            OtherFileOpenBesideALeftover => {
                fs::write(folder.join(NFS_LEFTOVER), b"held elsewhere")?;
                (File::open(&file)?, leftovers(&[NFS_LEFTOVER]))
            }
        };
        let opened = open_folder(&folder)?;

        let found = scan(opened.as_fd(), &listed, &[std::process::id()]);

        let expected = match aside {
            RenamedToALeftover | UnlinkedBesideALeftover | UnlinkedUnlisted => Some(Holder::Strata),
            OpenInASiblingFolder | OtherFileOpenBesideALeftover => None,
        };
        assert_eq!(found, expected, "{aside:?}");
        drop(held);
    }
    Ok(())
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Orphan(u32);

impl Drop for Orphan {
    fn drop(&mut self) {
        let _ = Command::new("kill").arg(self.0.to_string()).status();
    }
}

#[derive(Clone, Copy, Debug)]
enum Started {
    AsAHelper,
    ThroughAHelper,
    ForTheUser,
    ThroughAProgramStartedForTheUser,
    Elsewhere,
}

fn holding_sleep(
    started: Started,
    file: &Path,
) -> Result<(Killed, Option<Orphan>, u32), Box<dyn Error>> {
    use std::{io::BufRead, os::unix::process::CommandExt};
    let mut command = match started {
        Started::AsAHelper | Started::ForTheUser => {
            let mut sleep = Command::new("sleep");
            sleep.arg("30").stdin(File::open(file)?);
            sleep
        }
        Started::ThroughAHelper | Started::ThroughAProgramStartedForTheUser => {
            let mut shell = Command::new("sh");
            shell
                .args(["-c", "sleep 30 <\"$1\" & echo $!; wait", "sh"])
                .arg(file)
                .stdout(Stdio::piped());
            shell
        }
        Started::Elsewhere => {
            let mut shell = Command::new("sh");
            shell
                .args(["-c", "sleep 30 <\"$1\" >/dev/null 2>&1 & echo $!", "sh"])
                .arg(file)
                .stdout(Stdio::piped());
            shell
        }
    };
    if matches!(started, Started::AsAHelper | Started::ThroughAHelper) {
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let printed = match child.stdout.take() {
        Some(stdout) => {
            let mut line = String::new();
            std::io::BufReader::new(stdout).read_line(&mut line)?;
            Some(line.trim().parse::<u32>()?)
        }
        None => None,
    };
    if matches!(started, Started::Elsewhere) {
        child.wait()?;
    }
    let holder = printed.unwrap_or(child.id());
    let orphan = printed.map(Orphan);
    let started_by = Instant::now() + Duration::from_secs(5);
    while fs::read_to_string(format!("/proc/{holder}/comm"))?.trim_end() != "sleep" {
        if Instant::now() > started_by {
            return Err("the holder never ran sleep".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok((Killed(child), orphan, holder))
}

#[test]
fn a_helper_strata_started_is_strata_and_any_other_process_is_named() -> Result<(), Box<dyn Error>>
{
    use Started::*;
    for started in [
        AsAHelper,
        ThroughAHelper,
        ForTheUser,
        ThroughAProgramStartedForTheUser,
        Elsewhere,
    ] {
        let root = tempfile::tempdir()?;
        let folder = root.path().join("folder");
        fs::create_dir(&folder)?;
        let file = folder.join("open.txt");
        fs::write(&file, b"open")?;
        let (_child, _orphan, holder) = holding_sleep(started, &file)?;
        fs::rename(&file, folder.join(NFS_LEFTOVER))?;
        let opened = open_folder(&folder)?;

        let found = scan(opened.as_fd(), &leftovers(&[NFS_LEFTOVER]), &[holder]);

        let expected = match started {
            AsAHelper | ThroughAHelper => Holder::Strata,
            ForTheUser | ThroughAProgramStartedForTheUser | Elsewhere => {
                Holder::Program("sleep".to_owned())
            }
        };
        assert_eq!(found, Some(expected), "{started:?}");
    }
    Ok(())
}

#[test]
fn a_mapping_without_a_descriptor_is_found_by_its_device_and_inode() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join(NFS_LEFTOVER), b"mapped")?;
    let opened = open_folder(&folder)?;
    let target = Target::read(opened.as_fd(), &leftovers(&[NFS_LEFTOVER])).ok_or("the folder")?;
    let inode = fs::metadata(folder.join(NFS_LEFTOVER))?.ino();
    let (major, minor) = target.device;
    let line = |device: &str, inode: u64, path: &Path| {
        format!(
            "7f0000000000-7f0000001000 r--s 00000000 {device} {inode}                  {}",
            path.display()
        )
    };
    let device = format!("{major:02x}:{minor:02x}");
    let deleted = PathBuf::from(format!("{}/open.txt (deleted)", folder.display()));

    for (row, maps, expected) in [
        (
            "leftover",
            line(&device, inode, &folder.join(NFS_LEFTOVER)),
            true,
        ),
        ("deleted name", line(&device, inode, &deleted), true),
        ("another inode", line(&device, inode + 1, &deleted), false),
        ("another device", line("fff:ff", inode, &deleted), false),
        ("the folder itself", line(&device, inode, &folder), false),
        (
            "anonymous",
            "7f0000000000-7f0000001000 rw-p 00000000 00:00 0".to_owned(),
            false,
        ),
    ] {
        assert_eq!(target.maps(&maps), expected, "{row}");
    }
    Ok(())
}

#[test]
fn apps_are_named_by_their_systemd_unit_or_the_desktop_file_gio_launched() {
    let user = "0::/user.slice/user-1000.slice/user@1000.service/app.slice";
    let session = "0::/user.slice/user-1000.slice/session-2.scope\n";
    for (cgroup, expected) in [
        (
            format!("{user}/app-gnome-org.gnome.TextEditor-4242.scope\n"),
            vec!["org.gnome.TextEditor", "gnome-org.gnome.TextEditor"],
        ),
        (
            format!("{user}/app-Hyprland-org.libreoffice.LibreOffice\\x2dwriter@0f9b1c.service\n"),
            vec![
                "org.libreoffice.LibreOffice-writer",
                "Hyprland-org.libreoffice.LibreOffice-writer",
            ],
        ),
        (
            format!("{user}/app-org.kde.kate@3c2b.service\n"),
            vec!["org.kde.kate"],
        ),
        (
            format!("{user}/app-org.gnome.Terminal.slice/vte-spawn-5a1f.scope\n"),
            vec!["org.gnome.Terminal"],
        ),
        (
            format!(
                "12:pids:/user.slice\n1:name=systemd:{}/app-gimp-1234.scope\n",
                &user[3..]
            ),
            vec!["gimp"],
        ),
        (session.to_owned(), Vec::new()),
    ] {
        assert_eq!(app_ids(&cgroup, session), expected, "{cgroup}");
    }
    let strata = format!("{user}/app-Hyprland-io.github.strata@0f9b1c.service\n");
    let gnome_terminal =
        |tab: &str| format!("{user}/app-org.gnome.Terminal.slice/vte-spawn-{tab}.scope\n");
    for (cgroup, own, expected) in [
        (strata.clone(), strata.clone(), Vec::new()),
        (
            strata.clone(),
            session.to_owned(),
            vec!["io.github.strata", "Hyprland-io.github.strata"],
        ),
        (gnome_terminal("2"), gnome_terminal("1"), Vec::new()),
    ] {
        assert_eq!(app_ids(&cgroup, &own), expected, "{cgroup} in {own}");
    }

    let launched = |pid: &str| {
        format!(
            "HOME=/home/user\0GIO_LAUNCHED_DESKTOP_FILE=/usr/share/applications/org.gnome.TextEditor.desktop\0GIO_LAUNCHED_DESKTOP_FILE_PID={pid}\0"
        )
    };
    let desktop = Some(PathBuf::from(
        "/usr/share/applications/org.gnome.TextEditor.desktop",
    ));
    assert_eq!(
        launched_desktop_file(launched("4242").as_bytes(), 4242),
        desktop
    );
    assert_eq!(
        launched_desktop_file(launched("4242").as_bytes(), 4343),
        None
    );
    assert_eq!(launched_desktop_file(b"HOME=/home/user\0", 4242), None);
}
