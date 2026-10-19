//! Snap System-strip overlays into a right column beside the waterfall.
//!
//! Mirrors the band/mode dock: a [`egui::Panel::right`] reserves layout space
//! early in the frame; each matching `*_window` later paints into its rect in
//! [`SdroxideApp::system_dock_body_rects`] instead of a floating centred window.
//! Several overlays may be pinned at once — they stack in the column.

use std::collections::HashMap;

use eframe::egui::{self, RichText};

use super::{SdroxideApp, SystemWasOpen};

/// Payload while dragging a docked header to reorder the column.
#[derive(Clone, Copy)]
struct DraggedSystemDock(SystemDock);

#[cfg(test)]
mod persist_id_tests {
    use super::SystemDock;
    use crate::app::persist::SystemDockSnapshot;

    #[test]
    fn every_dock_kind_round_trips_through_its_persist_id() {
        for kind in SystemDock::ALL {
            assert_eq!(SystemDock::from_persist_id(kind.persist_id()), Some(kind));
        }
        assert_eq!(SystemDock::from_persist_id("nope"), None);
    }

    #[test]
    fn dock_snapshot_keeps_floating_open_set() {
        let snap = SystemDockSnapshot {
            order: vec!["logbook".into()],
            floating: vec!["audio".into()],
            ..SystemDockSnapshot::default()
        };
        let json = serde_json::to_string(&snap).expect("serialize");
        let back: SystemDockSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.order, vec!["logbook"]);
        assert_eq!(back.floating, vec!["audio"]);
        // Older layouts without `floating` still load.
        let old = r#"{"order":["audio"],"fraction":0.3}"#;
        let legacy: SystemDockSnapshot = serde_json::from_str(old).expect("legacy");
        assert!(legacy.floating.is_empty());
        assert_eq!(legacy.order, vec!["audio"]);
    }
}

/// Which System-strip overlay is snapped into the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::app) enum SystemDock {
    Logbook,
    Spots,
    Awards,
    Bands,
    Sat,
    Ism,
    /// Speakers / mic / WAV record — replaced the Public SDR strip chip.
    Audio,
    Mail,
    Memories,
    Scanner,
    Grid,
    Settings,
}

impl SystemDock {
    /// Every System-strip overlay that can float or pin into the dock column.
    pub(in crate::app) const ALL: [Self; 12] = [
        Self::Logbook,
        Self::Spots,
        Self::Awards,
        Self::Bands,
        Self::Sat,
        Self::Ism,
        Self::Audio,
        Self::Mail,
        Self::Memories,
        Self::Scanner,
        Self::Grid,
        Self::Settings,
    ];

    pub(in crate::app) fn title(self) -> &'static str {
        match self {
            Self::Logbook => "LOGBOOK",
            Self::Spots => "SPOTS",
            Self::Awards => "AWARDS",
            Self::Bands => "BANDS",
            Self::Sat => "SATELLITE",
            Self::Ism => "ISM DEVICES",
            Self::Audio => "AUDIO",
            Self::Mail => "MAIL",
            Self::Memories => "MEMORIES",
            Self::Scanner => "SCANNER",
            Self::Grid => "GRID TRACKER",
            Self::Settings => "SETTINGS",
        }
    }

    /// Salt passed to [`crate::layout::salted_id`] for the floating `Window`.
    /// Must stay identical to each `*_window` call site.
    fn window_salt(self) -> &'static str {
        match self {
            Self::Logbook => "LOGBOOK",
            Self::Spots => "SPOTS",
            Self::Awards => "AWARDS",
            Self::Bands => "BANDS",
            Self::Sat => "SATELLITE",
            Self::Ism => "Ism",
            Self::Audio => "AUDIO",
            Self::Mail => "MAIL",
            Self::Memories => "Memories",
            Self::Scanner => "Scanner",
            Self::Grid => "GRID TRACKER",
            Self::Settings => "settings-window-v2",
        }
    }

    /// Stable id for the dock snapshot on disk — not the title string, so a
    /// renamed overlay still restores.
    pub(in crate::app) fn persist_id(self) -> &'static str {
        match self {
            Self::Logbook => "logbook",
            Self::Spots => "spots",
            Self::Awards => "awards",
            Self::Bands => "bands",
            Self::Sat => "sat",
            Self::Ism => "ism",
            Self::Audio => "audio",
            Self::Mail => "mail",
            Self::Memories => "memories",
            Self::Scanner => "scanner",
            Self::Grid => "grid",
            Self::Settings => "settings",
        }
    }

    pub(in crate::app) fn from_persist_id(id: &str) -> Option<Self> {
        Some(match id {
            "logbook" => Self::Logbook,
            "spots" => Self::Spots,
            "awards" => Self::Awards,
            "bands" => Self::Bands,
            "sat" => Self::Sat,
            "ism" => Self::Ism,
            // Old Public SDR dock snapshots reopen as Audio.
            "audio" | "public_sdrs" => Self::Audio,
            "mail" => Self::Mail,
            "memories" => Self::Memories,
            "scanner" => Self::Scanner,
            "grid" => Self::Grid,
            "settings" => Self::Settings,
            _ => return None,
        })
    }

    fn open_flag(self, app: &SdroxideApp) -> bool {
        match self {
            Self::Logbook => app.show_logbook,
            Self::Spots => app.show_spots,
            Self::Awards => app.show_awards,
            Self::Bands => app.show_bands,
            Self::Sat => app.show_sat,
            Self::Ism => app.show_ism,
            Self::Audio => app.show_audio,
            Self::Mail => app.mail.open,
            Self::Memories => app.show_memories,
            Self::Scanner => app.show_scanner,
            Self::Grid => app.show_grid,
            Self::Settings => app.show_settings,
        }
    }

    fn set_open(self, app: &mut SdroxideApp, open: bool) {
        match self {
            Self::Logbook => app.show_logbook = open,
            Self::Spots => app.show_spots = open,
            Self::Awards => app.show_awards = open,
            Self::Bands => app.show_bands = open,
            Self::Sat => app.show_sat = open,
            Self::Ism => app.show_ism = open,
            Self::Audio => app.show_audio = open,
            Self::Mail => app.mail.open = open,
            Self::Memories => app.show_memories = open,
            Self::Scanner => app.show_scanner = open,
            Self::Grid => app.show_grid = open,
            Self::Settings => app.show_settings = open,
        }
    }

    fn center_pending_mut(self, app: &mut SdroxideApp) -> &mut bool {
        let p: &mut SystemWasOpen = match self {
            Self::Mail => {
                return &mut app.mail.center_pending;
            }
            _ => &mut app.system_center_pending,
        };
        match self {
            Self::Logbook => &mut p.logbook,
            Self::Spots => &mut p.spots,
            Self::Awards => &mut p.awards,
            Self::Bands => &mut p.bands,
            Self::Sat => &mut p.sat,
            Self::Ism => &mut p.ism,
            Self::Audio => &mut p.audio,
            Self::Memories => &mut p.memories,
            Self::Scanner => &mut p.scanner,
            Self::Grid => &mut p.grid,
            Self::Settings => &mut p.settings,
            Self::Mail => unreachable!("handled above"),
        }
    }
}

