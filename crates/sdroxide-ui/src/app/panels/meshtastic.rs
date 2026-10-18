//! The Meshtastic panel: messages, nodes, and a map.
//!
//! Desktop layout (two columns):
//!
//! ```text
//! [MESH … status …]                              [SETUP]
//! ┌─ MESSAGES ──────────────┬─ [NODES] [MAP] ───┐
//! │ decoded text log        │ node list or map  │
//! └─────────────────────────┴───────────────────┘
//! ```
//!
//! Phone panes: MESSAGES / NODES / MAP. SETUP stays a window.

use eframe::egui::{self, RichText};
use sdroxide_types::{
    Command, MeshtasticMessage, MeshtasticNode, MeshtasticPskKind, MeshtasticRegion,
    MeshtasticStatus,
};

use crate::app::SdroxideApp;
use crate::app::util::fmt_age;
use crate::theme;
use crate::theme::ThemedScroll;

impl SdroxideApp {
    pub(in crate::app) fn meshtastic_panel(
        &mut self,
        ui: &mut egui::Ui,
        cmds: &mut Vec<Command>,
        panel_h: f32,
    ) {
        let st: MeshtasticStatus = match self.meshtastic_status.as_ref() {
            Some(s) => (**s).clone(),
            None => {
                ui.label(RichText::new("starting the Meshtastic decoder…").weak());
                return;
            }
        };
        let now = crate::time::now_unix();
        let content_bottom = ui.cursor().top() + panel_h - 26.0;

        self.meshtastic_header(ui, cmds, &st);
        ui.add_space(3.0);

        let avail_h = (content_bottom - ui.cursor().top()).max(80.0);
        let pane = self.phone_pane(ui, self.state.rx[0].mode);
        let full_w = ui.available_width();

        ui.horizontal_top(|ui| match pane {
            Some(0) => {
                ui.allocate_ui_with_layout(
                    egui::vec2(full_w, avail_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_message_pane(ui, cmds, &st, avail_h),
                );
            }
            Some(1) => {
                ui.allocate_ui_with_layout(
                    egui::vec2(full_w, avail_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_node_list(ui, &st, now, avail_h),
                );
            }
            Some(2) | Some(_) => {
                ui.allocate_ui_with_layout(
                    egui::vec2(full_w, avail_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_map_pane(ui, &st, now, avail_h),
                );
            }
            None => {
                const HANDLE_W: f32 = 7.0;
                let msg_w = (full_w * self.view.meshtastic_split_fraction)
                    .clamp(200.0, (full_w - HANDLE_W - 180.0).max(200.0));
                ui.allocate_ui_with_layout(
                    egui::vec2(msg_w, avail_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_message_pane(ui, cmds, &st, avail_h),
                );
                let hh = crate::chrome::split_handle(ui, egui::vec2(HANDLE_W, avail_h), None);
                if hh.dragged() {
                    self.view.meshtastic_split_fraction =
                        ((msg_w + hh.drag_delta().x) / full_w.max(1.0)).clamp(0.25, 0.85);
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width().max(160.0), avail_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_side_column(ui, cmds, &st, now, avail_h),
                );
            }
        });
    }

    /// Right column: NODES / MAP tabs, then the selected body.
    fn meshtastic_side_column(
        &mut self,
        ui: &mut egui::Ui,
        _cmds: &mut Vec<Command>,
        st: &MeshtasticStatus,
        now: i64,
        avail_h: f32,
    ) {
        ui.horizontal(|ui| {
            let tab = self.view.meshtastic_side_tab;
            if crate::chrome::chip(ui, tab == 0, "NODES").clicked() {
                self.view.meshtastic_side_tab = 0;
            }
            if crate::chrome::chip(ui, tab == 1, "MAP").clicked() {
                self.view.meshtastic_side_tab = 1;
            }
            if self.view.meshtastic_side_tab == 0 {
                crate::chrome::row_tail(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.meshtastic_filter)
                            .hint_text("filter")
                            .desired_width(74.0),
                    );
                });
            }
        });
        let body_h = (avail_h - 26.0).max(60.0);
        match self.view.meshtastic_side_tab {
            1 => self.meshtastic_map_pane(ui, st, now, body_h),
            _ => self.meshtastic_node_list(ui, st, now, body_h),
        }
    }

    fn meshtastic_header(
        &mut self,
        ui: &mut egui::Ui,
        cmds: &mut Vec<Command>,
        st: &MeshtasticStatus,
    ) {
        let region = self.state.meshtastic.region;
        ui.horizontal(|ui| {
            ui.set_min_height(22.0);
            ui.label(RichText::new("MESH").size(11.0).strong().color(theme::CYAN()));
            ui.label(RichText::new("Meshtastic").weak().size(10.5));

            let mut pick = region;
            egui::ComboBox::from_id_salt("mesh-region-header")
                .selected_text(format!("{}  {:.3} MHz", region.label(), region.center_hz() / 1e6))
                .width(168.0)
                .show_ui(ui, |ui| {
                    for r in MeshtasticRegion::ALL {
                        ui.selectable_value(
                            &mut pick,
                            r,
                            format!("{} — {}", r.label(), r.description()),
                        )
                        .on_hover_text(format!("LongFast centre {:.3} MHz", r.center_hz() / 1e6));
                    }
                });
            if pick != region {
                let mut cfg = self.state.meshtastic;
                cfg.region = pick;
                cmds.push(Command::SetMeshtasticConfig(cfg));
                cmds.push(Command::SetVfo { vfo: self.state.active_vfo, hz: pick.center_hz() });
            }

            ui.separator();
            let modem = match (st.sf, st.bw_hz, st.cr) {
                (Some(sf), Some(bw), Some(cr)) => {
                    format!("SF{sf} / {} kHz / 4/{cr}", bw / 1000)
                }
                _ if !st.channel_name.is_empty() => st.channel_name.clone(),
                _ => "——".into(),
            };
            slot(ui, 150.0, &modem, theme::gray(150));
            if let Some(snr) = st.last_snr_db.or_else(|| {
                self.meshtastic_map
                    .selected
                    .as_ref()
                    .and_then(|id| st.nodes.iter().find(|n| &n.id == id))
                    .and_then(|n| n.snr_db)
            }) {
                slot(ui, 64.0, &format!("SNR {snr:.1}"), theme::CYAN());
            } else {
                slot(ui, 64.0, "SNR ——", theme::gray(120));
            }

            let (lamp, ink) = if st.unavailable.is_some() {
                ("NO LOCK", theme::HAZARD())
            } else if !st.listening {
                ("IDLE", theme::gray(120))
            } else if st.nodes.is_empty() && st.messages.is_empty() {
                ("IDLE", theme::YELLOW())
            } else {
                ("DECODE", theme::GREEN())
            };
            slot(ui, 72.0, lamp, ink);

            crate::chrome::row_tail(ui, |ui| {
                if crate::chrome::chip(ui, self.show_meshtastic_setup, "⚙ SETUP")
                    .on_hover_text("Region default and channel PSK")
                    .clicked()
                {
                    self.show_meshtastic_setup = !self.show_meshtastic_setup;
                }
            });
        });

        if let Some(why) = &st.unavailable {
            let dial = self.state.rx_freq_hz();
            let centre = self.state.meshtastic.region.center_hz();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(why).size(10.5).color(theme::HAZARD()));
                if (dial - centre).abs() > 1.0
                    && self.caps.as_ref().is_none_or(|c| c.may_rx_hz(centre))
                    && crate::chrome::chip(ui, false, format!("TUNE {:.3}", centre / 1e6)).clicked()
                {
                    cmds.push(Command::SetVfo { vfo: self.state.active_vfo, hz: centre });
                }
            });
        }
    }

    fn meshtastic_node_list(
        &mut self,
        ui: &mut egui::Ui,
        st: &MeshtasticStatus,
        now: i64,
        avail_h: f32,
    ) {
        // When embedded in the side column the NODES title is the tab; on phone
        // we still show a filter row.
        if self.phone_pane(ui, self.state.rx[0].mode).is_some() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("NODES").size(10.5).strong().color(theme::CYAN_DIM()));
                crate::chrome::row_tail(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.meshtastic_filter)
                            .hint_text("filter")
                            .desired_width(74.0),
                    );
                });
            });
        }

        let filter = self.meshtastic_filter.trim().to_ascii_uppercase();
        let mut rows: Vec<&MeshtasticNode> = st
            .nodes
            .iter()
            .filter(|n| {
                if filter.is_empty() {
                    return true;
                }
                n.id.to_ascii_uppercase().contains(&filter)
                    || n.short_name.to_ascii_uppercase().contains(&filter)
                    || n.long_name.to_ascii_uppercase().contains(&filter)
            })
            .collect();
        rows.sort_by(|a, b| b.last_heard_unix.cmp(&a.last_heard_unix));

        let selected = self.meshtastic_map.selected.clone();
        let selected_node = selected.as_ref().and_then(|id| st.nodes.iter().find(|n| &n.id == id));
        let header_h =
            if self.phone_pane(ui, self.state.rx[0].mode).is_some() { 22.0 } else { 0.0 };
        let body_h = (avail_h - header_h).max(48.0);

        if let Some(n) = selected_node {
            let full = ui.available_width();
            let list_w = (full * 0.48).clamp(120.0, full - 140.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(list_w, body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_node_rows(ui, &rows, &selected, now, body_h),
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2((full - list_w - 8.0).max(120.0), body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.meshtastic_node_card(ui, n, now, body_h),
                );
            });
        } else {
            self.meshtastic_node_rows(ui, &rows, &selected, now, body_h);
        }
    }

    fn meshtastic_node_rows(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[&MeshtasticNode],
        selected: &Option<String>,
        now: i64,
        list_h: f32,
    ) {
        if rows.is_empty() {
            empty_center(ui, egui::vec2(ui.available_width(), list_h), "no nodes heard yet");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("meshtastic-nodes")
            .max_height(list_h)
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for n in rows {
                    let sel = selected.as_deref() == Some(n.id.as_str());
                    if meshtastic_node_row(ui, n, now, sel) {
                        let id = n.id.clone();
                        if selected.as_deref() == Some(id.as_str()) {
                            self.meshtastic_map.selected = None;
                        } else {
                            self.meshtastic_map.selected = Some(id);
                        }
                    }
                }
            });
    }

    fn meshtastic_node_card(
        &mut self,
        ui: &mut egui::Ui,
        n: &MeshtasticNode,
        now: i64,
        card_h: f32,
    ) {
        egui::ScrollArea::vertical()
            .id_salt("meshtastic-card")
            .max_height(card_h)
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let title =
                    if !n.short_name.is_empty() { n.short_name.as_str() } else { n.id.as_str() };
                ui.label(RichText::new(title).strong().size(13.0).color(theme::CYAN()));
                if !n.long_name.is_empty() {
                    ui.label(RichText::new(&n.long_name).size(10.5).weak());
                }
                ui.label(
                    RichText::new(format!("{} · heard {}", n.id, fmt_age(now - n.last_heard_unix)))
                        .size(10.0)
                        .weak(),
                );
                ui.add_space(4.0);
                let fields: [(&str, Option<String>); 6] = [
                    ("SNR", n.snr_db.map(|s| format!("{s:.1} dB"))),
                    ("RSSI", n.rssi_dbm.map(|r| format!("{r:.0} dBm"))),
                    ("Hops", n.hops.map(|h| format!("{h}"))),
                    ("Battery", n.battery_pct.map(|b| format!("{b} %"))),
                    (
                        "Position",
                        match (n.lat, n.lon) {
                            (Some(lat), Some(lon)) => Some(format!("{lat:.5}, {lon:.5}")),
                            _ => None,
                        },
                    ),
                    ("Id", Some(n.id.clone())),
                ];
                egui::Grid::new("meshtastic-card-grid").num_columns(4).spacing([8.0, 3.0]).show(
                    ui,
                    |ui| {
                        for chunk in fields.chunks(2) {
                            for (key, val) in chunk {
                                ui.label(RichText::new(*key).size(10.0).weak());
                                ui.label(
                                    RichText::new(val.as_deref().unwrap_or("——"))
                                        .size(10.5)
                                        .color(theme::CYAN()),
                                );
                            }
                            if chunk.len() == 1 {
                                ui.label("");
                                ui.label("");
                            }
                            ui.end_row();
                        }
                    },
                );
            });
    }

    fn meshtastic_message_pane(
        &mut self,
        ui: &mut egui::Ui,
        cmds: &mut Vec<Command>,
        st: &MeshtasticStatus,
        avail_h: f32,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("MESSAGES").strong().color(theme::CYAN()));
            crate::chrome::row_tail(ui, |ui| {
                crate::app::panels::clear_rx_chip_at(ui, cmds, !st.messages.is_empty());
                crate::app::panels::save_text_chip(
                    ui,
                    !st.messages.is_empty(),
                    "sdroxide-meshtastic-log.txt",
                    "Save the Meshtastic message log to a file",
                    || crate::app::save_text::meshtastic_log_text(&st.messages),
                );
            });
        });

        let h = (avail_h - 22.0).max(40.0);
        if st.messages.is_empty() {
            empty_center(ui, egui::vec2(ui.available_width(), h), "no messages yet");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("meshtastic-messages")
            .max_height(h)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_themed(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for m in &st.messages {
                    meshtastic_msg_row(ui, m);
                }
            });
    }

    fn meshtastic_map_pane(&mut self, ui: &mut egui::Ui, st: &MeshtasticStatus, now: i64, h: f32) {
        let home = {
            let grid = self.digi_cfg_edit.my_grid.trim();
            (!grid.is_empty()).then(|| sdroxide_types::grid_to_latlon(grid)).flatten()
        };
        if let Some(id) =
            crate::meshtastic_map::show(ui, &mut self.meshtastic_map, &st.nodes, home, now, h)
        {
            self.meshtastic_map.selected = Some(id);
        }
    }

    pub(in crate::app) fn meshtastic_setup_window(
        &mut self,
        ctx: &egui::Context,
        cmds: &mut Vec<Command>,
    ) {
        crate::layout::arm_center_on_open(
            self.show_meshtastic_setup,
            &mut self.meshtastic_setup_was_open,
            &mut self.meshtastic_setup_center_pending,
        );
        if !self.show_meshtastic_setup {
            return;
        }
        let mut open = self.show_meshtastic_setup;
        let mut cfg = self.state.meshtastic;
        let size =
            egui::vec2(crate::layout::window_w(ctx, 420.0), crate::layout::window_h(ctx, 320.0));
        let pending = self.meshtastic_setup_center_pending;
        let resp = crate::layout::center_on_open(
            egui::Window::new("Meshtastic Setup")
                .id(crate::layout::salted_id(ctx, "MeshtasticSetup"))
                .open(&mut open)
                .frame(crate::chrome::window_frame())
                .resizable(false)
                .default_width(size.x),
            ctx,
            pending,
            size,
        )
        .show(ctx, |ui| {
            crate::chrome::window_body_bg(ui);
            egui::Grid::new("meshtastic-cfg").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Region");
                egui::ComboBox::from_id_salt("mesh-region")
                    .selected_text(format!(
                        "{}  ({:.3} MHz)",
                        cfg.region.label(),
                        cfg.region.center_hz() / 1e6
                    ))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for r in MeshtasticRegion::ALL {
                            ui.selectable_value(
                                &mut cfg.region,
                                r,
                                format!("{} — {}", r.label(), r.description()),
                            );
                        }
                    });
                ui.end_row();

                ui.label("Channel PSK");
                egui::ComboBox::from_id_salt("mesh-psk-kind")
                    .selected_text(match cfg.psk_kind {
                        MeshtasticPskKind::Default => "Default (public)".to_string(),
                        MeshtasticPskKind::Simple => format!("simple{}", cfg.psk_simple),
                        MeshtasticPskKind::Key16 => "AES-128 hex".to_string(),
                        MeshtasticPskKind::Key32 => "AES-256 hex".to_string(),
                    })
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut cfg.psk_kind,
                            MeshtasticPskKind::Default,
                            "Default (public LongFast)",
                        );
                        ui.selectable_value(
                            &mut cfg.psk_kind,
                            MeshtasticPskKind::Simple,
                            "simpleN shorthand",
                        );
                        ui.selectable_value(
                            &mut cfg.psk_kind,
                            MeshtasticPskKind::Key16,
                            "AES-128 hex (16 bytes)",
                        );
                        ui.selectable_value(
                            &mut cfg.psk_kind,
                            MeshtasticPskKind::Key32,
                            "AES-256 hex (32 bytes)",
                        );
                    });
                ui.end_row();

                if cfg.psk_kind == MeshtasticPskKind::Simple {
                    ui.label("simpleN");
                    ui.add(egui::Slider::new(&mut cfg.psk_simple, 1..=10).text("N"));
                    ui.end_row();
                }
                if matches!(cfg.psk_kind, MeshtasticPskKind::Key16 | MeshtasticPskKind::Key32) {
                    ui.label("Key (hex)");
                    let want = if cfg.psk_kind == MeshtasticPskKind::Key16 { 16 } else { 32 };
                    let mut hex = bytes_to_hex(&cfg.psk_bytes[..want]);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut hex)
                            .desired_width(260.0)
                            .hint_text(if want == 16 { "32 hex chars" } else { "64 hex chars" }),
                    );
                    if edit.changed()
                        && let Some(raw) = parse_hex_key(&hex, want)
                    {
                        cfg.psk_bytes = [0; 32];
                        cfg.psk_bytes[..want].copy_from_slice(&raw);
                    }
                    ui.end_row();
                }
            });
            ui.separator();
            ui.label(
                RichText::new(
                    "Region parks the dial on that plan's LongFast centre. Decode is \
                     LongFast (SF11 / 250 kHz / CR 4/5) via lorarx. Default PSK is the \
                     public mesh key.",
                )
                .size(10.0)
                .weak(),
            );
        });
        if let Some(r) = &resp {
            crate::chrome::paint_window_border(ctx, &r.response);
            self.meshtastic_setup_center_pending = false;
        }
        if cfg != self.state.meshtastic {
            let region_changed = cfg.region != self.state.meshtastic.region;
            cmds.push(Command::SetMeshtasticConfig(cfg));
            if region_changed {
                cmds.push(Command::SetVfo {
                    vfo: self.state.active_vfo,
                    hz: cfg.region.center_hz(),
                });
            }
        }
        self.show_meshtastic_setup = open;
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex_key(s: &str, want: usize) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() != want * 2 {
        return None;
    }
    let mut out = Vec::with_capacity(want);
    for i in 0..want {
        out.push(u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16).ok()?);
    }
    Some(out)
}

