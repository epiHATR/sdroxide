//! Background scheduled decode tasks (System TASKS chip).
//!
//! Create calendar windows of mode + frequency; the engine runs the winning
//! task when the radio is free, even if that mode's panel is not open.
//! Schedules may be one-shot or recurring (daily / weekly / interval).

use eframe::egui::{self, RichText};
use sdroxide_types::{
    BackgroundTask, BackgroundTaskSchedule, BackgroundTaskStatus, BackgroundTaskTz, Command, Mode,
    WEEKDAY_ALL, WEEKDAY_FRI, WEEKDAY_MON, WEEKDAY_SAT, WEEKDAY_SUN, WEEKDAY_THU, WEEKDAY_TUE,
    WEEKDAY_WED,
};

use crate::app::SdroxideApp;
use crate::theme;

impl SdroxideApp {
    pub(in crate::app) fn tasks_window(&mut self, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        crate::layout::arm_center_on_open(
            self.show_tasks,
            &mut self.system_was_open.tasks,
            &mut self.system_center_pending.tasks,
        );
        if !self.show_tasks && !self.system_docked_as(crate::app::SystemDock::Tasks) {
            return;
        }
        let id = crate::layout::salted_id(ctx, "Tasks");
        if self.system_docked_as(crate::app::SystemDock::Tasks) {
            if let Some(win) = self.system_dock_host("TASKS", id, crate::app::SystemDock::Tasks) {
                win.show(ctx, |ui| {
                    crate::chrome::window_body_bg(ui);
                    self.clip_system_dock_body(ui, crate::app::SystemDock::Tasks);
                    self.tasks_body(ui, cmds);
                });
            }
            return;
        }
        let mut open = self.show_tasks;
        const WANT_W: f32 = 680.0;
        const WANT_H: f32 = 620.0;
        let size =
            egui::vec2(crate::layout::window_w(ctx, WANT_W), crate::layout::window_h(ctx, WANT_H));
        let pending = self.system_center_pending.tasks;
        let resp = crate::layout::center_on_open(
            egui::Window::new("Background tasks")
                .id(id)
                .open(&mut open)
                .frame(crate::chrome::window_frame())
                .resizable(true)
                .default_width(size.x)
                .default_height(size.y)
                .min_width(440.0)
                .min_height(360.0)
                .max_width(WANT_W)
                .max_height(WANT_H),
            ctx,
            pending,
            size,
        )
        .show(ctx, |ui| {
            crate::chrome::window_body_bg(ui);
            self.tasks_body(ui, cmds);
        });
        if let Some(r) = &resp {
            crate::chrome::paint_window_border(ctx, &r.response);
            crate::layout::settle_center_pending(ctx, id, &mut self.system_center_pending.tasks);
        }
        if resp.as_ref().is_some_and(|r| self.system_title_pin(ctx, &r.response, "Tasks")) {
            self.dock_system(crate::app::SystemDock::Tasks);
        }
        self.show_tasks = open;
    }