/// Narrowest dock column that still fits typical System overlays.
const SYSTEM_DOCK_MIN_W: f32 = 220.0;
/// Widest share of the main content width the System dock may take when dragged.
/// The panadapter / operating column always keeps the other half.
const SYSTEM_DOCK_MAX_FRACTION: f32 = 0.5;
/// Width of [`crate::chrome::split_handle`] on the dock's inner edge.
/// Wider than the digi / JS8 7 pt strips so the line and dashes sit clearly in
/// the middle of the gutter instead of hugging the main / dock edges.
const SYSTEM_DOCK_HANDLE_W: f32 = 14.0;
/// Header-only height for a collapsed docked overlay (triangle + title + chips).
const SYSTEM_DOCK_COLLAPSED_H: f32 = 24.0;
/// Horizontal [`crate::chrome::split_handle`] under each expanded docked body.
const SYSTEM_DOCK_ROW_HANDLE_H: f32 = 7.0;
/// Thin rule between the header and the body inside an expanded slot.
const SYSTEM_DOCK_SEP_H: f32 = 6.0;
/// Expanded slot floor: three header heights (header + body + grip).
const SYSTEM_DOCK_SLOT_MIN_H: f32 = SYSTEM_DOCK_COLLAPSED_H * 3.0;
/// With two or more *expanded* overlays, no single slot may take more than
/// this fraction of the dock column. One expanded item ignores the cap and
/// fills the leftover height (collapsed headers still keep their strip).
const SYSTEM_DOCK_SLOT_MAX_FRACTION: f32 = 0.5;

impl SdroxideApp {
    /// Snapshot of the dock column for persistence (order, collapse, weights,
    /// width fraction) plus which overlays were open as floating windows.
    pub(in crate::app) fn system_dock_snapshot(&self) -> crate::app::persist::SystemDockSnapshot {
        let order: Vec<String> =
            self.system_dock.iter().map(|k| k.persist_id().to_string()).collect();
        let floating: Vec<String> = SystemDock::ALL
            .iter()
            .copied()
            .filter(|k| k.open_flag(self) && !self.system_dock.contains(k))
            .map(|k| k.persist_id().to_string())
            .collect();
        crate::app::persist::SystemDockSnapshot {
            order,
            collapsed: self
                .system_dock_collapsed
                .iter()
                .filter(|k| self.system_dock.contains(k))
                .map(|k| k.persist_id().to_string())
                .collect(),
            weights: self
                .system_dock_weights
                .iter()
                .filter(|(k, _)| self.system_dock.contains(k))
                .map(|(k, w)| (k.persist_id().to_string(), *w))
                .collect(),
            fraction: self.system_dock_fraction.clamp(0.15, SYSTEM_DOCK_MAX_FRACTION),
            floating,
        }
    }

    /// Restore a previously saved dock column and open/close overlays exactly.
    ///
    /// Every System overlay is closed first, then only the saved docked and
    /// floating sets are reopened — otherwise an Audio / Settings window that
    /// was docked before apply would fall out of the dock and keep floating.
    pub(in crate::app) fn apply_system_dock_snapshot(
        &mut self,
        snap: &crate::app::persist::SystemDockSnapshot,
    ) {
        for kind in SystemDock::ALL {
            kind.set_open(self, false);
            *kind.center_pending_mut(self) = false;
        }
        self.system_dock.clear();
        self.system_dock_collapsed.clear();
        self.system_dock_weights.clear();

        let mut order = Vec::new();
        for id in &snap.order {
            let Some(kind) = SystemDock::from_persist_id(id) else { continue };
            if order.contains(&kind) {
                continue;
            }
            order.push(kind);
            kind.set_open(self, true);
        }
        let mut collapsed = std::collections::HashSet::new();
        for id in &snap.collapsed {
            if let Some(kind) = SystemDock::from_persist_id(id) {
                if order.contains(&kind) {
                    collapsed.insert(kind);
                }
            }
        }
        let mut weights = HashMap::new();
        for (id, w) in &snap.weights {
            if let Some(kind) = SystemDock::from_persist_id(id) {
                if order.contains(&kind) {
                    weights.insert(kind, (*w).max(0.05));
                }
            }
        }
        // Any docked overlay missing a weight gets the mean so restore does not
        // leave a 1 px stub next to absolute pixel weights from a prior drag.
        if !order.is_empty() {
            let mean = if weights.is_empty() {
                1.0
            } else {
                weights.values().sum::<f32>() / weights.len() as f32
            };
            for &kind in &order {
                weights.entry(kind).or_insert(mean.max(1.0));
            }
        }
        self.system_dock = order;
        self.system_dock_collapsed = collapsed;
        self.system_dock_weights = weights;
        self.system_dock_fraction = snap.fraction.clamp(0.15, SYSTEM_DOCK_MAX_FRACTION);

        for id in &snap.floating {
            let Some(kind) = SystemDock::from_persist_id(id) else { continue };
            if self.system_dock.contains(&kind) {
                continue;
            }
            kind.set_open(self, true);
        }
    }

