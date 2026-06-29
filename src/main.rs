//! GUI layer of Stow (egui/eframe), styled after the StowWindow design spec.
//!
//! A "Source" drag-and-drop zone for folders to pack, and a destination that is
//! either a local folder or an SSH/SFTP target. The heavy work lives in the
//! core (`file_transfer` lib); this layer only renders and forwards commands.

mod theme;

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, Sense, Stroke, Vec2};

use file_transfer::core::engine::Event;
use file_transfer::core::secret;
use file_transfer::{
    Config, DestinationSpec, Engine, JobId, JobSpec, JobStatus, SshAuth, SshConfig,
};

use theme::{
    bold, extra, med, mono, reg, semi, ACCENT, ACCENT_DEEP, ERROR, INK, INK2, LINE, LINE_SOFT,
    MUTED, MUTED_SOFT, SUCCESS, SURFACE, SURFACE2,
};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 660.0])
            .with_min_inner_size([600.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Stow — pack and move",
        options,
        Box::new(|cc| {
            theme::install_fonts(&cc.egui_ctx);
            theme::apply_style(&cc.egui_ctx);
            Ok(Box::new(App::new(cc)))
        }),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum DestKind {
    Local,
    Ssh,
}

#[derive(PartialEq, Clone, Copy)]
enum AuthKind {
    Agent,
    Password,
    Key,
}

struct JobView {
    id: JobId,
    title: String,
    status: JobStatus,
}

struct App {
    engine: Engine,
    events: Receiver<Event>,

    sources: Vec<PathBuf>,
    remove_source: bool,
    checksum: bool,

    dest_kind: DestKind,
    dest: Option<PathBuf>,
    ssh_host: String,
    ssh_port: String,
    ssh_user: String,
    ssh_remote_dir: String,
    ssh_auth: AuthKind,
    ssh_secret: String,
    ssh_key_path: String,

    jobs: Vec<JobView>,
    log: Vec<String>,

    source_rect: egui::Rect,
    dest_rect: egui::Rect,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (evt_tx, evt_rx) = unbounded::<Event>();
        let ctx = cc.egui_ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let engine = Engine::start(Config::default(), evt_tx, wake);

        Self {
            engine,
            events: evt_rx,
            sources: Vec::new(),
            remove_source: false,
            checksum: false,
            dest_kind: DestKind::Local,
            dest: None,
            ssh_host: String::new(),
            ssh_port: "22".to_string(),
            ssh_user: String::new(),
            ssh_remote_dir: String::new(),
            ssh_auth: AuthKind::Agent,
            ssh_secret: String::new(),
            ssh_key_path: String::new(),
            jobs: Vec::new(),
            log: Vec::new(),
            source_rect: egui::Rect::NOTHING,
            dest_rect: egui::Rect::NOTHING,
        }
    }

    // ── core wiring (unchanged behavior) ──────────────────────────────────────
    fn drain_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::Accepted { id, source } => {
                    let title = source
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| source.display().to_string());
                    self.jobs.push(JobView {
                        id,
                        title,
                        status: JobStatus::Queued,
                    });
                }
                Event::Progress { id, status } => self.set_status(id, status),
                Event::Finished { id, status } => self.set_status(id, status),
                Event::Log { id, line } => {
                    let prefix = id.map(|i| format!("{i} ")).unwrap_or_default();
                    self.log.push(format!("{prefix}{line}"));
                }
            }
        }
    }

    fn set_status(&mut self, id: JobId, status: JobStatus) {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.status = status;
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let to_dest = pos.map(|p| self.dest_rect.contains(p)).unwrap_or(false);

        for path in dropped {
            if !path.is_dir() {
                self.log
                    .push(format!("skipped (not a folder): {}", path.display()));
                continue;
            }
            if to_dest {
                self.dest = Some(path);
            } else if !self.sources.contains(&path) {
                self.sources.push(path);
            }
        }
    }

    fn build_destination(&self) -> Result<DestinationSpec, String> {
        match self.dest_kind {
            DestKind::Local => {
                let dir = self.dest.clone().ok_or("no destination folder")?;
                Ok(DestinationSpec::Local { dir })
            }
            DestKind::Ssh => {
                let host = self.ssh_host.trim();
                let user = self.ssh_user.trim();
                let remote = self.ssh_remote_dir.trim();
                if host.is_empty() || user.is_empty() || remote.is_empty() {
                    return Err("host, user and remote directory are required".into());
                }
                let port: u16 = if self.ssh_port.trim().is_empty() {
                    22
                } else {
                    self.ssh_port
                        .trim()
                        .parse()
                        .map_err(|_| "invalid port".to_string())?
                };
                let account = format!("{user}@{host}:{port}");
                let auth = match self.ssh_auth {
                    AuthKind::Agent => SshAuth::Agent,
                    AuthKind::Password => {
                        if self.ssh_secret.is_empty() {
                            return Err("password is empty".into());
                        }
                        secret::store(&account, &self.ssh_secret).map_err(|e| e.to_string())?;
                        SshAuth::Password {
                            keychain_account: account,
                        }
                    }
                    AuthKind::Key => {
                        if self.ssh_key_path.trim().is_empty() {
                            return Err("key file path is required".into());
                        }
                        let passphrase_account = if self.ssh_secret.is_empty() {
                            None
                        } else {
                            let acc = format!("{account}/key");
                            secret::store(&acc, &self.ssh_secret).map_err(|e| e.to_string())?;
                            Some(acc)
                        };
                        SshAuth::Key {
                            path: PathBuf::from(self.ssh_key_path.trim()),
                            passphrase_account,
                        }
                    }
                };
                Ok(DestinationSpec::Ssh(SshConfig {
                    host: host.to_string(),
                    port,
                    user: user.to_string(),
                    remote_dir: remote.to_string(),
                    auth,
                }))
            }
        }
    }

    fn can_start(&self) -> bool {
        if self.sources.is_empty() {
            return false;
        }
        match self.dest_kind {
            DestKind::Local => self.dest.is_some(),
            DestKind::Ssh => {
                !self.ssh_host.trim().is_empty()
                    && !self.ssh_user.trim().is_empty()
                    && !self.ssh_remote_dir.trim().is_empty()
                    && (self.ssh_auth != AuthKind::Key || !self.ssh_key_path.trim().is_empty())
            }
        }
    }

    fn start_jobs(&mut self) {
        let dest = match self.build_destination() {
            Ok(d) => d,
            Err(e) => {
                self.log.push(format!("destination error: {e}"));
                return;
            }
        };
        for source in std::mem::take(&mut self.sources) {
            self.engine.submit(JobSpec {
                source,
                destination: dest.clone(),
                remove_source: self.remove_source,
                checksum: self.checksum,
            });
        }
    }

    // ── views ─────────────────────────────────────────────────────────────────
    fn source_column(&mut self, ui: &mut egui::Ui, hovering: bool, pointer: Option<egui::Pos2>) {
        ui.horizontal(|ui| {
            ui.label(bold("SOURCE", 11.0, MUTED));
            if !self.sources.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(semi(format!("{} folders", self.sources.len()), 12.0, MUTED_SOFT));
                });
            }
        });
        ui.add_space(4.0);

        let hi = hovering
            && pointer.map(|p| self.source_rect.contains(p)).unwrap_or(false);
        let mut remove = None;
        let resp = theme::drop_frame(hi).show(ui, |ui| {
            ui.set_min_height(148.0);
            ui.set_width(ui.available_width());
            if self.sources.is_empty() {
                empty_hint(ui, "Drop folders to pack here");
            } else {
                for (i, p) in self.sources.iter().enumerate() {
                    if item_row(ui, &folder_name(p)) {
                        remove = Some(i);
                    }
                }
            }
        });
        self.source_rect = resp.response.rect;
        if let Some(i) = remove {
            self.sources.remove(i);
        }

        ui.add_space(8.0);
        if outline_button(ui, "+  Add folders…").clicked() {
            if let Some(dirs) = rfd::FileDialog::new().pick_folders() {
                for d in dirs {
                    if d.is_dir() && !self.sources.contains(&d) {
                        self.sources.push(d);
                    }
                }
            }
        }
    }

    fn destination_column(&mut self, ui: &mut egui::Ui, hovering: bool, pointer: Option<egui::Pos2>) {
        ui.horizontal(|ui| {
            ui.label(bold("DESTINATION", 11.0, MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let sel = if self.dest_kind == DestKind::Local { 0 } else { 1 };
                if let Some(i) = segmented(ui, &["Local", "SSH"], sel, false) {
                    self.dest_kind = if i == 0 { DestKind::Local } else { DestKind::Ssh };
                }
            });
        });
        ui.add_space(4.0);

        match self.dest_kind {
            DestKind::Local => {
                let hi = hovering
                    && pointer.map(|p| self.dest_rect.contains(p)).unwrap_or(false);
                let mut clear = false;
                let resp = theme::drop_frame(hi).show(ui, |ui| {
                    ui.set_min_height(148.0);
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.add_space(28.0);
                        if let Some(dir) = self.dest.clone() {
                            ui.label(semi("📁", 22.0, ACCENT));
                            ui.add_space(4.0);
                            ui.label(bold(folder_name(&dir), 13.5, INK));
                            ui.label(mono(dir.display().to_string(), 11.0, MUTED_SOFT));
                            ui.add_space(6.0);
                            if small_ghost(ui, "✕ remove") {
                                clear = true;
                            }
                        } else {
                            ui.label(reg("📁", 26.0, MUTED_SOFT));
                            ui.add_space(6.0);
                            ui.label(med("Drop the folder for archives here", 13.5, MUTED_SOFT));
                        }
                    });
                });
                self.dest_rect = resp.response.rect;
                if clear {
                    self.dest = None;
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    if ui
                        .add_sized([w * 0.62, 38.0], outline("📂  Choose folder…"))
                        .clicked()
                    {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.dest = Some(d);
                        }
                    }
                    if ui
                        .add_sized([ui.available_width(), 38.0], ghost("Clear"))
                        .clicked()
                    {
                        self.dest = None;
                    }
                });
            }
            DestKind::Ssh => {
                self.dest_rect = egui::Rect::NOTHING;
                self.ssh_form(ui);
            }
        }
    }

    fn ssh_form(&mut self, ui: &mut egui::Ui) {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 9.0;

            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                field(ui, "HOST", &mut self.ssh_host, w);
                field(ui, "PORT", &mut self.ssh_port, ui.available_width());
            });
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                field(ui, "USER", &mut self.ssh_user, w);
                field(ui, "REMOTE DIR", &mut self.ssh_remote_dir, ui.available_width());
            });

            ui.label(bold("AUTH", 10.5, MUTED_SOFT));
            let sel = match self.ssh_auth {
                AuthKind::Agent => 0,
                AuthKind::Password => 1,
                AuthKind::Key => 2,
            };
            if let Some(i) = segmented(ui, &["Agent", "Password", "Key"], sel, true) {
                self.ssh_auth = [AuthKind::Agent, AuthKind::Password, AuthKind::Key][i];
            }

            match self.ssh_auth {
                AuthKind::Agent => {
                    ui.label(med("Uses the running SSH agent.", 12.0, MUTED_SOFT));
                }
                AuthKind::Password => {
                    keychain_label(ui, "PASSWORD");
                    ui.add(secret_edit(&mut self.ssh_secret, ui.available_width()));
                }
                AuthKind::Key => {
                    ui.label(bold("KEY FILE", 10.5, MUTED_SOFT));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ssh_key_path)
                            .desired_width(ui.available_width())
                            .font(egui::TextStyle::Monospace),
                    );
                    keychain_label(ui, "PASSPHRASE");
                    ui.add(secret_edit(&mut self.ssh_secret, ui.available_width()));
                }
            }
        });
    }

    fn jobs_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(extra("Jobs", 14.0, INK));
            let has_finished = self.jobs.iter().any(|j| j.status.is_finished());
            if has_finished {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if small_ghost(ui, "Clear finished") {
                        self.jobs.retain(|j| !j.status.is_finished());
                    }
                });
            }
        });
        ui.add_space(8.0);

        if self.jobs.is_empty() {
            theme::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(med("no jobs yet", 13.5, MUTED_SOFT));
                    ui.add_space(10.0);
                });
            });
            return;
        }

        let mut to_cancel = Vec::new();
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let n = self.jobs.len();
            for (idx, job) in self.jobs.iter().enumerate() {
                if render_job(ui, job) {
                    to_cancel.push(job.id);
                }
                if idx + 1 < n {
                    ui.add_space(6.0);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, LINE_SOFT);
                    ui.add_space(6.0);
                }
            }
        });
        for id in to_cancel {
            self.engine.cancel(id);
        }
    }

    fn log_section(&mut self, ui: &mut egui::Ui) {
        ui.label(extra("Log", 14.0, INK));
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(SURFACE2)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(78.0);
                ui.spacing_mut().item_spacing.y = 3.0;
                if self.log.is_empty() {
                    ui.label(mono("— ready —", 11.5, MUTED_SOFT));
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in &self.log {
                                ui.label(mono(line.clone(), 11.5, INK2));
                            }
                        });
                }
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.handle_drops(ctx);

        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let pointer = ctx.input(|i| i.pointer.latest_pos());

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::WIN_BG).inner_margin(Margin::same(18)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(extra("Pack folders into zip and move", 19.0, INK));
                        ui.add_space(16.0);

                        ui.columns(2, |cols| {
                            self.source_column(&mut cols[0], hovering, pointer);
                            self.destination_column(&mut cols[1], hovering, pointer);
                        });

                        ui.add_space(14.0);
                        separator(ui);
                        ui.add_space(12.0);
                        self.actions_row(ui);

                        ui.add_space(16.0);
                        self.jobs_section(ui);

                        ui.add_space(16.0);
                        self.log_section(ui);
                    });
            });
    }
}