    fn tasks_body(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Command>) {
        let editing = self.tasks_edit.is_some();

        ui.horizontal(|ui| {
            if !editing
                && crate::chrome::chip(ui, false, RichText::new("+ NEW TASK").size(11.0))
                    .on_hover_text("Create a scheduled decode window")
                    .clicked()
            {
                self.tasks_edit = Some(self.new_task_draft());
            }
            if crate::chrome::chip(ui, false, "EXPORT")
                .on_hover_text("Save the current task list as JSON")
                .clicked()
            {
                self.export_background_tasks();
            }
            if crate::chrome::chip(ui, false, "IMPORT")
                .on_hover_text(
                    "Load tasks from a JSON file — matching schedules replace existing ones",
                )
                .clicked()
            {
                crate::download::load_text(
                    "Background tasks JSON",
                    &["json"],
                    self.tasks_import_inbox.clone(),
                );
            }
            crate::chrome::row_tail(ui, |ui| {
                if let Some(note) = &self.tasks_transfer_note {
                    ui.label(RichText::new(note).size(10.0).weak());
                }
                let n = self.background_tasks.tasks.len();
                let running = self
                    .background_tasks
                    .tasks
                    .iter()
                    .any(|t| t.status == BackgroundTaskStatus::Running);
                ui.label(
                    RichText::new(if running {
                        format!("{n} task(s) · running")
                    } else {
                        format!("{n} task(s)")
                    })
                    .size(11.0)
                    .weak(),
                );
            });
        });
        ui.separator();

        // List stays visible while creating / editing — form sits underneath.
        let list_h = if editing { 160.0 } else { ui.available_height().max(120.0) };
        egui::ScrollArea::vertical().id_salt("tasks-list").max_height(list_h).show(ui, |ui| {
            if self.background_tasks.tasks.is_empty() {
                ui.label(RichText::new("No scheduled tasks yet.").weak());
                return;
            }
            let mut delete_id = None;
            let mut edit_id = None;
            let mut clone_id = None;
            for t in &self.background_tasks.tasks {
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(fmt_schedule_summary(t)).monospace().size(11.0).strong(),
                        );
                        ui.label(RichText::new(t.mode.label()).size(11.0).color(theme::CYAN()));
                        ui.label(RichText::new(fmt_freq(t.freq_hz)).monospace().size(11.0));
                        // row_tail is right-to-left: add DEL first so the
                        // visible order is [status] [EDIT] [clone] [DEL].
                        crate::chrome::row_tail(ui, |ui| {
                            if crate::chrome::chip(ui, false, "DEL").clicked() {
                                delete_id = Some(t.id);
                            }
                            let clone_sz = egui::vec2(22.0, 18.0);
                            if crate::chrome::chip_clone(ui, clone_sz)
                                .on_hover_text("Clone this task")
                                .clicked()
                            {
                                clone_id = Some(t.id);
                            }
                            if crate::chrome::chip(ui, false, "EDIT").clicked() {
                                edit_id = Some(t.id);
                            }
                            status_chip(ui, t.status);
                        });
                    });
                });
                ui.add_space(2.0);
            }
            if let Some(id) = edit_id {
                self.tasks_edit = self.background_tasks.tasks.iter().find(|t| t.id == id).cloned();
            }
            if let Some(id) = clone_id {
                if let Some(src) = self.background_tasks.tasks.iter().find(|t| t.id == id).cloned()
                {
                    let mut cfg = self.background_tasks.clone();
                    let new_id = cfg
                        .next_id
                        .max(cfg.tasks.iter().map(|t| t.id).max().unwrap_or(0).saturating_add(1));
                    let mut copy = src;
                    copy.id = new_id;
                    copy.status = BackgroundTaskStatus::Scheduled;
                    copy.updated_unix = crate::time::now_unix();
                    cfg.tasks.push(copy.sane());
                    cfg.next_id = new_id.saturating_add(1);
                    cfg.tasks.sort_by_key(|t| t.start_unix);
                    self.background_tasks = cfg.clone();
                    cmds.push(Command::SetBackgroundTasks(cfg));
                }
            }
            if let Some(id) = delete_id {
                let mut cfg = self.background_tasks.clone();
                cfg.tasks.retain(|t| t.id != id);
                if cfg.active_id == Some(id) {
                    cfg.active_id = None;
                }
                self.background_tasks = cfg.clone();
                cmds.push(Command::SetBackgroundTasks(cfg));
            }
        });

        if self.tasks_edit.is_some() {
            ui.separator();
            self.tasks_editor(ui, cmds);
        }
    }

    fn tasks_editor(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Command>) {
        let Some(mut draft) = self.tasks_edit.clone() else { return };
        let is_new = !self.background_tasks.tasks.iter().any(|t| t.id == draft.id);

        ui.label(RichText::new(if is_new { "New task" } else { "Edit task" }).strong().size(13.0));

        // Mode — Frequency
        ui.horizontal(|ui| {
            ui.label("Mode");
            egui::ComboBox::from_id_salt("task-mode")
                .selected_text(draft.mode.label())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for m in Mode::ALL {
                        ui.selectable_value(&mut draft.mode, m, m.label());
                    }
                });
            ui.label("—");
            ui.label("Frequency");
            let mut mhz = draft.freq_hz / 1e6;
            if ui
                .add(egui::DragValue::new(&mut mhz).speed(0.001).suffix(" MHz").max_decimals(6))
                .changed()
            {
                draft.freq_hz = (mhz * 1e6).clamp(1.0, 6_000_000_000.0);
            }
        });

        // Type
        ui.horizontal(|ui| {
            ui.label("Type");
            egui::ComboBox::from_id_salt("task-schedule")
                .selected_text(draft.schedule.label())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for s in BackgroundTaskSchedule::ALL {
                        ui.selectable_value(&mut draft.schedule, s, s.label());
                    }
                });
        });

        // Start / end | duration (and recurrence extras)
        match draft.schedule {
            BackgroundTaskSchedule::Once => {
                ui.horizontal(|ui| {
                    ui.label("Start");
                    draft.start_unix = edit_unix(ui, "task-start", draft.start_unix, &mut draft.tz);
                });
                ui.horizontal(|ui| {
                    ui.label("End");
                    draft.end_unix = edit_unix(ui, "task-end", draft.end_unix, &mut draft.tz);
                });
                draft.end_unix = draft.end_unix.max(draft.start_unix + 60);
                ui.label(
                    RichText::new(format!(
                        "{} → {}",
                        fmt_wall(draft.start_unix, draft.tz),
                        fmt_wall(draft.end_unix, draft.tz)
                    ))
                    .weak()
                    .size(11.0),
                );
                duration_presets(ui, &mut draft);
            }
            BackgroundTaskSchedule::Daily | BackgroundTaskSchedule::Weekly => {
                ui.horizontal(|ui| {
                    ui.label("Start");
                    draft.start_unix = edit_unix(ui, "task-start", draft.start_unix, &mut draft.tz);
                });
                let mut dur_min = (draft.duration_secs() / 60).max(1);
                ui.horizontal(|ui| {
                    ui.label("Duration");
                    if ui
                        .add(egui::DragValue::new(&mut dur_min).range(1..=24 * 60).suffix(" min"))
                        .changed()
                    {
                        draft.end_unix = draft.start_unix + dur_min * 60;
                    }
                });
                if draft.schedule == BackgroundTaskSchedule::Weekly {
                    if draft.weekdays == 0 {
                        draft.weekdays = WEEKDAY_ALL;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Days");
                        for (bit, label) in [
                            (WEEKDAY_SUN, "Sun"),
                            (WEEKDAY_MON, "Mon"),
                            (WEEKDAY_TUE, "Tue"),
                            (WEEKDAY_WED, "Wed"),
                            (WEEKDAY_THU, "Thu"),
                            (WEEKDAY_FRI, "Fri"),
                            (WEEKDAY_SAT, "Sat"),
                        ] {
                            let on = draft.weekdays & bit != 0;
                            if crate::chrome::chip(ui, on, label).clicked() {
                                draft.weekdays ^= bit;
                            }
                        }
                    });
                }
                until_row(ui, &mut draft);
            }
            BackgroundTaskSchedule::Interval => {
                ui.horizontal(|ui| {
                    ui.label("Start");
                    draft.start_unix = edit_unix(ui, "task-start", draft.start_unix, &mut draft.tz);
                });
                let mut dur_min = (draft.duration_secs() / 60).max(1);
                ui.horizontal(|ui| {
                    ui.label("Duration");
                    if ui
                        .add(egui::DragValue::new(&mut dur_min).range(1..=24 * 60).suffix(" min"))
                        .changed()
                    {
                        draft.end_unix = draft.start_unix + dur_min * 60;
                    }
                });
                if draft.period_secs == 0 {
                    draft.period_secs = 3600;
                }
                let mut period_min = (draft.period_secs / 60).max(1) as i64;
                ui.horizontal(|ui| {
                    ui.label("Every");
                    if ui
                        .add(
                            egui::DragValue::new(&mut period_min)
                                .range(1..=7 * 24 * 60)
                                .suffix(" min"),
                        )
                        .changed()
                    {
                        draft.period_secs = (period_min as u64) * 60;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Presets").weak().size(10.0));
                    for (label, secs) in [
                        ("1h", 3600u64),
                        ("2h", 7200),
                        ("6h", 21600),
                        ("12h", 43200),
                        ("1d", 86400),
                    ] {
                        if crate::chrome::chip(ui, draft.period_secs == secs, label).clicked() {
                            draft.period_secs = secs;
                        }
                    }
                });
                until_row(ui, &mut draft);
            }
        }

        ui.add_space(8.0);
        let mut cancel = false;
        let mut save = false;
        ui.horizontal(|ui| {
            if crate::chrome::chip(ui, false, "Cancel").clicked() {
                cancel = true;
            }
            if crate::chrome::chip_accent(
                ui,
                true,
                if is_new { "CREATE" } else { "SAVE" },
                theme::GREEN(),
                theme::INK_ON_BRIGHT(),
            )
            .clicked()
            {
                save = true;
            }
        });
        if cancel {
            self.tasks_edit = None;
            return;
        }
        if save {
            draft = draft.sane();
            draft.updated_unix = crate::time::now_unix();
            draft.device = self.task_device_label();
            let mut cfg = self.background_tasks.clone();
            if let Some(slot) = cfg.tasks.iter_mut().find(|t| t.id == draft.id) {
                *slot = draft.clone();
            } else {
                cfg.tasks.push(draft.clone());
                cfg.next_id = cfg.next_id.max(draft.id.saturating_add(1));
            }
            cfg.tasks.sort_by_key(|t| t.start_unix);
            self.background_tasks = cfg.clone();
            cmds.push(Command::SetBackgroundTasks(cfg));
            self.tasks_edit = None;
        } else {
            self.tasks_edit = Some(draft);
        }
    }

    fn export_background_tasks(&mut self) {
        match self.background_tasks.export_json() {
            Ok(json) => {
                crate::download::save("sdroxide-tasks.json", json.as_bytes());
                self.tasks_transfer_note = Some(format!(
                    "Exported {} task(s)",
                    self.background_tasks.tasks.len()
                ));
            }
            Err(e) => {
                self.tasks_transfer_note = Some(format!("Export failed: {e}"));
            }
        }
    }

    /// Drain a JSON file chosen via IMPORT. Overlaps open a confirm; clean
    /// adds merge immediately and show a success modal.
    pub(in crate::app) fn poll_tasks_import(&mut self, _cmds: &mut Vec<Command>) {
        let loaded = self.tasks_import_inbox.lock().ok().and_then(|mut g| g.take());
        let Some(outcome) = loaded else { return };
        let text = match outcome {
            Ok(loaded) => loaded.text,
            Err(e) => {
                self.tasks_transfer_note = Some(format!("Import failed: {e}"));
                return;
            }
        };
        let imported = match sdroxide_types::BackgroundTasksConfig::parse_import_json(&text) {
            Ok(t) => t,
            Err(e) => {
                self.tasks_transfer_note = Some(format!("Import failed: {e}"));
                return;
            }
        };
        if imported.is_empty() {
            self.tasks_transfer_note = Some("Import: file had no tasks".into());
            return;
        }
        let (_added, replaced) = self.background_tasks.import_preview(&imported);
        if replaced > 0 {
            self.tasks_import_pending = Some(imported);
        } else {
            self.commit_tasks_import(imported, _cmds);
        }
    }

    fn commit_tasks_import(&mut self, imported: Vec<sdroxide_types::BackgroundTask>, cmds: &mut Vec<Command>) {
        let device = self.caps.is_some().then(|| self.task_device_label());
        let mut cfg = self.background_tasks.clone();
        let (added, replaced) =
            cfg.merge_import(imported, device.as_deref(), crate::time::now_unix());
        self.background_tasks = cfg.clone();
        cmds.push(Command::SetBackgroundTasks(cfg));
        self.tasks_transfer_note = Some(format!("Imported: {added} added, {replaced} replaced"));
        self.tasks_import_done = Some((added, replaced));
        self.tasks_import_pending = None;
    }

    /// Confirm replace / acknowledge successful import. Call after floating
    /// system popups so Settings cannot cover the question.
    pub(in crate::app) fn resolve_tasks_import_modals(
        &mut self,
        ctx: &egui::Context,
        cmds: &mut Vec<Command>,
    ) {
        if let Some(imported) = self.tasks_import_pending.clone() {
            let (added, replaced) = self.background_tasks.import_preview(&imported);
            let body = format!(
                "Import {n} task(s)?\n\n\
                 {added} new · {replaced} will replace an existing schedule \
                 (same type, mode, frequency, and window).",
                n = imported.len(),
            );
            match crate::chrome::confirm_modal(ctx, "tasks-import-replace", &body, "Import") {
                Some(crate::chrome::ConfirmChoice::Confirm) => {
                    self.commit_tasks_import(imported, cmds);
                }
                Some(crate::chrome::ConfirmChoice::Cancel) => {
                    self.tasks_import_pending = None;
                    self.tasks_transfer_note = Some("Import cancelled".into());
                }
                None => {}
            }
        }

        if let Some((added, replaced)) = self.tasks_import_done {
            let body = if replaced == 0 {
                format!("Imported {added} task(s).")
            } else if added == 0 {
                format!("Replaced {replaced} existing task(s).")
            } else {
                format!("Imported {added} new task(s) and replaced {replaced}.")
            };
            let answered = crate::chrome::centered_modal(ctx, "tasks-import-done", (), |ui| {
                ui.label(RichText::new(body).size(11.5));
                ui.add_space(4.0);
                crate::chrome::chip(ui, false, "OK").clicked().then_some(())
            });
            if answered.is_some() {
                self.tasks_import_done = None;
            }
        }
    }

    fn new_task_draft(&self) -> BackgroundTask {
        let now = crate::time::now_unix();
        let id = self.background_tasks.next_id.max(
            self.background_tasks.tasks.iter().map(|t| t.id).max().unwrap_or(0).saturating_add(1),
        );
        BackgroundTask {
            id,
            start_unix: now,
            end_unix: now + 3600,
            mode: self.state.rx[0].mode,
            freq_hz: self.state.rx_freq_hz(),
            device: self.task_device_label(),
            status: BackgroundTaskStatus::Scheduled,
            updated_unix: now,
            schedule: BackgroundTaskSchedule::Once,
            weekdays: WEEKDAY_ALL,
            period_secs: 3600,
            until_unix: 0,
            tz: BackgroundTaskTz::Local,
        }
        .sane()
    }

    fn task_device_label(&self) -> String {
        self.caps
            .as_ref()
            .map(|c| {
                if !c.label.trim().is_empty() {
                    c.label.clone()
                } else if !c.driver.trim().is_empty() {
                    c.driver.clone()
                } else {
                    String::new()
                }
            })
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("radio {}", self.radio_id.saturating_add(1)))
    }
}

