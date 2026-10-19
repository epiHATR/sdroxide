//! ADS-B MAP as a second OS window (native) or a second browser tab (wasm).
//!
//! Same bargain as [`crate::solar3d`]: natively a deferred egui viewport that
//! desktop eframe turns into a real window; in the browser a standalone
//! [`AdsbMapApp`] opened at `?view=adsb`.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::{Arc, Mutex, MutexGuard};

    use eframe::egui::{self, RichText};
    use sdroxide_types::{AdsbAircraft, AdsbSettings};

    use crate::adsb_map::{self, AdsbMapKind, AdsbMapState};
    use crate::egui_wgpu::RenderState;
    use crate::solar3d::state::{AdsbTraffic, Focus, SolarUi};
    use crate::theme;
    use crate::view::{Solar3dView, solar_layer};

    fn viewport_id(salt: u32) -> egui::ViewportId {
        if salt == 0 {
            egui::ViewportId::from_hash_of("sdroxide-adsb-map")
        } else {
            egui::ViewportId::from_hash_of(("sdroxide-adsb-map", salt))
        }
    }

    const FALLBACK: egui::Vec2 = egui::vec2(1280.0, 900.0);

    fn fill_monitor(monitor: egui::Vec2) -> egui::Vec2 {
        let chrome = egui::vec2(16.0, 40.0);
        let floor = egui::vec2(480.0, 320.0);
        let want = (monitor - chrome - egui::vec2(0.0, 48.0)).max(floor);
        crate::layout::fit_inner_size(monitor, want + chrome, want, floor).unwrap_or(want)
    }

    fn window_geometry(
        monitor: Option<egui::Vec2>,
        seed: Option<sdroxide_types::Solar3dWindow>,
    ) -> (egui::Vec2, Option<egui::Pos2>) {
        let Some(monitor) = monitor.filter(|m| m.x > 1.0 && m.y > 1.0) else {
            let want = seed.map_or(FALLBACK, |s| egui::Vec2::from(s.size));
            return (want, seed.and_then(|s| s.pos).map(egui::Pos2::from));
        };
        let want = seed.map_or_else(|| fill_monitor(monitor), |s| egui::Vec2::from(s.size));
        let chrome = egui::vec2(16.0, 40.0);
        let size = crate::layout::fit_inner_size(
            monitor,
            want + chrome,
            want,
            egui::vec2(480.0, 320.0),
        )
        .unwrap_or(want);
        (size, seed.and_then(|s| s.pos).map(egui::Pos2::from))
    }

    fn new_globe_ui() -> SolarUi {
        let mut view = Solar3dView::default();
        view.focus = Focus::Earth.to_id();
        view.layers = solar_layer::LABELS | solar_layer::CLOUDS;
        view.auto = false;
        view.open = false;
        view.body_scale = 1.0;
        view.sun_scale = 1.0;
        view.moon_orbit_scale = 1.0;
        view.pitch = crate::solar3d::camera::ADSB_ELEV_DEFAULT;
        view.yaw = 0.0;
        view.dist = crate::solar3d::camera::ADSB_DIST_DEFAULT;
        let mut state = SolarUi::new(view);
        state.fly_home_pending = true;
        state.adsb_globe = true;
        state
    }

    struct Inner {
        map: AdsbMapState,
        globe: SolarUi,
        aircraft: Vec<AdsbAircraft>,
        grid: String,
        home: Option<(f64, f64)>,
        cfg: AdsbSettings,
        now: i64,
        close_requested: bool,
        window_seed: Option<sdroxide_types::Solar3dWindow>,
        window_now: Option<sdroxide_types::Solar3dWindow>,
    }

    /// Native host for the ADS-B MAP second window.
    pub struct AdsbMapWindow {
        pub open: bool,
        state: Arc<Mutex<Inner>>,
        render_state: Option<RenderState>,
        gpu_ready: bool,
    }

    impl AdsbMapWindow {
        pub fn new(render_state: Option<RenderState>) -> Self {
            Self {
                open: false,
                state: Arc::new(Mutex::new(Inner {
                    map: AdsbMapState::default(),
                    globe: new_globe_ui(),
                    aircraft: Vec::new(),
                    grid: String::new(),
                    home: None,
                    cfg: AdsbSettings::default(),
                    now: 0,
                    close_requested: false,
                    window_seed: None,
                    window_now: None,
                })),
                render_state,
                gpu_ready: false,
            }
        }

        fn lock(&self) -> MutexGuard<'_, Inner> {
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }

        pub fn set_window_seed(&self, window: Option<sdroxide_types::Solar3dWindow>) {
            self.lock().window_seed = window;
        }

        pub fn window_now(&self) -> Option<sdroxide_types::Solar3dWindow> {
            self.lock().window_now
        }

        /// Match the in-pane map mode when the window is (re)opened.
        pub fn sync_kind(&self, kind: AdsbMapKind) {
            self.lock().map.kind = kind;
        }

        /// Publish live traffic and emit the deferred viewport while `open`.
        ///
        /// Returns `Some(selected)` so the panel can stay in lock-step with
        /// clicks inside the window. `None` while the window is closed.
        pub fn viewport(
            &mut self,
            ctx: &egui::Context,
            aircraft: &[AdsbAircraft],
            grid: &str,
            home: Option<(f64, f64)>,
            cfg: AdsbSettings,
            now: i64,
            selected: Option<u32>,
        ) -> Option<Option<u32>> {
            if !self.open {
                return None;
            }

            if !self.gpu_ready {
                if let Some(rs) = &self.render_state {
                    crate::solar3d::gpu::init(rs);
                }
                self.gpu_ready = true;
            }

            {
                let mut st = self.lock();
                st.aircraft = aircraft.to_vec();
                st.grid = grid.to_owned();
                st.home = home;
                st.cfg = cfg;
                st.now = now;
                st.map.selected = selected;
            }

            let state = Arc::clone(&self.state);
            let vid = viewport_id(crate::layout::radio_salt(ctx));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("sdroxide — ADS-B map")
                .with_min_inner_size([480.0, 320.0])
                .with_clamp_size_to_monitor_size(true);
            if ctx.cumulative_pass_nr_for(vid) == 0 {
                let seed = self.lock().window_seed;
                let monitor = ctx.input(|i| i.viewport().monitor_size);
                let (size, pos) = window_geometry(monitor, seed);
                let maximized = seed.is_none_or(|s| s.maximized);
                if maximized {
                    builder = builder.with_maximized(true);
                } else {
                    builder = builder.with_inner_size(size);
                    if let Some(pos) = pos {
                        builder = builder.with_position(pos);
                    }
                }
            }
            ctx.show_viewport_deferred(vid, builder, move |ui, _class| {
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                let size = ui.ctx().content_rect().size();
                if size.x > 1.0 && size.y > 1.0 {
                    let (pos, maximized) = ui.ctx().input(|i| {
                        let v = i.viewport();
                        (v.outer_rect.map(|r| r.min), v.maximized.unwrap_or(false))
                    });
                    let geom = sdroxide_types::Solar3dWindow {
                        size: [size.x.round(), size.y.round()],
                        pos: pos.map(|p| [p.x.round(), p.y.round()]),
                        maximized,
                    };
                    if st.window_now != Some(geom) {
                        st.window_now = Some(geom);
                    }
                }
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    st.close_requested = true;
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    return;
                }
                paint_window(ui, &mut st);
            });

            let (close, selected) = {
                let st = self.lock();
                (st.close_requested, st.map.selected)
            };
            if close {
                self.lock().close_requested = false;
                self.open = false;
                return None;
            }
            Some(selected)
        }
    }

    fn paint_window(ui: &mut egui::Ui, st: &mut Inner) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                RichText::new("ADS-B MAP")
                    .strong()
                    .size(12.0)
                    .color(theme::TEXT_STRONG()),
            );
            let flat = st.map.kind == AdsbMapKind::Flat;
            if crate::chrome::chip(ui, flat, RichText::new("2D").size(10.0))
                .on_hover_text("Top-down radar picture.")
                .clicked()
            {
                st.map.kind = AdsbMapKind::Flat;
            }
            if crate::chrome::chip(ui, !flat, RichText::new("3D").size(10.0))
                .on_hover_text("Earth globe focused on your QTH.")
                .clicked()
            {
                st.map.kind = AdsbMapKind::Air3d;
            }
        });
        ui.add_space(4.0);

        let h = ui.available_height().max(crate::adsb_map::MIN_HEIGHT);
        let clicked = match st.map.kind {
            AdsbMapKind::Flat => {
                adsb_map::show(ui, &mut st.map, &st.aircraft, st.home, st.now, st.cfg, h);
                None
            }
            AdsbMapKind::Air3d => {
                st.globe.set_qth(&st.grid);
                st.globe.adsb = AdsbTraffic {
                    aircraft: st
                        .aircraft
                        .iter()
                        .filter(|a| !a.pos_stale(st.now, st.cfg.drop_map_s))
                        .filter_map(|a| crate::solar3d::adsb_marker(a, st.map.selected))
                        .collect(),
                };
                crate::solar3d::overlay::adsb_earth_pane(ui, &mut st.globe)
            }
        };
        if let Some(icao) = clicked {
            st.map.selected = (st.map.selected != Some(icao)).then_some(icao);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::AdsbMapWindow;

/// Browser-tab ADS-B map (`?view=adsb`), fed by a control WebSocket.
#[cfg(feature = "remote")]
pub mod app {
    use eframe::egui;
    use sdroxide_types::{AdsbStatus, RadioController, RadioEvent, RadioState};

    use crate::adsb_map::{AdsbMapKind, AdsbMapState};
    use crate::remote::RemoteController;
    use crate::solar3d::AdsbGlobe;
    use crate::theme;

    pub struct AdsbMapApp {
        ctrl: RemoteController,
        url: String,
        map: AdsbMapState,
        globe: AdsbGlobe,
        status: Option<AdsbStatus>,
        radio: RadioState,
        grid: String,
        login: crate::login::LoginForm,
    }

    impl AdsbMapApp {
        /// `url` is the radio control `/ws` (or `/ws/<id>`) endpoint.
        pub fn new(cc: &eframe::CreationContext<'_>, url: &str) -> Result<Self, String> {
            let ui = crate::app::persist::load_ui_settings(cc.storage);
            crate::theme::set_look(ui.theme, ui.button_style, ui.window_style, ui.box_style);
            crate::theme::set_font_sizes(
                ui.skimmer_font_size,
                ui.waterfall_font_size,
                ui.menu_font_size,
            );
            crate::theme::set_map_cities(ui.map_cities);
            crate::theme::apply(&cc.egui_ctx);
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                crate::solar3d::gpu::init(rs);
            }

            let ctx = cc.egui_ctx.clone();
            let ctrl = RemoteController::connect(url, None, move || {
                crate::repaint::after_ms(&ctx, 33)
            })?;

            Ok(AdsbMapApp {
                ctrl,
                url: url.to_owned(),
                map: AdsbMapState::default(),
                globe: AdsbGlobe::new(cc.wgpu_render_state.clone()),
                status: None,
                radio: RadioState::default(),
                grid: String::new(),
                login: Default::default(),
            })
        }

        fn home(&self) -> Option<(f64, f64)> {
            let g = self.grid.trim();
            (!g.is_empty())
                .then(|| sdroxide_types::grid_to_latlon(g))
                .flatten()
        }
    }

    impl eframe::App for AdsbMapApp {
        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            while let Some(ev) = self.ctrl.poll_event() {
                match ev {
                    RadioEvent::State(s) => self.radio = s,
                    RadioEvent::AdsbStatus(s) => self.status = Some(*s),
                    RadioEvent::Ft8Status(s) => {
                        self.grid = s.config.my_grid.clone();
                    }
                    _ => {}
                }
            }

            let phase = self.ctrl.auth_phase();
            let station = crate::login::station_key(&self.url);
            self.login.settle(&phase, &station);
            if phase.is_pending() {
                let rs = frame.wgpu_render_state();
                if let Some(login) = crate::login::screen(ui, &mut self.login, &phase, rs) {
                    self.ctrl.send_auth(login.username, login.password);
                }
                crate::repaint::after_ms(ui.ctx(), 250);
                return;
            }

            ui.painter().rect_filled(ui.max_rect(), 0.0, theme::scope().ground);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(
                    egui::RichText::new("ADS-B MAP")
                        .strong()
                        .size(12.0)
                        .color(theme::TEXT_STRONG()),
                );
                let flat = self.map.kind == AdsbMapKind::Flat;
                if crate::chrome::chip(ui, flat, egui::RichText::new("2D").size(10.0)).clicked() {
                    self.map.kind = AdsbMapKind::Flat;
                }
                if crate::chrome::chip(ui, !flat, egui::RichText::new("3D").size(10.0)).clicked() {
                    self.map.kind = AdsbMapKind::Air3d;
                }
            });
            ui.add_space(4.0);

            let Some(st) = self.status.as_ref() else {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("waiting for ADS-B…").weak().size(13.0));
                });
                crate::repaint::after_ms(ui.ctx(), 33);
                return;
            };
            let cfg = self.radio.adsb;
            let now = crate::time::now_unix();
            let home = self.home();
            let h = ui.available_height().max(crate::adsb_map::MIN_HEIGHT);
            match self.map.kind {
                AdsbMapKind::Flat => {
                    crate::adsb_map::show(ui, &mut self.map, &st.aircraft, home, now, cfg, h);
                }
                AdsbMapKind::Air3d => {
                    let selected = self.map.selected;
                    let clicked =
                        self.globe.show(ui, &st.aircraft, &self.grid, selected, now, cfg, h);
                    if let Some(icao) = clicked {
                        self.map.selected = (selected != Some(icao)).then_some(icao);
                    }
                }
            }
            crate::repaint::after_ms(ui.ctx(), 33);
        }
    }
}

#[cfg(feature = "remote")]
pub use app::AdsbMapApp;