impl App {
    fn actions_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.remove_source, med("Remove source after success", 13.0, INK2));
            ui.add_space(8.0);
            ui.checkbox(&mut self.checksum, med("sha256", 13.0, INK2));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let can_start = self.can_start();
                let start = egui::Button::new(extra(
                    "▶  Start",
                    14.0,
                    if can_start { Color32::WHITE } else { MUTED_SOFT },
                ))
                .fill(if can_start { ACCENT } else { SURFACE2 })
                .corner_radius(CornerRadius::same(10))
                .stroke(Stroke::NONE)
                .min_size(Vec2::new(0.0, 38.0));
                if ui.add_enabled(can_start, start).clicked() {
                    self.start_jobs();
                }
                if !self.sources.is_empty() && ui.add(ghost("Clear sources")).clicked() {
                    self.sources.clear();
                }
            });
        });
    }
}

// ── reusable widgets ──────────────────────────────────────────────────────────

fn folder_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

fn empty_hint(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(20.0);
        ui.label(reg("📁", 28.0, MUTED_SOFT));
        ui.add_space(8.0);
        ui.label(med(text, 13.5, MUTED_SOFT));
        ui.add_space(20.0);
    });
}

/// A source item row: ✕ button + folder glyph + name. Returns true if removed.
fn item_row(ui: &mut egui::Ui, name: &str) -> bool {
    let mut removed = false;
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let x = egui::Button::new(reg("✕", 12.0, MUTED))
                    .fill(SURFACE2)
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::splat(20.0));
                if ui.add(x).clicked() {
                    removed = true;
                }
                ui.label(reg("📁", 15.0, ACCENT));
                ui.label(semi(truncate(name, 28), 14.0, INK));
            });
        });
    removed
}