fn duration_presets(ui: &mut egui::Ui, draft: &mut BackgroundTask) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("End +").weak().size(10.0));
        for (label, secs) in [("30m", 1800i64), ("1h", 3600), ("2h", 7200), ("4h", 14400)] {
            if crate::chrome::chip(ui, false, label).clicked() {
                let now = crate::time::now_unix();
                if draft.start_unix < now {
                    draft.start_unix = now;
                }
                draft.end_unix = draft.start_unix + secs;
            }
        }
        if crate::chrome::chip(ui, false, "now→+1h").clicked() {
            let now = crate::time::now_unix();
            draft.start_unix = now;
            draft.end_unix = now + 3600;
        }
    });
}

fn until_row(ui: &mut egui::Ui, draft: &mut BackgroundTask) {
    let forever = draft.until_unix == 0;
    ui.horizontal(|ui| {
        ui.label("Until");
        if crate::chrome::chip(ui, forever, "forever").clicked() {
            draft.until_unix = 0;
        }
        if !forever {
            draft.until_unix = edit_unix(ui, "task-until", draft.until_unix, &mut draft.tz);
        } else if crate::chrome::chip(ui, false, "set end…").clicked() {
            draft.until_unix = draft.start_unix + 7 * 86_400;
        }
    });
    if draft.until_unix > 0 {
        ui.label(RichText::new(fmt_wall(draft.until_unix, draft.tz)).weak().size(11.0));
    }
}

