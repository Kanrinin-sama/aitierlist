use crate::settings::Settings;
use crate::theme;
use crate::types::Table;
use eframe::egui;
use std::sync::mpsc::{Receiver, channel};

type SetupResponse = Result<(Option<String>, crate::host_install::Discovery), String>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Configure,
    Review,
    Done,
}

pub(super) struct Generator {
    pub(super) open: bool,
    response: Option<Receiver<SetupResponse>>,
    discovery: Option<crate::host_install::Discovery>,
    markdown: Option<String>,
    table: Option<Table>,
    isolated: bool,
    settings: Option<Settings>,
    revision: u64,
    selected: std::collections::BTreeSet<String>,
    add_to_path: bool,
    error: String,
    preview_rx: Option<Receiver<Result<crate::host_install::InstallPreview, String>>>,
    preview: Option<crate::host_install::InstallPreview>,
    preview_dirty: bool,
    pub(super) install_rx: Option<Receiver<Result<crate::host_install::InstallResult, String>>>,
    installed: Option<crate::host_install::InstallResult>,
    native_horizon: String,
    step: Step,
}

impl Generator {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            response: None,
            discovery: None,
            markdown: None,
            table: None,
            isolated: true,
            settings: None,
            revision: 0,
            selected: std::collections::BTreeSet::new(),
            add_to_path: false,
            error: String::new(),
            preview_rx: None,
            preview: None,
            preview_dirty: false,
            install_rx: None,
            installed: None,
            native_horizon: String::new(),
            step: Step::Configure,
        }
    }

    pub(super) fn prepare(
        &mut self,
        table: Table,
        settings: Settings,
        revision: u64,
        context: egui::Context,
    ) {
        if self.install_rx.is_some() {
            return;
        }
        self.open = true;
        self.step = Step::Configure;
        self.settings = Some(settings);
        self.table = Some(table.clone());
        self.isolated = true;
        self.revision = revision;
        self.error.clear();
        self.markdown = None;
        self.installed = None;
        self.preview = None;
        self.preview_rx = None;
        self.preview_dirty = false;
        self.add_to_path = false;
        self.native_horizon.clear();
        let (tx, rx) = channel();
        self.response = Some(rx);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let discovery = crate::host_install::discover(&table)?;
                let markdown = crate::orchestration::render_with_discovery(&table, &discovery)?;
                Ok((Some(markdown), discovery))
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            context.request_repaint();
        });
    }

    pub(super) fn pump(&mut self) {
        if let Some(rx) = &self.response
            && let Ok(result) = rx.try_recv()
        {
            self.response = None;
            match result {
                Ok((markdown, discovery)) => {
                    self.selected.clear();
                    if let Some(host) = &discovery.recommended_host
                        && discovery.hosts.iter().any(|candidate| {
                            candidate.id == *host
                                && candidate.installed
                                && candidate.executable.is_some()
                                && (!self.isolated || candidate.isolation_supported)
                        })
                    {
                        self.selected.insert(host.clone());
                    }
                    self.preview_dirty = markdown.is_some();
                    self.markdown = markdown;
                    self.discovery = Some(discovery);
                }
                Err(error) => self.error = error,
            }
        }
        if let Some(rx) = &self.preview_rx
            && let Ok(result) = rx.try_recv()
        {
            self.preview_rx = None;
            match result {
                Ok(preview) => self.preview = Some(preview),
                Err(error) => self.error = error,
            }
        }
        if let Some(rx) = &self.install_rx
            && let Ok(result) = rx.try_recv()
        {
            self.install_rx = None;
            self.preview = None;
            match result {
                Ok(mut result) => {
                    if let Some(previous) = self.installed.take() {
                        for launcher in previous.launchers {
                            if !result
                                .launchers
                                .iter()
                                .any(|current| current.host_id == launcher.host_id)
                            {
                                result.launchers.push(launcher);
                            }
                        }
                    }
                    self.installed = Some(result);
                    self.step = Step::Done;
                }
                Err(error) => self.error = error,
            }
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        table: &Table,
        settings: &Settings,
        revision: u64,
        score_current: bool,
    ) -> bool {
        if !self.open {
            return false;
        }
        let context = ui.ctx().clone();
        let current = score_current
            && self.revision == revision
            && self
                .settings
                .as_ref()
                .is_some_and(|prepared| prepared.scoring_matches(settings));
        let mut regenerate = false;
        let mut retry_failed = false;
        let mut finish = false;

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.install_rx.is_none(), egui::Button::new("Back to team"))
                    .clicked()
                {
                    finish = true;
                }
            });
        });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            step_label(ui, "1", "Configure", self.step == Step::Configure);
            ui.separator();
            step_label(ui, "2", "Review", self.step == Step::Review);
            ui.separator();
            step_label(ui, "3", "Done", self.step == Step::Done);
        });
        ui.add_space(18.0);

        egui::ScrollArea::vertical()
            .id_salt(match self.step {
                Step::Configure => "setup_configure_scroll",
                Step::Review => "setup_review_scroll",
                Step::Done => "setup_done_scroll",
            })
            .auto_shrink([false, false])
            .show(ui, |ui| match self.step {
                Step::Configure => {
                    self.configure(ui, table, settings, current, score_current, &mut regenerate);
                }
                Step::Review => {
                    self.review(ui, current);
                }
                Step::Done => {
                    self.done(
                        ui,
                        score_current,
                        &mut regenerate,
                        &mut retry_failed,
                        &mut finish,
                    );
                }
            });

        if retry_failed && let Some(installed) = &self.installed {
            self.selected = installed
                .failures
                .iter()
                .map(|failure| failure.host_id.clone())
                .collect();
            self.add_to_path = false;
            self.preview_dirty = true;
            self.step = Step::Review;
        }
        if regenerate {
            self.prepare(table.clone(), settings.clone(), revision, context.clone());
        }
        self.prepare_preview(context);
        finish
    }

    fn configure(
        &mut self,
        ui: &mut egui::Ui,
        table: &Table,
        settings: &Settings,
        current: bool,
        score_current: bool,
        regenerate: &mut bool,
    ) {
        if self.response.is_some() {
            ui.spinner();
            ui.label("Preparing your plan and finding compatible tools…");
        }
        if !current && self.response.is_none() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Your team settings changed after this setup began.",
            );
            *regenerate = ui
                .add_enabled(
                    score_current && self.install_rx.is_none(),
                    egui::Button::new("Update setup from current team"),
                )
                .clicked();
        }
        if !self.error.is_empty() {
            ui.colored_label(theme::ERROR, &self.error);
            if ui
                .add_enabled(
                    current && self.install_rx.is_none() && self.installed.is_none(),
                    egui::Button::new("Try again"),
                )
                .clicked()
            {
                *regenerate = true;
            }
        }
        let Some(discovery) = &self.discovery else {
            return;
        };

        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.strong("Runtime isolation");
                ui.colored_label(theme::MUTED, "Choose how generated workers access tools and project files.");
                ui.add_enabled_ui(
                    self.install_rx.is_none()
                        && self.installed.is_none()
                        && self.response.is_none()
                        && self.preview_rx.is_none(),
                    |ui| {
                        let previous = self.isolated;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.isolated, true, "Isolated · Recommended");
                            ui.selectable_value(&mut self.isolated, false, "Use existing tool setup");
                        });
                        if previous != self.isolated {
                            self.selected.retain(|id| {
                                discovery.hosts.iter().any(|host| {
                                    host.id == *id
                                        && host.executable.is_some()
                                        && (!self.isolated || host.isolation_supported)
                                })
                            });
                            if self.selected.is_empty()
                                && let Some(recommended) = &discovery.recommended_host
                                && discovery.hosts.iter().any(|host| {
                                    host.id == *recommended
                                        && host.executable.is_some()
                                        && (!self.isolated || host.isolation_supported)
                                })
                            {
                                self.selected.insert(recommended.clone());
                            }
                            self.preview_dirty = true;
                        }
                    },
                );
                ui.colored_label(
                    theme::MUTED,
                    if self.isolated {
                        "Workers run through a separate generated runtime. Only tools with enforceable isolation are available."
                    } else {
                        "Workers use your existing tool configuration and a portable conductor document."
                    },
                );
                if self.isolated {
                    match crate::workspace::resolve(settings) {
                        Ok(paths) => {
                            ui.label(format!("Workspace · {}", paths.root.display()));
                        }
                        Err(error) => {
                            ui.colored_label(theme::ERROR, format!("Workspace unavailable · {error:#}"));
                        }
                    }
                }
            });

        ui.add_space(14.0);
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.strong("Connected tools");
                ui.colored_label(theme::MUTED, "Select every host that should receive a launcher. You can continue without one to save the runtime or Markdown only.");
                if let Some(route) = crate::orchestration::recommended_route(table) {
                    ui.label(format!("Orchestrator allocation · {} / {}", route.provider_id, route.plan_id));
                }
                ui.add_enabled_ui(
                    self.install_rx.is_none()
                        && self.installed.is_none()
                        && self.response.is_none()
                        && self.preview_rx.is_none(),
                    |ui| {
                        for host in discovery
                            .hosts
                            .iter()
                            .filter(|host| !host.kind.is_multi_provider() || host.detection != "absent")
                        {
                            let mut selected = self.selected.contains(&host.id);
                            let status = match host.detection.as_str() {
                                "desktop_only" => "Desktop app found; CLI unverified",
                                "config_only" => "Configuration found; CLI needs setup",
                                "invalid_override" => "Configured path needs setup",
                                "absent" => "Not detected",
                                _ if host.executable.is_none() => "CLI found; launch setup needed",
                                _ => host.auth_status.as_str(),
                            };
                            let recommended = discovery.recommended_host.as_deref() == Some(host.id.as_str());
                            let label = format!(
                                "{}{} · {}",
                                host.name,
                                if recommended { " · Recommended" } else { "" },
                                status
                            );
                            let response = ui.add_enabled(
                                host.installed
                                    && host.executable.is_some()
                                    && (!self.isolated || host.isolation_supported),
                                egui::Checkbox::new(&mut selected, label),
                            );
                            let details = if self.isolated && !host.isolation_message.is_empty() {
                                format!("{}\n\n{}", host.message, host.isolation_message)
                            } else {
                                host.message.clone()
                            };
                            if response.on_hover_text(details).changed() {
                                if selected {
                                    self.selected.insert(host.id.clone());
                                } else {
                                    self.selected.remove(&host.id);
                                }
                                self.preview_dirty = true;
                            }
                        }
                        if ui
                            .checkbox(&mut self.add_to_path, "Add launcher directory to my user PATH")
                            .changed()
                        {
                            self.preview_dirty = true;
                        }
                    },
                );
                if !discovery.setup.message.is_empty() {
                    ui.label(&discovery.setup.message);
                }
                for issue in &discovery.setup.issues {
                    ui.label(issue);
                }
                for note in &discovery.notes {
                    if self.isolated
                        && note.starts_with("Launchers preserve the caller's working directory.")
                    {
                        ui.colored_label(
                            theme::MUTED,
                            "Isolated launchers use a generated clean workspace and approved model bindings.",
                        );
                    } else {
                        ui.colored_label(theme::MUTED, note);
                    }
                }
            });

        ui.add_space(18.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ready = current
                && self.markdown.is_some()
                && self.preview.is_some()
                && self.response.is_none()
                && self.preview_rx.is_none()
                && !self.preview_dirty;
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(egui::RichText::new("Continue to review").color(theme::TEXT))
                        .fill(theme::BLUE),
                )
                .clicked()
            {
                self.step = Step::Review;
            }
            if self.preview_rx.is_some() || self.preview_dirty {
                ui.spinner();
                ui.colored_label(theme::MUTED, "Preparing exact destinations…");
            }
        });
    }

    fn review(&mut self, ui: &mut egui::Ui, current: bool) {
        if !current {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Team settings changed. Return to Configure and update this setup before generating.",
            );
        }
        ui.heading("Review what will be created");
        ui.colored_label(
            theme::MUTED,
            "Nothing is written until you start generation.",
        );
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.strong("Configuration");
                ui.label(if self.isolated {
                    "Isolated runtime"
                } else {
                    "Existing tool setup"
                });
                ui.label(if self.add_to_path {
                    "Launcher directory will be added to user PATH"
                } else {
                    "User PATH will not change"
                });
                if self.selected.is_empty() {
                    ui.label(if self.isolated {
                        "No host launcher · save runtime bundle only"
                    } else {
                        "No host launcher · save portable Markdown only"
                    });
                } else {
                    ui.label(format!(
                        "Hosts · {}",
                        self.selected.iter().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
            });
        ui.add_space(12.0);
        if self.preview_rx.is_some() || self.preview_dirty {
            ui.spinner();
            ui.label("Refreshing exact destinations…");
        } else if let Some(preview) = &self.preview {
            egui::Frame::new()
                .fill(theme::SURFACE)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(10)
                .inner_margin(18)
                .show(ui, |ui| {
                    egui::CollapsingHeader::new(format!(
                        "Exact destinations ({})",
                        preview.destinations.len()
                    ))
                    .default_open(false)
                    .show(ui, |ui| {
                        for destination in &preview.destinations {
                            ui.monospace(destination.display().to_string());
                        }
                    });
                    for impact in &preview.impacts {
                        ui.add(egui::Label::new(impact).wrap());
                    }
                    for command in &preview.commands {
                        ui.monospace(command);
                    }
                });
            ui.add_space(18.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let may_install = self.installed.is_none()
                    || self.installed.as_ref().is_some_and(|result| {
                        !result.failures.is_empty()
                            && preview.host_ids.iter().all(|host| {
                                result
                                    .failures
                                    .iter()
                                    .any(|failure| failure.host_id == *host)
                            })
                    });
                let label = if preview.host_ids.is_empty() {
                    if preview.isolated {
                        "Generate runtime"
                    } else {
                        "Save portable Markdown"
                    }
                } else {
                    "Generate and install"
                };
                if ui
                    .add_enabled(
                        current && self.install_rx.is_none() && may_install,
                        egui::Button::new(egui::RichText::new(label).color(theme::TEXT))
                            .fill(theme::BLUE),
                    )
                    .clicked()
                {
                    let preview = preview.clone();
                    let (tx, rx) = channel();
                    self.install_rx = Some(rx);
                    self.error.clear();
                    let context = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(
                            crate::host_install::install(preview)
                                .map_err(|error| format!("{error:#}")),
                        );
                        context.request_repaint();
                    });
                }
            });
        }
        if self.install_rx.is_some() {
            ui.spinner();
            ui.label("Generating and installing…");
        }
        if !self.error.is_empty() {
            ui.colored_label(theme::ERROR, &self.error);
        }
        ui.add_space(12.0);
        if ui
            .add_enabled(
                self.install_rx.is_none(),
                egui::Button::new("Back to configure"),
            )
            .clicked()
        {
            self.step = Step::Configure;
        }
    }

    fn done(
        &mut self,
        ui: &mut egui::Ui,
        score_current: bool,
        regenerate: &mut bool,
        retry_failed: &mut bool,
        finish: &mut bool,
    ) {
        let Some(installed) = &self.installed else {
            self.step = Step::Configure;
            return;
        };
        ui.colored_label(theme::SUCCESS, "Setup generated");
        ui.heading(if installed.isolated {
            "Your isolated runtime is ready"
        } else {
            "Your tool setup is ready"
        });
        ui.colored_label(
            theme::MUTED,
            format!("Saved in {}", installed.directory.display()),
        );
        ui.add_space(12.0);
        let document_path = if installed.isolated {
            installed.directory.join("runtime.md")
        } else {
            installed.policy_path.clone()
        };
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.strong("Generated files");
                ui.monospace(document_path.display().to_string());
                if ui
                    .button(if installed.isolated {
                        "Copy runtime path"
                    } else {
                        "Copy Markdown path"
                    })
                    .clicked()
                {
                    ui.ctx().copy_text(document_path.display().to_string());
                }
                for launcher in &installed.launchers {
                    ui.separator();
                    ui.colored_label(theme::SUCCESS, format!("{} · Installed", launcher.host_id));
                    ui.label(&launcher.adapter);
                    ui.monospace(launcher.path.display().to_string());
                    ui.horizontal(|ui| {
                        ui.monospace(&launcher.command);
                        if ui.button("Copy command").clicked() {
                            ui.ctx().copy_text(launcher.command.clone());
                        }
                    });
                }
                for failure in &installed.failures {
                    ui.colored_label(
                        theme::ERROR,
                        format!("{} · {}", failure.host_id, failure.message),
                    );
                }
            });
        ui.add_space(12.0);
        if ui.button("Refresh native horizon").clicked() {
            let generation_id = installed
                .directory
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            self.native_horizon = crate::agent_setup::paths()
                .and_then(|paths| std::fs::read(&paths.ledger).map_err(anyhow::Error::from))
                .and_then(|bytes| {
                    serde_json::from_slice::<crate::agent_setup::Ledger>(&bytes)
                        .map_err(anyhow::Error::from)
                })
                .and_then(|ledger| {
                    let pointer = ledger
                        .horizons
                        .iter()
                        .find(|pointer| pointer.generation_id == generation_id)
                        .ok_or_else(|| {
                            anyhow::anyhow!("No committed horizon for this generation")
                        })?;
                    let bytes = std::fs::read(&pointer.artifact_path)?;
                    let horizon: crate::native_scheduler::NativeHorizon =
                        serde_json::from_slice(&bytes)?;
                    anyhow::ensure!(
                        horizon.horizon_id == pointer.id
                            && horizon.policy_version == pointer.policy_version
                            && horizon.profile_identity == pointer.profile_identity
                            && horizon.ledger_revision == pointer.committed_revision,
                        "Committed horizon pointer and artifact differ"
                    );
                    crate::orchestration::render_native_horizon(&horizon)
                })
                .unwrap_or_else(|error| format!("Committed native horizon unavailable: {error:#}"));
        }
        if !self.native_horizon.is_empty() {
            egui::CollapsingHeader::new("Live native allocation")
                .default_open(true)
                .show(ui, |ui| {
                    ui.monospace(&self.native_horizon);
                });
        }
        for note in &installed.notes {
            ui.add(egui::Label::new(note).wrap());
        }
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            if !installed.failures.is_empty()
                && ui
                    .add_enabled(
                        score_current && self.install_rx.is_none(),
                        egui::Button::new("Retry failed hosts"),
                    )
                    .clicked()
            {
                *retry_failed = true;
            }
            if ui
                .add_enabled(
                    score_current && self.install_rx.is_none(),
                    egui::Button::new("Generate another setup"),
                )
                .clicked()
            {
                *regenerate = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Back to team").color(theme::TEXT))
                            .fill(theme::BLUE),
                    )
                    .clicked()
                {
                    *finish = true;
                }
            });
        });
    }

    fn prepare_preview(&mut self, context: egui::Context) {
        if self.preview_dirty
            && self.install_rx.is_none()
            && self.preview_rx.is_none()
            && self.response.is_none()
            && let (Some(markdown), Some(discovery), Some(table)) =
                (&self.markdown, &self.discovery, &self.table)
        {
            self.preview_dirty = false;
            self.preview = None;
            let markdown = markdown.clone();
            let discovery = discovery.clone();
            let table = table.clone();
            let options = crate::host_install::InstallOptions {
                isolated: self.isolated,
            };
            let hosts = self.selected.iter().cloned().collect::<Vec<_>>();
            let add_to_path = self.add_to_path;
            let installed = self.installed.clone();
            let (tx, rx) = channel();
            self.preview_rx = Some(rx);
            std::thread::spawn(move || {
                let result = if let Some(installed) = installed {
                    crate::host_install::prepare_retry(&discovery, &hosts, &installed)
                } else {
                    crate::host_install::prepare_install(
                        &markdown,
                        &discovery,
                        &hosts,
                        add_to_path,
                        &table,
                        options,
                    )
                };
                let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                context.request_repaint();
            });
        }
    }
}

fn step_label(ui: &mut egui::Ui, number: &str, label: &str, active: bool) {
    let color = if active { theme::CYAN } else { theme::MUTED };
    ui.colored_label(color, format!("{number}  {label}"));
}
