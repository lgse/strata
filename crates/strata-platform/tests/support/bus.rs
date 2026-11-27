// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::timeout;

pub struct Bus {
    child: Child,
    address: String,
    services: PathBuf,
}

impl Bus {
    pub(super) fn start(
        dir: &Path,
        logs: &Path,
        env: &[(String, String)],
        spawned: impl FnOnce(u32),
    ) -> Bus {
        let services = dir.join("dbus-services");
        std::fs::create_dir_all(&services).expect("bus service directory");
        let config = dir.join("bus.conf");
        let socket = dir.join("bus");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <servicedir>{}</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                socket.display(),
                services.display()
            ),
        )
        .expect("bus config");
        let log = logs.join("bus.log");
        let stderr = File::create(&log).expect("bus log");
        let mut child = Command::new("dbus-daemon")
            .args(["--nofork", "--print-address=1"])
            .arg(format!("--config-file={}", config.display()))
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap_or_else(|e| panic!("starting dbus-daemon (is it installed?): {e}"));
        spawned(child.id());
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line.trim().to_owned());
        });
        let address = match rx.recv_timeout(timeout()) {
            Ok(a) if !a.is_empty() => a,
            _ => {
                let _ = child.kill();
                panic!("dbus-daemon gave no address\n{}", super::tail(&log, 30));
            }
        };
        Bus {
            child,
            address,
            services,
        }
    }

    /// `DBUS_SESSION_BUS_ADDRESS` for the session's processes.
    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn service_dir(&self) -> &Path {
        &self.services
    }

    pub fn add_service(&self, name: &str, exec: &str) {
        let file = self.services.join(format!("{name}.service"));
        let part = self.services.join(format!(".{name}.service.part"));
        std::fs::write(
            &part,
            format!("[D-BUS Service]\nName={name}\nExec={exec}\n"),
        )
        .and_then(|()| std::fs::rename(&part, &file))
        .unwrap_or_else(|e| panic!("writing {}: {e}", file.display()));
        let reload = Command::new("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.ReloadConfig",
            ])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("dbus-send (is it installed?): {e}"));
        assert!(
            reload.status.success(),
            "ReloadConfig failed: {}",
            String::from_utf8_lossy(&reload.stderr)
        );
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
