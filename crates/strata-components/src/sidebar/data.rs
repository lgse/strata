//! What the sidebar lists, as plain data the host rebuilds each view: places, pins and
//! devices. Which places show, the pins' order, which devices exist and how they group by
//! drive are Strata's: the sidebar draws what it is given, in the order given.

use rune::Color;
use strata_kit::icons::IconName;

/// A row's identity, stable across rebuilds. Ids are part of element keys
/// (`<key>-place-<id>`, `<key>-pin-<id>`, `<key>-device-<id>`), so they hold only
/// characters a key may: letters, digits, `-` and `_`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemId {
    /// A built-in place: `home`, `trash`, `downloads`, …
    Place(String),
    /// A pinned folder.
    Pin(String),
    /// A volume, mount or network share under DEVICES.
    Device(String),
}

impl ItemId {
    /// The id inside.
    pub fn id(&self) -> &str {
        match self {
            ItemId::Place(id) | ItemId::Pin(id) | ItemId::Device(id) => id,
        }
    }

    /// The row's key under the sidebar's `key`.
    pub fn key(&self, key: &str) -> String {
        let kind = match self {
            ItemId::Place(_) => "place",
            ItemId::Pin(_) => "pin",
            ItemId::Device(_) => "device",
        };
        format!("{key}-{kind}-{}", self.id())
    }
}

/// A row's icon: a semantic one, or a catalog icon by name (a folder's custom icon).
#[derive(Clone, Debug, PartialEq)]
pub enum RowIcon {
    Name(IconName),
    /// A catalog icon key, falling back to [`IconName::Folder`] when the catalog has none.
    Catalog(String),
}

/// What a built-in place is, where it matters to the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    /// A folder: Home, Desktop, Documents, …
    Folder,
    /// The Trash: a drop target, and C23's flight target ([`super::TRASH_TARGET`]).
    Trash {
        /// Trash holds items: it draws [`IconName::TrashFull`].
        full: bool,
    },
    /// Network, Recent and the like: places that aren't folders.
    Other,
}

/// A built-in place.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Place {
    pub id: String,
    pub label: String,
    pub icon: RowIcon,
    /// The folder's saved colour (folder customization), as the user chose it.
    pub tint: Option<Color>,
    pub kind: PlaceKind,
    /// Files dropped on the row go into the place ([`super::SidebarIntent::DropInto`]).
    pub accepts_drops: bool,
    /// The `g` chord's keycap, shown while [`super::SidebarState::keycaps`] is on.
    pub keycap: Option<String>,
}

impl Place {
    /// A folder place that takes drops.
    pub fn folder(id: impl Into<String>, label: impl Into<String>, icon: IconName) -> Place {
        Place {
            id: id.into(),
            label: label.into(),
            icon: RowIcon::Name(icon),
            tint: None,
            kind: PlaceKind::Folder,
            accepts_drops: true,
            keycap: None,
        }
    }

    /// The Trash, empty or not, which takes drops.
    pub fn trash(label: impl Into<String>, full: bool) -> Place {
        Place {
            kind: PlaceKind::Trash { full },
            ..Place::folder("trash", label, IconName::Trash)
        }
    }

    /// A place that isn't a folder (Network, Recent): it takes no drops.
    pub fn other(id: impl Into<String>, label: impl Into<String>, icon: IconName) -> Place {
        Place {
            kind: PlaceKind::Other,
            accepts_drops: false,
            ..Place::folder(id, label, icon)
        }
    }

    pub fn tint(mut self, tint: Option<Color>) -> Place {
        self.tint = tint;
        self
    }

    pub fn icon(mut self, icon: RowIcon) -> Place {
        self.icon = icon;
        self
    }

    pub fn keycap(mut self, keycap: impl Into<String>) -> Place {
        self.keycap = Some(keycap.into());
        self
    }

    pub fn accepts_drops(mut self, accepts: bool) -> Place {
        self.accepts_drops = accepts;
        self
    }

    pub fn item(&self) -> ItemId {
        ItemId::Place(self.id.clone())
    }
}

/// A pinned folder.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Pin {
    pub id: String,
    pub label: String,
    pub icon: RowIcon,
    pub tint: Option<Color>,
    /// Files dropped on the row go into the folder. False for remote pins: they keep their
    /// gaps but their rows take no drops.
    pub accepts_drops: bool,
    pub keycap: Option<String>,
}

impl Pin {
    /// A local folder pin, which takes drops.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Pin {
        Pin {
            id: id.into(),
            label: label.into(),
            icon: RowIcon::Name(IconName::Folder),
            tint: None,
            accepts_drops: true,
            keycap: None,
        }
    }

    /// A remote pin (SMB, SFTP): its row takes no drops.
    pub fn remote(id: impl Into<String>, label: impl Into<String>) -> Pin {
        Pin {
            icon: RowIcon::Name(IconName::Network),
            accepts_drops: false,
            ..Pin::new(id, label)
        }
    }

    pub fn tint(mut self, tint: Option<Color>) -> Pin {
        self.tint = tint;
        self
    }

    pub fn icon(mut self, icon: RowIcon) -> Pin {
        self.icon = icon;
        self
    }

    pub fn keycap(mut self, keycap: impl Into<String>) -> Pin {
        self.keycap = Some(keycap.into());
        self
    }

    pub fn item(&self) -> ItemId {
        ItemId::Pin(self.id.clone())
    }
}

