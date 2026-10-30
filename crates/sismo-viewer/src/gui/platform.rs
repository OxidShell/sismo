//! Desktop services through istmo plugins: file dialogs, revealing
//! folders and the system language.

use std::path::{Path, PathBuf};

use istmo::{Runtime, RuntimeConfig, RuntimeInit};
use istmo_file_picker::{
    DesktopFilePicker, FileFilter, FilePickerClient, FilePickerHost, PickConfig, SaveConfig,
};
use istmo_opener::{DesktopOpener, OpenerClient, OpenerHost};

/// Starts the istmo runtime with the desktop hosts the viewer uses.
/// Call once, before the window launches.
pub fn install() {
    let RuntimeInit { runtime, outbound } = match Runtime::init(RuntimeConfig::inline()) {
        Ok(init) => init,
        Err(error) => {
            eprintln!("istmo runtime unavailable: {error}");
            return;
        }
    };
    let picker = DesktopFilePicker::new();
    picker.install_release_hook(&runtime);
    runtime.register_host(FilePickerHost::new(picker));
    runtime.register_host(OpenerHost::new(DesktopOpener));
    if let Ok(mut info) = istmo_plugins::system_info::SystemInfoPublisher::install(&runtime) {
        info.tick();
    }
    // Local hosts short-circuit; drain whatever else so dispatch never stalls.
    std::thread::spawn(move || while outbound.recv().is_ok() {});
}

fn sismo_filter() -> FileFilter {
    FileFilter {
        mime_types: vec!["application/octet-stream".into()],
        extensions: vec!["sismo".into()],
        uti_types: Vec::new(),
    }
}

/// Asks for a `.sismo` recording; `None` when cancelled or unavailable.
pub async fn pick_recording(title: String) -> Option<PathBuf> {
    let picker = FilePickerClient::acquire().ok()?;
    let config = PickConfig {
        dialog_title: Some(title),
        filter: sismo_filter(),
        start_directory_hint: None,
        allow_multiple: false,
    };
    let file = picker.pick_file_owned(config).await.ok()??;
    let path = picker.path(file.handle.id()).await.ok()??;
    Some(PathBuf::from(path))
}

/// Asks where to save; `None` when cancelled or unavailable.
pub async fn save_path(title: String, suggested: String, extension: &str) -> Option<PathBuf> {
    let picker = FilePickerClient::acquire().ok()?;
    let config = SaveConfig {
        dialog_title: Some(title),
        suggested_name: Some(suggested),
        mime_type: None,
        filter: FileFilter {
            mime_types: Vec::new(),
            extensions: vec![extension.to_owned()],
            uti_types: Vec::new(),
        },
        start_directory_hint: None,
        overwrite_prompt: true,
    };
    let file = picker.save_file_owned(config).await.ok()??;
    let path = picker.path(file.handle.id()).await.ok()??;
    Some(PathBuf::from(path))
}

/// Opens `dir` in the system file manager.
pub fn reveal(dir: &Path) {
    let Ok(client) = OpenerClient::acquire() else {
        return;
    };
    let url = format!("file://{}", dir.display());
    freya::prelude::spawn_in_window(async move {
        let _ = client.open(url).await;
    });
}

/// Primary language subtag of the desktop locale (`es`, `en`…).
#[must_use]
pub fn system_language() -> Option<String> {
    istmo_plugins::SystemInfo::acquire()
        .ok()?
        .current_locale()
        .ok()
        .flatten()
        .map(|locale| locale.language)
}
