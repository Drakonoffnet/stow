//! GUI layer of `file_transfer` (egui/eframe).
//!
//! A "Source" drag-and-drop zone for folders to pack, and a destination that is
//! either a local folder (drag-and-drop) or an SSH/SFTP target configured in a
//! form. The heavy work lives in the core (`file_transfer` lib).

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui;

use file_transfer::core::engine::Event;
use file_transfer::core::secret;
use file_transfer::{
    Config, DestinationSpec, Engine, JobId, JobSpec, JobStatus, SshAuth, SshConfig,
};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 640.0])
            .with_min_inner_size([560.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "file_transfer — pack and move",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

/// Destination kind selected in the UI.
#[derive(PartialEq, Clone, Copy)]
enum DestKind {
    Local,
    Ssh,
}

/// SSH authentication method selected in the UI.
#[derive(PartialEq, Clone, Copy)]
enum AuthKind {
    Agent,
    Password,
    Key,
}

/// Displayed state of a single job.
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

    // Destination.
    dest_kind: DestKind,
    dest: Option<PathBuf>, // local folder
    ssh_host: String,
    ssh_port: String,
    ssh_user: String,
    ssh_remote_dir: String,
    ssh_auth: AuthKind,
    ssh_secret: String, // password or key passphrase; saved to the keychain on start
    ssh_key_path: String,

    jobs: Vec<JobView>,
    log: Vec<String>,

    // Zone rectangles from the previous frame — used to tell where a drop landed.
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

    /// Accept dropped folders into the source list, or the local destination.
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

    /// Build the destination spec from the current form, storing any secret in
    /// the keychain. Returns a human-readable error on invalid input.
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

    fn destination_column(&mut self, ui: &mut egui::Ui, hovering: bool, pointer: Option<egui::Pos2>) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.dest_kind, DestKind::Local, "Local");
            ui.selectable_value(&mut self.dest_kind, DestKind::Ssh, "SSH");
        });

        match self.dest_kind {
            DestKind::Local => {
                let dst_hi = hovering
                    && pointer.map(|p| self.dest_rect.contains(p)).unwrap_or(false);
                let dest_lines: Vec<PathBuf> = self.dest.clone().into_iter().collect();
                self.dest_rect = drop_zone(
                    ui,
                    "Destination",
                    "Drop the folder for archives here",
                    &dest_lines,
                    dst_hi,
                );
            }
            DestKind::Ssh => {
                // No drop target in SSH mode — keep drops going to the source list.
                self.dest_rect = egui::Rect::NOTHING;
                self.ssh_form(ui);
            }
        }
    }

    fn ssh_form(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("ssh_form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Host");
                ui.text_edit_singleline(&mut self.ssh_host);
                ui.end_row();

                ui.label("Port");
                ui.text_edit_singleline(&mut self.ssh_port);
                ui.end_row();

                ui.label("User");
                ui.text_edit_singleline(&mut self.ssh_user);
                ui.end_row();

                ui.label("Remote dir");
                ui.text_edit_singleline(&mut self.ssh_remote_dir);
                ui.end_row();
            });

        ui.horizontal(|ui| {
            ui.label("Auth:");
            ui.selectable_value(&mut self.ssh_auth, AuthKind::Agent, "Agent");
            ui.selectable_value(&mut self.ssh_auth, AuthKind::Password, "Password");
            ui.selectable_value(&mut self.ssh_auth, AuthKind::Key, "Key");
        });

        match self.ssh_auth {
            AuthKind::Agent => {
                ui.weak("Uses the running SSH agent.");
            }
            AuthKind::Password => {
                ui.horizontal(|ui| {
                    ui.label("Password");
                    ui.add(egui::TextEdit::singleline(&mut self.ssh_secret).password(true));
                });
                ui.weak("Saved to the macOS Keychain on start.");
            }
            AuthKind::Key => {
                ui.horizontal(|ui| {
                    ui.label("Key file");
                    ui.text_edit_singleline(&mut self.ssh_key_path);
                });
                ui.horizontal(|ui| {
                    ui.label("Passphrase");
                    ui.add(egui::TextEdit::singleline(&mut self.ssh_secret).password(true));
                });
                ui.weak("Passphrase is optional; saved to the Keychain if set.");
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.handle_drops(ctx);

        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let pointer = ctx.input(|i| i.pointer.latest_pos());

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Pack folders into zip and move");
            ui.add_space(6.0);

            ui.columns(2, |cols| {
                let src_hi = hovering
                    && pointer.map(|p| self.source_rect.contains(p)).unwrap_or(false);
                self.source_rect = drop_zone(
                    &mut cols[0],
                    "Source",
                    "Drop folders to pack here",
                    &self.sources,
                    src_hi,
                );

                self.destination_column(&mut cols[1], hovering, pointer);
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.remove_source, "Remove source after success");
                ui.checkbox(&mut self.checksum, "sha256");
            });

            ui.add_space(4.0);
            let can_start = self.can_start();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_start, egui::Button::new("▶ Start"))
                    .clicked()
                {
                    self.start_jobs();
                }
                if !self.sources.is_empty() && ui.button("Clear sources").clicked() {
                    self.sources.clear();
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Jobs:");
                let has_finished = self.jobs.iter().any(|j| j.status.is_finished());
                if has_finished && ui.button("Clear finished").clicked() {
                    self.jobs.retain(|j| !j.status.is_finished());
                }
            });

            let mut to_cancel: Vec<JobId> = Vec::new();
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.jobs.is_empty() {
                        ui.weak("no jobs yet");
                    }
                    for job in &self.jobs {
                        if render_job(ui, job) {
                            to_cancel.push(job.id);
                        }
                    }
                });
            for id in to_cancel {
                self.engine.cancel(id);
            }

            if !self.log.is_empty() {
                ui.separator();
                ui.label("Log:");
                egui::ScrollArea::vertical()
                    .id_salt("log")
                    .max_height(120.0)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.monospace(line);
                        }
                    });
            }
        });
    }
}