    /// Whether this overlay is snapped into the right panel (expanded or collapsed).
    ///
    /// Collapsed items still count as docked so their floating window stays
    /// suppressed; only [`Self::system_dock_host`] cares whether a body rect
    /// was reserved this frame.
    pub(in crate::app) fn system_docked_as(&self, kind: SystemDock) -> bool {
        self.system_dock.contains(&kind)
    }

    /// A System-strip overlay is open as a centred floating window (not pinned).
    pub(in crate::app) fn any_floating_system_popup(&self) -> bool {
        SystemDock::ALL.iter().any(|&kind| kind.open_flag(self) && !self.system_docked_as(kind))
    }

    /// A mode SETUP / other Foreground floating popup is open (not System-dock).
    ///
    /// These share the same modal dim as System overlays so the panadapter and
    /// decode list cannot be clicked while the dialog is up.
    pub(in crate::app) fn any_floating_mode_popup(&self) -> bool {
        self.show_digi_settings
            || self.show_adsb_setup
            || self.show_ais_setup
            || self.show_rs41_setup
            || self.show_meshtastic_setup
            || self.show_vdl2_setup
            || self.show_public_sdrs
    }

    /// Any centred floating dialog that should block the background.
    pub(in crate::app) fn any_floating_modal_popup(&self) -> bool {
        self.any_floating_system_popup() || self.any_floating_mode_popup()
    }

    /// Layer id for the modal veil. **Middle** so it can never cover Foreground
    /// System windows or their ComboBox / menu popups (also Foreground).
    fn modal_dim_layer(ctx: &egui::Context) -> egui::LayerId {
        let id = match crate::layout::radio_salt(ctx) {
            0 => egui::Id::new("sdroxide-modal-dim"),
            n => egui::Id::new(("sdroxide-modal-dim", n)),
        };
        egui::LayerId::new(egui::Order::Middle, id)
    }

