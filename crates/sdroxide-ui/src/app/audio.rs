//! System AUDIO panel: speakers / microphone pickers, level meters, and a
//! timed WAV recording control.
//!
//! Replaces the PUBLIC SDR system-strip chip. Opens as a floating window or
//! snaps into the System dock column like every other strip overlay.

use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke};

use sdroxide_types::Command;

use crate::app::SdroxideApp;
use crate::app::settings::general::device_combo_width;
use crate::chrome::StyledCombo;
use crate::theme;

/// Width of the left label column so every control shares one left edge.
const LABEL_W: f32 = 72.0;

impl SdroxideApp {
    pub(in crate::app) fn audio_window(&mut self, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        crate::layout::arm_center_on_open(
            self.show_audio,
            &mut self.system_was_open.audio,
            &mut self.system_center_pending.audio,
        );
        let docked = self.system_docked_as(crate::app::SystemDock::Audio);
        if !self.show_audio && !docked {
            return;
        }
        // Refresh the device list when the panel opens (enumeration is not free).
        if !self.audio_devices_queried {
            self.audio_devices = self.ctrl.audio_devices();
            self.audio_devices_queried = true;
        }

        let id = crate::layout::salted_id(ctx, "AUDIO");
        if docked {
            if let Some(win) = self.system_dock_host("AUDIO", id, crate::app::SystemDock::Audio) {
                win.show(ctx, |ui| {
                    crate::chrome::window_body_bg(ui);
                    self.clip_system_dock_body(ui, crate::app::SystemDock::Audio);
                    self.audio_body(ui, cmds);
                });
            }
            return;
        }
        let mut open = self.show_audio;
        let size =
            egui::vec2(crate::layout::window_w(ctx, 400.0), crate::layout::window_h(ctx, 360.0));
        let pending = self.system_center_pending.audio;
        let resp = crate::layout::center_on_open(
            egui::Window::new("AUDIO")
                .id(id)
                .open(&mut open)
                .frame(crate::chrome::window_frame())
                .resizable(true)
                .default_width(size.x)
                .default_height(size.y),
            ctx,
            pending,
            size,
        )
        .show(ctx, |ui| {
            crate::chrome::window_body_bg(ui);
            self.audio_body(ui, cmds);
        });
        if let Some(r) = &resp {
            crate::chrome::paint_window_border(ctx, &r.response);
            crate::layout::settle_center_pending(ctx, id, &mut self.system_center_pending.audio);
        }
        if resp.as_ref().is_some_and(|r| self.system_title_pin(ctx, &r.response, "AUDIO")) {
            self.dock_system(crate::app::SystemDock::Audio);
        }
        self.show_audio = open;
    }

