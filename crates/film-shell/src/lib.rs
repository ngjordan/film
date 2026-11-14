//! Film's reusable desktop shell. The host owns logging, updates, graphics setup and TCP listeners.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use chrono::TimeZone;
use filmcraft_engine::{Session, autosave::AutosaveConfig};
use filmcraft_ui_egui::{FilmcraftApp, theme};
use serde_json::json;
use std::path::{Path, PathBuf};

// Reuse the standalone platform implementations, not another copy of device/menu code.
#[path = "../../../apps/filmcraft/src/appearance.rs"]
mod appearance;
#[path = "../../../apps/filmcraft/src/audio.rs"]
mod audio;
#[path = "../../../apps/filmcraft/src/audio_in.rs"]
mod audio_in;
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[path = "../../../apps/filmcraft/src/file_filters.rs"]
mod file_filters;
#[cfg(target_os = "macos")]
#[path = "../../../apps/filmcraft/src/native_color.rs"]
mod native_color;
#[cfg(target_os = "macos")]
#[path = "../../../apps/filmcraft/src/native_menu.rs"]
mod native_menu;
#[path = "../../../apps/filmcraft/src/window_raise.rs"]
mod window_raise;

#[derive(Default)]
pub struct Launch {
    pub files: Vec<String>,
    /// Separate profile/recovery directory, or the app's existing platform default.
    pub data_dir: Option<PathBuf>,
    pub demo: bool,
    /// An authenticated host may supply a queue; this module never binds a socket.
    pub control: Option<std::sync::mpsc::Receiver<filmcraft_ui_egui::ControlRequest>>,
}

pub struct Shell {
    app: FilmcraftApp,
    ctx: egui::Context,
    fonts: egui::FontDefinitions,
    zoom: f32,
}