fn render_job(ui: &mut egui::Ui, job: &JobView) -> bool {
    let mut cancel = false;
    ui.horizontal(|ui| {
        ui.label(mono(job.id.to_string(), 11.5, MUTED_SOFT));
        ui.label(semi(truncate(&job.title, 26), 14.0, INK));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            match &job.status {
                JobStatus::Queued => {
                    ui.label(med("queued…", 12.0, MUTED_SOFT));
                }
                JobStatus::Archiving { done, total } => {
                    if small_ghost(ui, "Cancel") {
                        cancel = true;
                    }
                    ui.label(mono(format!("packing {done} / {total}"), 12.0, ACCENT_DEEP));
                }
                JobStatus::Transferring => {
                    if small_ghost(ui, "Cancel") {
                        cancel = true;
                    }
                    ui.label(semi("moving…", 12.0, ACCENT_DEEP));
                }
                JobStatus::Done { .. } => {
                    ui.label(semi("✔ done", 12.0, SUCCESS));
                }
                JobStatus::Failed { .. } => {
                    ui.label(semi("✘ failed", 12.0, ERROR));
                }
                JobStatus::Canceled => {
                    ui.label(med("canceled", 12.0, MUTED));
                }
            }
        });
    });

    match &job.status {
        JobStatus::Archiving { done, total } => {
            let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
            ui.add_space(5.0);
            thin_bar(ui, frac);
        }
        JobStatus::Transferring => {
            ui.add_space(5.0);
            thin_bar(ui, 0.92);
        }
        JobStatus::Done { output, sha256 } => {
            ui.add_space(4.0);
            ui.label(mono(format!("✔ {}", output.display()), 11.0, MUTED));
            if let Some(h) = sha256 {
                ui.label(mono(format!("sha256: {h}"), 11.0, MUTED_SOFT));
            }
        }
        JobStatus::Failed { error } => {
            ui.add_space(4.0);
            ui.label(mono(error.clone(), 11.0, ERROR));
        }
        _ => {}
    }
    cancel
}