    fn audio_body(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Command>) {
        let dim = |s: &str| RichText::new(s).size(10.5).color(theme::CYAN_DIM());
        let af =
            dbfs_frac(self.meters.as_ref().map(|m| m.passband_dbfs).unwrap_or(f32::NEG_INFINITY));
        let (l, r) = (af, af);
        let mic =
            self.meters.as_ref().and_then(|m| m.tx).map(|t| t.alc.clamp(0.0, 1.0)).unwrap_or(0.0);

        // One control-column width for every combo and meter so they share an edge.
        let control_w = (ui.available_width() - LABEL_W - 10.0).max(120.0);

        egui::Grid::new("audio-panel")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .min_col_width(LABEL_W)
            .show(ui, |ui| {
                match self.audio_devices.clone() {
                    Some(devs) => {
                        row_label(ui, dim("Input"));
                        let mut pick_in: Option<Option<String>> = None;
                        device_combo_width(
                            ui,
                            "audio-in",
                            &devs.inputs,
                            &devs.selected_input,
                            control_w,
                            |n| pick_in = Some(n),
                        );
                        if let Some(n) = pick_in {
                            self.ctrl.set_audio_device(false, n);
                            self.audio_devices_queried = false;
                        }
                        ui.end_row();

                        row_label(ui, dim("L"));
                        level_bar(ui, l, control_w);
                        ui.end_row();

                        row_label(ui, dim("R"));
                        level_bar(ui, r, control_w);
                        ui.end_row();

                        section_gap(ui);
                        ui.end_row();

                        row_label(ui, dim("Output"));
                        let mut pick_out: Option<Option<String>> = None;
                        device_combo_width(
                            ui,
                            "audio-out",
                            &devs.outputs,
                            &devs.selected_output,
                            control_w,
                            |n| pick_out = Some(n),
                        );
                        if let Some(n) = pick_out {
                            self.ctrl.set_audio_device(true, n);
                            self.audio_devices_queried = false;
                        }
                        ui.end_row();

                        row_label(ui, dim("Mic"));
                        level_bar(ui, mic, control_w);
                        ui.end_row();

                        row_label(ui, dim("Channels"));
                        let mut routing = devs.routing;
                        egui::ComboBox::from_id_salt("audio-routing")
                            .width(control_w)
                            .selected_text(routing.label())
                            .show_styled(ui, |ui| {
                                for r in sdroxide_types::AudioRouting::ALL {
                                    ui.selectable_value(&mut routing, r, r.label());
                                }
                            });
                        if routing != devs.routing {
                            self.ctrl.set_audio_routing(routing);
                            self.audio_devices_queried = false;
                        }
                        ui.end_row();
                    }
                    None => {
                        ui.label(
                            RichText::new(
                                "Audio devices are not switchable here — the browser or \
                                 remote host owns the sound path.",
                            )
                            .size(10.5)
                            .color(theme::CYAN_DIM()),
                        );
                        ui.end_row();
                        row_label(ui, dim("L"));
                        level_bar(ui, l, control_w);
                        ui.end_row();
                        row_label(ui, dim("R"));
                        level_bar(ui, r, control_w);
                        ui.end_row();
                        row_label(ui, dim("Mic"));
                        level_bar(ui, mic, control_w);
                        ui.end_row();
                    }
                }

                section_gap(ui);
                ui.end_row();

                // ── Recording ──────────────────────────────────────────────
                let recording = self.state.recording;
                let paused = self.state.recording_paused;
                self.sync_rec_clock(recording, paused);

                row_label(ui, RichText::new("Recording").size(11.0).strong().color(theme::CYAN()));
                ui.allocate_ui_with_layout(
                    egui::vec2(control_w, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let btn = egui::vec2(28.0, 22.0);
                        let rec_on = recording && !paused;
                        // Same breathing fill as the top-bar REC chip while a
                        // file is actually being written ("fuse" pulse).
                        let rec_fill = if rec_on {
                            crate::repaint::animate(ui.ctx());
                            crate::app::top_bar::rec_chip_fill(ui.input(|i| i.time))
                        } else {
                            theme::ALERT()
                        };
                        // Painted disc (not ●) — the bundled font has no
                        // reliable record glyph and a missing one draws as tofu.
                        // ⏹ stop / ⏸ pause stay typed; those shapes exist.
                        let rec = crate::chrome::chip_accent_sized(
                            ui,
                            rec_on,
                            RichText::new(""),
                            rec_fill,
                            Color32::WHITE,
                            btn,
                        );
                        if ui.is_rect_visible(rec.rect) {
                            let ink = if rec_on {
                                Color32::WHITE
                            } else {
                                ui.style().interact_selectable(&rec, rec_on).fg_stroke.color
                            };
                            let r = (rec.rect.height() * 0.22).clamp(3.5, 5.5);
                            ui.painter().circle_filled(rec.rect.center(), r, ink);
                        }
                        if rec
                            .on_hover_text(if paused {
                                "Resume recording"
                            } else if recording {
                                "Recording"
                            } else {
                                "Start a WAV recording — RX left, TX right (or mono if MONO is \
                             on in the top-bar REC popup)"
                            })
                            .clicked()
                        {
                            if paused {
                                cmds.push(Command::SetRecordingPause(false));
                            } else if !recording {
                                self.recording_stop_at = None;
                                self.rec_clip = None;
                                self.audio_rec_elapsed_before_pause = 0.0;
                                cmds.push(Command::SetRecordingWav(true));
                                cmds.push(Command::SetRecording(true));
                            }
                        }

                        let stop_enabled = recording;
                        let stop = ui
                            .add_enabled_ui(stop_enabled, |ui| {
                                crate::chrome::chip_sized(
                                    ui,
                                    false,
                                    RichText::new("⏹").size(12.0),
                                    btn,
                                )
                            })
                            .inner
                            .on_hover_text("Stop recording and close the file");
                        if stop_enabled && stop.clicked() {
                            cmds.push(Command::SetRecording(false));
                            self.recording_stop_at = None;
                            self.rec_clip = None;
                            self.audio_rec_since = None;
                            self.audio_rec_elapsed_before_pause = 0.0;
                        }

                        let pause_enabled = recording;
                        let pause = ui
                            .add_enabled_ui(pause_enabled, |ui| {
                                crate::chrome::chip_sized(
                                    ui,
                                    paused,
                                    RichText::new(if paused { "▶" } else { "⏸" }).size(12.0),
                                    btn,
                                )
                            })
                            .inner
                            .on_hover_text(if paused {
                                "Resume — keep writing to the same file"
                            } else {
                                "Pause — freeze the clock; the file stays open"
                            });
                        if pause_enabled && pause.clicked() {
                            cmds.push(Command::SetRecordingPause(!paused));
                        }

                        // Open folder — painted mark (not ↗ / emoji); the font
                        // has neither a folder glyph nor a reliable reveal mark.
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let folder =
                                crate::chrome::chip_sized(ui, false, RichText::new(""), btn);
                            if ui.is_rect_visible(folder.rect) {
                                paint_folder_icon(ui, &folder);
                            }
                            if folder
                                .on_hover_text("Open the folder where recordings are saved")
                                .clicked()
                            {
                                open_recordings_folder();
                            }
                        }

                        let clock = if recording {
                            fmt_rec_clock(self.rec_elapsed_secs(paused))
                        } else {
                            "00:00:00:000".into()
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(clock)
                                    .size(16.0)
                                    .strong()
                                    .color(if recording {
                                        if paused { theme::YELLOW() } else { theme::ALERT() }
                                    } else {
                                        theme::CYAN_DIM()
                                    })
                                    .monospace(),
                            );
                        });
                        if recording && !paused {
                            crate::repaint::schedule_ms(ui.ctx(), 33);
                        }
                    },
                );
                ui.end_row();