/// How a device row's release button reads, from what the volume or mount allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// It can eject: "Eject".
    Eject,
    /// It can only unmount: "Unmount".
    Unmount,
}

impl Release {
    /// The button's tooltip and accessible name.
    pub fn label(self) -> &'static str {
        match self {
            Release::Eject => "Eject",
            Release::Unmount => "Unmount",
        }
    }
}

/// An encrypted device's state, which names its lock button after its action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockState {
    /// Locked: an "Unlock" button with a lock icon.
    Locked,
    /// Unlocked: a "Lock" button with an open-lock icon.
    Unlocked,
}

/// A row under DEVICES.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Device {
    pub id: String,
    /// The Strata label, else the system name.
    pub label: String,
    pub icon: IconName,
    pub mounted: bool,
    /// Its mount path when mounted, else its device path: the row's accessible description.
    pub description: String,
    /// The release button, or none (a fixed disk, a network share).
    pub release: Option<Release>,
    /// The lock button, on encrypted devices only.
    pub lock: Option<LockState>,
    /// A release is running: the row stays, disabled, with a spinner in the release column.
    pub pending: bool,
    /// Files dropped on the row go to the device's root.
    pub accepts_drops: bool,
}

impl Device {
    /// A mounted device that takes drops, with no buttons.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        icon: IconName,
        description: impl Into<String>,
    ) -> Device {
        Device {
            id: id.into(),
            label: label.into(),
            icon,
            mounted: true,
            description: description.into(),
            release: None,
            lock: None,
            pending: false,
            accepts_drops: true,
        }
    }

    pub fn mounted(mut self, mounted: bool) -> Device {
        self.mounted = mounted;
        self.accepts_drops &= mounted;
        self
    }

    pub fn release(mut self, release: Release) -> Device {
        self.release = Some(release);
        self
    }

    pub fn lock(mut self, lock: LockState) -> Device {
        self.lock = Some(lock);
        self
    }

    pub fn pending(mut self, pending: bool) -> Device {
        self.pending = pending;
        self
    }

    pub fn accepts_drops(mut self, accepts: bool) -> Device {
        self.accepts_drops = accepts;
        self
    }

    pub fn item(&self) -> ItemId {
        ItemId::Device(self.id.clone())
    }
}

/// The devices of one drive (or the rows with no drive, or the network shares), in the order
/// Strata sorted them. A divider separates each group from the one before it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DeviceGroup {
    pub devices: Vec<Device>,
    /// A divider goes before this group (unless it is the first). False for the stand-in of a
    /// device whose release is still pending, which follows every group with no divider.
    pub divided: bool,
}

impl DeviceGroup {
    pub fn new(devices: Vec<Device>) -> DeviceGroup {
        DeviceGroup {
            devices,
            divided: true,
        }
    }

    pub fn divided(mut self, divided: bool) -> DeviceGroup {
        self.divided = divided;
        self
    }
}

/// One pass over DEVICES, in order: rows, and the dividers between groups.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DeviceLine<'a> {
    /// A divider before the group with this index.
    Divider(usize),
    Device(&'a Device),
}

/// DEVICES as drawn: a divider before each divided group that has rows and follows a group
/// with rows, never above the first row or below the last.
pub(crate) fn device_lines(groups: &[DeviceGroup]) -> Vec<DeviceLine<'_>> {
    let mut lines = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        if group.devices.is_empty() {
            continue;
        }
        if group.divided && !lines.is_empty() {
            lines.push(DeviceLine::Divider(g));
        }
        lines.extend(group.devices.iter().map(DeviceLine::Device));
    }
    lines
}

/// Everything the sidebar lists this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct SidebarData<'a> {
    pub places: &'a [Place],
    pub pins: &'a [Pin],
    pub devices: &'a [DeviceGroup],
    /// The open location's row, which shows the accent tint. `None` when the open location
    /// has no row.
    pub current: Option<&'a ItemId>,
}

impl SidebarData<'_> {
    /// How many device rows DEVICES holds: what its collapsed count says. Dividers aren't rows.
    pub fn device_count(&self) -> usize {
        self.devices.iter().map(|g| g.devices.len()).sum()
    }

    /// The index of the open pin, if the open location is a pin.
    pub fn current_pin(&self) -> Option<usize> {
        match self.current {
            Some(ItemId::Pin(id)) => self.pins.iter().position(|p| &p.id == id),
            _ => None,
        }
    }
}