fn status_chip(ui: &mut egui::Ui, status: BackgroundTaskStatus) {
    let (fill, ink) = match status {
        BackgroundTaskStatus::Running => (theme::GREEN(), theme::INK_ON_BRIGHT()),
        BackgroundTaskStatus::Waiting => (theme::scope().warn, theme::INK_ON_BRIGHT()),
        BackgroundTaskStatus::Done => (theme::gray(80), theme::TEXT_STRONG()),
        BackgroundTaskStatus::Scheduled => (theme::CYAN(), theme::INK_ON_BRIGHT()),
    };
    crate::chrome::chip_accent(ui, true, status.label(), fill, ink);
}

fn fmt_freq(hz: f64) -> String {
    format!("{:.6} MHz", hz / 1e6)
}

fn fmt_schedule_summary(t: &BackgroundTask) -> String {
    let dur = t.duration_secs();
    match t.schedule {
        BackgroundTaskSchedule::Once => {
            format!("{} → {}", fmt_wall(t.start_unix, t.tz), fmt_wall(t.end_unix, t.tz))
        }
        BackgroundTaskSchedule::Daily => {
            format!("daily {} · {}", fmt_tod(t.start_unix, t.tz), fmt_duration(dur))
        }
        BackgroundTaskSchedule::Weekly => {
            format!(
                "{} {} · {}",
                fmt_weekdays(t.weekdays),
                fmt_tod(t.start_unix, t.tz),
                fmt_duration(dur)
            )
        }
        BackgroundTaskSchedule::Interval => {
            format!(
                "every {} · {} from {}",
                fmt_duration(t.period_secs as i64),
                fmt_duration(dur),
                fmt_wall(t.start_unix, t.tz)
            )
        }
    }
}

