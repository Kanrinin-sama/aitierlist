use crate::settings::{BestInHouseMode, Settings, load_settings, save_settings};
use crate::types::{CacheState, Seat, Table, Tier};
use eframe::egui;
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender, channel},
};
use std::time::{Duration, Instant};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const REFRESH_COOLDOWN_SECONDS: u64 = 16;
const AA_INTELLIGENCE_METHODOLOGY_URL: &str =
    "https://artificialanalysis.ai/methodology/intelligence-benchmarking";
const VENDOR_TABS: [(&str, &str); 5] = [
    ("Codex", "openai"),
    ("Claude", "anthropic"),
    ("Muse", "muse"),
    ("Gemini", "google"),
    ("Grok", "xai"),
];

type InstallResult = (
    Result<(), String>,
    Option<blockitall_update::StagedArtifact>,
);

fn competence_text(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "Unknown".to_string())
}

fn research_route_name(row: &crate::types::Row, native_harness: &str) -> String {
    if native_harness
        .to_ascii_lowercase()
        .starts_with("antigravity")
    {
        return match row.effort.as_deref() {
            Some(effort) if !effort.is_empty() => format!("AGY - {} ({effort})", row.model),
            _ => format!("AGY - {}", row.model),
        };
    }
    row.display_name()
}

type ComparisonResponse = (
    u64,
    Seat,
    Tier,
    usize,
    crate::comparison::CounterfactualReport,
);

type SetupResponse = Result<(Option<String>, crate::host_install::Discovery), String>;

struct Generator {
    open: bool,
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
    install_rx: Option<Receiver<Result<crate::host_install::InstallResult, String>>>,
    installed: Option<crate::host_install::InstallResult>,
    native_horizon: String,
}

impl Generator {
    fn new(context: egui::Context) -> Self {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = crate::host_install::discover(&Table::empty())
                .map(|discovery| (None, discovery))
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            context.request_repaint();
        });
        Self {
            open: false,
            response: Some(rx),
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
        }
    }

    fn prepare(&mut self, table: Table, settings: Settings, revision: u64, context: egui::Context) {
        if self.install_rx.is_some() {
            return;
        }
        self.open = true;
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

    fn pump(&mut self) {
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
                }
                Err(error) => self.error = error,
            }
        }
    }
    fn show(
        &mut self,
        context: &egui::Context,
        table: &Table,
        settings: &Settings,
        revision: u64,
        score_current: bool,
    ) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut regenerate = false;
        let mut retry_failed = false;
        let current = score_current
            && self.revision == revision
            && self
                .settings
                .as_ref()
                .is_some_and(|prepared| prepared.scoring_matches(settings));
        egui::Window::new("Generate Orchestrator").open(&mut open).default_width(660.0).resizable(true).show(context, |ui| {
            egui::ScrollArea::vertical().max_height(650.0).show(ui, |ui| {
                if self.response.is_some() {
                    ui.spinner();
                    ui.label("Preparing the conductor and discovering local hosts...");
                }
                if !current && self.response.is_none() {
                    ui.label("Settings changed. Generate again to use the current role policy.");
                    regenerate = ui.add_enabled(score_current && self.install_rx.is_none(), egui::Button::new("Generate again")).clicked();
                }
                if !self.error.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
                    if ui.add_enabled(score_current && self.install_rx.is_none() && self.installed.is_none(), egui::Button::new("Retry preparation")).clicked() { regenerate = true; }
                }
                if let Some(discovery) = &self.discovery {
                    ui.strong("Create an isolated orchestrator system?");
                    ui.add_enabled_ui(self.install_rx.is_none() && self.installed.is_none() && self.response.is_none() && self.preview_rx.is_none(), |ui| {
                        let previous = self.isolated;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.isolated, true, "Yes (recommended)");
                            ui.selectable_value(&mut self.isolated, false, "No - normal tool setup");
                        });
                        if previous != self.isolated {
                            self.selected.retain(|id| discovery.hosts.iter().any(|host| host.id == *id && host.executable.is_some() && (!self.isolated || host.isolation_supported)));
                            if self.selected.is_empty()
                                && let Some(recommended) = &discovery.recommended_host
                                && discovery.hosts.iter().any(|host| host.id == *recommended && host.executable.is_some() && (!self.isolated || host.isolation_supported)) {
                                self.selected.insert(recommended.clone());
                            }
                            self.preview_dirty = true;
                        }
                    });
                    ui.label(if self.isolated { "Separate generated runtime and dispatcher. Only tools with enforceable isolation can be selected; authentication requirements are shown below." } else { "Use existing tool configuration and the portable conductor document." });
                    if self.isolated {
                        match crate::workspace::resolve(settings) {
                            Ok(paths) => ui.label(format!("Collaboration workspace: {}", paths.root.display())),
                            Err(error) => ui.colored_label(egui::Color32::LIGHT_RED, format!("Collaboration workspace unavailable: {error:#}")),
                        };
                        ui.small("Change or browse this root in Settings. Isolated workers use it for user work; private fleet state remains in app data.");
                    }
                    ui.label("Choose hosts to install. The recommended orchestrator is selected by default when available.");
                    if let Some(route) = crate::orchestration::recommended_route(table) {
                        ui.small(format!("Fixed orchestrator allocation: {} / {}.", route.provider_id, route.plan_id));
                    }
                    ui.label("Multi-provider tools use their configured connection and billing; onboarding verifies the model and funded account.");
                    ui.add_enabled_ui(self.install_rx.is_none() && self.installed.is_none() && self.response.is_none() && self.preview_rx.is_none(), |ui| {
                        for host in discovery.hosts.iter().filter(|host| !host.kind.is_multi_provider() || host.detection != "absent") {
                            let mut selected = self.selected.contains(&host.id);
                            let status = match host.detection.as_str() {
                                "desktop_only" => "desktop app detected; CLI unverified",
                                "config_only" => "configuration found; CLI needs setup",
                                "invalid_override" => "configured path needs setup",
                                "absent" => "not detected",
                                _ if host.executable.is_none() => "CLI detected; launch setup needed",
                                _ => host.auth_status.as_str(),
                            };
                            let recommended = discovery.recommended_host.as_deref() == Some(host.id.as_str());
                            let multi_provider = host.kind.is_multi_provider();
                            let label = format!("{}{}{} - {}", host.name, if multi_provider { " (multi-provider harness)" } else { "" }, if recommended { " (recommended orchestrator)" } else { "" }, status);
                            if ui.add_enabled(host.installed && host.executable.is_some() && (!self.isolated || host.isolation_supported), egui::Checkbox::new(&mut selected, label)).changed() {
                                if selected { self.selected.insert(host.id.clone()); } else { self.selected.remove(&host.id); }
                                self.preview_dirty = true;
                            }
                            if !host.message.is_empty() { ui.small(&host.message); }
                            if self.isolated { ui.small(&host.isolation_message); }
                        }
                        let absent = discovery.hosts.iter().filter(|host| host.kind.is_multi_provider() && host.detection == "absent")
                            .map(|host| host.name.as_str()).collect::<Vec<_>>();
                        if !absent.is_empty() {
                            ui.small(format!("Checked for, not found: {}", absent.join(", ")));
                        }
                        if ui.checkbox(&mut self.add_to_path, "Add launcher directory to my user PATH").changed() {
                            self.preview_dirty = true;
                        }
                    });
                    if !discovery.setup.message.is_empty() { ui.label(&discovery.setup.message); }
                    for issue in &discovery.setup.issues { ui.label(issue); }
                    for note in &discovery.notes {
                        if self.isolated && note.starts_with("Launchers preserve the caller's working directory.") {
                            ui.small("Isolated launchers use a generated clean workspace and approved model bindings.");
                        } else {
                            ui.small(note);
                        }
                    }
                }
                if self.markdown.is_some() {
                    ui.label(if self.isolated { "Leaving hosts unchecked saves an isolated runtime bundle without a conductor launcher." } else { "Leaving all hosts unchecked exports portable Markdown only. Launch commands use absolute paths unless PATH is explicitly added." });
                }
                if self.preview_rx.is_some() || self.preview_dirty {
                    ui.spinner(); ui.label("Preparing exact destinations...");
                } else if let Some(preview) = &self.preview {
                    ui.separator();
                    ui.strong("Review before installing");
                    for destination in &preview.destinations { ui.label(destination.display().to_string()); }
                    for impact in &preview.impacts { ui.add(egui::Label::new(impact).wrap()); }
                    for command in &preview.commands { ui.monospace(command); }
                    if (self.installed.is_none() || self.installed.as_ref().is_some_and(|result| !result.failures.is_empty() && preview.host_ids.iter().all(|host| result.failures.iter().any(|failure| failure.host_id == *host)))) && ui.add_enabled(current && self.install_rx.is_none(), egui::Button::new(if preview.host_ids.is_empty() { if preview.isolated { "Save isolated runtime" } else { "Save portable Markdown" } } else { "OK - generate and install selected hosts" })).clicked() {
                        let preview = preview.clone();
                        let (tx, rx) = channel();
                        self.install_rx = Some(rx);
                        self.error.clear();
                        let context = context.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(crate::host_install::install(preview).map_err(|error| format!("{error:#}")));
                            context.request_repaint();
                        });
                    }
                }
                if self.install_rx.is_some() { ui.spinner(); ui.label("Installing..."); }
                if let Some(installed) = &self.installed {
                    ui.label(format!("Installed generation: {}", installed.directory.display()));
                    ui.label(if installed.isolated { "Isolated runtime generated" } else { "Normal tool setup generated" });
                    let document_path = if installed.isolated { installed.directory.join("runtime.md") } else { installed.policy_path.clone() };
                    ui.colored_label(egui::Color32::GREEN, format!("Generated {}", document_path.display()));
                    if ui.button("Refresh native horizon").clicked() {
                        let generation_id = installed.directory.file_name().and_then(|name| name.to_str()).unwrap_or_default();
                        self.native_horizon = crate::agent_setup::paths()
                            .and_then(|paths| std::fs::read(&paths.ledger).map_err(anyhow::Error::from))
                            .and_then(|bytes| serde_json::from_slice::<crate::agent_setup::Ledger>(&bytes).map_err(anyhow::Error::from))
                            .and_then(|ledger| {
                                let pointer = ledger.horizons.iter().find(|pointer| pointer.generation_id == generation_id).ok_or_else(|| anyhow::anyhow!("No committed horizon for this generation"))?;
                                let bytes = std::fs::read(&pointer.artifact_path)?;
                                let horizon: crate::native_scheduler::NativeHorizon = serde_json::from_slice(&bytes)?;
                                anyhow::ensure!(horizon.horizon_id == pointer.id && horizon.policy_version == pointer.policy_version && horizon.profile_identity == pointer.profile_identity && horizon.ledger_revision == pointer.committed_revision, "Committed horizon pointer and artifact differ");
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
                    if ui.button(if installed.isolated { "Copy runtime path" } else { "Copy Markdown path" }).clicked() { ui.ctx().copy_text(document_path.display().to_string()); }
                    for launcher in &installed.launchers {
                        ui.colored_label(egui::Color32::GREEN, format!("{}: Installed", launcher.host_id));
                        ui.label(&launcher.adapter);
                        ui.label(launcher.path.display().to_string());
                        ui.horizontal(|ui| {
                            ui.monospace(&launcher.command);
                            if ui.button("Copy command").clicked() { ui.ctx().copy_text(launcher.command.clone()); }
                        });
                    }
                    for failure in &installed.failures {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("{}: {}", failure.host_id, failure.message));
                    }
                    if !installed.failures.is_empty() && ui.add_enabled(current && self.install_rx.is_none(), egui::Button::new("Prepare retry for failed hosts only")).clicked() {
                        retry_failed = true;
                    }
                    for note in &installed.notes { ui.add(egui::Label::new(note).wrap()); }
                    ui.label(if installed.launchers.is_empty() { "Bundle saved without a conductor launcher. Generate with a host to launch." } else { "Run the copied command from any working directory. First launch verifies the machine profile and asks consolidated onboarding questions before paid delegation." });
                    if ui.add_enabled(score_current && self.install_rx.is_none(), egui::Button::new("Generate another setup")).clicked() { regenerate = true; }
                }
            });
        });
        self.open = open;
        if retry_failed && let Some(installed) = &self.installed {
            self.selected = installed
                .failures
                .iter()
                .map(|failure| failure.host_id.clone())
                .collect();
            self.add_to_path = false;
            self.preview_dirty = true;
        }
        if regenerate {
            self.prepare(table.clone(), settings.clone(), revision, context.clone());
        }
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
            let context = context.clone();
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

