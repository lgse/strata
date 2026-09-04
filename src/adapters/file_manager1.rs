// SPDX-License-Identifier: GPL-3.0-or-later

//! `org.freedesktop.FileManager1`, the interface browsers and GTK/GNOME apps
//! call for "Open file location". Those callers talk to the well-known bus
//! name directly instead of going through `xdg-open` and `mimeapps.list`, so
//! without this Strata is skipped even when it owns `inode/directory`.

use gtk::{gio, glib, prelude::*};

use crate::model::Location;

use super::location_for_file;

const BUS_NAME: &str = "org.freedesktop.FileManager1";
const OBJECT_PATH: &str = "/org/freedesktop/FileManager1";
const INTERFACE_XML: &str = r#"<node>
  <interface name="org.freedesktop.FileManager1">
    <method name="ShowFolders">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
    <method name="ShowItems">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
    <method name="ShowItemProperties">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
  </interface>
</node>"#;

/// One window to open: the directory to browse, the entries to select inside
/// it, and whether the caller asked for their properties.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RevealRequest {
    pub directory: Location,
    pub selection: Vec<String>,
    pub properties: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Method {
    Folders,
    Items,
    ItemProperties,
}

impl Method {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "ShowFolders" => Some(Self::Folders),
            "ShowItems" => Some(Self::Items),
            "ShowItemProperties" => Some(Self::ItemProperties),
            _ => None,
        }
    }

    fn reveals_parent(self) -> bool {
        !matches!(self, Self::Folders)
    }

    fn wants_properties(self) -> bool {
        matches!(self, Self::ItemProperties)
    }
}

/// Claims the well-known name on `connection` and answers reveal calls with
/// `handler`, once per directory the call named.
pub(crate) fn export_file_manager(
    connection: &gio::DBusConnection,
    handler: impl Fn(RevealRequest) + 'static,
) -> Result<(), glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(INTERFACE_XML)?;
    let interface = node.lookup_interface(BUS_NAME).ok_or_else(|| {
        glib::Error::new(gio::IOErrorEnum::Failed, "missing interface definition")
    })?;
    connection
        .register_object(OBJECT_PATH, &interface)
        .method_call(move |_, _, _, _, method, parameters, invocation| {
            let Some(method) = Method::from_name(method) else {
                invocation.return_error(gio::DBusError::UnknownMethod, "unknown method");
                return;
            };
            let Some((uris, _startup_id)) = parameters.get::<(Vec<String>, String)>() else {
                invocation.return_error(gio::DBusError::InvalidArgs, "expected (as, s)");
                return;
            };
            for request in reveal_requests(method, &uris) {
                handler(request);
            }
            invocation.return_value(None);
        })
        .build()?;
    gio::bus_own_name_on_connection(
        connection,
        BUS_NAME,
        gio::BusNameOwnerFlags::NONE,
        |_, name| tracing::debug!(name, "acquired file manager bus name"),
        |_, name| tracing::debug!(name, "file manager bus name held elsewhere"),
    );
    Ok(())
}

/// Groups the named URIs into one request per directory, keeping the order
/// the caller listed them in so the first URI opens the first window.
fn reveal_requests(method: Method, uris: &[String]) -> Vec<RevealRequest> {
    let mut requests: Vec<RevealRequest> = Vec::new();
    for uri in uris {
        let target = gio::File::for_uri(uri);
        let (directory, name) = match method.reveals_parent().then(|| target.parent()).flatten() {
            Some(parent) => (parent, target.basename()),
            // A filesystem root has no parent to reveal it in.
            None => (target, None),
        };
        let Some(directory) = location_for_file(&directory) else {
            continue;
        };
        let name = name.map(|name| name.to_string_lossy().into_owned());
        match requests
            .iter_mut()
            .find(|request| request.directory == directory)
        {
            Some(request) => request.selection.extend(name),
            None => requests.push(RevealRequest {
                directory,
                selection: name.into_iter().collect(),
                properties: method.wants_properties(),
            }),
        }
    }
    requests
}

#[cfg(test)]
mod tests;