impl Shell {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let mut shell = Self::new_for_host(&cc.egui_ctx, cc.storage, launch);
        if let Some(rs) = cc.wgpu_render_state.clone() {
            shell.app.set_wgpu(rs);
        }
        shell
    }

    /// Settings use the profile, not eframe storage. No process-wide services start here.
    pub fn new_for_host(ctx: &egui::Context, _storage: Option<&dyn eframe::Storage>, launch: Launch) -> Self {
        filmcraft_ui_egui::install_ime_reset_callbacks(ctx);
        let mut session = Session::default();
        if let Some(dir) = launch.data_dir.or_else(filmcraft_engine::autosave::default_data_dir) {
            let mut cfg = AutosaveConfig::new(dir);
            cfg.local_offset = |unix| chrono::Local.timestamp_opt(unix, 0).single().map(|d| d.offset().local_minus_utc()).unwrap_or(0);
            if let Err(e) = session.start_autosave(cfg) {
                log::warn!("auto-save and crash recovery unavailable: {e}");
            }
        }
        if launch.demo
            && let Err(e) = session.execute("file.openDemoProject", json!({}))
        {
            log::warn!("{e}");
        }
        session.voiceover.input = Some(Box::new(audio_in::CpalIn::new(&session.prefs.audio_hardware.device_class)));
        let mut app = FilmcraftApp::new(session);
        // The desktop-only host services: the system clipboard and the
        // filesystem. A browser tab has neither.
        #[cfg(not(target_arch = "wasm32"))]
        desktop::install(&mut app);
        app.audio = Some(Box::new(audio::CpalOut::new()));
        let rate = app.session.active_sequence().map(|q| q.settings.sample_rate);
        app.scrub_out = Some(Box::new(audio::CpalScrubOut::new(&app.session.prefs.audio_hardware, rate)));
        if let Some(rx) = launch.control {
            app = app.with_control(rx);
        }
        // Install before logic marks the lazy theme initialized. Never read ctx.fonts here.
        app.install_look(ctx);
        app.apply_prefs(ctx);
        let fonts = theme::font_definitions();
        let native = ctx.native_pixels_per_point().filter(|n| n.is_finite() && *n > 0.0).unwrap_or(1.0);
        let zoom = app.session.prefs.appearance.ui_scale_factor().map_or(1.0, |scale| scale / native);
        ctx.set_zoom_factor(zoom);
        let mut shell = Self { app, ctx: ctx.clone(), fonts, zoom };
        for path in launch.files {
            shell.open_path(&path);
        }
        shell
    }

    pub fn logic(&mut self, ctx: &egui::Context) {
        self.app.logic_for_host(ctx);
    }

    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        eframe::App::raw_input_hook(&mut self.app, ctx, raw);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // A module constructed inside a running pass gets its font replacement next pass.
        if !ui.ctx().fonts(|f| f.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
            ui.ctx().request_repaint();
            return;
        }
        self.app.ui_for_host(ui);
        self.zoom = ui.ctx().zoom_factor();
    }

    pub fn backdrop(&self) -> [f32; 4] {
        self.app.tokens.panel_bg.to_normalized_gamma_f32()
    }

    pub fn open_path(&mut self, path: &str) {
        let project = Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("fcproj"));
        if project && filmcraft_ui_egui::panels::clip_dialogs::ask_before_open(&mut self.app, path).is_some() {
            return;
        }
        let result = if project {
            self.app.session.execute("file.open", json!({"path": path}))
        } else {
            self.app.session.execute("file.import", json!({"paths": [path]}))
        };
        match result {
            Err(e) => self.app.status(e.to_string()),
            Ok(value) => {
                if let Some(errors) = value.get("errors").and_then(serde_json::Value::as_array) {
                    self.app.status(errors.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join("; "));
                }
            }
        }
        self.ctx.request_repaint();
    }

    /// Flush recovery without invoking the standalone updater or terminating the process.
    pub fn save_state(&mut self) {
        self.app.save_state_for_host();
    }

    pub fn can_close(&self) -> bool {
        self.app.can_close_for_host()
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn fonts(&self) -> egui::FontDefinitions {
        self.fonts.clone()
    }

    pub fn install_native_menu(&mut self) {
        #[cfg(target_os = "macos")]
        {
            let (rx, update, text_menu) = native_menu::install(&self.app, self.ctx.clone());
            self.app.hooks.pick_color = Some(native_color::picker(text_menu));
            self.app.command_inbox = Some(rx);
            self.app.hooks.menu_changed = Some(update);
            self.app.ui.show_menu_bar = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_construction_waits_for_eguis_pending_font_install() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut shell = None;
        let mut first = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(!ui.ctx().fonts(|f| f.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))));
            let module = shell.insert(Shell::new_for_host(ui.ctx(), None, Launch { data_dir: Some(dir.path().into()), ..Default::default() }));
            module.logic(ui.ctx());
            module.ui(ui);
        });
        first.textures_delta.clear();
        let mut second = ctx.run_ui(egui::RawInput::default(), |ui| {
            let module = shell.as_mut().unwrap();
            assert!(ui.ctx().fonts(|f| f.definitions().font_data.contains_key("inter")));
            module.logic(ui.ctx());
            module.ui(ui);
        });
        assert!(!second.shapes.is_empty());
        second.textures_delta.clear();
        shell.as_mut().unwrap().save_state();
    }

    #[test]
    fn hosted_first_frame_has_film_fonts_and_dirty_open_is_guarded() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut shell = Shell::new_for_host(&ctx, None, Launch { data_dir: Some(dir.path().into()), ..Default::default() });
        assert!(shell.fonts().font_data.contains_key("inter"));
        assert!(shell.fonts().families.contains_key(&egui::FontFamily::Name("semibold".into())));
        assert!(shell.can_close());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            shell.logic(ui.ctx());
            shell.ui(ui);
        });
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
        assert!(ctx.fonts(|f| f.definitions().font_data.contains_key("inter")));
        shell.app.session.execute("file.newProject", json!({"name": "Changed"})).unwrap();
        shell.app.session.execute("file.newBin", json!({"name": "Edit"})).unwrap();
        assert!(!shell.can_close());
        shell.open_path("another.fcproj");
        assert_eq!(shell.app.ui.clip_dialog.as_ref().unwrap().command, "file.open");
        assert_eq!(shell.app.session.project.name, "Changed");
        shell.save_state();
    }
}
