//! Native dialog and clipboard hooks, matching the standalone app.
use super::{appearance, file_filters, window_raise};
use filmcraft_ui_egui::FilmcraftApp;

pub(super) fn install(app: &mut FilmcraftApp) {
    app.hooks.pick_files = Some(Box::new(|exts: &[&str]| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("Media"), &file_filters::extensions(exts))
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }));
    // Link Media ▸ Locate…, Attach Proxies, Reconnect Full Resolution: one path, not imported.
    app.hooks.pick_file_for_relink = Some(Box::new(|exts: &[&str], _hint| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("Media"), &file_filters::extensions(exts))
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_save = Some(Box::new(|name: &str| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("FilmCraft Project"), &file_filters::extensions(&["fcproj"]))
            .set_file_name(name)
            .save_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_save_as = Some(Box::new(|filter: &str, exts: &[&str], name: &str| {
        // a full path (Quick Export, Export mode) opens the dialog in its folder
        let path = std::path::Path::new(name);
        let mut dialog = rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t(filter), &file_filters::extensions(exts))
            .set_file_name(path.file_name().map_or(name.into(), std::ffi::OsStr::to_string_lossy));
        if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file().map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_folder = Some(Box::new(|| rfd::FileDialog::new().pick_folder().map(|p| p.to_string_lossy().to_string())));
    app.hooks.pick_folder_at = Some(Box::new(|dir: &str| rfd::FileDialog::new().set_directory(dir).pick_folder().map(|p| p.to_string_lossy().into_owned())));
    // Linux: the XDG portal call blocks until the user chooses, which would stop the frame
    // loop (GNOME offers to force-quit the window after about five seconds, #260). Show the
    // dialog on its own thread instead; the app keeps running and acts on the answer.
    #[cfg(target_os = "linux")]
    {
        app.hooks.run_dialog = Some(Box::new(run_dialog_off_thread));
    }
    app.hooks.open_path = Some(Box::new(open_path));
    // Help menu, About and Discord links (#642): eframe is built without its `links`
    // feature, so `ctx.open_url` does nothing on the desktop. Detached, so a launcher
    // that waits for the browser never freezes the UI.
    app.hooks.open_url = Some(Box::new(|url: &str| open::that_detached(url).map_err(|e| e.to_string())));
    app.hooks.clipboard_media = Some(Box::new(clipboard_media));
    // Settings ▸ General ▸ Interface Language ▸ System Language (#218).
    app.hooks.cursor_screen_position = Some(Box::new(filmcraft_platform::cursor::cursor_screen_position));
    app.hooks.system_languages = Some(Box::new(|| sys_locale::get_locales().collect()));
    // Settings ▸ Appearance ▸ Appearance Mode ▸ Sync with system on Linux desktops whose
    // compositor reports no theme to winit (no polling: see appearance.rs).
    app.hooks.system_theme = appearance::service();
    app.hooks.raise_without_focus = Some(Box::new(|| {
        window_raise::raise_without_focus();
    }));
    app.hooks.pick_open_file = Some(Box::new(|filter: &str, exts: &[&str]| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t(filter), &file_filters::extensions(exts))
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
    app.hooks.pick_open_project = Some(Box::new(|| {
        rfd::FileDialog::new()
            .add_filter(filmcraft_ui_egui::i18n::t("FilmCraft Project"), &file_filters::extensions(&["fcproj"]))
            // #1644: where the system MIME database doesn't know `.fcproj`, the project
            // filter hides every saved project; "All files" keeps them reachable.
            .add_filter(filmcraft_ui_egui::i18n::t("All files"), &file_filters::all_files())
            .pick_file()
            .map(|p| p.to_string_lossy().to_string())
    }));
}

/// Files copied in the file manager, else an image, on the system clipboard (Paste, #611).
fn clipboard_media() -> Option<filmcraft_ui_egui::ClipboardMedia> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Some(filmcraft_ui_egui::ClipboardMedia::Files(files.iter().map(|p| p.to_string_lossy().into_owned()).collect()));
    }
    let image = clipboard.get_image().ok()?;
    Some(filmcraft_ui_egui::ClipboardMedia::Image { width: image.width, height: image.height, rgba: image.bytes.into_owned() })
}

/// Open a file in its default application, or reveal it in the file manager (Edit Original,
/// Reveal Log Files).
fn open_path(path: &str, reveal: bool) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(path);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        if reveal {
            c.arg(format!("/select,{path}"));
        } else {
            c.arg(path);
        }
        c
    } else {
        // Linux file managers have no select-file flag: open the parent folder instead.
        let mut c = std::process::Command::new("xdg-open");
        let p = std::path::Path::new(path);
        c.arg(if reveal { p.parent().unwrap_or(p) } else { p });
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("can't open {path}: {e}"))
}

