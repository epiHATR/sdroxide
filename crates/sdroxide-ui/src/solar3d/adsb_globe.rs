//! ADS-B MAP 3D: the solar Earth's textured globe, in the panel pane.
//!
//! Same wgpu Earth (Natural Earth land / borders / rivers / cities) as the
//! solar-system window, locked on Earth with altitude-lifted aircraft. No
//! deferred viewport and no solar chrome — just the globe the MAP chip asked
//! for.

use eframe::egui;
use sdroxide_types::{AdsbAircraft, AdsbSettings};

use super::gpu;
use super::overlay;
use super::state::{AdsbMarker, AdsbTraffic, Focus, SolarUi};
use crate::egui_wgpu::RenderState;
use crate::view::{Solar3dView, solar_layer};

/// Host for the ADS-B Earth pane. Owns a dedicated [`SolarUi`] so the solar
/// window's camera and layers stay untouched.
pub struct AdsbGlobe {
    state: SolarUi,
    render_state: Option<RenderState>,
    gpu_ready: bool,
}

impl AdsbGlobe {
    pub fn new(render_state: Option<RenderState>) -> Self {
        let mut view = Solar3dView::default();
        view.focus = Focus::Earth.to_id();
        // Earth + labels (+ clouds when a feed later shares data). No planets,
        // orbits, QSO arcs or sats — this pane is an air picture, not an orrery.
        view.layers = solar_layer::LABELS | solar_layer::CLOUDS;
        view.auto = false;
        view.open = false;
        // True Earth scale — the solar view's 20× body slider would put a
        // regional camera inside the mesh and defeat f32 precision at the QTH.
        view.body_scale = 1.0;
        view.sun_scale = 1.0;
        view.moon_orbit_scale = 1.0;
        // Local ENU camera: 45° elevation, regional distance (see camera::ADSB_*).
        view.pitch = super::camera::ADSB_ELEV_DEFAULT;
        view.yaw = 0.0;
        view.dist = super::camera::ADSB_DIST_DEFAULT;
        let mut state = SolarUi::new(view);
        state.fly_home_pending = true;
        state.adsb_globe = true;
        AdsbGlobe { state, render_state, gpu_ready: false }
    }

    /// Paint the globe into the ADS-B MAP pane. Returns the ICAO clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        aircraft: &[AdsbAircraft],
        grid: &str,
        selected: Option<u32>,
        now: i64,
        cfg: AdsbSettings,
        max_h: f32,
    ) -> Option<u32> {
        let avail_w = ui.available_width();
        if avail_w < crate::adsb_map::MIN_HEIGHT {
            return None;
        }
        let h = max_h.min(avail_w).max(crate::adsb_map::MIN_HEIGHT);
        ui.set_min_height(h);

        if self.render_state.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("3D globe needs a GPU renderer")
                        .weak()
                        .size(12.0),
                );
            });
            return None;
        }
        if !self.gpu_ready {
            if let Some(rs) = &self.render_state {
                gpu::init(rs);
            }
            self.gpu_ready = true;
        }

        self.state.set_qth(grid);
        self.state.adsb = AdsbTraffic {
            aircraft: aircraft
                .iter()
                .filter(|a| !a.pos_stale(now, cfg.drop_map_s))
                .filter_map(|a| marker(a, selected))
                .collect(),
        };

        overlay::adsb_earth_pane(ui, &mut self.state)
    }
}

fn marker(a: &AdsbAircraft, selected: Option<u32>) -> Option<AdsbMarker> {
    let (lat, lon) = a.lat.zip(a.lon)?;
    let alt_ft = if a.on_ground { 0.0 } else { a.altitude_ft.unwrap_or(0) as f32 };
    let call = {
        let c = a.callsign.trim();
        (!c.is_empty()).then(|| std::sync::Arc::<str>::from(c.to_owned()))
    };
    let mut tip = a.label();
    tip.push_str(&format!("\n{} · {}", a.fmt_altitude(), a.fmt_speed()));
    if let Some(vr) = a.vertical_rate_fpm.filter(|v| v.abs() > 100) {
        tip.push_str(&format!("\n{vr:+} ft/min"));
    }
    if let Some(e) = &a.emergency {
        tip.push_str(&format!("\n{e}"));
    }
    Some(AdsbMarker {
        icao: a.icao,
        lat,
        lon,
        alt_ft,
        on_ground: a.on_ground,
        track_deg: a.track_deg,
        call,
        tip,
        selected: selected == Some(a.icao),
        emergency: a.emergency.is_some(),
    })
}