                if recording {
                    row_label(ui, dim("Level"));
                    level_bar(ui, af, control_w);
                    ui.end_row();
                }
            });

        if let Some(f) = &self.state.recording_file {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W + 10.0);
                ui.label(RichText::new(f).size(9.5).color(theme::CYAN_DIM()));
            });
        }
        if let Some((deadline, secs)) = self.recording_stop_at {
            let left = (deadline - crate::time::now_unix()).max(0);
            ui.horizontal(|ui| {
                ui.add_space(LABEL_W + 10.0);
                ui.label(
                    RichText::new(format!("Stopping in {left}s (clip {secs}s)"))
                        .size(9.5)
                        .color(theme::YELLOW()),
                );
            });
            crate::repaint::schedule_ms(ui.ctx(), 1_000);
        }
    }

    fn sync_rec_clock(&mut self, recording: bool, paused: bool) {
        if !recording {
            self.audio_rec_since = None;
            self.audio_rec_elapsed_before_pause = 0.0;
            return;
        }
        if paused {
            // Fold any running span into the frozen total (covers a pause that
            // arrived from outside this panel as well as the local button).
            if let Some(since) = self.audio_rec_since.take() {
                self.audio_rec_elapsed_before_pause +=
                    (crate::time::now_unix_f64() - since).max(0.0);
            }
            return;
        }
        if self.audio_rec_since.is_none() {
            self.audio_rec_since = Some(crate::time::now_unix_f64());
        }
    }

    fn rec_elapsed_secs(&self, paused: bool) -> f64 {
        let frozen = self.audio_rec_elapsed_before_pause;
        if paused {
            return frozen;
        }
        let live =
            self.audio_rec_since.map(|s| (crate::time::now_unix_f64() - s).max(0.0)).unwrap_or(0.0);
        frozen + live
    }
}

