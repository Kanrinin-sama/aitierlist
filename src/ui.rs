use crate::settings::{Settings, load_settings, save_settings};
use crate::types::{CacheState, Seat, Table, Tier};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const REFRESH_COOLDOWN_SECONDS: u64 = 16;

type ComparisonResponse = (
    u64,
    Seat,
    Tier,
    usize,
    crate::comparison::CounterfactualReport,
);

pub struct App {
    table: Table,
    panel_widths: std::collections::HashMap<(Seat, Tier), f32>,
    agent_hours_text: String,
    agent_hours_invalid: bool,
    refresh_in_flight: bool,
    retry_after: Option<Instant>,
    refresh_warning: String,
    table_rx: Receiver<(Table, bool, Settings)>,
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
    update_rx: Option<Receiver<Result<crate::update::Checked, String>>>,
    available_update: Option<crate::update::Available>,
    staged_update: Option<crate::update::StagedUpdate>,
    install_rx: Option<Receiver<(String, Option<crate::update::StagedUpdate>)>>,
    comparison_choice: std::collections::HashMap<(Seat, Tier), usize>,
    comparison_generation: u64,
    comparison_pending: Option<(u64, Seat, Tier, usize)>,
    comparison_result: Option<ComparisonResponse>,
    comparison_tx: Sender<ComparisonResponse>,
    comparison_rx: Receiver<ComparisonResponse>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
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
                    let table =
                        crate::engine::score(rows.clone(), &settings, *state, fetched.clone());
                    if table_tx
                        .send((table, fetched_rows, settings.clone()))
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
            panel_widths: std::collections::HashMap::new(),
            agent_hours_text: settings.agent_hours.to_string(),
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
            handoff_status: crate::update::last_handoff_status()
                .map(|status| {
                    format!(
                        "Update {}: {} — {}",
                        status.version, status.outcome, status.detail
                    )
                })
                .unwrap_or_default(),
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