fn slot(ui: &mut egui::Ui, w: f32, text: &str, color: egui::Color32) {
    ui.allocate_ui_with_layout(
        egui::vec2(w, 16.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(RichText::new(text).size(10.5).color(color));
        },
    );
}

/// Placeholder centered in an empty pane.
fn empty_center(ui: &mut egui::Ui, size: egui::Vec2, text: &str) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(11.0),
        theme::gray(120),
    );
}

fn meshtastic_node_row(ui: &mut egui::Ui, n: &MeshtasticNode, now: i64, selected: bool) -> bool {
    let name = if !n.short_name.is_empty() { n.short_name.as_str() } else { n.id.as_str() };
    let age = fmt_age(now - n.last_heard_unix);
    let snr = n.snr_db.map(|s| format!("{s:.0}")).unwrap_or_else(|| "—".into());
    let hops = n.hops.map(|h| format!("{h}")).unwrap_or_else(|| "—".into());
    let bat = n.battery_pct.map(|b| format!("{b}%")).unwrap_or_else(|| "—".into());
    let line = format!("{name:<8}  {age:>5}  SNR {snr:>3}  hop {hops}  {bat}");
    let ink = if selected { theme::CYAN() } else { theme::gray(180) };
    let resp = ui.add(
        egui::Label::new(RichText::new(line).monospace().size(11.0).color(ink))
            .sense(egui::Sense::click()),
    );
    resp.clicked()
}

fn meshtastic_msg_row(ui: &mut egui::Ui, m: &MeshtasticMessage) {
    let when = {
        let secs = m.unix.max(0) as u64;
        let h = (secs / 3600) % 24;
        let min = (secs / 60) % 60;
        let s = secs % 60;
        format!("{h:02}:{min:02}:{s:02}")
    };
    let to = if m.to.is_empty() { "bcast" } else { m.to.as_str() };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(when).monospace().size(10.0).weak());
        ui.label(RichText::new(&m.from).monospace().size(10.5).color(theme::CYAN()));
        ui.label(RichText::new(format!("→ {to}")).monospace().size(10.0).weak());
        if let Some(snr) = m.snr_db {
            ui.label(RichText::new(format!("{snr:.0} dB")).size(10.0).weak());
        }
    });
    if !m.text.is_empty() {
        ui.label(RichText::new(&m.text).size(11.0));
    }
    if !m.raw.is_empty() {
        ui.label(RichText::new(format!("RAW {}", m.raw)).monospace().size(9.5).weak());
    }
    ui.add_space(2.0);
}
