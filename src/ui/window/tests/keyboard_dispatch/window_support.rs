// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn recorder_app(id: &str, name: &str, mime_types: &str, output: &std::path::Path) {
    let associations = glib::user_config_dir().join("mimeapps.list");
    let mut lines: Vec<String> = std::fs::read_to_string(&associations)
        .map(|list| list.lines().map(str::to_owned).collect())
        .unwrap_or_else(|_| vec!["[Added Associations]".to_owned()]);
    for mime_type in mime_types.split(';').filter(|value| !value.is_empty()) {
        let key = format!("{mime_type}=");
        match lines.iter_mut().find(|line| line.starts_with(&key)) {
            Some(line) => line.push_str(&format!("{id}.desktop;")),
            None => lines.push(format!("{key}{id}.desktop;")),
        }
    }
    std::fs::create_dir_all(glib::user_config_dir()).expect("config");
    std::fs::write(&associations, lines.join("\n") + "\n").expect("associations");
    let applications = glib::user_data_dir().join("applications");
    std::fs::create_dir_all(&applications).expect("applications");
    let script = applications.join(format!("{id}.sh"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            output.display()
        ),
    )
    .expect("recorder");
    std::fs::set_permissions(
        &script,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .expect("executable");
    std::fs::write(
        applications.join(format!("{id}.desktop")),
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={} %F\nMimeType={mime_types}\n",
            script.display()
        ),
    )
    .expect("desktop file");
}

/// Avoid host launches and asynchronous Open With fallback dialogs in key-routing tests.
pub(super) fn default_recorder_app(mime_type: &str, output: &std::path::Path) {
    let id = "strata-default-recorder";
    recorder_app(id, "Default Recorder", &format!("{mime_type};"), output);
    let associations = glib::user_config_dir().join("mimeapps.list");
    let mut list = std::fs::read_to_string(&associations).expect("associations");
    list.push_str(&format!(
        "[Default Applications]\n{mime_type}={id}.desktop\n"
    ));
    std::fs::write(&associations, list).expect("default application");
}

pub(super) fn child_commands() -> Vec<String> {
    let output = std::process::Command::new("ps")
        .args(["--ppid", &std::process::id().to_string(), "-o", "comm="])
        .output()
        .expect("ps lists child processes");
    let mut commands = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|command| !command.is_empty() && *command != "ps")
        .map(str::to_owned)
        .collect::<Vec<_>>();
    commands.sort();
    commands
}

pub(super) fn composed_window() -> (
    gtk::ApplicationWindow,
    super::super::super::composition::WindowContent,
) {
    let preferences = PreferenceManager::shared();
    let application = gtk::gio::Application::default()
        .and_downcast::<gtk::Application>()
        .unwrap_or_else(|| {
            let application =
                gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("test application registration");
            application
        });
    let window = gtk::ApplicationWindow::builder()
        .application(&application)
        .default_width(1200)
        .default_height(760)
        .build();
    let content = super::super::super::composition::WindowContent::new(&window, &preferences);
    content.bind(&window, &preferences);
    window.present();
    (window, content)
}

pub(super) fn press_phase(
    window: &gtk::ApplicationWindow,
    phase: gtk::PropagationPhase,
    key: Key,
    modifiers: ModifierType,
) -> bool {
    let controllers = window.observe_controllers();
    let keys: Vec<_> = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .filter(|keys| keys.propagation_phase() == phase)
        .collect();
    assert!(!keys.is_empty(), "{phase:?} key controller");
    keys.iter()
        .any(|keys| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]))
}

pub(super) fn settings_layer(
    content: &super::super::super::composition::WindowContent,
) -> Option<gtk::Widget> {
    children(content.overlay().upcast_ref())
        .find(|widget| widget.has_css_class("settings-backdrop"))
}
