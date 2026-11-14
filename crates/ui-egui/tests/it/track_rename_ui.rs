//! Double-clicking a track header's name renames the track in place (Enter commits).

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn demo() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(r) = reply.try_recv() {
                assert_eq!(r["ok"], true, "{method} {params}: {r}");
                self.frames(2);
                return r["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }
}

#[test]
fn double_clicking_the_track_name_renames_it() {
    let mut d = Driver::demo();
    d.ok("ui.click", json!({"id": "timeline.track.V1.name", "count": 2}));
    d.frames(3);
    assert!(d.harness.state().ui.timeline.rename_track.is_some(), "the double-click opens the rename field");
    d.ok("ui.key", json!({"key": "Ctrl+A"}));
    d.ok("ui.type", json!({"text": "Titles"}));
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    assert!(d.harness.state().ui.timeline.rename_track.is_none());
    assert_eq!(d.harness.state().session.active_sequence().unwrap().video_tracks[0].name, "Titles");
}