fn fmt_wall(unix: i64, tz: BackgroundTaskTz) -> String {
    let (y, mo, d, h, m) = civil_parts(wall_secs(unix, tz));
    match tz {
        BackgroundTaskTz::Utc => format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}Z"),
        BackgroundTaskTz::Local => format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}"),
    }
}

fn fmt_tod(unix: i64, tz: BackgroundTaskTz) -> String {
    let rem = wall_secs(unix, tz).rem_euclid(86_400);
    match tz {
        BackgroundTaskTz::Utc => format!("{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60),
        BackgroundTaskTz::Local => format!("{:02}:{:02}", rem / 3600, (rem % 3600) / 60),
    }
}

fn wall_secs(unix: i64, tz: BackgroundTaskTz) -> i64 {
    match tz {
        BackgroundTaskTz::Utc => unix,
        BackgroundTaskTz::Local => unix + crate::time::local_offset_at(unix),
    }
}

fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs % 86_400 == 0 && secs >= 86_400 {
        format!("{}d", secs / 86_400)
    } else if secs % 3600 == 0 && secs >= 3600 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn fmt_weekdays(mask: u8) -> String {
    let days = if mask == 0 { WEEKDAY_ALL } else { mask };
    if days == WEEKDAY_ALL {
        return "all days".into();
    }
    let labels = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
    let mut out = String::new();
    for (i, label) in labels.iter().enumerate() {
        if days & (1 << i) != 0 {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(label);
        }
    }
    out
}

/// Edit a Unix instant as civil fields in `tz`. The zone dropdown sits after
/// date/hour/minute; toggling it keeps the absolute instant and redraws digits.
fn edit_unix(ui: &mut egui::Ui, salt: &str, unix: i64, tz: &mut BackgroundTaskTz) -> i64 {
    let (mut y, mut mo, mut d, mut h, mut mi) = civil_parts(wall_secs(unix, *tz));
    ui.add(egui::DragValue::new(&mut y).range(1970..=2100).prefix("Y "));
    ui.add(egui::DragValue::new(&mut mo).range(1..=12).prefix("M "));
    ui.add(egui::DragValue::new(&mut d).range(1..=31).prefix("D "));
    ui.add(egui::DragValue::new(&mut h).range(0..=23).prefix("h "));
    ui.add(egui::DragValue::new(&mut mi).range(0..=59).prefix("m "));
    egui::ComboBox::from_id_salt(salt).selected_text(tz.label()).width(88.0).show_ui(ui, |ui| {
        for z in BackgroundTaskTz::ALL {
            ui.selectable_value(tz, z, z.label());
        }
    });
    let wall = days_from_civil(y, mo as u32, d as u32) * 86_400 + h * 3600 + mi * 60;
    match *tz {
        BackgroundTaskTz::Utc => wall,
        BackgroundTaskTz::Local => {
            // Offset depends on the instant (DST). One refinement is enough.
            let approx = wall - crate::time::local_offset_at(unix);
            wall - crate::time::local_offset_at(approx)
        }
    }
}

fn civil_parts(unix: i64) -> (i64, i64, i64, i64, i64) {
    let secs = unix.max(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let (y, mo, d) = civil_from_days(days);
    (y, mo as i64, d as i64, h, m)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant's algorithms.
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let m = m.clamp(1, 12);
    let d = d.clamp(1, 31);
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}