/// Show `dialog` with rfd on a worker thread and answer on the returned channel (#260). Filter
/// names are translated here, on the UI thread (the interface language is per thread). A dialog
/// thread that can't start or panics drops the sender: the app takes that as cancelled.
#[cfg(target_os = "linux")]
fn run_dialog_off_thread(dialog: filmcraft_ui_egui::native_dialogs::FileDialog) -> std::sync::mpsc::Receiver<Vec<String>> {
    use filmcraft_ui_egui::native_dialogs::FileDialog;
    let t = |s: &str| filmcraft_ui_egui::i18n::t(s).to_string();
    let media = t("Media");
    let project = t("FilmCraft Project");
    let all_files = t("All files");
    let all_files_exts = file_filters::all_files();
    let project_exts = project_patterns();
    let dialog = prepare_dialog(dialog);
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
        let shown = std::panic::catch_unwind(move || {
            let path = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
            match dialog {
                FileDialog::OpenFiles { exts } => {
                    rfd::FileDialog::new().add_filter(&media, &exts).pick_files().unwrap_or_default().into_iter().map(path).collect()
                }
                FileDialog::OpenFile { filter, exts } => rfd::FileDialog::new().add_filter(&filter, &exts).pick_file().map(path).into_iter().collect(),
                FileDialog::Relink { exts, .. } => rfd::FileDialog::new().add_filter(&media, &exts).pick_file().map(path).into_iter().collect(),
                FileDialog::OpenProject => rfd::FileDialog::new()
                    .add_filter(&project, &project_exts)
                    // #1644: same "All files" fallback as the synchronous picker, so a
                    // project hidden by an unregistered `.fcproj` MIME type stays openable.
                    .add_filter(&all_files, &all_files_exts)
                    .pick_file()
                    .map(path)
                    .into_iter()
                    .collect(),
                FileDialog::SaveProject { name } => {
                    rfd::FileDialog::new().add_filter(&project, &project_exts).set_file_name(name).save_file().map(path).into_iter().collect()
                }
                FileDialog::SaveAs { filter, exts, name, .. } => {
                    rfd::FileDialog::new().add_filter(&filter, &exts).set_file_name(name).save_file().map(path).into_iter().collect()
                }
                FileDialog::Folder { at } => {
                    let d = rfd::FileDialog::new();
                    let d = match at {
                        Some(dir) if !dir.is_empty() => d.set_directory(dir),
                        _ => d,
                    };
                    d.pick_folder().map(path).into_iter().collect::<Vec<_>>()
                }
            }
        });
        match shown {
            Ok(paths) => {
                let _ = tx.send(paths);
            }
            Err(_) => log::warn!("the file dialog thread panicked; taken as cancelled"),
        }
    });
    if let Err(e) = spawned {
        log::warn!("could not start the file dialog thread: {e}");
    }
    rx
}

/// A dialog as the Linux dialog thread shows it: filter names translated (here, on the UI thread:
/// the interface language is per thread) and every extension list turned into the
/// case-insensitive patterns the synchronous pickers use, so `CLIP.MP4` stays visible (#262).
#[cfg(target_os = "linux")]
fn prepare_dialog(dialog: filmcraft_ui_egui::native_dialogs::FileDialog) -> filmcraft_ui_egui::native_dialogs::FileDialog {
    use filmcraft_ui_egui::native_dialogs::FileDialog;
    let t = |s: &str| filmcraft_ui_egui::i18n::t(s).to_string();
    let patterns = |exts: Vec<String>| file_filters::extensions(&exts.iter().map(String::as_str).collect::<Vec<_>>());
    match dialog {
        FileDialog::OpenFiles { exts } => FileDialog::OpenFiles { exts: patterns(exts) },
        FileDialog::OpenFile { filter, exts } => FileDialog::OpenFile { filter: t(&filter), exts: patterns(exts) },
        FileDialog::SaveAs { filter, exts, name, adds_ext } => FileDialog::SaveAs { filter: t(&filter), exts: patterns(exts), name, adds_ext },
        FileDialog::Relink { exts, hint } => FileDialog::Relink { exts: patterns(exts), hint },
        other => other,
    }
}

/// The project dialogs' filter patterns (`EDIT.FCPROJ` included).
#[cfg(target_os = "linux")]
fn project_patterns() -> Vec<String> {
    file_filters::extensions(&["fcproj"])
}