    /// Fullscreen veil over Middle chrome (top bar, panadapter, …).
    ///
    /// Stays in [`Order::Middle`]: click-blocking of underneath UI is
    /// [`Memory::set_modal_layer`] on the front Foreground window, not z-order
    /// fights between the dim and dropdowns.
    fn show_modal_dim(ctx: &egui::Context) -> egui::LayerId {
        let layer = Self::modal_dim_layer(ctx);
        let rect = ctx.content_rect();
        let area = egui::Area::new(layer.id)
            .kind(egui::UiKind::Modal)
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            .sense(egui::Sense::hover())
            .interactable(true);
        area.show(ctx, |ui| {
            let bg_sense = egui::Sense::CLICK | egui::Sense::DRAG;
            let mut backdrop = ui.new_child(egui::UiBuilder::new().sense(bg_sense).max_rect(rect));
            backdrop.set_min_size(rect.size());
            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(100));
            let _ = backdrop.response();
        });
        // Above other Middle panels so the veil actually covers them.
        ctx.move_to_top(layer);
        layer
    }

    /// Early veil over Middle chrome while any floating modal is open.
    pub(in crate::app) fn popup_input_guard(&self, ctx: &egui::Context) {
        if self.floating_modal_stack.is_empty() {
            return;
        }
        Self::show_modal_dim(ctx);
    }

    /// Window salts for every open floating modal (System + mode SETUP).
    ///
    /// Order is undefined — use [`Self::floating_modal_stack`] for z-order.
    fn open_floating_modal_salts(&self) -> Vec<&'static str> {
        let mut salts = Vec::new();
        for kind in SystemDock::ALL {
            if kind.open_flag(self) && !self.system_docked_as(kind) {
                salts.push(kind.window_salt());
            }
        }
        // Mode SETUP / Public SDRs — salts must match each `*_window` call site.
        let mode_salts: &[(&str, bool)] = &[
            ("DigiSetup", self.show_digi_settings),
            ("AdsbSetup", self.show_adsb_setup),
            ("AisSetup", self.show_ais_setup),
            ("Rs41Setup", self.show_rs41_setup),
            ("MeshtasticSetup", self.show_meshtastic_setup),
            ("Vdl2Setup", self.show_vdl2_setup),
            ("PUBLIC SDRS", self.show_public_sdrs),
        ];
        for &(salt, open) in mode_salts {
            if open {
                salts.push(salt);
            }
        }
        salts
    }

    /// Drop closed modals; append newly opened ones so they become frontmost.
    pub(in crate::app) fn sync_floating_modal_stack(&mut self) {
        let open = self.open_floating_modal_salts();
        self.floating_modal_stack.retain(|s| open.contains(s));
        for salt in open {
            if !self.floating_modal_stack.contains(&salt) {
                self.floating_modal_stack.push(salt);
            }
        }
    }

    /// Put an already-open (or about-to-open) floating modal above the others.
    ///
    /// Used when re-opening Settings from a mode SETUP link while both stay up.
    pub(in crate::app) fn bring_floating_modal_to_front(&mut self, salt: &'static str) {
        self.floating_modal_stack.retain(|s| *s != salt);
        self.floating_modal_stack.push(salt);
        // So raise does not treat the same pointer press on the parent SETUP
        // window as "bring parent to front" and undo this.
        self.floating_modal_front_forced = true;
    }

    /// True while a nested [`crate::chrome::confirm_modal`] is armed.
    ///
    /// Those use `egui::Modal` (not [`egui::Popup`]). Raising the front
    /// Settings / SETUP window after they paint would `move_to_top` over them.
    pub(in crate::app) fn nested_confirm_open(&self) -> bool {
        self.confirm_close_radio.is_some()
            || self.confirm_undock.is_some()
            || self.sat_win.confirm_unwatch.is_some()
    }

    /// Keep the front floating modal interactive; block everything under it.
    ///
    /// - Dim is Middle (see [`Self::show_modal_dim`]) — never covers this window
    ///   or its ComboBox / context menus (Foreground).
    /// - [`Memory::set_modal_layer`] on the front window blocks Middle chrome
    ///   and any older Foreground modals below it.
    /// - Skip `move_to_top` / `set_modal_layer` while an egui [`Popup`] or a
    ///   nested confirm Modal is open so neither is buried under Settings.
    pub(in crate::app) fn raise_floating_system_popups(&mut self, ctx: &egui::Context) {
        self.sync_floating_modal_stack();
        if !self.floating_modal_front_forced {
            if let Some(pos) = ctx.pointer_interact_pos() {
                if ctx.input(|i| i.pointer.any_pressed()) {
                    if let Some(layer) = ctx.layer_id_at(pos) {
                        if let Some(&salt) = self.floating_modal_stack.iter().find(|&&s| {
                            layer
                                == egui::LayerId::new(
                                    egui::Order::Foreground,
                                    crate::layout::salted_id(ctx, s),
                                )
                        }) {
                            self.bring_floating_modal_to_front(salt);
                        }
                    }
                }
            }
        }
        self.floating_modal_front_forced = false;

        let Some(&front) = self.floating_modal_stack.last() else {
            return;
        };
        // Dim was drawn before the top bar; panels registered after it can sit
        // above it among Middle. Re-raise so the veil covers them. Still under
        // every Foreground window / ComboBox.
        ctx.move_to_top(Self::modal_dim_layer(ctx));
        let win = egui::LayerId::new(
            egui::Order::Foreground,
            crate::layout::salted_id(ctx, front),
        );
        // Popup dropdowns and confirm Modals both live in Foreground; re-raising
        // the system window would cover them. Leave z-order and modal layer alone.
        if egui::Popup::is_any_open(ctx) || self.nested_confirm_open() {
            return;
        }
        ctx.move_to_top(win);
        // Front window (+ pin sublayer, open ComboBox above it) may interact;
        // Middle chrome, the dim, and older Foreground modals may not.
        ctx.memory_mut(|mem| mem.set_modal_layer(win));
    }

    /// Snap `kind` into the right panel. Already-docked overlays stay; this one
    /// is appended if it is not already there (and starts expanded).
    pub(in crate::app) fn dock_system(&mut self, kind: SystemDock) {
        kind.set_open(self, true);
        if !self.system_dock.contains(&kind) {
            self.system_dock.push(kind);
        }
        self.system_dock_collapsed.remove(&kind);
        // Match the mean of existing body shares so a new pin does not start
        // as a 1 px stub next to absolute pixel weights from prior drags.
        if !self.system_dock_weights.contains_key(&kind) {
            let mean = if self.system_dock_weights.is_empty() {
                1.0
            } else {
                self.system_dock_weights.values().sum::<f32>()
                    / self.system_dock_weights.len() as f32
            };
            self.system_dock_weights.insert(kind, mean.max(1.0));
        }
    }

    /// Return one docked overlay to a floating window and re-center it.
    pub(in crate::app) fn undock_system(&mut self, kind: SystemDock) {
        self.system_dock.retain(|k| *k != kind);
        self.system_dock_body_rects.remove(&kind);
        self.system_dock_collapsed.remove(&kind);
        self.system_dock_weights.remove(&kind);
        kind.set_open(self, true);
        *kind.center_pending_mut(self) = true;
    }

    /// Close one docked overlay and drop it from the column.
    pub(in crate::app) fn close_system_dock(&mut self, kind: SystemDock) {
        self.system_dock.retain(|k| *k != kind);
        self.system_dock_body_rects.remove(&kind);
        self.system_dock_collapsed.remove(&kind);
        self.system_dock_weights.remove(&kind);
        kind.set_open(self, false);
    }

    /// Move `from` next to `onto` in the dock stack (`before` → above).
    fn reorder_system_dock(&mut self, from: SystemDock, onto: SystemDock, before: bool) {
        if from == onto {
            return;
        }
        let Some(from_i) = self.system_dock.iter().position(|k| *k == from) else {
            return;
        };
        self.system_dock.remove(from_i);
        let Some(onto_i) = self.system_dock.iter().position(|k| *k == onto) else {
            self.system_dock.insert(from_i.min(self.system_dock.len()), from);
            return;
        };
        let insert_at = if before { onto_i } else { onto_i + 1 };
        self.system_dock.insert(insert_at, from);
    }

    /// Relative body share for `kind` (any positive scale — layout normalizes).
    fn system_dock_weight(&self, kind: SystemDock) -> f32 {
        self.system_dock_weights.get(&kind).copied().unwrap_or(1.0).max(0.05)
    }

    /// Body height floor so the whole expanded slot stays ≥ [`SYSTEM_DOCK_SLOT_MIN_H`].
    fn system_dock_body_min() -> f32 {
        (SYSTEM_DOCK_SLOT_MIN_H
            - SYSTEM_DOCK_COLLAPSED_H
            - SYSTEM_DOCK_SEP_H
            - SYSTEM_DOCK_ROW_HANDLE_H)
            .max(SYSTEM_DOCK_COLLAPSED_H)
    }

    /// Chrome inside one expanded slot: header + separator + row grip.
    fn system_dock_expand_chrome() -> f32 {
        SYSTEM_DOCK_COLLAPSED_H + SYSTEM_DOCK_SEP_H + SYSTEM_DOCK_ROW_HANDLE_H
    }

    /// Per-body ceiling when several overlays are expanded: each slot ≤ half
    /// the column, so the body max is half the column minus that slot's chrome.
    fn system_dock_body_cap(col_h: f32, n_expanded: usize) -> Option<f32> {
        if n_expanded <= 1 {
            return None;
        }
        let slot_max = (col_h * SYSTEM_DOCK_SLOT_MAX_FRACTION).max(0.0);
        Some((slot_max - Self::system_dock_expand_chrome()).max(Self::system_dock_body_min()))
    }

    /// Split `body_pool` across expanded overlays.
    ///
    /// - **One** expanded item → takes the whole pool (full remaining column).
    /// - **Two or more** → share by weight, each capped at half the dock
    ///   column ([`SYSTEM_DOCK_SLOT_MAX_FRACTION`]), then redistribute leftover
    ///   to slots still under the cap so the column stays filled when possible.
    fn system_dock_body_heights(
        &self,
        expanded: &[SystemDock],
        body_pool: f32,
        body_min: f32,
        body_cap: Option<f32>,
    ) -> HashMap<SystemDock, f32> {
        let mut out = HashMap::new();
        let n = expanded.len();
        if n == 0 || body_pool <= 0.0 {
            return out;
        }

        // Single expanded overlay: fill the column (minus collapsed strips).
        if n == 1 {
            out.insert(expanded[0], body_pool.max(0.0));
            return out;
        }

        let n_f = n as f32;
        let cap = body_cap.unwrap_or(body_pool).max(body_min);
        // When the column cannot fit every min, share the pool evenly.
        if body_pool < body_min * n_f {
            let each = body_pool / n_f;
            for &k in expanded {
                out.insert(k, each.min(cap));
            }
            return out;
        }

        let weight_sum: f32 =
            expanded.iter().map(|k| self.system_dock_weight(*k)).sum::<f32>().max(0.05);
        let mut heights: Vec<(SystemDock, f32)> = expanded
            .iter()
            .map(|&k| {
                // Ideal = weight share of the pool, then clamp to [min, cap].
                let ideal = body_pool * (self.system_dock_weight(k) / weight_sum);
                (k, ideal.clamp(body_min, cap))
            })
            .collect();

        // Lift anyone under the floor; take the deficit from those above it.
        let mut deficit = 0.0_f32;
        for (_, h) in &mut heights {
            if *h < body_min {
                deficit += body_min - *h;
                *h = body_min;
            }
        }
        if deficit > 0.0 {
            let donors: Vec<usize> = heights
                .iter()
                .enumerate()
                .filter(|(_, (_, h))| *h > body_min + 0.5)
                .map(|(i, _)| i)
                .collect();
            let donor_extra: f32 =
                donors.iter().map(|&i| heights[i].1 - body_min).sum::<f32>().max(0.05);
            for i in donors {
                let take = deficit * ((heights[i].1 - body_min) / donor_extra);
                heights[i].1 = (heights[i].1 - take).max(body_min);
            }
        }

        // Cap may leave unused pool — grow slots still under the ceiling.
        let mut guard = 0;
        while guard < 8 {
            guard += 1;
            let used: f32 = heights.iter().map(|(_, h)| *h).sum();
            let rem = body_pool - used;
            if rem <= 0.25 {
                break;
            }
            let room: Vec<usize> = heights
                .iter()
                .enumerate()
                .filter(|(_, (_, h))| *h + 0.25 < cap)
                .map(|(i, _)| i)
                .collect();
            if room.is_empty() {
                break;
            }
            let room_w: f32 =
                room.iter().map(|&i| self.system_dock_weight(heights[i].0)).sum::<f32>().max(0.05);
            for i in room {
                let share = rem * (self.system_dock_weight(heights[i].0) / room_w);
                heights[i].1 = (heights[i].1 + share).min(cap);
            }
        }

        for (k, h) in heights {
            out.insert(k, h.max(0.0));
        }
        out
    }

    /// Pin control in a floating System window's title bar, in the slot
    /// immediately to the right of egui's collapse triangle.
    ///
    /// Call after `.show()` with that window's response. `window_salt` is
    /// kept for call-site clarity (matches the floating window id salt).
    /// Hidden when the layout cannot dock. Returns whether the operator
    /// clicked it this frame.
    pub(in crate::app) fn system_title_pin(
        &self,
        ctx: &egui::Context,
        win: &egui::Response,
        _window_salt: &'static str,
    ) -> bool {
        if self.band_dock_room.is_none() {
            return false;
        }
        // Match egui 0.36 `title_ui` exactly: the outer window frame's inner
        // margin is zeroed before the title draws, and the title AtomLayout
        // uses `Frame::window` → `spacing.window_margin`. Collapse is a
        // heading-height square; the triangle is then centered (shrunk) inside
        // it to `icon_width`. Same slot, one gap to the right, same top.
        let style = ctx.style_of(ctx.theme());
        let heading_h =
            ctx.fonts_mut(|f| f.row_height(&egui::TextStyle::Heading.resolve(style.as_ref())));
        let margin = style.spacing.window_margin;
        let spacing = style.spacing.item_spacing.x;
        let icon = style.spacing.icon_width;
        let alloc = heading_h;
        let pin_min = egui::pos2(
            win.rect.left() + margin.leftf() + alloc + spacing,
            win.rect.top() + margin.topf(),
        );

        let mut clicked = false;
        let inner = egui::Area::new(win.id.with("system-title-pin"))
            // Must match the parent window's order — `set_sublayer` panics if
            // they differ (System popups are Foreground above the modal dim).
            .order(win.layer_id.order)
            .fixed_pos(pin_min)
            .sense(egui::Sense::click())
            .show(ctx, |ui| {
                // Exact allocation size (no Button chrome) so the hit target
                // matches the collapse square; glyph centered like the triangle.
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(alloc, alloc), egui::Sense::click());
                let font = egui::FontId::proportional((icon * 0.95).clamp(11.0, 16.0));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "📌",
                    font,
                    crate::theme::TEXT_STRONG(),
                );
                if resp.on_hover_text("Snap into the right panel beside the waterfall").clicked() {
                    clicked = true;
                }
            });
        // Ride the window's layer so the pin hides/moves with it. Safe again
        // now that the window is not itself a sublayer of the modal dim.
        ctx.set_sublayer(win.layer_id, inner.response.layer_id);
        clicked
    }

    /// Floating-window builder replaced by a fixed host inside the dock column.
    ///
    /// The host is clipped and size-locked to `kind`'s body rect so stacked
    /// overlays cannot paint over each other.
    pub(in crate::app) fn system_dock_host<'a>(
        &self,
        title: &'a str,
        id: egui::Id,
        kind: SystemDock,
    ) -> Option<egui::Window<'a>> {
        let rect = *self.system_dock_body_rects.get(&kind)?;
        Some(
            egui::Window::new(title)
                // Separate id from the floating window so persisted size/pos
                // does not fight the dock host. Salted per kind so stacked
                // hosts do not share one AreaState.
                .id(id.with("system-dock-host").with(kind.title()))
                .title_bar(false)
                .fixed_pos(rect.min)
                .fixed_size(rect.size())
                .max_size(rect.size())
                .constrain_to(rect)
                .resizable(false)
                .frame(egui::Frame::NONE)
                .fade_in(false)
                .fade_out(false),
        )
    }

    /// Clip the docked host's content to its reserved body rect.
    ///
    /// Call at the start of every docked `.show()` closure so scroll areas and
    /// tall content cannot paint into the slot below.
    pub(in crate::app) fn clip_system_dock_body(&self, ui: &mut egui::Ui, kind: SystemDock) {
        let Some(rect) = self.system_dock_body_rects.get(&kind).copied() else {
            return;
        };
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        ui.set_max_size(rect.size());
    }

    /// Reserve the right column and record a body rect per docked overlay.
    ///
    /// Call after the top bar (and beside / before the band dock) so the
    /// waterfall shrinks around the column. Slots partition the column height
    /// exactly — they never grow past their share, so they cannot overlap.
    pub(in crate::app) fn system_dock_panel(&mut self, ui: &mut egui::Ui) {
        self.system_dock_body_rects.clear();
        // Chip (or ×) closed an overlay — drop it from the column.
        let still_open: Vec<SystemDock> =
            self.system_dock.iter().copied().filter(|k| k.open_flag(self)).collect();
        self.system_dock_collapsed.retain(|k| still_open.contains(k));
        self.system_dock_weights.retain(|k, _| still_open.contains(k));
        self.system_dock = still_open;
        if self.system_dock.is_empty() {
            return;
        }
        // `band_dock_room` only answers "is the layout wide enough to dock".
        // Width is driven by `system_dock_fraction`, capped at half the content
        // so the main panel always keeps ≥ 50%.
        if self.band_dock_room.is_none() {
            // Too narrow / phone: keep the dock list so widening restores the
            // column; matching windows paint floating until then.
            return;
        }
        let content_w = ui.available_width();
        let max_w = (content_w * SYSTEM_DOCK_MAX_FRACTION).max(1.0);
        let min_w = SYSTEM_DOCK_MIN_W.min(max_w);
        let dock_w = (content_w * self.system_dock_fraction).clamp(min_w, max_w);
        // Keep the stored fraction honest after a window resize clamps the px.
        self.system_dock_fraction = dock_w / content_w.max(1.0);
        let docked = self.system_dock.clone();

        let mut close: Option<SystemDock> = None;
        let mut undock: Option<SystemDock> = None;
        let mut toggle_collapse: Option<SystemDock> = None;
        // (dragged kind, drop target, insert above target)
        let mut reorder: Option<(SystemDock, SystemDock, bool)> = None;
        let mut body_rects = HashMap::new();
        let mut drag_dx = 0.0_f32;
        // (upper expanded, lower expanded, pixel delta — drag down grows upper)
        let mut height_drag: Option<(SystemDock, SystemDock, f32)> = None;
        let collapsed = self.system_dock_collapsed.clone();
        let expanded: Vec<SystemDock> =
            docked.iter().copied().filter(|k| !collapsed.contains(k)).collect();
        let body_min = Self::system_dock_body_min();
        let expand_chrome = Self::system_dock_expand_chrome();
        // Captured for the post-panel drag clamp (50% of column when n ≥ 2).
        let mut layout_col_h = 0.0_f32;

        // Handle sits inside the panel on the inner (main-facing) edge — egui's
        // own Panel resize separator is a plain line, not the split-handle chrome
        // used everywhere else in the app.
        egui::Panel::right(crate::layout::salted_id(ui.ctx(), "system-dock"))
            .resizable(false)
            .exact_size(dock_w + SYSTEM_DOCK_HANDLE_W)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL())
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ui, |ui| {
                let avail_h = ui.available_height().max(0.0);
                // No gap between handle and body — spacing used to steal the
                // 7 pt strip and the vertical grip vanished.
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal(|ui| {
                    let hresp = crate::chrome::split_handle(
                        ui,
                        egui::vec2(SYSTEM_DOCK_HANDLE_W, avail_h),
                        None,
                    );
                    if hresp.dragged() {
                        // Drag toward the main column (negative x) widens the dock.
                        drag_dx = hresp.drag_delta().x;
                    }

                    ui.allocate_ui_with_layout(
                        egui::vec2(dock_w, avail_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(9, 7))
                                .show(ui, |ui| {
                                    let n = docked.len().max(1);
                                    let gap = 4.0;
                                    let gaps = gap * (n.saturating_sub(1) as f32);
                                    let col_h = ui.available_height().max(0.0);
                                    layout_col_h = col_h;
                                    let avail_w = ui.available_width();
                                    let n_expanded = expanded.len();
                                    let n_collapsed = n - n_expanded;
                                    let collapsed_h =
                                        SYSTEM_DOCK_COLLAPSED_H * n_collapsed as f32;
                                    let body_pool = (col_h
                                        - gaps
                                        - collapsed_h
                                        - expand_chrome * n_expanded as f32)
                                        .max(0.0);
                                    let body_cap =
                                        Self::system_dock_body_cap(col_h, n_expanded);
                                    let body_heights = self.system_dock_body_heights(
                                        &expanded,
                                        body_pool,
                                        body_min,
                                        body_cap,
                                    );

                                    for (i, kind) in docked.iter().copied().enumerate() {
                                        let is_expanded = !collapsed.contains(&kind);
                                        let body_h = if is_expanded {
                                            body_heights.get(&kind).copied().unwrap_or(0.0)
                                        } else {
                                            0.0
                                        };
                                        let slot_h = if is_expanded {
                                            expand_chrome + body_h
                                        } else {
                                            SYSTEM_DOCK_COLLAPSED_H
                                        };
                                        let (slot_rect, slot_resp) = ui.allocate_exact_size(
                                            egui::vec2(avail_w, slot_h),
                                            egui::Sense::hover(),
                                        );
                                        // Drop target: insert above/below based on
                                        // pointer Y vs the slot mid-line.
                                        let drop_before = ui
                                            .input(|inp| inp.pointer.interact_pos())
                                            .map(|p| p.y < slot_rect.center().y)
                                            .unwrap_or(true);
                                        if let Some(payload) =
                                            slot_resp.dnd_hover_payload::<DraggedSystemDock>()
                                        {
                                            if payload.0 != kind {
                                                let y = if drop_before {
                                                    slot_rect.top()
                                                } else {
                                                    slot_rect.bottom() - 1.0
                                                };
                                                ui.painter().hline(
                                                    slot_rect.x_range(),
                                                    y,
                                                    egui::Stroke::new(
                                                        2.0,
                                                        crate::theme::CYAN(),
                                                    ),
                                                );
                                            }
                                        }
                                        if let Some(payload) =
                                            slot_resp.dnd_release_payload::<DraggedSystemDock>()
                                        {
                                            if payload.0 != kind {
                                                reorder =
                                                    Some((payload.0, kind, drop_before));
                                            }
                                        }
                                        ui.scope_builder(
                                            egui::UiBuilder::new().max_rect(slot_rect),
                                            |ui| {
                                                ui.set_clip_rect(slot_rect);
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                // Full header is the reorder bar.
                                                // Collapse / 📌 / × are painted first
                                                // and excluded from the drag sense so
                                                // their hover/click never starts a drag.
                                                let (header_rect, _) = ui
                                                    .allocate_exact_size(
                                                        egui::vec2(
                                                            avail_w,
                                                            SYSTEM_DOCK_COLLAPSED_H,
                                                        ),
                                                        egui::Sense::hover(),
                                                    );
                                                let drag_id = crate::layout::salted_id(
                                                    ui.ctx(),
                                                    "system-dock-drag",
                                                )
                                                .with(kind.title());
                                                let mut over_controls = false;
                                                ui.scope_builder(
                                                    egui::UiBuilder::new()
                                                        .max_rect(header_rect),
                                                    |ui| {
                                                        ui.spacing_mut().item_spacing.x = 4.0;
                                                        ui.horizontal(|ui| {
                                                            let icon =
                                                                ui.spacing().icon_width;
                                                            let (_, icon_resp) = ui
                                                                .allocate_exact_size(
                                                                    egui::vec2(icon, icon),
                                                                    egui::Sense::click(),
                                                                );
                                                            let openness = if is_expanded {
                                                                1.0
                                                            } else {
                                                                0.0
                                                            };
                                                            egui::collapsing_header::paint_default_icon(
                                                                ui, openness, &icon_resp,
                                                            );
                                                            if icon_resp.contains_pointer() {
                                                                over_controls = true;
                                                            }
                                                            if icon_resp
                                                                .on_hover_text(
                                                                    if is_expanded {
                                                                        "Collapse"
                                                                    } else {
                                                                        "Expand"
                                                                    },
                                                                )
                                                                .clicked()
                                                            {
                                                                toggle_collapse = Some(kind);
                                                            }
                                                            ui.label(
                                                                RichText::new(kind.title())
                                                                    .size(11.0)
                                                                    .strong()
                                                                    .color(
                                                                        crate::theme::CYAN(),
                                                                    ),
                                                            );
                                                            ui.with_layout(
                                                                egui::Layout::right_to_left(
                                                                    egui::Align::Center,
                                                                ),
                                                                |ui| {
                                                                    let close_r =
                                                                        crate::chrome::chip(
                                                                            ui, false, "×",
                                                                        )
                                                                        .on_hover_text(
                                                                            "Close",
                                                                        );
                                                                    if close_r.clicked() {
                                                                        close = Some(kind);
                                                                    }
                                                                    if close_r
                                                                        .contains_pointer()
                                                                    {
                                                                        over_controls = true;
                                                                    }
                                                                    let pin_r =
                                                                        crate::chrome::chip(
                                                                            ui, true, "📌",
                                                                        )
                                                                        .on_hover_text(
                                                                            "Return to a floating window",
                                                                        );
                                                                    if pin_r.clicked() {
                                                                        // Armed here; the centered
                                                                        // modal below confirms.
                                                                        undock = Some(kind);
                                                                    }
                                                                    if pin_r
                                                                        .contains_pointer()
                                                                    {
                                                                        over_controls = true;
                                                                    }
                                                                },
                                                            );

                                                            let dragging = ui
                                                                .ctx()
                                                                .is_being_dragged(drag_id);
                                                            if dragging {
                                                                egui::DragAndDrop::set_payload(
                                                                    ui.ctx(),
                                                                    DraggedSystemDock(kind),
                                                                );
                                                                ui.ctx().request_repaint();
                                                                ui.ctx().set_cursor_icon(
                                                                    egui::CursorIcon::Grabbing,
                                                                );
                                                                // Follow-the-pointer title
                                                                // chip so the stack does
                                                                // not look frozen mid-drag.
                                                                if let Some(pos) = ui
                                                                    .ctx()
                                                                    .pointer_interact_pos()
                                                                {
                                                                    egui::Area::new(
                                                                        drag_id.with(
                                                                            "drag-preview",
                                                                        ),
                                                                    )
                                                                    .order(
                                                                        egui::Order::Tooltip,
                                                                    )
                                                                    .fixed_pos(
                                                                        pos + egui::vec2(
                                                                            12.0, 8.0,
                                                                        ),
                                                                    )
                                                                    .show(
                                                                        ui.ctx(),
                                                                        |ui| {
                                                                            egui::Frame::new()
                                                                                .fill(
                                                                                    crate::theme::PANEL(),
                                                                                )
                                                                                .stroke(
                                                                                    egui::Stroke::new(
                                                                                        1.0,
                                                                                        crate::theme::CYAN(),
                                                                                    ),
                                                                                )
                                                                                .inner_margin(6.0)
                                                                                .show(ui, |ui| {
                                                                                    ui.label(
                                                                                        RichText::new(
                                                                                            kind.title(),
                                                                                        )
                                                                                        .size(11.0)
                                                                                        .strong()
                                                                                        .color(
                                                                                            crate::theme::CYAN(),
                                                                                        ),
                                                                                    );
                                                                                });
                                                                        },
                                                                    );
                                                                }
                                                            } else if !over_controls {
                                                                // Header drag still reorders;
                                                                // no grip glyph before the title.
                                                                let _ = ui
                                                                    .interact(
                                                                        header_rect,
                                                                        drag_id,
                                                                        egui::Sense::drag(),
                                                                    )
                                                                    .on_hover_cursor(
                                                                        egui::CursorIcon::Grab,
                                                                    )
                                                                    .on_hover_text(
                                                                        "Drag to reorder",
                                                                    );
                                                            }
                                                        });
                                                    },
                                                );
                                                if is_expanded {
                                                    // Fixed-height rule so chrome
                                                    // math matches slot allocation.
                                                    let (sep_rect, _) = ui
                                                        .allocate_exact_size(
                                                            egui::vec2(
                                                                avail_w,
                                                                SYSTEM_DOCK_SEP_H,
                                                            ),
                                                            egui::Sense::hover(),
                                                        );
                                                    let y = sep_rect.center().y;
                                                    ui.painter().hline(
                                                        sep_rect.x_range(),
                                                        y,
                                                        egui::Stroke::new(
                                                            1.0,
                                                            crate::theme::gray(55),
                                                        ),
                                                    );

                                                    let (rect, _) = ui.allocate_exact_size(
                                                        egui::vec2(avail_w, body_h),
                                                        egui::Sense::hover(),
                                                    );
                                                    body_rects.insert(kind, rect);

                                                    // Horizontal split-handle under the
                                                    // body. Interactive only when another
                                                    // expanded item sits below — then a
                                                    // drag trades height 1:1.
                                                    let next = expanded
                                                        .iter()
                                                        .skip_while(|k| **k != kind)
                                                        .nth(1)
                                                        .copied();
                                                    let hresp = crate::chrome::split_handle(
                                                        ui,
                                                        egui::vec2(
                                                            avail_w,
                                                            SYSTEM_DOCK_ROW_HANDLE_H,
                                                        ),
                                                        None,
                                                    );
                                                    if let Some(next) = next {
                                                        let dy = hresp.drag_delta().y;
                                                        if hresp
                                                            .on_hover_text(
                                                                "Drag to resize this docked panel",
                                                            )
                                                            .dragged()
                                                        {
                                                            height_drag =
                                                                Some((kind, next, dy));
                                                            ui.ctx().request_repaint();
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                        if i + 1 < n {
                                            ui.add_space(gap);
                                        }
                                    }
                                });
                        },
                    );
                });
            });

        if drag_dx != 0.0 {
            let new_w = (dock_w - drag_dx).clamp(min_w, max_w);
            self.system_dock_fraction = new_w / content_w.max(1.0);
        }

        if let Some((a, b, dy)) = height_drag {
            if dy != 0.0 {
                // Promote every laid-out body to an absolute weight first so a
                // prior equal-share `1.0` does not sit next to pixel heights
                // and collapse the rest of the stack.
                for (k, rect) in &body_rects {
                    self.system_dock_weights.insert(*k, rect.height().max(0.05));
                }
                // Trade body pixels 1:1 — same feel as digi / waterfall splits.
                // Keep each side under the multi-item 50% column cap.
                let ha = body_rects.get(&a).map(|r| r.height()).unwrap_or(body_min);
                let hb = body_rects.get(&b).map(|r| r.height()).unwrap_or(body_min);
                let pair_px = (ha + hb).max(1.0);
                let cap = Self::system_dock_body_cap(layout_col_h, expanded.len())
                    .unwrap_or(pair_px)
                    .min(pair_px - body_min);
                let lo = body_min.min(cap);
                let new_ha = (ha + dy).clamp(lo, cap);
                let new_hb = (pair_px - new_ha).clamp(lo, cap);
                // If the pair no longer sums (both at cap), keep the drag side.
                let new_hb =
                    if (new_ha + new_hb - pair_px).abs() > 0.5 { pair_px - new_ha } else { new_hb };
                self.system_dock_weights.insert(a, new_ha.max(0.05));
                self.system_dock_weights.insert(b, new_hb.max(0.05));
            }
        }

        if let Some(kind) = toggle_collapse {
            if !self.system_dock_collapsed.remove(&kind) {
                self.system_dock_collapsed.insert(kind);
            }
        }

        if let Some((from, onto, before)) = reorder {
            self.reorder_system_dock(from, onto, before);
        }

        self.system_dock_body_rects = body_rects;

        if let Some(kind) = close {
            self.close_system_dock(kind);
        } else if let Some(kind) = undock {
            self.confirm_undock = Some(kind);
        }
        // Confirm paints after `raise_floating_system_popups` (see frame.rs).
    }

    /// Centered confirm before returning a docked overlay to a floating window.
    pub(in crate::app) fn resolve_undock_confirm(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.confirm_undock else {
            return;
        };
        let body = format!(
            "Undock {}? It leaves the dock column and opens as a floating window again.",
            kind.title(),
        );
        match crate::chrome::confirm_modal(ctx, "system-undock", &body, "Undock") {
            Some(crate::chrome::ConfirmChoice::Confirm) => {
                self.undock_system(kind);
                self.confirm_undock = None;
            }
            Some(crate::chrome::ConfirmChoice::Cancel) => self.confirm_undock = None,
            None => {}
        }
    }
}
