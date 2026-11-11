// SPDX-License-Identifier: MIT

use super::*;

impl Scope {
    pub fn mount(&self) -> Option<(MountType, MountSite)> {
        self.run(Op::MountType, &[], |scope| {
            let root = scope.quirk(|quirks| quirks.mount_root).unwrap_or("/");
            let site = MountSite {
                point: scope.root.clone(),
                root: PathBuf::from(root),
            };
            Ok(scope.mount_type().map(|mount_type| (mount_type, site)))
        })
        .ok()
        .flatten()
    }

    pub fn mount_type(&self) -> Option<MountType> {
        self.emulated_or(|| self.host.clone())
    }

    pub fn keeps_folder_births_its_base_restamps(&self) -> bool {
        self.host
            .as_ref()
            .is_some_and(|base| !base.identity().folder_births)
            && self
                .mount_type()
                .is_some_and(|mount| mount.identity().folder_births)
    }

    pub fn current_mount_type(&self) -> Option<MountType> {
        self.emulated_or(|| host_mount_type(self.mount_id, &self.root))
    }

    pub fn emulated_or(&self, host: impl FnOnce() -> Option<MountType>) -> Option<MountType> {
        let serverino_disabled = self.state().serverino_disabled;
        let mut mount = match self.quirk(|quirks| quirks.mount_type) {
            Some((fs_type, super_options)) => MountType {
                fs_type: fs_type.to_owned(),
                super_options: super_options.to_owned(),
            },
            None => host()?,
        };
        if serverino_disabled {
            mount.super_options = mount
                .super_options
                .split(',')
                .filter(|option| *option != "serverino")
                .collect::<Vec<_>>()
                .join(",");
        }
        Some(mount)
    }

    pub fn mounts(&self) -> MountTable {
        MountTable::parse(
            self.mount_id
                .zip(self.mount_type())
                .map(|(id, mount)| {
                    let (fs_type, options) = (mount.fs_type, mount.super_options);
                    format!("{id} 0 0:0 / / rw - {fs_type} test {options}\n")
                })
                .unwrap_or_default(),
        )
    }

    pub fn server_numbers(&self) -> bool {
        self.mount_type().is_some_and(|mount| {
            mount.server_numbers()
                || (mount.fs_type == "cifs"
                    && mount
                        .super_options
                        .split(',')
                        .any(|option| option == "posix"))
        })
    }
}

pub fn host_mount_type(mount_id: Option<u64>, root: &Path) -> Option<MountType> {
    let table = MountTable::parse(std::fs::read("/proc/self/mountinfo").ok()?);
    let mount = match mount_id {
        Some(id) => table.mount_of(id),
        None => table.mount_at(root),
    };
    mount.map(|(mount_type, _)| mount_type.clone())
}