/// Map roughly −60‥0 dBFS onto 0‥1 for the bars.
fn dbfs_frac(dbfs: f32) -> f32 {
    ((dbfs + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Elapsed recording time as `HH:MM:SS:mmm`.
fn fmt_rec_clock(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{h:02}:{m:02}:{s:02}:{ms:03}")
}

/// Folder silhouette for the recordings-folder chip — painted like
/// [`crate::chrome::chip_link`], because the UI font has no folder glyph.
#[cfg(not(target_arch = "wasm32"))]
fn paint_folder_icon(ui: &egui::Ui, resp: &egui::Response) {
    let ink = ui.style().interact_selectable(resp, false).fg_stroke.color;
    let r = resp.rect;
    let c = r.center();
    let w = r.height() * 0.52;
    let h = r.height() * 0.34;
    let tab_w = w * 0.40;
    let tab_h = h * 0.32;
    let left = c.x - w * 0.5;
    let right = c.x + w * 0.5;
    let top = c.y - h * 0.28;
    let bottom = c.y + h * 0.55;
    let tab_right = left + tab_w;
    let tab_top = top - tab_h;
    let pts = [
        Pos2::new(left, top),
        Pos2::new(left, tab_top),
        Pos2::new(tab_right, tab_top),
        Pos2::new(tab_right + tab_h * 0.5, top),
        Pos2::new(right, top),
        Pos2::new(right, bottom),
        Pos2::new(left, bottom),
    ];
    ui.painter().add(egui::Shape::convex_polygon(pts.to_vec(), ink, Stroke::NONE));
}

#[cfg(not(target_arch = "wasm32"))]
fn open_recordings_folder() {
    let dir = match sdroxide_config::recordings_dir() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("recordings folder: {e}");
            return;
        }
    };
    let result = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer").arg(&dir).spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(&dir).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(&dir).spawn()
        }
    };
    if let Err(e) = result {
        tracing::warn!("opening recordings folder {}: {e}", dir.display());
    }
}

/// Left-aligned label in the fixed left column.
fn row_label(ui: &mut egui::Ui, text: RichText) {
    ui.allocate_ui_with_layout(
        egui::vec2(LABEL_W, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(text);
        },
    );
}

/// Vertical gap between Input / Output / Recording blocks (grid-safe: no `add_space`).
fn section_gap(ui: &mut egui::Ui) {
    ui.allocate_exact_size(egui::vec2(LABEL_W, 12.0), Sense::hover());
    ui.allocate_exact_size(egui::vec2(1.0, 12.0), Sense::hover());
}

/// One horizontal VU bar filling `width`.
fn level_bar(ui: &mut egui::Ui, frac: f32, width: f32) {
    let row_h = 8.0;
    let bar_h = 4.0;
    let w = width.max(40.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, row_h), Sense::hover());
    let painter = ui.painter_at(rect);
    let bar = egui::Rect::from_center_size(rect.center(), egui::vec2(w, bar_h));
    painter.rect_filled(bar, 1.0, theme::gray(40));
    let fill_w = bar.width() * frac.clamp(0.0, 1.0);
    if fill_w > 0.5 {
        let fill = egui::Rect::from_min_size(bar.min, egui::vec2(fill_w, bar.height()));
        let color = if frac > 0.9 {
            theme::ALERT()
        } else if frac > 0.7 {
            theme::YELLOW()
        } else {
            theme::GREEN()
        };
        painter.rect_filled(fill, 1.0, color);
    }
    painter.rect_stroke(bar, 1.0, Stroke::new(1.0, theme::gray(80)), egui::StrokeKind::Outside);
}