fn thin_bar(ui: &mut egui::Ui, frac: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), LINE_SOFT);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fill, CornerRadius::same(3), ACCENT);
}

/// Equal-width segmented control. Returns the clicked index, if any.
fn segmented(ui: &mut egui::Ui, options: &[&str], selected: usize, fill_width: bool) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(SURFACE2)
        .corner_radius(CornerRadius::same(if fill_width { 8 } else { 9 }))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let seg_w = if fill_width {
                (ui.available_width() - 2.0 * options.len() as f32) / options.len() as f32
            } else {
                0.0
            };
            ui.horizontal(|ui| {
                for (i, label) in options.iter().enumerate() {
                    let on = i == selected;
                    let txt = semi(*label, 12.0, if on { INK } else { MUTED });
                    let mut btn = egui::Button::new(txt)
                        .fill(if on { SURFACE } else { Color32::TRANSPARENT })
                        .corner_radius(CornerRadius::same(7))
                        .stroke(Stroke::NONE);
                    if fill_width {
                        btn = btn.min_size(Vec2::new(seg_w.max(1.0), 24.0));
                    }
                    if ui.add(btn).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32) {
    ui.vertical(|ui| {
        ui.label(bold(label, 10.5, MUTED_SOFT));
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(width)
                .font(egui::TextStyle::Monospace),
        );
    });
}

fn keychain_label(ui: &mut egui::Ui, name: &str) {
    ui.horizontal(|ui| {
        ui.label(bold(name, 10.5, MUTED_SOFT));
        ui.label(semi("🔒 Keychain", 9.5, SUCCESS));
    });
}

fn secret_edit(value: &mut String, width: f32) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(value)
        .password(true)
        .desired_width(width)
}

fn outline(text: &str) -> egui::Button<'static> {
    egui::Button::new(bold(text, 13.5, INK))
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(10))
}

fn outline_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_sized([ui.available_width(), 38.0], outline(text))
}

fn ghost(text: &str) -> egui::Button<'static> {
    egui::Button::new(bold(text, 13.0, MUTED))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(10))
}

fn small_ghost(ui: &mut egui::Ui, text: &str) -> bool {
    let btn = egui::Button::new(semi(text, 11.5, MUTED))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(7));
    ui.add(btn).clicked()
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, LINE_SOFT);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}