        if crate::update::due_for_check() {
            app.trigger_update_check(cc.egui_ctx.clone());
        } else if let Some(version) = crate::update::cached_published_version() {
            app.update_status = format!("Last seen: {version}");
        }
        app
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
        ui.label("Add a hard qualification requirement only when the seat needs one. The automatic pick already balances competence and capacity.");
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
                                "Quality loss", "Capacity loss", "Worst loss",
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
                                ui.label(format!("{:.3}", point.competence));
                                ui.label(point.attempt_limit.to_string());
                                ui.label(format!("{:.1}", point.tasks_per_week));
                                ui.label(format!("{:.1}–{:.1}", point.tasks_low, point.tasks_high));
                                ui.label(format!(
                                    "{:.1}%",
                                    point.competence_shortfall * 100.0
                                ));
                                ui.label(format!("{:.1}%", point.capacity_shortfall * 100.0));
                                ui.label(format!("{:.1}%", point.worst_shortfall * 100.0));
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
            ui.label("No competence–capacity choices are available.");
        }
            });
        selected_floor
    }

    fn detail_content(&self, ui: &mut egui::Ui, seat: Seat, tier: Tier) -> Option<f64> {
        let selected_floor;
        if let Some(pick) = self.table.get_pick(seat, tier) {
            let selected_row = self.table.rows.get(pick.row_index);

            ui.heading("Selected policy");
            if let Some(row) = selected_row {
                ui.label(egui::RichText::new(row.display_name()).strong().size(15.0));
                ui.label(format!("Vendor: {} | Harness: {}", row.vendor, row.harness));
            }
            ui.add_space(4.0);
            ui.label(format!("Role competence: {:.3} of {:.3} best available", pick.competence, pick.best_competence)).on_hover_text(
                "Fixed role-weighted benchmark utility on a 0–1 scale. It contributes to automatic selection and any optional minimum; it is not a retry success probability.",
            );
            if let Some(floor) = pick.competence_floor {
                ui.label(format!("Required competence minimum: {floor:.3}"));
            } else {
                ui.label("Automatic competence–capacity balance");
            }
            ui.label(format!(
                "Worst proportional loss: {:.1}%",
                pick.worst_shortfall * 100.0
            ));
            ui.label(format!(
                "Competence retained: {:.1}% · worst-case capacity retained: {:.1}%",
                (1.0 - pick.competence_shortfall) * 100.0,
                (1.0 - pick.capacity_shortfall) * 100.0,
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
                ui.label(format!("Fixed role competence: {:.3}", pick.competence));
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
                                    .map(|metric| format!("{:.1}%", metric.pass * 100.0))
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
                            let leader = self.table.rows.get(scenario.row_index)
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
            ui.heading(format!("Top-{} robust policies", pick.top.len().min(4)));
            ui.label(
                "Ordered by automatic worst proportional loss across competence and capacity.",
            );
            for (idx, cand) in pick.top.iter().take(4).enumerate() {
                ui.group(|ui| {
                    let cand_row = self.table.rows.get(cand.row_index);
                    let name = cand_row
                        .map(|r| r.display_name())
                        .unwrap_or_else(|| format!("Row #{}", cand.row_index));
                    ui.strong(format!("#{}: {}", idx + 1, name));
                    ui.label(format!(
                        "Competence {:.3}/{:.3} ({:.1}% loss) | Capacity loss {:.1}% | Worst loss {:.1}%",
                        cand.competence,
                        cand.best_competence,
                        cand.competence_shortfall * 100.0,
                        cand.capacity_shortfall * 100.0,
                        cand.worst_shortfall * 100.0,
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
                "No candidate meets the configured competence minimum for this seat and tier.",
            );
            selected_floor = self.frontier_content(ui, seat, tier);
        }

        ui.add_space(10.0);
        ui.heading("Source & Timestamps");
        ui.label("Source: Artificial Analysis");
        ui.label(format!("Source Fetched: {}", self.table.source_fetched_at));
        ui.label(format!("Generated At: {}", self.table.generated_at));
        ui.label(format!("Cache State: {}", self.table.cache_state.name()));
        selected_floor
    }

    fn counterfactual_content(&mut self, ui: &mut egui::Ui, seat: Seat, tier: Tier) {
        let Some(pick) = self.table.get_pick(seat, tier) else {
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
        ui.label("Each estimate changes only this configuration. Competitors stay unchanged while ideal points and retry caps are recomputed across 24 bounded sampled changes.");
        let selected_name = self
            .table
            .rows
            .get(*choice)
            .map(|row| row.display_name())
            .unwrap_or_else(|| format!("Row #{}", *choice));
        egui::ComboBox::from_id_salt(("comparison_choice", seat, tier))
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for row_index in alternatives {
                    let name = self
                        .table
                        .rows
                        .get(row_index)
                        .map(|row| row.display_name())
                        .unwrap_or_else(|| format!("Row #{row_index}"));
                    ui.selectable_value(choice, row_index, name);
                }
            });

        let request = (self.comparison_generation, seat, tier, *choice);
        let pending = self.comparison_pending.is_some();
        let score_is_current =
            self.scored_settings.as_ref() == Some(&self.settings) && !self.refresh_in_flight;
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
            let rows = self.table.rows.clone();
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
                    "Current alternative: competence {:.3}, {:.1} agent completions/week, MAX {}",
                    target.competence, target.autonomous_tasks_per_week, target.attempt_limit
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
        std::thread::spawn(move || {
            let outcome =
                crate::update::check(env!("CARGO_PKG_VERSION")).map_err(|e| format!("{e:#}"));
            let _ = tx.send(outcome);
            context.request_repaint();
        });
    }

    fn start_install(&mut self, context: egui::Context) {
        let staged = self.staged_update.take();
        let available = self.available_update.clone();
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
                    .ok_or_else(|| anyhow::anyhow!("No update is available"))
                    .and_then(|available| crate::update::download(&available, |_, _| {})),
            };
            let failure = match downloaded {
                Ok(mut staged) => match crate::update::install(&mut staged) {
                    Ok(never) => match never {},
                    Err(error) => (format!("{error:#}"), Some(staged)),
                },
                Err(error) => (format!("{error:#}"), None),
            };
            let _ = tx.send(failure);
            context.request_repaint();
        });
    }

    fn pump_channels(&mut self) {
        while let Ok((new_table, fetched_rows, scored_settings)) = self.table_rx.try_recv() {
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
            self.panel_widths.clear();
            self.invalidate_comparisons();
            self.table = new_table;
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
                    crate::update::Outcome::Available(av) => {
                        let status =
                            format!("Update {} available via {}", av.version, checked.transport);
                        self.available_update = Some(av);
                        status
                    }
                    crate::update::Outcome::UpToDate { latest } => {
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
            && let Ok((error, staged)) = rx.try_recv()
        {
            self.install_rx = None;
            self.staged_update = staged;
            self.update_status = error;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_channels();
        let ctx = ui.ctx().clone();
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
                        .add_enabled(self.install_rx.is_none(), egui::Button::new(label))
                        .clicked();
                }
                if !self.handoff_status.is_empty() {
                    ui.label(&self.handoff_status);
                }
                ui.label(&self.update_status);
            });
        });

        let panel_open = self.side_panel_open && self.selected_seat_tier.is_some();
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
                                self.counterfactual_content(ui, seat, tier);
                                ui.separator();
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
                ui.heading("Workflow hours per week");
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
            });
            ui.label("Combined weekly time for agent attempts and rescue work. Example: 10 hours across 3 parallel agents = 30 workflow hours.");
            ui.add_space(8.0);
            egui::ScrollArea::both()
                .id_salt("roles_table_scroll")
                .auto_shrink([false, false])
                .max_height(ui.available_height())
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.strong("Coding Agent Tier List");
                    ui.add(
                        egui::Label::new(
                            "Picks automatically balance proportional competence loss with worst-case proportional capacity loss. Optional competence minimums remain available for hard requirements.",
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
                                "The automatic choice minimizes its largest percentage loss: competence versus the best available competence, or capacity versus each scenario's capacity leader.",
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
                            ui.strong("Worst loss").on_hover_text(
                                "Largest proportional shortfall across competence and capacity scenarios. Smaller is better.",
                            );
                            ui.strong("Min/cycle");
                            ui.strong("$/cycle");
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
                                    let maybe_pick = self.table.get_pick(seat, tier);

                                    let (
                                        agent_name,
                                        competence,
                                        worst_shortfall,
                                        min_task,
                                        cost_task,
                                        tasks_wk,
                                        a_star_hours,
                                    ) = if let Some(pick) = maybe_pick {
                                        let mut name = self
                                            .table
                                            .rows
                                            .get(pick.row_index)
                                            .map(|r| r.display_name())
                                            .unwrap_or_else(|| "-".into());
                                        if pick.competence_floor.is_none() {
                                            name.push_str(" · Automatic");
                                        }
                                        let competence = format!("{:.3}", pick.competence);
                                        let worst_shortfall =
                                            format!("{:.1}%", pick.worst_shortfall * 100.0);
                                        let min_task = format!("{:.2}", pick.minutes_per_task);
                                        let cost_task = format!("${:.2}", pick.cost_per_task);
                                        let tasks_wk = format!("{:.1}", pick.tasks_per_week);
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
                                            worst_shortfall,
                                            min_task,
                                            cost_task,
                                            tasks_wk,
                                            a_star_hours,
                                        )
                                    } else {
                                        (
                                            "No candidate meets requirements".into(),
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
                                    if ui.selectable_label(is_selected, &worst_shortfall).clicked() {
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
                                                "{left}: competence {:.3}, {:.1}/wk; {right}: competence {:.3}, {:.1}/wk. Scenario production difference ranges {:+.1} to {:+.1}/wk (right minus left).",
                                                comparison.left_competence,
                                                comparison.left_autonomous_tasks_per_week,
                                                comparison.right_competence,
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
                                        [percent(row.swe), percent(row.term), percent(row.qna)]
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
                                            ui.label(value);
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
                        ui.label("Retry failure correlation:").on_hover_text(
                            "0 treats attempts as independent. Larger values make repeat failures more likely by reducing conditional success after failures.",
                        );
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.rho)
                                    .speed(0.001)
                                    .range(0.0..=0.999),
                            )
                            .changed();
                    });

                    ui.separator();
                    egui::CollapsingHeader::new("Optional competence minimums")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(
                                "The automatic choice already balances competence and capacity. Enable a minimum only to enforce a hard qualification requirement.",
                            ).wrap());
                            for seat in Seat::ALL {
                                let floor = self
                                    .settings
                                    .competence_floors
                                    .entry(seat.name().to_string())
                                    .or_insert(None);
                                ui.horizontal(|ui| {
                                    let mut enabled = floor.is_some();
                                    if ui.checkbox(&mut enabled, seat.name()).changed() {
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
        if install_clicked {
            self.start_install(ctx);
        }
    }
}