pub struct App {
    table: Table,
    vendor_tables: [Table; 5],
    selected_tab: usize,
    table_revision: u64,
    generator: Generator,
    panel_widths: std::collections::HashMap<(Seat, Tier), f32>,
    agent_hours_text: String,
    rho_override_text: String,
    collaboration_root_text: String,
    subscription_count_text: std::collections::BTreeMap<String, String>,
    workspace_status: String,
    workspace_picker: Option<Receiver<Result<Option<std::path::PathBuf>, String>>>,
    agent_hours_invalid: bool,
    refresh_in_flight: bool,
    retry_after: Option<Instant>,
    refresh_warning: String,
    table_rx: Receiver<(Table, [Table; 5], bool, Settings)>,
    scored_settings: Option<Settings>,
    settings_tx: Sender<(Settings, bool)>,
    engine_error_rx: Receiver<String>,
    engine_status: String,
    refresh_clicked_at: Option<Instant>,
    settings: Settings,
    settings_open: bool,
    selected_seat_tier: Option<(Seat, Tier)>,
    side_panel_open: bool,
    update_status: String,
    handoff_status: String,
    updater: Arc<Mutex<blockitall_update::Updater>>,
    health_guard: Option<blockitall_update::HealthGuard>,
    close_after_handoff: bool,
    update_rx: Option<Receiver<Result<blockitall_update::Checked, String>>>,
    available_update: Option<blockitall_update::Available>,
    staged_update: Option<blockitall_update::StagedArtifact>,
    install_rx: Option<Receiver<InstallResult>>,
    comparison_choice: std::collections::HashMap<(Seat, Tier), usize>,
    comparison_generation: u64,
    comparison_pending: Option<(u64, Seat, Tier, usize)>,
    comparison_result: Option<ComparisonResponse>,
    comparison_tx: Sender<ComparisonResponse>,
    comparison_rx: Receiver<ComparisonResponse>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        updater: blockitall_update::Updater,
        health_guard: blockitall_update::HealthGuard,
        handoff_status: String,
    ) -> Self {
        let (table_tx, table_rx) = channel();
        let settings = load_settings();
        let table = Table::empty();
        let (settings_tx, settings_rx) = channel::<(Settings, bool)>();
        let (comparison_tx, comparison_rx) = channel();
        let (engine_error_tx, engine_error_rx) = channel();
        let context = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            let mut loaded = None;
            'requests: while let Ok((mut settings, mut force_refresh)) = settings_rx.recv() {
                for (pending, pending_force) in settings_rx.try_iter() {
                    settings = pending;
                    force_refresh |= pending_force;
                }
                let mut fetched_rows = false;
                loop {
                    if force_refresh || loaded.is_none() {
                        fetched_rows = true;
                        match crate::aa::load_rows(force_refresh, f64::from(settings.cache_hours)) {
                            Ok(rows) => loaded = Some(rows),
                            Err(error) => {
                                let _ = engine_error_tx.send(format!("Engine: {error:#}"));
                                context.request_repaint();
                                continue 'requests;
                            }
                        }
                    }
                    force_refresh = false;
                    for (pending, pending_force) in settings_rx.try_iter() {
                        settings = pending;
                        force_refresh |= pending_force;
                    }
                    if !force_refresh {
                        break;
                    }
                }
                if let Some((rows, state, fetched)) = &loaded {
                    let table = crate::engine::score(
                        rows.clone(),
                        &settings,
                        *state,
                        fetched.clone(),
                        None,
                    );
                    let vendor_tables = VENDOR_TABS.map(|(_, vendor)| {
                        crate::engine::score(
                            rows.clone(),
                            &settings,
                            *state,
                            fetched.clone(),
                            Some(vendor),
                        )
                    });
                    if table_tx
                        .send((table, vendor_tables, fetched_rows, settings.clone()))
                        .is_err()
                    {
                        break;
                    }
                    context.request_repaint();
                }
            }
        });
        let _ = settings_tx.send((settings.clone(), false));

        let mut app = Self {
            table,
            vendor_tables: std::array::from_fn(|_| Table::empty()),
            selected_tab: 0,
            table_revision: 0,
            generator: Generator::new(cc.egui_ctx.clone()),
            panel_widths: std::collections::HashMap::new(),
            agent_hours_text: settings.agent_hours.to_string(),
            rho_override_text: settings
                .rho_override
                .map(|value| value.to_string())
                .unwrap_or_default(),
            collaboration_root_text: crate::workspace::resolve(&settings)
                .map(|paths| paths.root.display().to_string())
                .unwrap_or_default(),
            subscription_count_text: crate::subscriptions::PROVIDERS
                .iter()
                .map(|provider| {
                    (
                        provider.id.to_string(),
                        settings.subscription_count(provider.id).to_string(),
                    )
                })
                .collect(),
            workspace_status: String::new(),
            workspace_picker: None,
            agent_hours_invalid: false,
            refresh_in_flight: true,
            retry_after: None,
            refresh_warning: String::new(),
            table_rx,
            scored_settings: None,
            settings_tx,
            engine_error_rx,
            engine_status: "Loading rows…".to_string(),
            refresh_clicked_at: None,
            settings,
            settings_open: false,
            selected_seat_tier: None,
            side_panel_open: false,
            update_status: String::new(),
            handoff_status,
            updater: Arc::new(Mutex::new(updater)),
            health_guard: Some(health_guard),
            close_after_handoff: false,
            update_rx: None,
            available_update: None,
            staged_update: None,
            install_rx: None,
            comparison_choice: std::collections::HashMap::new(),
            comparison_generation: 0,
            comparison_pending: None,
            comparison_result: None,
            comparison_tx,
            comparison_rx,
        };

        let update_state = app.updater.lock().ok();
        if update_state
            .as_ref()
            .is_some_and(|updater| updater.due_for_check())
        {
            drop(update_state);
            app.trigger_update_check(cc.egui_ctx.clone());
        } else if let Some(version) =
            update_state.and_then(|updater| updater.cached_published_version().ok().flatten())
        {
            app.update_status = format!("Last seen: {version}");
        }
        app
    }

    fn selected_table(&self) -> &Table {
        if self.selected_tab == 0 {
            &self.table
        } else {
            &self.vendor_tables[self.selected_tab - 1]
        }
    }

    fn portfolio_content(&mut self, ui: &mut egui::Ui) {
        ui.label("One fixed orchestrator binding coordinates the whole plan. Every worker seat has four risk-qualified workflow policies. Forecast figures are proxy capacity, not native admission.");
        if self.settings.subscriptions.is_empty() {
            ui.add_space(8.0);
            ui.strong("Choose your subscriptions to build a weekly schedule.");
            if ui.button("Open settings").clicked() {
                self.settings_open = true;
            }
            return;
        }
        if !self.refresh_warning.is_empty() {
            ui.colored_label(egui::Color32::YELLOW, &self.refresh_warning);
        }
        if !self
            .scored_settings
            .as_ref()
            .is_some_and(|scored| scored.scoring_matches(&self.settings))
            || self.refresh_in_flight
        {
            if self.refresh_in_flight || !self.engine_status.is_empty() {
                ui.spinner();
                ui.label(if self.engine_status.is_empty() {
                    "Recalculating your weekly schedule..."
                } else {
                    &self.engine_status
                });
            } else {
                ui.label("Your current schedule is unavailable. Refresh to retry.");
            }
            return;
        }
        if self.source_expired() {
            ui.colored_label(egui::Color32::YELLOW, "Data is stale.");
        }
        let Some(portfolio) = &self.table.portfolio else {
            ui.label("No portfolio data is available. Refresh to load estimates.");
            return;
        };
        let price_prefix = if portfolio.pools.iter().any(|pool| pool.price_is_estimate) {
            "est. $"
        } else {
            "$"
        };
        let dispatch = &portfolio.dispatch;
        let executable = dispatch.executable && portfolio.conductor.is_some();
        ui.strong(format!("{} - {:.1} available h/account - {} proxy jobs - {} proxy-fit - {} native admitted - {price_prefix}{:.2}/month",
            dispatch.policy_version,
            portfolio.available_hours_per_provider, dispatch.forecast_changes, dispatch.admitted_changes,
            dispatch.native_admission_changes, portfolio.monthly_price));
        let status = if !executable {
            "No executable policy".to_string()
        } else if dispatch.solver.proven_optimal {
            "Optimal within the declared scheduler and numerical tolerances".to_string()
        } else if let Some(gap) = dispatch.solver.relative_gap {
            format!(
                "Feasible policy; role utility gap <= {:.2}% for admitted work",
                gap * 100.0
            )
        } else {
            "Feasible policy; role utility gap unavailable".to_string()
        };
        ui.add(egui::Label::new(status).wrap());
        if !portfolio.message.is_empty() {
            ui.add(egui::Label::new(&portfolio.message).wrap());
        } else if !dispatch.solver.message.is_empty() {
            ui.add(egui::Label::new(&dispatch.solver.message).wrap());
        }
        if ui
            .add_enabled(
                executable && self.generator.install_rx.is_none(),
                egui::Button::new("Generate Orchestrator..."),
            )
            .clicked()
        {
            self.generator.prepare(
                self.table.clone(),
                self.settings.clone(),
                self.table_revision,
                ui.ctx().clone(),
            );
        }
        egui::CollapsingHeader::new("Select a workflow from the risk vector").show(ui, |ui| {
            for demand in dispatch.classes.iter().rev() {
                ui.label(format!("{}: {}", demand.class.name(), demand.condition));
            }
            ui.label("Record consequence, uncertainty, coupling, reversibility, evidence need, tool risk, correlation, and deadline risk. Apply the first matching template from Extensive to Focused. File count informs load only.");
            ui.label(format!("Proxy assumption: one job per {:.1} working hours; {:.0}% repair incidence. These values neither authorize jobs nor establish native feasibility.", dispatch.cadence_hours, dispatch.repair_incidence * 100.0));
        });
        ui.add_space(8.0);
        ui.strong("Role policy");
        ui.label("Recommendations compare published model/effort configurations. Efforts absent from the current evidence are not scored or invented.");
        egui::Grid::new("portfolio_roles_grid").striped(true).spacing([12.0, 3.0]).min_col_width(50.0).show(ui, |ui| {
            for (heading, explanation) in [
                ("Workflow", "The orchestrator is fixed for the whole plan. Select each job's workflow from its recorded risk vector."),
                ("Agent", "Exact native harness, model, and reasoning effort appear on hover."),
                ("Subscription", "All routes share the same provider account and nested limits."),
                ("Attempts", "Maximum same-model attempts per visit, stopping on acceptance."),
                ("Competence", "Reference role competence, not actual task success probability."),
                ("Visits/wk", "Expected visits; full reserved visits and class admissions appear on hover."),
                ("Reserved USD", "Full weekly API-equivalent usage reservation; expected usage on hover."),
                ("Expected / reserve h", "Conservative weekly runtime bound across all visits and capped retries, not expected work duration. Expected and per-visit hours appear on hover."),
            ] {
                ui.strong(heading).on_hover_text(explanation);
            }
            ui.end_row();
            ui.strong("Orchestrator");
            for _ in 1..8 { ui.label(""); }
            ui.end_row();
            if let Some(conductor) = &portfolio.conductor
                && let Some(row) = self.table.rows.get(conductor.row_index)
            {
                ui.label("Whole plan").on_hover_text("One fixed model and effort coordinates every admitted class across all concurrent projects.");
                ui.label(row.display_name()).on_hover_text(format!("Benchmark harness: {}\nNative harness: {}\nModel: {}\nEffort: {}", row.harness, conductor.native_harness, row.model, row.effort.as_deref().unwrap_or("unspecified")));
                ui.label(&conductor.provider_id).on_hover_text(&conductor.plan_id);
                ui.label(conductor.attempt_limit.to_string());
                ui.label(competence_text(Some(conductor.competence))).on_hover_text(format!("Global orchestrator utility {:.4}.", conductor.utility));
                ui.label(conductor.expected_visits.to_string()).on_hover_text(format!("{} coordination visits reserved across all admitted changes. Proxy cost and decode time also apply each class's declared resource factor.", conductor.reserved_visits));
                ui.label(format!("{:.2}", conductor.reserved_usage)).on_hover_text(format!("Expected {:.2} USD; full reservation per visit {:.6} USD.", conductor.expected_usage, conductor.per_call_reserved_usage));
                ui.label(format!("{:.2} / {:.2}", conductor.expected_hours, conductor.reserved_hours)).on_hover_text(format!("Expected weekly hours across all conductor calls; high bound per visit {:.6} hours.", conductor.per_call_reserved_hours));
                ui.end_row();
            } else {
                ui.label("Whole plan");
                ui.add(egui::Label::new("No feasible fixed Orchestrator assignment; see the policy status above.").wrap());
                for _ in 2..8 { ui.label("-"); }
                ui.end_row();
            }
            for seat in crate::orchestration::ROLE_ORDER.into_iter().filter(|seat| *seat != Seat::Orchestrator) {
                let Some(role) = portfolio.roles.iter().find(|role| role.seat == seat) else { continue; };
                for _ in 0..8 {
                    ui.add(egui::Separator::default().horizontal().spacing(2.0));
                }
                ui.end_row();
                ui.strong(seat.name());
                for _ in 1..8 { ui.label(""); }
                ui.end_row();
                for rule in &role.rules {
                    if seat == Seat::NetResearch {
                        ui.label(format!("{} ({})", rule.class.name(), rule.planned_jobs))
                        .on_hover_text(format!("Policy ID: {}\nTrigger: {}\nFallback: {}\nCalibration: {}", rule.policy_id, rule.condition, rule.fallback_policy, rule.calibration));
                    } else {
                        ui.label(format!("{} ({})", rule.class.name(), rule.planned_jobs)).on_hover_text(format!("Policy ID: {}\nTrigger: {}\nExact account claim: {}\nFallback: {}\nCalibration: {}{}", rule.policy_id, rule.condition, rule.account_claim, rule.fallback_policy, rule.calibration, if rule.recommendation_only { format!("\nNo allocated jobs: {}", rule.message) } else { String::new() }));
                    }
                    if let Some(row) = rule.row_index.and_then(|index| self.table.rows.get(index)) {
                        ui.label(row.display_name()).on_hover_ui(|ui| {
                            ui.label(format!("Harness: {}\nModel: {}\nEffort: {}", row.harness, row.model, row.effort.as_deref().unwrap_or("unspecified")));
                            ui.label(&rule.failure_action);
                            ui.separator();
                            ui.strong("Conditional fallback policy");
                            ui.label(&rule.fallback_policy);
                            ui.label("Each fallback requires exact eligibility and a fresh native hold. The list is ordered policy, not a round-robin menu.");
                            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for alternative in rule.lower_effort.iter().chain(&rule.within_provider_alternatives).chain(&rule.surplus_alternatives) {
                                if let Some(candidate) = self.table.rows.get(alternative.row_index) {
                                    ui.label(format!("{} / {} - {}", alternative.provider_id, alternative.plan_id, candidate.display_name()));
                                    ui.small(format!("{}; effort {}; cap {}; competence {:.3}; role utility {:.3}; expected ${:.4} / {:.4}h; reserve ${:.4} / {:.4}h per visit", candidate.harness,
                                        candidate.effort.as_deref().unwrap_or("unspecified"), alternative.attempt_limit, alternative.competence, alternative.quality, alternative.per_call_expected_usage, alternative.per_call_expected_hours, alternative.per_call_reserved_usage, alternative.per_call_reserved_hours));
                                }
                            }
                            });
                            if rule.lower_effort.is_none() && rule.within_provider_alternatives.is_empty() && rule.surplus_alternatives.is_empty() {
                                ui.label("No qualified alternative in the current evidence.");
                            }
                        });
                        ui.label(rule.provider_id.as_deref().unwrap_or("-"))
                            .on_hover_text(rule.plan_id.as_deref().unwrap_or("-"));
                        ui.label(rule.attempt_limit.to_string());
                        ui.label(competence_text(rule.competence)).on_hover_text(format!("Role utility {:.4}; weighted benchmark score, with each signal counted once.", rule.utility));
                        ui.label(format!("{:.1}", rule.expected_visits))
                            .on_hover_text(format!("{} visits reserved at full cap. Expected autonomous reference-equivalent service units: {:.1}.", rule.reserved_visits, rule.expected_completions));
                        ui.label(format!("{:.2}", rule.reserved_usage))
                            .on_hover_text(format!("Expected {:.2} USD; full reservation per visit {:.6} USD.", rule.nominal_usage, rule.per_call_reserved_usage));
                        ui.label(format!("{:.2} / {:.2}", rule.nominal_hours, rule.reserved_hours))
                            .on_hover_text(format!("Expected {:.2} weekly hours across visits; high bound per visit {:.6} hours.", rule.nominal_hours, rule.per_call_reserved_hours));
                    } else {
                        if rule.research_candidates.is_empty() {
                            ui.add(egui::Label::new(&rule.message).wrap());
                            for _ in 2..8 {
                                ui.label("-");
                            }
                        } else {
                            let primary = rule.research_candidates.iter().find(|candidate| candidate.primary).unwrap_or(&rule.research_candidates[0]);
                            let primary_row = &self.table.rows[primary.row_index];
                            ui.label(research_route_name(primary_row, &primary.native_harness)).on_hover_ui(|ui| {
                                        ui.label(format!("Default: {} / {} / {}", primary.native_harness, primary_row.model, primary_row.effort.as_deref().unwrap_or("unspecified")));
                                        ui.small(format!("Accuracy {:.4} × {:.2}; non-hallucination {:.4} × {:.2}; LCR {:.4} × {:.2}; HLE {} × {:.2}", primary.accuracy, primary.accuracy_weight, primary.non_hallucination, primary.non_hallucination_weight, primary.lcr, primary.lcr_weight, primary.hle.map(|value| format!("{value:.4}")).unwrap_or_else(|| "n/a".to_string()), primary.hle_weight));
                                        ui.small(format!("Diagnostics: GPQA {}; GDP.pdf {}", primary.gpqa_diagnostic.map(|value| format!("{value:.4}")).unwrap_or_else(|| "n/a".to_string()), primary.gdp_pdf_diagnostic.map(|value| format!("{value:.4}")).unwrap_or_else(|| "n/a".to_string())));
                                        ui.small(format!("AA reference workload: {} USD; {} decode h", primary.expected_usd.map(|value| format!("{value:.4}")).unwrap_or_else(|| "Unknown".to_string()), primary.decode_hours.map(|value| format!("{value:.4}")).unwrap_or_else(|| "Unknown".to_string())));
                                        ui.small(&primary.source);
                                        ui.hyperlink_to("Artificial Analysis methodology", AA_INTELLIGENCE_METHODOLOGY_URL);
                                        ui.small("The weights express this template's policy priorities; the score is not task-success probability. Resource values are benchmark-workload proxies, not native latency or quota.");
                                        ui.label(&primary.eligibility);
                                        for (rank, candidate) in rule.research_candidates.iter().filter(|candidate| !candidate.primary).enumerate() {
                                            let row = &self.table.rows[candidate.row_index];
                                            ui.separator();
                                            ui.strong(format!("Alternative {} · {} · score {:.4}", rank + 1, research_route_name(row, &candidate.native_harness), candidate.score));
                                            ui.small(format!("{} / {}; accuracy {:.4}; non-hallucination {:.4}; LCR {:.4}; HLE {}; AA proxy {} USD / {} h", candidate.provider_id, candidate.plan_id, candidate.accuracy, candidate.non_hallucination, candidate.lcr, candidate.hle.map(|value| format!("{value:.4}")).unwrap_or_else(|| "n/a".to_string()), candidate.expected_usd.map(|value| format!("{value:.4}")).unwrap_or_else(|| "Unknown".to_string()), candidate.decode_hours.map(|value| format!("{value:.4}")).unwrap_or_else(|| "Unknown".to_string())));
                                            ui.small(&candidate.eligibility);
                                        }
                            });
                            ui.label(&primary.provider_id).on_hover_text(format!("Plan: {}\nNative harness: {}\nBinding: {}\nBenchmark harness: {}", primary.plan_id, primary.native_harness, primary.binding_id, primary_row.harness));
                            ui.label("-").on_hover_text("Research attempt caps are set during task-specific native qualification.");
                            ui.label(competence_text(Some(primary.score))).on_hover_text("Weighted AA benchmark score; policy weights, not task-success probability.");
                            ui.label("On demand");
                            ui.label("-").on_hover_text(format!("No native reservation. AA reference-workload proxy: {} USD.", primary.expected_usd.map(|value| format!("{value:.4}")).unwrap_or_else(|| "Unknown".to_string())));
                            ui.label("-").on_hover_text(format!("No native reserved hours. AA reference-workload decode proxy: {} hours.", primary.decode_hours.map(|value| format!("{value:.4}")).unwrap_or_else(|| "Unknown".to_string())));
                        }
                    }
                    ui.end_row();
                }
            }
        });
        ui.add_space(8.0);
        ui.strong("Subscription budgets and time");
        egui::Grid::new("portfolio_allowances")
            .striped(true)
            .spacing([12.0, 3.0])
            .show(ui, |ui| {
                for heading in [
                    "Subscription",
                    "Allowance USD",
                    "Expected USD",
                    "Reserved USD",
                    "Unreserved USD",
                    "Reserve / available h",
                ] {
                    ui.strong(heading);
                }
                ui.end_row();
                for pool in &portfolio.pools {
                    ui.label(format!(
                        "{} - {} × {}",
                        pool.provider_name, pool.plan_name, pool.subscription_count
                    ));
                    ui.label(format!("{:.2}", pool.weekly_capacity));
                    ui.label(format!("{:.2}", pool.nominal_usage));
                    ui.label(format!("{:.2}", pool.reserved_usage));
                    ui.label(format!("{:.2}", pool.remaining_reserved_capacity));
                    ui.label(format!(
                        "{:.2} / {:.1}",
                        pool.reserved_hours, pool.available_hours
                    ))
                    .on_hover_text(format!(
                        "Expected weekly runtime {:.2} hours; {:.2} available hours remain outside the high reserve.",
                        pool.scheduled_hours, pool.remaining_reserved_hours
                    ));
                    ui.end_row();
                }
            });
        egui::CollapsingHeader::new("Math and account allocation").show(ui, |ui| {
            ui.add(egui::Label::new(&portfolio.math_audit.objective).wrap());
            ui.add(egui::Label::new(&portfolio.math_audit.coordination_proxy).wrap());
            ui.label(format!("Orchestrator share of modeled spend: {:.1}% expected / {:.1}% full reserve.", portfolio.math_audit.orchestrator_expected_spend_share * 100.0, portfolio.math_audit.orchestrator_reserved_spend_share * 100.0));
            ui.label("Task classes scale reference volume, not measured difficulty. The four-hour cadence and repair incidence are planning assumptions. Released reservations can fund qualified useful work; expected usage is not a promise to spend the full allowance in the working window.");
            egui::Grid::new("portfolio_math_audit").striped(true).show(ui, |ui| {
                for heading in ["Role", "Account", "Expected / reserve USD", "Expected / reserve h"] { ui.strong(heading); }
                ui.end_row();
                for usage in &portfolio.math_audit.role_accounts {
                    ui.label(usage.seat.name());
                    ui.label(&usage.provider_id);
                    ui.label(format!("{:.2} / {:.2}", usage.expected_usage, usage.reserved_usage));
                    ui.label(format!("{:.2} / {:.2}", usage.expected_hours, usage.reserved_hours));
                    ui.end_row();
                }
            });
        });
        egui::CollapsingHeader::new("Workflow, forecast, and assumptions").show(ui, |ui| {
            ui.add(egui::Label::new(format!("{}: {}", dispatch.solver.status, dispatch.solver.message)).wrap());
            if !portfolio.message.is_empty() {
                ui.add(egui::Label::new(&portfolio.message).wrap());
            }
            ui.label("The proxy forecast reserves a brief, context, implementation, checks, one repair, and close. Runtime expands each authorized task ID from its risk-qualified workflow and invokes only required seats, including Net Research when evidence is required.");
            ui.label("The dispatcher verifies exact bindings, billing, research tools, permissions, account windows, and concurrency before native admission. Modeled USD, proxy-fit jobs, and forecast slots are not native quota or runtime launch times.");
            ui.label(format!("Reserved workflow makespan: {:.2} / {:.1} hours. This is the static baseline feasibility estimate, not a runtime calendar to wait for.", dispatch.reserved_makespan_hours, portfolio.available_hours_per_provider));
            ui.label(format!("Role utility {:.4}; proven optimal within declared scheduler and numerical tolerances: {}; role utility gap for admitted workload: {}.", dispatch.solver.quality, dispatch.solver.proven_optimal,
                dispatch.solver.relative_gap.map(|gap| format!("{:.2}%", gap * 100.0)).unwrap_or_else(|| "unavailable".to_string())));
            ui.label(format!("Role utility bound for admitted workload: {}; LP nodes: {}. Optimality is qualified by the declared scheduler and numerical tolerances.",
                dispatch.solver.bound.map(|bound| format!("{bound:.6}")).unwrap_or_else(|| "unavailable".to_string()), dispatch.solver.nodes));
            for demand in &dispatch.classes {
                ui.label(format!("{}: {} forecast, {} admitted, {} deferred; resource factor {:.2}", demand.class.name(), demand.forecast_changes, demand.admitted_changes,
                    demand.forecast_changes.saturating_sub(demand.admitted_changes), demand.resource_factor));
            }
            for limit in &portfolio.limits {
                ui.label(format!("{} {}: {:.2} reserved / {:.2} allowance USD; {:.2} unreserved.", limit.provider_id, limit.name, limit.reserved_usage, limit.weekly_capacity, limit.remaining_capacity));
            }
            for assumption in &portfolio.assumptions {
                ui.add(egui::Label::new(assumption).wrap());
            }
            for pool in &portfolio.pools {
                ui.hyperlink_to(format!("{} {} plan source", pool.provider_name, pool.plan_name), &pool.source_url);
                ui.add(egui::Label::new(&pool.allowance_basis).wrap());
            }
        });
    }

    fn frontier_content(&self, ui: &mut egui::Ui, seat: Seat, tier: Tier) -> Option<f64> {
        let mut selected_floor = None;
        let frontier = self
            .table
            .frontiers
            .iter()
            .find(|frontier| frontier.seat == seat && frontier.tier == tier);
        ui.add_space(10.0);
        egui::CollapsingHeader::new("Optional competence minimums")
            .default_open(false)
            .show(ui, |ui| {
        ui.label("Add a hard qualification requirement only when the seat needs one. The automatic pick maximizes nominal verified reference tasks per week, with the scenario band shown.");
        if let Some(frontier) = frontier {
            egui::ScrollArea::both()
                .id_salt(("frontier_scroll", seat, tier))
                .max_height(280.0)
                .show(ui, |ui| {
                    egui::Grid::new(("frontier_grid", seat, tier))
                        .striped(true)
                        .show(ui, |ui| {
                            for heading in [
                                "", "Minimum", "Agent", "Score", "MAX", "Agent/wk", "Range",
                                "Worst capacity shortfall",
                            ] {
                                ui.strong(heading);
                            }
                            ui.end_row();
                            for point in &frontier.points {
                                let name = self
                                    .table
                                    .rows
                                    .get(point.row_index)
                                    .map(|row| row.display_name())
                                    .unwrap_or_else(|| format!("Row #{}", point.row_index));
                                if ui.button("Use minimum").clicked() {
                                    selected_floor = Some(point.competence_floor);
                                }
                                ui.label(format!("{:.3}", point.competence_floor));
                                ui.add_sized(
                                    [190.0, ui.spacing().interact_size.y],
                                    egui::Label::new(name).wrap(),
                                );
                                ui.label(competence_text(point.competence));
                                ui.label(point.attempt_limit.to_string());
                                ui.label(format!("{:.1}", point.tasks_per_week));
                                ui.label(format!("{:.1}–{:.1}", point.tasks_low, point.tasks_high));
                                ui.label(format!("{:.1}%", point.capacity_shortfall * 100.0));
                                ui.end_row();
                            }
                        });
                });
            egui::CollapsingHeader::new(format!(
                "Excluded candidates ({})",
                frontier.excluded.len()
            ))
            .default_open(false)
            .show(ui, |ui| {
                for excluded in &frontier.excluded {
                    let name = self
                        .table
                        .rows
                        .get(excluded.row_index)
                        .map(|row| row.display_name())
                        .unwrap_or_else(|| format!("Row #{}", excluded.row_index));
                    ui.label(format!("{name}: {}", excluded.reason));
                }
            });
        } else {
            ui.label("No competence-minimum choices are available.");
        }
            });
        selected_floor
    }

    fn detail_content(&self, ui: &mut egui::Ui, seat: Seat, tier: Tier) -> Option<f64> {
        if seat == Seat::NetResearch {
            let table = self.selected_table();
            let Some(pick) = table.research_tiers.iter().find(|pick| pick.tier == tier) else {
                ui.label("No AA research route is available at this tier.");
                return None;
            };
            ui.label(&pick.scope);
            let Some(primary) = &pick.primary else {
                ui.label("No eligible research configuration is available.");
                return None;
            };
            let row = table.rows.get(primary.row_index)?;
            ui.heading("Selected policy");
            ui.label(
                egui::RichText::new(research_route_name(row, &primary.native_harness))
                    .strong()
                    .size(15.0),
            );
            ui.label(format!(
                "AA research score: {}",
                competence_text(Some(primary.score))
            ));
            ui.label(format!(
                "Route: {} / {} · {}",
                primary.provider_id, primary.plan_id, primary.native_harness
            ));
            ui.label(format!(
                "Accuracy {:.4} × {:.2}; non-hallucination {:.4} × {:.2}; LCR {:.4} × {:.2}",
                primary.accuracy,
                primary.accuracy_weight,
                primary.non_hallucination,
                primary.non_hallucination_weight,
                primary.lcr,
                primary.lcr_weight
            ));
            ui.label(format!(
                "Diagnostics: GPQA {}; GDP.pdf {}",
                primary
                    .gpqa_diagnostic
                    .map(|value| format!("{value:.4}"))
                    .unwrap_or_else(|| "n/a".to_string()),
                primary
                    .gdp_pdf_diagnostic
                    .map(|value| format!("{value:.4}"))
                    .unwrap_or_else(|| "n/a".to_string())
            ));
            ui.label(format!(
                "AA reference workload: {} USD · {} decode minutes",
                primary
                    .expected_usd
                    .map(|value| format!("{value:.4}"))
                    .unwrap_or_else(|| "Unknown".to_string()),
                primary
                    .decode_hours
                    .map(|value| format!("{:.2}", value * 60.0))
                    .unwrap_or_else(|| "Unknown".to_string())
            ));
            ui.label(&primary.eligibility);
            ui.small(&primary.source);
            ui.hyperlink_to(
                "Artificial Analysis methodology",
                AA_INTELLIGENCE_METHODOLOGY_URL,
            );
            egui::CollapsingHeader::new(format!("Alternatives ({})", pick.alternatives.len()))
                .show(ui, |ui| {
                    for candidate in &pick.alternatives {
                        let Some(candidate_row) = table.rows.get(candidate.row_index) else {
                            continue;
                        };
                        ui.strong(format!(
                            "{} · score {}",
                            research_route_name(candidate_row, &candidate.native_harness),
                            competence_text(Some(candidate.score))
                        ));
                        ui.small(format!(
                            "{} / {} · AA reference {} USD / {} decode min",
                            candidate.provider_id,
                            candidate.plan_id,
                            candidate
                                .expected_usd
                                .map(|value| format!("{value:.4}"))
                                .unwrap_or_else(|| "Unknown".to_string()),
                            candidate
                                .decode_hours
                                .map(|value| format!("{:.2}", value * 60.0))
                                .unwrap_or_else(|| "Unknown".to_string())
                        ));
                        ui.small(&candidate.eligibility);
                    }
                });
            return None;
        }
        let table = self.selected_table();
        let selected_floor;
        if let Some(pick) = table.get_pick(seat, tier) {
            let selected_row = table.rows.get(pick.row_index);

            ui.heading("Selected policy");
            if let Some(row) = selected_row {
                ui.label(egui::RichText::new(row.display_name()).strong().size(15.0));
                ui.label(format!("Vendor: {} | Harness: {}", row.vendor, row.harness));
                ui.label(row.retry.description());
            }
            ui.add_space(4.0);
            ui.label(format!("Role competence: {}", competence_text(pick.competence))).on_hover_text(
                "Fixed role-weighted benchmark utility on a 0–1 scale. It is displayed and enforces any optional minimum; it breaks ties after nominal capacity and worst scenario capacity shortfall. It is not a retry success probability.",
            );
            if let Some(floor) = pick.competence_floor {
                ui.label(format!("Required competence minimum: {floor:.3}"));
            } else {
                ui.label("Most nominal verified reference tasks per week; scenario band shown");
            }
            ui.label(format!(
                "Worst scenario capacity shortfall: {:.1}%",
                pick.capacity_shortfall * 100.0
            ));
            ui.label(format!(
                "Estimated {:.2} min/cycle | ${:.2}/cycle total",
                pick.minutes_per_task, pick.cost_per_task,
            ));
            ui.label(format!(
                "Per-cycle cost: ${:.2} vendor usage + ${:.2} external rescue",
                pick.model_cost_per_task, pick.escalation_cost_per_task,
            ));
            ui.label(format!(
                "MAX {} attempts/cycle | Estimated rescue share: {:.1}%",
                pick.attempt_limit,
                pick.escalation_rate * 100.0,
            ))
            .on_hover_text(
                "One attempt cap applies to every task type in this role. Attempts stop on success. Tasks still unfinished at the cap use the configured escalation time and cost.",
            );
            ui.label(format!(
                "Agent completions: {:.1}/week (scenario range {:.1}–{:.1})",
                pick.tasks_per_week, pick.tasks_low, pick.tasks_high,
            )).on_hover_text(
                "Reference tasks completed by the agent before the retry cap. Assisted completions are reported separately and never credited to the agent.",
            );
            ui.label(format!(
                "Assisted completions: {:.1}/week · rescue work: {:.1} hours/week",
                pick.assisted_tasks_per_week, pick.escalation_hours_per_week,
            ));
            ui.label(format!(
                "Workflow time: {:.1} agent hours + {:.1} rescue hours/week across {:.1} cycles",
                pick.agent_hours_per_week, pick.escalation_hours_per_week, pick.cycles_per_week,
            ));
            if let Some(capacity) = pick.streams_star {
                ui.label(format!("Capacity multiple: {:.2}x", capacity)).on_hover_text(
                    "How many times the current weekly workload the estimated plan allowance can fund.",
                );
            }
            selected_floor = self.frontier_content(ui, seat, tier);

            ui.add_space(10.0);
            ui.heading("Role competence and reference workload");
            ui.add(egui::Label::new(
                "Role competence and operating capacity are separate. The competence profile expresses the seat's capability judgment. Retry success, time, and cost come only from the reference workload.",
            ).wrap());
            if let Some(row) = selected_row {
                ui.label(format!(
                    "Fixed role competence: {}",
                    competence_text(pick.competence)
                ));
                if let Some(components) = crate::engine::competence_components(row, seat) {
                    egui::Grid::new(("competence_grid", seat, tier))
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Capability");
                            ui.strong("Score");
                            ui.strong("Fixed weight");
                            ui.end_row();
                            for (name, score, weight) in components {
                                ui.label(name);
                                ui.label(format!("{score:.3}"));
                                ui.label(format!("{:.1}%", weight * 100.0));
                                ui.end_row();
                            }
                        });
                }
                ui.add_space(6.0);
                ui.label(format!(
                    "Reference workload inputs: {}",
                    crate::engine::reference_workload_name(seat)
                ));
                egui::Grid::new(("workload_grid", seat, tier))
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Task component");
                        ui.strong("Reference weight");
                        ui.strong("Pass");
                        ui.strong("Time/attempt");
                        ui.strong("Cost/attempt");
                        ui.end_row();
                        for (benchmark, weight) in crate::engine::reference_workload(seat) {
                            let metric = row
                                .task_metrics
                                .iter()
                                .find(|metric| metric.benchmark == *benchmark);
                            ui.add_sized(
                                [88.0, ui.spacing().interact_size.y],
                                egui::Label::new(benchmark.name()).wrap(),
                            );
                            ui.label(format!("{:.1}%", weight * 100.0));
                            ui.label(
                                metric
                                    .map(|metric| {
                                        format!(
                                            "{:.1}%",
                                            row.retry.adjusted_pass(*benchmark, metric.pass)
                                                * 100.0
                                        )
                                    })
                                    .unwrap_or_else(|| "Unknown".into()),
                            );
                            let time = ui.label(
                                metric
                                    .map(|metric| format!("{:.2} min", metric.seconds / 60.0))
                                    .unwrap_or_else(|| "Unknown".into()),
                            );
                            if let Some(metric) = metric {
                                time.on_hover_text(&metric.time_basis);
                            }
                            let cost = ui.label(
                                metric
                                    .map(|metric| format!("${:.3}", metric.usd))
                                    .unwrap_or_else(|| "Unknown".into()),
                            );
                            if let Some(metric) = metric {
                                cost.on_hover_text(&metric.cost_basis);
                            }
                            ui.end_row();
                        }
                    });

                ui.label(format!(
                    "Intelligence index: {}",
                    row.smart
                        .map(|value| format!("{:.1} points", value * 100.0))
                        .unwrap_or_else(|| "Unknown".into()),
                ))
                .on_hover_text(
                    "Normalized intelligence index; contributes half of Orchestrator competence. It is not a success probability.",
                );

                if self.settings.show_hallucination {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Hallucination reliability indicator:");
                        ui.label(
                            row.halluc
                                .map(|value| format!("{:.1}%", value * 100.0))
                                .unwrap_or_else(|| "Unknown".into()),
                        );
                    });
                    ui.add(egui::Label::new(
                        "This optional source indicator is informational. It is not used as a retry-transition probability, and missing data is not treated as perfect reliability.",
                    ).wrap());
                }
            }

            ui.add_space(10.0);
            ui.heading("Declared capacity scenarios");
            ui.add(egui::Label::new(format!(
                "The engine evaluates joint corners at the configured {:.1}% stress distance, allowance endpoints, and both resource-time bases. This finite scenario set is assumed rather than measured. The selected candidate and retry cap stay fixed; each scenario comparator may use its own hindsight-best cap.",
                self.settings.assumption_span_pct,
            )).wrap());
            egui::CollapsingHeader::new(format!("Scenarios ({})", pick.scenarios.len()))
                .default_open(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("scenario_scroll", seat, tier))
                        .max_height(260.0)
                        .show(ui, |ui| {
                        for scenario in &pick.scenarios {
                            let leader = table.rows.get(scenario.row_index)
                                .map(|row| row.display_name())
                                .unwrap_or_else(|| format!("Row #{}", scenario.row_index));
                            ui.label(format!(
                                "{}: autonomous-capacity leader {} MAX {} at {:.1}; selected policy {:.1} agent completions/week ({:.1}% shortfall)",
                                scenario.name,
                                leader,
                                scenario.attempt_limit,
                                scenario.tasks_per_week,
                                scenario.selected_tasks_per_week,
                                scenario.capacity_shortfall * 100.0,
                            ));
                        }
                    });
                });

            ui.add_space(10.0);
            ui.heading(format!(
                "Top-{} nominal throughput policies",
                pick.top.len().min(4)
            ));
            ui.label(
                "Ordered by most nominal verified reference tasks per week, with the scenario band shown.",
            );
            for (idx, cand) in pick.top.iter().take(4).enumerate() {
                ui.group(|ui| {
                    let cand_row = table.rows.get(cand.row_index);
                    let name = cand_row
                        .map(|r| r.display_name())
                        .unwrap_or_else(|| format!("Row #{}", cand.row_index));
                    ui.strong(format!("#{}: {}", idx + 1, name));
                    ui.label(format!(
                        "Competence {} | Worst scenario capacity shortfall {:.1}%",
                        competence_text(cand.competence),
                        cand.capacity_shortfall * 100.0,
                    ));
                    ui.label(format!(
                        "{:.1} agent completions/wk ({:.1}–{:.1}) | {:.2} min/cycle | ${:.2}/cycle | MAX {} attempts",
                        cand.tasks_per_week,
                        cand.tasks_low,
                        cand.tasks_high,
                        cand.minutes_per_task,
                        cand.cost_per_task,
                        cand.attempt_limit,
                    ));
                    ui.label(format!(
                        "Capacity multiple: {}",
                        cand.streams_star
                            .map(|s| format!("{:.2}x", s))
                            .unwrap_or_else(|| "-".into())
                    ));
                    ui.label(format!(
                        "${:.2} vendor usage/cycle + ${:.2} rescue cost/cycle | rescue {:.1}%",
                        cand.model_cost_per_task,
                        cand.escalation_cost_per_task,
                        cand.escalation_rate * 100.0,
                    ));
                });
            }
        } else {
            ui.colored_label(
                egui::Color32::YELLOW,
                if self
                    .settings
                    .competence_floors
                    .get(seat.name())
                    .copied()
                    .flatten()
                    .is_some()
                {
                    "No candidate meets the configured competence minimum for this seat and tier."
                        .to_string()
                } else if self.selected_tab != 0 {
                    format!(
                        "{} has no eligible plan or candidate at this tier.",
                        VENDOR_TABS[self.selected_tab - 1].0
                    )
                } else {
                    "No eligible candidate has this seat's reference workload data.".to_string()
                },
            );
            selected_floor = self.frontier_content(ui, seat, tier);
        }

        ui.add_space(10.0);
        ui.heading("Source & Timestamps");
        ui.label("Source: Artificial Analysis");
        ui.label(format!("Source Fetched: {}", table.source_fetched_at));
        ui.label(format!("Generated At: {}", table.generated_at));
        ui.label(format!("Cache State: {}", table.cache_state.name()));
        selected_floor
    }

    fn counterfactual_content(&mut self, ui: &mut egui::Ui, seat: Seat, tier: Tier) {
        let table = if self.selected_tab == 0 {
            &self.table
        } else {
            &self.vendor_tables[self.selected_tab - 1]
        };
        let Some(pick) = table.get_pick(seat, tier) else {
            return;
        };
        let mut alternatives = Vec::new();
        for candidate in &pick.top {
            if candidate.row_index != pick.row_index && !alternatives.contains(&candidate.row_index)
            {
                alternatives.push(candidate.row_index);
            }
        }
        if alternatives.is_empty() {
            return;
        }
        let choice = self
            .comparison_choice
            .entry((seat, tier))
            .or_insert(alternatives[0]);
        if !alternatives.contains(choice) {
            *choice = alternatives[0];
        }

        ui.add_space(10.0);
        ui.heading("What would change this pick?");
        ui.label("Each estimate changes only this configuration. Competitors stay unchanged while nominal throughput, scenario bands, and retry caps are recomputed across 24 bounded sampled changes.");
        let selected_name = table
            .rows
            .get(*choice)
            .map(|row| row.display_name())
            .unwrap_or_else(|| format!("Row #{}", *choice));
        egui::ComboBox::from_id_salt(("comparison_choice", seat, tier))
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for row_index in alternatives {
                    let name = table
                        .rows
                        .get(row_index)
                        .map(|row| row.display_name())
                        .unwrap_or_else(|| format!("Row #{row_index}"));
                    ui.selectable_value(choice, row_index, name);
                }
            });

        let request = (self.comparison_generation, seat, tier, *choice);
        let pending = self.comparison_pending.is_some();
        let score_is_current = self
            .scored_settings
            .as_ref()
            .is_some_and(|scored| scored.scoring_matches(&self.settings))
            && !self.refresh_in_flight;
        if ui
            .add_enabled(
                !pending && score_is_current,
                egui::Button::new(if pending {
                    "Calculating…"
                } else {
                    "Calculate"
                }),
            )
            .clicked()
        {
            let rows = table.rows.clone();
            let settings = self.settings.clone();
            let sender = self.comparison_tx.clone();
            let context = ui.ctx().clone();
            self.comparison_pending = Some(request);
            self.comparison_result = None;
            std::thread::spawn(move || {
                let report =
                    crate::comparison::compare_candidate(&rows, &settings, seat, tier, request.3);
                let _ = sender.send((request.0, seat, tier, request.3, report));
                context.request_repaint();
            });
        }

        if let Some((generation, result_seat, result_tier, row_index, report)) =
            &self.comparison_result
            && (*generation, *result_seat, *result_tier, *row_index) == request
        {
            if let Some(target) = &report.target {
                ui.label(format!(
                    "Current alternative: competence {}, {:.1} agent completions/week, MAX {}",
                    competence_text(target.competence),
                    target.autonomous_tasks_per_week,
                    target.attempt_limit
                ));
            }
            for (label, threshold) in [
                ("Agent runtime reduction", &report.runtime),
                ("Vendor usage cost per attempt", &report.vendor_usage_cost),
                ("Allowance for this configuration", &report.allowance),
            ] {
                ui.group(|ui| {
                    ui.strong(label);
                    ui.label(&threshold.detail);
                    if let Some(factor) = threshold.applied_factor {
                        if label == "Allowance for this configuration" {
                            ui.label(format!(
                                "Estimated winning allowance: about {factor:.2}× current"
                            ));
                        } else {
                            ui.label(format!(
                                "Estimated winning reduction: about {:.1}%",
                                (1.0 - factor) * 100.0,
                            ));
                        }
                    }
                });
            }
        }
    }

    fn invalidate_comparisons(&mut self) {
        if !self
            .scored_settings
            .as_ref()
            .is_some_and(|scored| scored.scoring_matches(&self.settings))
        {
            self.generator.markdown = None;
        }

        self.comparison_generation = self.comparison_generation.wrapping_add(1);
        self.comparison_pending = None;
        self.comparison_result = None;
    }

    fn detail_width(&mut self, ui: &mut egui::Ui, seat: Seat, tier: Tier) -> f32 {
        if let Some(width) = self.panel_widths.get(&(seat, tier)) {
            return *width;
        }
        let mut measure = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("detail_measure", seat, tier))
                .max_rect(egui::Rect::from_min_size(
                    ui.min_rect().min,
                    egui::Vec2::ZERO,
                ))
                .layout(egui::Layout::top_down(egui::Align::Min))
                .invisible()
                .sizing_pass(),
        );
        measure.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let _ = self.detail_content(&mut measure, seat, tier);
        let heading = ui.painter().layout_no_wrap(
            format!("{} - {}", seat.name(), tier.name()),
            egui::TextStyle::Heading.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        let width = measure
            .min_rect()
            .width()
            .max(heading.size().x + ui.spacing().interact_size.y + ui.spacing().item_spacing.x)
            + 2.0 * f32::from(egui::Frame::side_top_panel(ui.style()).inner_margin.left)
            + ui.spacing().scroll.allocated_width();
        self.panel_widths.insert((seat, tier), width);
        width
    }

    fn source_expired(&self) -> bool {
        OffsetDateTime::parse(&self.table.source_fetched_at, &Rfc3339)
            .map(|fetched| {
                OffsetDateTime::now_utc()
                    >= fetched + time::Duration::hours(i64::from(self.settings.cache_hours))
            })
            .unwrap_or(true)
    }

    fn start_refresh(&mut self) {
        if self.refresh_in_flight {
            return;
        }
        if self.settings_tx.send((self.settings.clone(), true)).is_ok() {
            self.invalidate_comparisons();
            self.scored_settings = None;
            self.refresh_in_flight = true;
            self.refresh_clicked_at = Some(Instant::now());
            self.engine_status = "Refreshing…".to_string();
        }
    }

    fn trigger_update_check(&mut self, context: egui::Context) {
        let (tx, rx) = channel();
        self.update_rx = Some(rx);
        if self.install_rx.is_none() {
            self.update_status = "Checking for updates…".to_string();
        }
        let updater = Arc::clone(&self.updater);
        std::thread::spawn(move || {
            let outcome = updater
                .lock()
                .map_err(|_| "the updater is unavailable".to_owned())
                .and_then(|mut updater| updater.check().map_err(|error| format!("{error:#}")));
            let _ = tx.send(outcome);
            context.request_repaint();
        });
    }

    fn start_install(&mut self, context: egui::Context) {
        let staged = self.staged_update.take();
        let available = self.available_update.clone();
        let updater = Arc::clone(&self.updater);
        let (tx, rx) = channel();
        self.install_rx = Some(rx);
        self.update_status = if staged.is_some() {
            "Installing…"
        } else {
            "Downloading…"
        }
        .to_string();
        self.handoff_status.clear();
        std::thread::spawn(move || {
            let downloaded = match staged {
                Some(staged) => Ok(staged),
                None => available
                    .ok_or_else(|| "No update is available".to_owned())
                    .and_then(|available| {
                        updater
                            .lock()
                            .map_err(|_| "the updater is unavailable".to_owned())?
                            .download(&available, |_, _| {})
                            .map_err(|error| format!("{error:#}"))
                    }),
            };
            let result = match downloaded {
                Ok(mut staged) => match updater
                    .lock()
                    .map_err(|_| "the updater is unavailable".to_owned())
                    .and_then(|updater| {
                        updater
                            .hand_off(&mut staged, true)
                            .map_err(|error| format!("{error:#}"))
                    }) {
                    Ok(()) => (Ok(()), None),
                    Err(error) => (Err(error), Some(staged)),
                },
                Err(error) => (Err(error), None),
            };
            let _ = tx.send(result);
            context.request_repaint();
        });
    }

    fn pump_channels(&mut self) {
        if let Some(result) = self
            .workspace_picker
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.workspace_picker = None;
            match result {
                Ok(Some(path)) => self.collaboration_root_text = path.display().to_string(),
                Ok(None) => {}
                Err(error) => self.workspace_status = error,
            }
        }
        self.generator.pump();
        while let Ok((new_table, vendor_tables, fetched_rows, scored_settings)) =
            self.table_rx.try_recv()
        {
            if fetched_rows {
                self.refresh_in_flight = false;
                if new_table.cache_state == CacheState::Live {
                    self.refresh_warning.clear();
                    self.retry_after = None;
                } else if self.refresh_clicked_at.is_some()
                    || OffsetDateTime::parse(&new_table.source_fetched_at, &Rfc3339)
                        .map(|fetched| {
                            OffsetDateTime::now_utc()
                                >= fetched
                                    + time::Duration::hours(i64::from(self.settings.cache_hours))
                        })
                        .unwrap_or(true)
                {
                    self.refresh_warning = format!(
                        "Live refresh unavailable; using {} data from {}.",
                        new_table.cache_state.name(),
                        new_table.source_fetched_at,
                    );
                    self.retry_after = Some(Instant::now() + Duration::from_secs(300));
                }
            }
            if !scored_settings.scoring_matches(&self.settings) {
                continue;
            }
            self.panel_widths.clear();
            self.invalidate_comparisons();
            self.generator.markdown = None;
            self.table_revision = self.table_revision.wrapping_add(1);
            self.table = new_table;
            self.vendor_tables = vendor_tables;
            self.comparison_choice.clear();
            self.scored_settings = Some(scored_settings);
            if !self.refresh_in_flight {
                self.engine_status.clear();
            }
        }

        while let Ok(response) = self.comparison_rx.try_recv() {
            let request = (response.0, response.1, response.2, response.3);
            if response.0 == self.comparison_generation && self.comparison_pending == Some(request)
            {
                self.comparison_pending = None;
                self.comparison_result = Some(response);
            }
        }

        if let Ok(error) = self.engine_error_rx.try_recv() {
            self.refresh_in_flight = false;
            self.retry_after = Some(Instant::now() + Duration::from_secs(300));
            self.refresh_warning = error;
            self.engine_status.clear();
        }
        if let Some(rx) = &self.update_rx
            && let Ok(res) = rx.try_recv()
        {
            self.update_rx = None;
            let status = match res {
                Ok(checked) => match checked.outcome {
                    blockitall_update::Outcome::Available(av) => {
                        let status = format!(
                            "Update {} available via {}",
                            av.version(),
                            checked.transport
                        );
                        self.available_update = Some(*av);
                        status
                    }
                    blockitall_update::Outcome::UpToDate { latest } => {
                        self.available_update = None;
                        format!("Up to date ({latest}) via {}", checked.transport)
                    }
                },
                Err(error) => error,
            };
            if self.install_rx.is_none() {
                self.update_status = status;
            }
        }
        if let Some(rx) = &self.install_rx
            && let Ok((result, staged)) = rx.try_recv()
        {
            self.install_rx = None;
            self.staged_update = staged;
            match result {
                Ok(()) => {
                    self.update_status = "Restarting…".to_owned();
                    self.close_after_handoff = true;
                }
                Err(error) => self.update_status = error,
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_channels();
        let ctx = ui.ctx().clone();
        if self.close_after_handoff {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let mut install_clicked = false;
        if !self.refresh_in_flight
            && self.source_expired()
            && self
                .retry_after
                .is_none_or(|deadline| Instant::now() >= deadline)
        {
            self.start_refresh();
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        egui::Panel::bottom("bottom_strip").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Last pull: {}", self.table.source_fetched_at));
                let cache_label = match self.table.cache_state {
                    CacheState::Disk => Some("cached"),
                    CacheState::Baked => Some("bundled"),
                    CacheState::Live => None,
                };
                if let Some(label) = cache_label {
                    ui.colored_label(egui::Color32::from_rgb(220, 160, 60), label);
                }
                ui.separator();
                let mut cooldown_seconds = self.refresh_clicked_at.map_or(0, |clicked_at| {
                    REFRESH_COOLDOWN_SECONDS.saturating_sub(clicked_at.elapsed().as_secs())
                });
                let refresh_label = if cooldown_seconds > 0 {
                    format!("Refresh data ({cooldown_seconds}s)")
                } else {
                    "Refresh data".to_string()
                };
                if ui
                    .add_enabled(
                        cooldown_seconds == 0 && !self.refresh_in_flight,
                        egui::Button::new(refresh_label),
                    )
                    .clicked()
                {
                    self.start_refresh();
                    cooldown_seconds = REFRESH_COOLDOWN_SECONDS;
                }
                if cooldown_seconds > 0 {
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
                if let Ok(fetched_at) =
                    OffsetDateTime::parse(&self.table.source_fetched_at, &Rfc3339)
                {
                    let remaining = fetched_at
                        + time::Duration::hours(i64::from(self.settings.cache_hours))
                        - OffsetDateTime::now_utc();
                    if remaining <= time::Duration::ZERO {
                        if self.refresh_in_flight {
                            ui.label("auto refresh in progress");
                        } else if let Some(deadline) = self.retry_after {
                            ui.label(format!(
                                "auto retry in {}s",
                                deadline.saturating_duration_since(Instant::now()).as_secs()
                            ));
                        } else {
                            ui.label("next auto due");
                        }
                    } else {
                        let minutes = remaining.whole_minutes();
                        if minutes >= 60 {
                            ui.label(format!("next auto in {}h {}m", minutes / 60, minutes % 60));
                        } else {
                            ui.label(format!("next auto in {minutes}m"));
                        }
                        if cooldown_seconds == 0 {
                            ui.ctx().request_repaint_after(Duration::from_secs(1));
                        }
                    }
                }
                ui.separator();
                ui.hyperlink_to(
                    "Source: Artificial Analysis (artificialanalysis.ai)",
                    "https://artificialanalysis.ai",
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙").clicked() {
                        self.settings_open = !self.settings_open;
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                if ui.button("Check for updates").clicked() {
                    self.handoff_status.clear();
                    self.trigger_update_check(ctx.clone());
                }
                if self.staged_update.is_some() || self.available_update.is_some() {
                    let label = if self.staged_update.is_some() {
                        "Retry install"
                    } else {
                        "Install"
                    };
                    install_clicked = ui
                        .add_enabled(
                            self.install_rx.is_none()
                                && !self.refresh_in_flight
                                && self.comparison_pending.is_none(),
                            egui::Button::new(label),
                        )
                        .clicked();
                }
                if !self.handoff_status.is_empty() {
                    ui.label(&self.handoff_status);
                }
                ui.label(&self.update_status);
            });
        });

        let panel_open = self.side_panel_open
            && self.selected_seat_tier.is_some()
            && !(self.selected_tab == 0
                && self.settings.best_in_house_mode == BestInHouseMode::PerPlan);
        let t = ctx.animate_bool(egui::Id::new("side_panel_anim"), panel_open);
        if t > 0.001 {
            let (seat, tier) = self.selected_seat_tier.unwrap();
            let mut selected_floor = None;
            let maximum = (ui.max_rect().width() * 0.45).max(1.0);
            let panel_width = self
                .detail_width(ui, seat, tier)
                .clamp(240.0_f32.min(maximum), maximum)
                * t;
            egui::Panel::right("breakdown_panel")
                .exact_size(panel_width)
                .resizable(false)
                .show(ui, |ui| {
                    if let Some((seat, tier)) = self.selected_seat_tier {
                        ui.horizontal(|ui| {
                            ui.heading(format!("{} - {}", seat.name(), tier.name()));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let response =
                                        ui.add(egui::Button::new("").min_size(egui::Vec2::splat(
                                            ui.spacing().interact_size.y,
                                        )));
                                    let rect = response.rect.shrink(6.0);
                                    let stroke = ui.style().interact(&response).fg_stroke;
                                    ui.painter().line_segment(
                                        [rect.left_top(), rect.right_bottom()],
                                        stroke,
                                    );
                                    ui.painter().line_segment(
                                        [rect.right_top(), rect.left_bottom()],
                                        stroke,
                                    );
                                    if response.on_hover_text("Close details").clicked() {
                                        self.side_panel_open = false;
                                    }
                                },
                            );
                        });
                        ui.separator();

                        egui::ScrollArea::vertical()
                            .id_salt(("detail_scroll", seat, tier))
                            .auto_shrink([false, false])
                            .max_height(ui.available_height())
                            .show(ui, |ui| {
                                if seat != Seat::NetResearch {
                                    self.counterfactual_content(ui, seat, tier);
                                    ui.separator();
                                }
                                selected_floor = self.detail_content(ui, seat, tier);
                            });
                    }
                });
            if let Some(floor) = selected_floor {
                self.settings
                    .competence_floors
                    .insert(seat.name().to_string(), Some(floor));
                self.settings = self.settings.clone().normalize();
                let _ = save_settings(&self.settings);
                self.invalidate_comparisons();
                let _ = self.settings_tx.send((self.settings.clone(), false));
                self.engine_status = "Scoring…".to_string();
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Working hours per week");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.agent_hours_text).desired_width(80.0),
                );
                if response.lost_focus()
                    || (response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                {
                    if let Ok(hours) = self.agent_hours_text.trim().parse::<f64>()
                        && hours.is_finite()
                        && (1.0..=1680.0).contains(&hours)
                    {
                        self.agent_hours_invalid = false;
                        self.agent_hours_text = hours.to_string();
                        if self.settings.agent_hours != hours {
                            self.settings.agent_hours = hours;
                            let _ = save_settings(&self.settings);
                            self.invalidate_comparisons();
                            let _ = self.settings_tx.send((self.settings.clone(), false));
                            self.engine_status = "Scoring…".to_string();
                        }
                    } else {
                        self.agent_hours_invalid = true;
                    }
                }
                if self.agent_hours_invalid {
                    ui.colored_label(egui::Color32::YELLOW, "Enter a number from 1 to 1680.");
                }
            ui.heading("Concurrent projects");
                if ui.add(egui::DragValue::new(&mut self.settings.orchestrators).range(1..=64)).changed() {
                    let _ = save_settings(&self.settings);
                    self.invalidate_comparisons();
                    let _ = self.settings_tx.send((self.settings.clone(), false));
                    self.engine_status = "Scoring…".to_string();
                }
            });
            ui.small("Enter how many project orchestrators run at the same time. They use the same fixed orchestrator model and share each subscription allowance; the count scales aggregate demand without cloning quota.");
            if self.selected_tab == 0 && self.settings.best_in_house_mode == BestInHouseMode::PerPlan {
                ui.label("Each selected subscription can run for these hours alongside the others. Total scheduled agent hours can exceed the working week.");
            } else {
                ui.label("Each independent pick uses these combined hours for agent attempts and rescue work. Example: 10 hours across 3 parallel agents = 30 workflow hours.");
            }
            ui.add_space(8.0);
            egui::ScrollArea::both()
                .id_salt("roles_table_scroll")
                .auto_shrink([false, false])
                .max_height(ui.available_height())
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.strong("Coding Agent Tier List");
                    let previous_tab = self.selected_tab;
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.selected_tab, 0, "Best in house");
                        for (index, (label, _)) in VENDOR_TABS.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_tab, index + 1, *label);
                        }
                    });
                    if self.selected_tab != previous_tab {
                        self.panel_widths.clear();
                        self.comparison_choice.clear();
                        self.invalidate_comparisons();
                    }
                    if self.selected_tab == 0 {
                        let previous_mode = self.settings.best_in_house_mode;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.settings.best_in_house_mode, BestInHouseMode::Absolute, "Absolute best");
                            ui.selectable_value(&mut self.settings.best_in_house_mode, BestInHouseMode::PerPlan, "For you");
                        });
                        if self.settings.best_in_house_mode != previous_mode {
                            self.side_panel_open = false;
                            let _ = save_settings(&self.settings);
                        }
                        if self.settings.best_in_house_mode == BestInHouseMode::PerPlan {
                            self.portfolio_content(ui);
                            return;
                        }
                    }
                    ui.add(
                        egui::Label::new(
                            "Picks maximize nominal verified reference tasks per week, with the scenario band shown. Optional competence minimums enforce hard requirements.",
                        )
                        .wrap(),
                    )
                    .on_hover_text(
                        "Every candidate and retry cap is evaluated across the declared joint resource scenarios. Estimates include retries and declared escalation.",
                    );
                    egui::CollapsingHeader::new("How estimates work")
                        .default_open(false)
                        .show(ui, |ui| {
                            for explanation in [
                                "Role competence is a fixed weighted utility of benchmark capabilities. It is a qualification measure, not a task success probability.",
                                "Implementer and Debugger capacity uses the coding reference workload. Other seats use Repository Q&A as the reference workload. These are reference units, not observed real-role output.",
                                "The engine chooses a candidate and maximum retry count before the scenario is known. Attempts stop on success; unfinished cycles use the configured rescue time and cost without crediting that completion to the agent.",
                                "The automatic choice maximizes nominal verified reference tasks per week. Exact ties prefer smaller worst scenario capacity shortfall, higher known competence, then shorter retry caps.",
                                "Scenarios combine the configured stress distance, allowance endpoints, and both resource-time bases. The distance is an assumption, not measured uncertainty or a confidence interval.",
                                "Hallucination is an optional reliability indicator only. It is not a retry-transition probability, and missing values are not imputed as perfect reliability.",
                            ] {
                                ui.add(egui::Label::new(explanation).wrap());
                            }
                        });
                    if !self.refresh_warning.is_empty() {
                        ui.colored_label(egui::Color32::YELLOW, &self.refresh_warning);
                    }
                    if self.source_expired() && !self.table.rows.is_empty() {
                        ui.colored_label(egui::Color32::YELLOW, "Data is stale.");
                    }
                    if !self.engine_status.is_empty() {
                        ui.label(&self.engine_status);
                    }
                    ui.add_space(4.0);

                    egui::Grid::new("roles_table_grid")
                        .striped(true)
                        .spacing([12.0, 3.0])
                        .min_col_width(50.0)
                        .show(ui, |ui| {
                            ui.strong("Role / budget");
                            ui.strong("Agent");
                            ui.strong("Competence").on_hover_text(
                                "Fixed role-weighted benchmark utility on a 0–1 scale. It is not a task success probability.",
                            );
                            ui.strong("Capacity shortfall").on_hover_text(
                                "Worst proportional capacity shortfall across the scenario band. Breaks exact nominal-throughput ties; per-scenario values are diagnostics.",
                            );
                            ui.strong("Min/reference");
                            ui.strong("$/reference");
                            ui.strong("Agent completions/week").on_hover_text(
                                "Estimated reference tasks completed by the agent before rescue. Rescue completions are reported separately.",
                            );
                            for (heading, explanation) in [
                                (
                                    "Allowance used",
                                    "Configured total agent hours divided by the estimated hours needed to exhaust the allowance, capped at 100%.",
                                ),
                                (
                                    "Hours to use allowance",
                                    "Estimated agent hours needed to exhaust the plan allowance.",
                                ),
                            ] {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.strong(heading).on_hover_text(explanation);
                                    },
                                );
                            }
                            ui.end_row();

                            for seat in [
                                Seat::Orchestrator,
                                Seat::Implementer,
                                Seat::Debugger,
                                Seat::Reviewer,
                                Seat::Sanity,
                                Seat::Comprehension,
                            ] {
                                if seat != Seat::Orchestrator {
                                    for _ in 0..9 {
                                        ui.add(
                                            egui::Separator::default().horizontal().spacing(2.0),
                                        );
                                    }
                                    ui.end_row();
                                }
                                ui.strong(seat.name());
                                for _ in 1..9 {
                                    ui.label("");
                                }
                                ui.end_row();
                                for tier in Tier::ALL {
                                    let is_selected = self.selected_seat_tier == Some((seat, tier));
                                    let label = tier.name();
                                    let table = self.selected_table();
                                    let maybe_pick = table.get_pick(seat, tier);

                                    let (
                                        agent_name,
                                        competence,
                                        capacity_shortfall,
                                        min_task,
                                        cost_task,
                                        tasks_wk,
                                        a_star_hours,
                                    ) = if let Some(pick) = maybe_pick {
                                        let name = table
                                            .rows
                                            .get(pick.row_index)
                                            .map(|r| r.display_name())
                                            .unwrap_or_else(|| "-".into());
                                        let competence = competence_text(pick.competence);
                                        let capacity_shortfall =
                                            format!("{:.1}%", pick.capacity_shortfall * 100.0);
                                        let min_task = format!("{:.2}", pick.minutes_per_task);
                                        let cost_task = format!("${:.2}", pick.cost_per_task);
                                        let tasks_wk = format!("{:.1} ({:.1}–{:.1})", pick.tasks_per_week, pick.tasks_low, pick.tasks_high);
                                        let a_star_hours = if tier == Tier::Api {
                                            "-".to_string()
                                        } else {
                                            pick.a_star_hours
                                                .map(|s| format!("{:.1}", s))
                                                .unwrap_or_else(|| "-".into())
                                        };
                                        (
                                            name,
                                            competence,
                                            capacity_shortfall,
                                            min_task,
                                            cost_task,
                                            tasks_wk,
                                            a_star_hours,
                                        )
                                    } else {
                                        (
                                            if self.selected_tab != 0
                                                && self
                                                    .settings
                                                    .competence_floors
                                                    .get(seat.name())
                                                    .copied()
                                                    .flatten()
                                                    .is_none()
                                            {
                                                format!(
                                                    "{} has no eligible plan or candidate at this tier.",
                                                    VENDOR_TABS[self.selected_tab - 1].0
                                                )
                                            } else {
                                                "No candidate meets requirements".into()
                                            },
                                            "-".into(),
                                            "-".into(),
                                            "-".into(),
                                            "-".into(),
                                            "-".into(),
                                            "-".into(),
                                        )
                                    };

                                    let mut clicked = false;

                                    if ui.selectable_label(is_selected, label).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &agent_name).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &competence).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &capacity_shortfall).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &min_task).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &cost_task).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &tasks_wk).clicked() {
                                        clicked = true;
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                maybe_pick
                                                    .and_then(|pick| pick.util_pct)
                                                    .map(|value| format!("{value:.1}%"))
                                                    .unwrap_or_else(|| "-".into()),
                                            );
                                        },
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            clicked |= ui
                                                .selectable_label(is_selected, &a_star_hours)
                                                .clicked();
                                        },
                                    );
                                    if clicked {
                                        self.selected_seat_tier = Some((seat, tier));
                                        self.side_panel_open = true;
                                    }

                                    ui.end_row();
                                }
                            }
                            for _ in 0..9 {
                                ui.add(egui::Separator::default().horizontal().spacing(2.0));
                            }
                            ui.end_row();
                            ui.strong(Seat::NetResearch.name());
                            for _ in 1..9 {
                                ui.label("");
                            }
                            ui.end_row();
                            for tier in Tier::ALL {
                                let is_selected = self.selected_seat_tier == Some((Seat::NetResearch, tier));
                                let (name, score, minutes, cost, eligibility) = {
                                    let table = self.selected_table();
                                    let primary = table
                                        .research_tiers
                                        .iter()
                                        .find(|pick| pick.tier == tier)
                                        .and_then(|pick| pick.primary.as_ref());
                                    if let Some(candidate) = primary {
                                        let name = table
                                            .rows
                                            .get(candidate.row_index)
                                            .map(|row| research_route_name(row, &candidate.native_harness))
                                            .unwrap_or_else(|| "-".to_string());
                                        (
                                            name,
                                            competence_text(Some(candidate.score)),
                                            candidate.decode_hours.map(|hours| format!("{:.2}", hours * 60.0)).unwrap_or_else(|| "Unknown".to_string()),
                                            candidate.expected_usd.map(|usd| format!("${usd:.2}")).unwrap_or_else(|| "Unknown".to_string()),
                                            candidate.eligibility.clone(),
                                        )
                                    } else {
                                        ("No eligible research configuration".to_string(), "-".to_string(), "Unknown".to_string(), "Unknown".to_string(), "No qualified route is available at this tier.".to_string())
                                    }
                                };
                                let mut clicked = ui.selectable_label(is_selected, tier.name()).clicked();
                                clicked |= ui.selectable_label(is_selected, name).on_hover_text(&eligibility).clicked();
                                clicked |= ui.selectable_label(is_selected, score).on_hover_text("AA research score with Standard scope weights; not task-success probability.").clicked();
                                clicked |= ui.selectable_label(is_selected, "-").on_hover_text("No research capacity shortfall is inferred.").clicked();
                                clicked |= ui.selectable_label(is_selected, minutes).on_hover_text("AA reference-workload decode minutes; not native research latency.").clicked();
                                clicked |= ui.selectable_label(is_selected, cost).on_hover_text("AA reference-workload USD; not native cost, quota, or a hold.").clicked();
                                clicked |= ui.selectable_label(is_selected, "-").on_hover_text("No research completions per week are inferred.").clicked();
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label("-").on_hover_text("Native allowance use is resolved from real account meters.");
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    clicked |= ui.selectable_label(is_selected, "-").on_hover_text("No native allowance-exhaustion time is inferred.").clicked();
                                });
                                if clicked {
                                    self.selected_seat_tier = Some((Seat::NetResearch, tier));
                                    self.side_panel_open = true;
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new("Plan price, competence, and production comparisons")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label("Each pair compares the selected subscription policies using actual monthly prices, raw competence, and autonomous output across all declared scenarios.");
                            for seat in Seat::ALL {
                                let comparisons = self.table.plan_comparisons
                                    .iter()
                                    .filter(|comparison| comparison.seat == seat)
                                    .collect::<Vec<_>>();
                                if comparisons.is_empty() {
                                    continue;
                                }
                                ui.group(|ui| {
                                    ui.strong(seat.name());
                                    for comparison in comparisons {
                                        let price = |tier: Tier, value: Option<f64>| value
                                            .map(|value| format!("{} (${value:.0}/month)", tier.name()))
                                            .unwrap_or_else(|| format!("{} (price unknown)", tier.name()));
                                        let left = price(comparison.left_tier, comparison.left_price);
                                        let right = price(comparison.right_tier, comparison.right_price);
                                        if let Some(dominant) = comparison.dominant_tier {
                                            ui.label(format!(
                                                "{left} vs {right}: {} costs no more and is no worse in competence or any autonomous-capacity scenario.",
                                                dominant.name(),
                                            ));
                                        } else if comparison.equivalent {
                                            ui.label(format!("{left} vs {right}: selected policies are equivalent on competence and autonomous production."));
                                        } else {
                                            ui.label(format!(
                                                "{left}: competence {}, {:.1}/wk; {right}: competence {}, {:.1}/wk. Scenario production difference ranges {:+.1} to {:+.1}/wk (right minus left).",
                                                competence_text(comparison.left_competence),
                                                comparison.left_autonomous_tasks_per_week,
                                                competence_text(comparison.right_competence),
                                                comparison.right_autonomous_tasks_per_week,
                                                comparison.worst_capacity_delta,
                                                comparison.best_capacity_delta,
                                            ));
                                        }
                                    }
                                });
                            }
                        });
                    for (heading, bare_model, columns) in [
                        (
                            "Coding Agents",
                            false,
                            [
                                "rank", "Agent", "SWE", "Term", "QnA", "Vendor", "Wait", "Cost",
                                "Agent tasks/wk",
                            ],
                        ),
                        (
                            "Models (bare API)",
                            true,
                            [
                                "rank", "Model", "Term", "Logic", "Tok/s", "Vendor", "Wait",
                                "Cost", "Agent tasks/wk",
                            ],
                        ),
                    ] {
                        ui.add_space(8.0);
                        ui.strong(heading);
                        ui.add_space(4.0);
                        egui::Grid::new(heading)
                            .striped(true)
                            .spacing([12.0, 3.0])
                            .min_col_width(50.0)
                            .show(ui, |ui| {
                                for (column, heading) in columns.iter().enumerate() {
                                    let direction = if matches!(column, 1 | 5) {
                                        egui::Layout::left_to_right(egui::Align::Center)
                                    } else {
                                        egui::Layout::right_to_left(egui::Align::Center)
                                    };
                                    ui.with_layout(direction, |ui| {
                                        let explanation = match *heading {
                                            "SWE" => Some("DeepSWE software engineering benchmark."),
                                            "Term" => Some("Terminal-Bench terminal task benchmark."),
                                            "QnA" => Some("Codebase question-answering benchmark."),
                                            "Logic" => Some("Combined GPQA and Humanity's Last Exam reasoning score."),
                                            "Tok/s" => Some("Output tokens generated per second."),
                                            _ => None,
                                        };
                                        let response = ui.strong(*heading);
                                        if let Some(explanation) = explanation {
                                            response.on_hover_text(explanation);
                                        }
                                    });
                                }
                                ui.end_row();
                                for (rank, row) in self
                                    .table
                                    .rows
                                    .iter()
                                    .filter(|row| (row.harness == "model") == bare_model)
                                    .take(20)
                                    .enumerate()
                                {
                                    let percent = |value: Option<f64>| {
                                        value
                                            .map(|value| format!("{:.1}", value * 100.0))
                                            .unwrap_or_else(|| "-".into())
                                    };
                                    let metrics = if bare_model {
                                        [
                                            percent(row.term),
                                            percent(row.logic),
                                            row.speed
                                                .map(|value| format!("{:.0}", value.round()))
                                                .unwrap_or_else(|| "-".into()),
                                        ]
                                    } else {
                                        [percent(row.swe.map(|value| row.retry.adjusted_pass(crate::types::Benchmark::Swe, value))), percent(row.term), percent(row.qna)]
                                    };
                                    let tasks = crate::engine::implementation_cycle(
                                        row,
                                        &self.settings,
                                    )
                                    .map(|cycle| {
                                        self.settings.agent_hours * 3600.0 / cycle.wall
                                            * (1.0 - cycle.unfinished)
                                    })
                                    .filter(|tasks| tasks.is_finite())
                                    .map(|tasks| format!("{tasks:.1}"))
                                    .unwrap_or_else(|| "Unknown".into());
                                    let [first, second, third] = metrics;
                                    for (column, value) in [
                                        (rank + 1).to_string(),
                                        row.display_name(),
                                        first,
                                        second,
                                        third,
                                        row.vendor.clone(),
                                        format!("{:.2}m", row.wait_seconds / 60.0),
                                        format!("${:.2}", row.attempt_usd),
                                        tasks,
                                    ]
                                    .into_iter()
                                    .enumerate()
                                    {
                                        let direction = if matches!(column, 1 | 5) {
                                            egui::Layout::left_to_right(egui::Align::Center)
                                        } else {
                                            egui::Layout::right_to_left(egui::Align::Center)
                                        };
                                        ui.with_layout(direction, |ui| {
                                            let response = ui.label(value);
                                            if column == 1 {
                                                response.on_hover_text(row.retry.description());
                                            }
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                });
        });

        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(true)
                .default_width(450.0)
                .show(&ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("settings_scroll")
                        .show(ui, |ui| {
                    let mut changed = false;
                    ui.heading("Collaboration workspace");
                    ui.label("User projects, briefs, decisions, research, deliverables, and scratch files live here. Profiles, authentication, ledgers, and generated runtimes remain in private app data.");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.collaboration_root_text).desired_width(300.0));
                        if ui.add_enabled(self.workspace_picker.is_none(), egui::Button::new("Browse...")).clicked() {
                            let (sender, receiver) = channel();
                            self.workspace_picker = Some(receiver);
                            let context = ctx.clone();
                            std::thread::spawn(move || {
                                let result = crate::workspace::pick_root().map_err(|error| format!("Browse failed: {error:#}"));
                                let _ = sender.send(result);
                                context.request_repaint();
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Use default").clicked() {
                            match crate::workspace::default_root() {
                                Ok(path) => {
                                    self.collaboration_root_text = path.display().to_string();
                                    self.settings.collaboration_root = None;
                                    changed = true;
                                }
                                Err(error) => self.workspace_status = format!("Default unavailable: {error:#}"),
                            }
                        }
                        if ui.button("Apply and prepare").clicked() {
                            let root = std::path::PathBuf::from(self.collaboration_root_text.trim());
                            if !root.is_absolute() {
                                self.workspace_status = "Choose an absolute local path.".to_owned();
                            } else {
                                match crate::workspace::prepare(&root) {
                                    Ok(paths) => {
                                        let default = crate::workspace::default_root().ok();
                                        self.settings.collaboration_root = (default.as_ref() != Some(&paths.root)).then_some(paths.root.clone());
                                        self.collaboration_root_text = paths.root.display().to_string();
                                        self.workspace_status = format!("Prepared {}", paths.root.display());
                                        changed = true;
                                    }
                                    Err(error) => self.workspace_status = format!("Workspace setup failed: {error:#}"),
                                }
                            }
                        }
                    });
                    if !self.workspace_status.is_empty() {
                        ui.label(&self.workspace_status);
                    }
                    ui.separator();
                    ui.heading("My subscriptions");
                    ui.label("Choose a plan and how many separate subscriptions you own. Each account contributes one allowance and one working-time lane shared across all seven roles.");
                    let mut monthly_spend = 0.0;
                    let mut estimated_price = false;
                    for provider in crate::subscriptions::PROVIDERS {
                        let mut selected = self.settings.subscriptions.get(provider.id).cloned().unwrap_or_default();
                        let previous = selected.clone();
                        let label = provider.plans.iter().find(|plan| plan.id == selected)
                            .map(|plan| format!("{} · {}{:.2}/month", plan.name, if plan.price_is_estimate { "est. $" } else { "$" }, plan.monthly_price))
                            .unwrap_or_else(|| "None".to_string());
                        let count_text = self
                            .subscription_count_text
                            .entry(provider.id.to_string())
                            .or_insert_with(|| self.settings.subscription_count(provider.id).to_string());
                        let mut count_changed = false;
                        ui.horizontal(|ui| {
                            ui.label(provider.name);
                            egui::ComboBox::from_id_salt(("subscription", provider.id))
                                .selected_text(label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut selected, String::new(), "None");
                                    for plan in provider.plans {
                                        ui.selectable_value(&mut selected, plan.id.to_string(),
                                            format!("{} · {}{:.2}/month", plan.name, if plan.price_is_estimate { "est. $" } else { "$" }, plan.monthly_price));
                                    }
                                });
                            ui.label("×");
                            count_changed = ui
                                .add(egui::TextEdit::singleline(count_text).desired_width(42.0))
                                .changed();
                        });
                        let parsed_count = count_text.parse::<usize>().ok().filter(|count| *count > 0);
                        if count_changed && let Some(count) = parsed_count {
                            self.settings
                                .subscription_counts
                                .insert(provider.id.to_string(), count);
                            changed = true;
                        }
                        if parsed_count.is_none() {
                            ui.colored_label(egui::Color32::YELLOW, "Count must be a positive integer; the last saved count remains active.");
                        }
                        if selected != previous {
                            if selected.is_empty() {
                                self.settings.subscriptions.remove(provider.id);
                            } else {
                                self.settings.subscriptions.insert(provider.id.to_string(), selected.clone());
                            }
                            changed = true;
                        }
                        if let Some(plan) = provider.plans.iter().find(|plan| plan.id == selected) {
                            monthly_spend += plan.monthly_price
                                * self.settings.subscription_count(provider.id) as f64;
                            estimated_price |= plan.price_is_estimate;
                        }
                    }
                    ui.strong(format!("{}: ${monthly_spend:.2}", if estimated_price { "Estimated monthly spend" } else { "Total monthly spend" }));
                    ui.separator();
                    ui.heading("Engine & cache configuration");
                    ui.horizontal(|ui| {
                        ui.label("Rescue time (minutes):").on_hover_text(
                            "An unfinished cycle uses this additional rescue time. Assisted completions consume workflow time but are not credited to the agent.",
                        );
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.escalation_minutes)
                                    .range(0.0..=10080.0),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Rescue cost (USD):").on_hover_text(
                            "An unfinished cycle uses this additional external rescue cost. This setting applies to every role.",
                        );
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.escalation_usd)
                                    .range(0.0..=1000000.0),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Retry correlation override:").on_hover_text(
                            "Blank derives correlation separately for each row and suite. A number from 0 to 0.999 overrides every suite; 0 treats attempts as independent.",
                        );
                        if ui.add(egui::TextEdit::singleline(&mut self.rho_override_text)
                            .hint_text("derived")
                            .desired_width(80.0)).changed() {
                            if self.rho_override_text.trim().is_empty() {
                                self.settings.rho_override = None;
                                changed = true;
                            } else if let Ok(value) = self.rho_override_text.trim().parse::<f64>()
                                && value.is_finite()
                                && (0.0..=0.999).contains(&value) {
                                self.settings.rho_override = Some(value);
                                changed = true;
                            }
                        }
                        if !self.rho_override_text.trim().is_empty()
                            && !self.rho_override_text.trim().parse::<f64>().is_ok_and(|value|
                                value.is_finite() && (0.0..=0.999).contains(&value)) {
                            ui.colored_label(egui::Color32::YELLOW, "Use 0–0.999 or leave blank");
                        }
                    });

                    ui.separator();
                    egui::CollapsingHeader::new("Optional competence minimums")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(
                                "Enable a minimum to enforce a hard qualification requirement for a role in either recommendation mode.",
                            ).wrap());
                            for seat in Seat::ALL {
                                let floor = self
                                    .settings
                                    .competence_floors
                                    .entry(seat.name().to_string())
                                    .or_insert(None);
                                ui.horizontal(|ui| {
                                    let mut enabled = floor.is_some();
                                    let response = ui.checkbox(&mut enabled, seat.name());
                                    if seat == Seat::Orchestrator {
                                        response.clone().on_hover_text("Per-plan Orchestrator competence uses the AA Intelligence Index only. Absolute tier-list Orchestrator competence retains its equal GPQA and Intelligence Index blend.");
                                    }
                                    if response.changed() {
                                        *floor = enabled.then_some(0.0);
                                        changed = true;
                                    }
                                    if let Some(value) = floor {
                                        changed |= ui
                                            .add(
                                                egui::DragValue::new(value)
                                                    .speed(0.001)
                                                    .range(0.0..=1.0)
                                                    .fixed_decimals(3),
                                            )
                                            .changed();
                                    } else {
                                        ui.label("automatic");
                                    }
                                });
                            }
                        });

                    ui.horizontal(|ui| {
                        ui.label("Assumption scenario distance:").on_hover_text(
                            "Percentage used in joint low/high runtime, model-cost, retry-correlation, and escalation-time stress scenarios. It is an assumed distance, not measured error.",
                        );
                        changed |= ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings.assumption_span_pct,
                                    0.0..=90.0,
                                )
                                .suffix("%")
                                .step_by(1.0),
                            )
                            .changed();
                    });

                    ui.horizontal(|ui| {
                        ui.label("Cache validity (hours):");
                        if ui
                            .add(egui::Slider::new(&mut self.settings.cache_hours, 1..=48))
                            .changed()
                        {
                            changed = true;
                        }
                    });

                    if ui
                        .checkbox(
                            &mut self.settings.show_hallucination,
                            "Show hallucination reliability indicator",
                        )
                        .changed()
                    {
                        changed = true;
                    }

                    ui.separator();
                    ui.heading("Plan Tier Prices (USD/month)");
                    ui.horizontal(|ui| {
                        ui.label("$200 Tier:");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.settings.plan_prices.t200)
                                    .prefix("$")
                                    .range(1.0..=1000.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("$100 Tier:");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.settings.plan_prices.t100)
                                    .prefix("$")
                                    .range(1.0..=1000.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("$20 Tier:");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.settings.plan_prices.t20)
                                    .prefix("$")
                                    .range(1.0..=1000.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });

                    ui.separator();
                    ui.heading("Per-Vendor Allowance Overrides (USD/wk)");
                    ui.label("Leave blank for the default plan allowance");
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for (vendor, val) in self.settings.vendor_overrides.iter_mut() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{vendor}:"));
                                    if crate::engine::plan_for(vendor, f64::INFINITY).is_none() {
                                        ui.label("API only unless overridden");
                                    }
                                    let mut text = val.map(|v| v.to_string()).unwrap_or_default();
                                    if ui.text_edit_singleline(&mut text).changed() {
                                        let trimmed = text.trim();
                                        if trimmed.is_empty() {
                                            *val = None;
                                        } else if let Ok(parsed) = trimmed.parse::<f64>() {
                                            *val = Some(parsed);
                                        }
                                        changed = true;
                                    }
                                });
                            }
                        });

                    if changed {
                        self.settings = self.settings.clone().normalize();
                        let _ = save_settings(&self.settings);
                        self.invalidate_comparisons();
                        let _ = self.settings_tx.send((self.settings.clone(), false));
                        self.engine_status = "Scoring…".to_string();
                    }
                    });
                });
            self.settings_open = open;
        }
        self.generator.show(
            &ctx,
            &self.table,
            &self.settings,
            self.table_revision,
            !self.refresh_in_flight
                && self
                    .scored_settings
                    .as_ref()
                    .is_some_and(|scored| scored.scoring_matches(&self.settings)),
        );
        if install_clicked {
            self.start_install(ctx);
        }
        if let Some(guard) = self.health_guard.take()
            && let Err(error) = guard.confirm_healthy()
        {
            self.update_status = format!("Update health confirmation failed: {error:#}");
            self.close_after_handoff = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
