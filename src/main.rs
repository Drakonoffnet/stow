//! GUI layer of `file_transfer` (egui/eframe).
//!
//! Two drag-and-drop zones: "Source" (folders to pack) and "Destination"
//! (folder for the finished archives). The heavy work lives in the core
//! (`file_transfer` lib).

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui;

use file_transfer::core::engine::Event;
use file_transfer::{Config, DestinationSpec, Engine, JobId, JobSpec, JobStatus};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 560.0])
            .with_min_inner_size([520.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "file_transfer — pack and move",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
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
    dest: Option<PathBuf>,
    remove_source: bool,
    checksum: bool,

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
            dest: None,
            remove_source: false,
            checksum: false,
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

    /// Accept dropped files and sort them into the zones.
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

    fn start_jobs(&mut self) {
        let Some(dir) = self.dest.clone() else { return };
        for source in std::mem::take(&mut self.sources) {
            self.engine.submit(JobSpec {
                source,
                destination: DestinationSpec::Local { dir: dir.clone() },
                remove_source: self.remove_source,
                checksum: self.checksum,
            });
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

                let dst_hi = hovering
                    && pointer.map(|p| self.dest_rect.contains(p)).unwrap_or(false);
                let dest_lines: Vec<PathBuf> = self.dest.clone().into_iter().collect();
                self.dest_rect = drop_zone(
                    &mut cols[1],
                    "Destination",
                    "Drop the folder for archives here",
                    &dest_lines,
                    dst_hi,
                );
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.remove_source, "Remove source after success");
                ui.checkbox(&mut self.checksum, "sha256");
            });

            ui.add_space(4.0);
            let can_start = self.dest.is_some() && !self.sources.is_empty();
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
            ui.label("Jobs:");
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.jobs.is_empty() {
                        ui.weak("no jobs yet");
                    }
                    for job in &self.jobs {
                        render_job(ui, job);
                    }
                });

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

fn render_job(ui: &mut egui::Ui, job: &JobView) {
    ui.horizontal(|ui| {
        ui.strong(&job.title);
        ui.weak(job.id.to_string());
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
}
