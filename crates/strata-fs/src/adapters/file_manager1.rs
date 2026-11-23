// SPDX-License-Identifier: MIT

use crate::model::Location;

use super::{location_for_file, reveal_target_for_file};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevealRequest {
    pub directory: Location,
    pub selection: Vec<Location>,
    pub properties: bool,
}

/// Which of the interface's calls a request came by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
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

pub fn export_file_manager(
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

pub fn reveal_requests(method: Method, uris: &[String]) -> Vec<RevealRequest> {
    let mut requests: Vec<RevealRequest> = Vec::new();
    for uri in uris {
        let target = gio::File::for_uri(uri);
        // A filesystem root has no parent to reveal it in.
        let Some((directory, item)) = method
            .reveals_parent()
            .then(|| reveal_target_for_file(&target))
            .flatten()
            .map(|(directory, item)| (directory, Some(item)))
            .or_else(|| location_for_file(&target).map(|directory| (directory, None)))
        else {
            continue;
        };
        match requests
            .iter_mut()
            .find(|request| request.directory == directory)
        {
            Some(request) => request.selection.extend(item),
            None => requests.push(RevealRequest {
                directory,
                selection: item.into_iter().collect(),
                properties: method.wants_properties(),
            }),
        }
    }
    requests
}

#[cfg(test)]
mod tests;