/// Draw a drag-and-drop zone and return its rectangle (used to detect the target).
fn drop_zone(
    ui: &mut egui::Ui,
    title: &str,
    hint: &str,
    items: &[PathBuf],
    highlight: bool,
) -> egui::Rect {
    let stroke = if highlight {
        egui::Stroke::new(2.0, egui::Color32::from_rgb(90, 170, 255))
    } else {
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
    };
    let frame = egui::Frame::group(ui.style())
        .stroke(stroke)
        .inner_margin(egui::Margin::same(10));

    let resp = frame.show(ui, |ui| {
        ui.set_min_height(120.0);
        ui.set_width(ui.available_width());
        ui.strong(title);
        if items.is_empty() {
            ui.weak(hint);
        } else {
            for p in items {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string());
                ui.label(format!("📁 {name}"));
            }
        }
    });
    resp.response.rect
}

/// Render a single job. Returns `true` if the user requested cancellation.
fn render_job(ui: &mut egui::Ui, job: &JobView) -> bool {
    let mut cancel_requested = false;
    ui.horizontal(|ui| {
        ui.strong(&job.title);
        ui.weak(job.id.to_string());
        if job.status.is_active() && ui.button("Cancel").clicked() {
            cancel_requested = true;
        }
    });
    match &job.status {
        JobStatus::Queued => {
            ui.weak("queued…");
        }
        JobStatus::Archiving { done, total } => {
            let frac = if *total > 0 {
                *done as f32 / *total as f32
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(frac).text(format!("packing {done}/{total}")),
            );
        }
        JobStatus::Transferring => {
            ui.add(egui::ProgressBar::new(0.99).text("moving…"));
        }
        JobStatus::Done { output, sha256 } => {
            ui.colored_label(
                egui::Color32::from_rgb(80, 200, 120),
                format!("✔ done: {}", output.display()),
            );
            if let Some(h) = sha256 {
                ui.monospace(format!("sha256: {h}"));
            }
        }
        JobStatus::Failed { error } => {
            ui.colored_label(egui::Color32::from_rgb(230, 100, 100), format!("✘ {error}"));
        }
        JobStatus::Canceled => {
            ui.weak("canceled");
        }
    }
    ui.add_space(4.0);
    cancel_requested
}
