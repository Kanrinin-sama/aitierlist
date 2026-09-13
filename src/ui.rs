use crate::portfolio::WorkClass;
use crate::settings::{BestInHouseMode, Settings, load_settings, save_settings};
use crate::types::{CacheState, Seat, Table, Tier};

mod about_view;
mod evidence_view;
mod settings_view;
mod setup_view;

use setup_view::Generator;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Team,
    Rankings,
    Setup,
    Settings,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Why,
    Alternatives,
    Evidence,
}
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

fn role_model_name(row: &crate::types::Row) -> String {
    let name = row.display_name();
    name.strip_suffix(" (benchmark proxy)")
        .unwrap_or(&name)
        .to_owned()
}

fn evidence_level_text(level: crate::engine::RoleEvidenceLevel) -> &'static str {
    match level {
        crate::engine::RoleEvidenceLevel::ExactHarness => "exact harness",
        crate::engine::RoleEvidenceLevel::ModelLevel => "model-level",
        crate::engine::RoleEvidenceLevel::CrossHarnessProxy => "cross-harness proxy",
        crate::engine::RoleEvidenceLevel::Unknown => "unknown",
    }
}

fn role_evidence_summary(row: &crate::types::Row, seat: Seat) -> String {
    crate::engine::role_evidence(row, seat).map_or_else(
        || "No role evidence".to_owned(),
        |components| {
            components
                .into_iter()
                .map(|component| {
                    format!(
                        "{} {} · {}",
                        component.name,
                        competence_text(component.value),
                        evidence_level_text(component.level)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        },
    )
}

fn role_evidence_sources(row: &crate::types::Row, seat: Seat) -> String {
    crate::engine::role_evidence(row, seat).map_or_else(
        || "No role-specific evidence is linked.".to_owned(),
        |components| {
            components
                .into_iter()
                .map(|component| {
                    format!(
                        "{}: {} · {}",
                        component.name,
                        evidence_level_text(component.level),
                        component
                            .source_id
                            .as_deref()
                            .unwrap_or(component.basis.as_str())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

fn role_evidence_compact(row: &crate::types::Row, seat: Seat) -> String {
    if seat == Seat::Orchestrator {
        if row.smart.is_none() {
            return "Orchestration evidence unknown".to_owned();
        }
        let diagnostic = crate::engine::role_evidence(row, seat)
            .into_iter()
            .flatten()
            .any(|component| component.benchmark.is_some() && component.value.is_some());
        if diagnostic {
            return "Intelligence Index · orchestration competence · LCR/HLE diagnostic".to_owned();
        }
        return "Intelligence Index · orchestration competence".to_owned();
    }
    let Some(components) = crate::engine::role_evidence(row, seat) else {
        return "Unknown".to_owned();
    };
    let mut exact = 0;
    let mut model = 0;
    let mut proxy = 0;
    let mut unknown = 0;
    for component in components {
        match component.level {
            crate::engine::RoleEvidenceLevel::ExactHarness => exact += 1,
            crate::engine::RoleEvidenceLevel::ModelLevel => model += 1,
            crate::engine::RoleEvidenceLevel::CrossHarnessProxy => proxy += 1,
            crate::engine::RoleEvidenceLevel::Unknown => unknown += 1,
        }
    }
    [
        (exact, "exact"),
        (model, "model-level"),
        (proxy, "harness proxy"),
        (unknown, "unknown"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, label)| format!("{count} {label}"))
    .collect::<Vec<_>>()
    .join(" · ")
}

fn role_evidence_hover(row: &crate::types::Row, seat: Seat) -> String {
    format!(
        "{}\n{}",
        role_evidence_summary(row, seat),
        role_evidence_sources(row, seat)
    )
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
    role_model_name(row)
}

type ComparisonResponse = (
    u64,
    Seat,
    Tier,
    usize,
    crate::comparison::CounterfactualReport,
);

enum TableResponse {
    Primary(Box<Table>, bool, Settings),
    Vendors(Box<[Table; 5]>, Settings),
}

fn rows_expired(fetched_at: &str, cache_hours: u32) -> bool {
    OffsetDateTime::parse(fetched_at, &Rfc3339)
        .map(|fetched| {
            OffsetDateTime::now_utc() >= fetched + time::Duration::hours(i64::from(cache_hours))
        })
        .unwrap_or(true)
}

pub struct App {
    brand: egui::TextureHandle,
    section: Section,
    settings_tab: usize,
    workflow_class: WorkClass,
    ranking_seat: Seat,
    ranking_tier: Tier,
    detail_tab: DetailTab,
    edit_plan: bool,
    table: Table,
    vendor_tables: [Table; 5],
    selected_tab: usize,
    table_revision: u64,
    generator: Generator,
    native_connections: crate::native_connections::NativeConnections,
    native_connection_status: String,
    desktop: Option<crate::desktop::Desktop>,
    window_visible: bool,
    hide_command_sent: bool,
    scoring_started: bool,
    quit_requested: bool,
    panel_widths: std::collections::HashMap<(Seat, Tier), f32>,
    rho_override_text: String,
    collaboration_root_text: String,
    subscription_count_text: std::collections::BTreeMap<String, String>,
    workspace_status: String,
    background_status: String,
    workspace_picker: Option<Receiver<Result<Option<std::path::PathBuf>, String>>>,
    refresh_in_flight: bool,
    retry_after: Option<Instant>,
    refresh_warning: String,
    table_rx: Receiver<TableResponse>,
    scored_settings: Option<Settings>,
    settings_tx: Sender<(Settings, bool)>,
    engine_error_rx: Receiver<String>,
    engine_status: String,
    refresh_clicked_at: Option<Instant>,
    settings: Settings,
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
    conductor_comparison_rx: Receiver<(u64, Vec<crate::portfolio::ConductorTradeoff>)>,
    conductor_comparison_tx: Sender<(u64, Vec<crate::portfolio::ConductorTradeoff>)>,
    conductor_comparison_pending: bool,
    conductor_comparison: Vec<crate::portfolio::ConductorTradeoff>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        updater: blockitall_update::Updater,
        health_guard: Option<blockitall_update::HealthGuard>,
        handoff_status: String,
        desktop: Option<crate::desktop::Desktop>,
        open_subscriptions: bool,
    ) -> Self {
        let (table_tx, table_rx) = channel();
        let settings = load_settings();
        let table = Table::empty();
        let (settings_tx, settings_rx) = channel::<(Settings, bool)>();
        let (comparison_tx, comparison_rx) = channel();
        let (conductor_comparison_tx, conductor_comparison_rx) = channel();
        let (engine_error_tx, engine_error_rx) = channel();
        let context = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            let mut loaded: Option<(Vec<crate::types::Row>, CacheState, String)> = None;
            'requests: while let Ok((mut settings, mut force_refresh)) = settings_rx.recv() {
                let mut published_cached = false;
                for (pending, pending_force) in settings_rx.try_iter() {
                    settings = pending;
                    force_refresh |= pending_force;
                }
                if loaded.is_none() {
                    match crate::aa::cached_rows() {
                        Ok(rows) => loaded = Some(rows),
                        Err(error) => {
                            let _ = engine_error_tx.send(format!("Engine: {error:#}"));
                            context.request_repaint();
                            continue 'requests;
                        }
                    }
                    let (rows, state, fetched) = loaded.as_ref().expect("cached rows loaded");
                    let table = crate::engine::score(
                        rows.clone(),
                        &settings,
                        *state,
                        fetched.clone(),
                        None,
                    );
                    let fresh = !rows_expired(fetched, settings.cache_hours);
                    if table_tx
                        .send(TableResponse::Primary(
                            Box::new(table),
                            fresh,
                            settings.clone(),
                        ))
                        .is_err()
                    {
                        break;
                    }
                    context.request_repaint();
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
                        .send(TableResponse::Vendors(
                            Box::new(vendor_tables),
                            settings.clone(),
                        ))
                        .is_err()
                    {
                        break;
                    }
                    context.request_repaint();
                    published_cached = true;
                }
                let cached_stale = loaded
                    .as_ref()
                    .is_none_or(|(_, _, fetched)| rows_expired(fetched, settings.cache_hours))
                    || crate::aa::upstream_stale();
                let mut fetched_rows = force_refresh || cached_stale;
                if fetched_rows {
                    match crate::aa::load_rows(force_refresh, f64::from(settings.cache_hours)) {
                        Ok(rows) => loaded = Some(rows),
                        Err(error) => {
                            let _ = engine_error_tx.send(format!("Engine: {error:#}"));
                            context.request_repaint();
                            continue 'requests;
                        }
                    }
                } else if published_cached {
                    continue;
                }
                let mut refresh_again = false;
                for (pending, pending_force) in settings_rx.try_iter() {
                    settings = pending;
                    refresh_again |= pending_force;
                }
                if refresh_again && !fetched_rows {
                    match crate::aa::load_rows(true, f64::from(settings.cache_hours)) {
                        Ok(rows) => loaded = Some(rows),
                        Err(error) => {
                            let _ = engine_error_tx.send(format!("Engine: {error:#}"));
                            context.request_repaint();
                            continue 'requests;
                        }
                    }
                    fetched_rows = true;
                }
                let Some((rows, state, fetched)) = &loaded else {
                    continue;
                };
                let table =
                    crate::engine::score(rows.clone(), &settings, *state, fetched.clone(), None);
                if table_tx
                    .send(TableResponse::Primary(
                        Box::new(table),
                        fetched_rows,
                        settings.clone(),
                    ))
                    .is_err()
                {
                    break;
                }
                context.request_repaint();
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
                    .send(TableResponse::Vendors(
                        Box::new(vendor_tables),
                        settings.clone(),
                    ))
                    .is_err()
                {
                    break;
                }
                context.request_repaint();
            }
        });
        let mut app = Self {
            brand: crate::theme::load_brand(&cc.egui_ctx),
            section: if open_subscriptions {
                Section::Settings
            } else {
                Section::Team
            },
            settings_tab: 0,
            workflow_class: WorkClass::Standard,
            ranking_seat: Seat::Implementer,
            ranking_tier: Tier::Api,
            detail_tab: DetailTab::Why,
            edit_plan: false,
            table,
            vendor_tables: std::array::from_fn(|_| Table::empty()),
            selected_tab: 0,
            table_revision: 0,
            generator: Generator::new(),
            native_connections: crate::native_connections::NativeConnections::new(
                cc.egui_ctx.clone(),
            ),
            native_connection_status: String::new(),
            desktop,
            window_visible: true,
            hide_command_sent: false,
            scoring_started: false,
            quit_requested: false,
            panel_widths: std::collections::HashMap::new(),
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
            background_status: String::new(),
            workspace_picker: None,
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
            selected_seat_tier: None,
            side_panel_open: false,
            update_status: String::new(),
            handoff_status,
            updater: Arc::new(Mutex::new(updater)),
            health_guard,
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
            conductor_comparison_rx,
            conductor_comparison_tx,
            conductor_comparison_pending: false,
            conductor_comparison: Vec::new(),
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
                                    .map(role_model_name)
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
                        .map(role_model_name)
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
                "Accuracy {:.4} × {:.2}; no incorrect answer (all questions) {:.4} × {:.2}; LCR {:.4} × {:.2}",
                primary.accuracy,
                primary.accuracy_weight,
                primary.non_wrong,
                primary.non_wrong_weight,
                primary.lcr,
                primary.lcr_weight
            ));
            ui.small(
                "The research score is the fixed-weight geometric mean of the selected components; the weights are task-policy weights.",
            );
            if primary.hle_weight > 0.0 {
                ui.label(format!(
                    "HLE {} × {:.2}",
                    primary
                        .hle
                        .map_or_else(|| "Unknown".to_owned(), |value| format!("{value:.4}")),
                    primary.hle_weight
                ));
            }
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
                ui.label(
                    egui::RichText::new(role_model_name(row))
                        .strong()
                        .size(15.0),
                );
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

            if let Some(portfolio) = table.portfolio.as_ref() {
                let choices = portfolio
                    .close_choices
                    .iter()
                    .filter(|choice| choice.seat == seat)
                    .collect::<Vec<_>>();
                if !choices.is_empty() {
                    egui::CollapsingHeader::new("Conditional joint-plan alternatives").show(
                        ui,
                        |ui| {
                            for choice in choices {
                                let challenger = table
                                    .rows
                                    .get(choice.challenger_row_index)
                                    .map(role_model_name)
                                    .unwrap_or_else(|| {
                                        format!("Row #{}", choice.challenger_row_index)
                                    });
                                ui.label(format!(
                                    "{}{} · scenario assignment utility {:.3}",
                                    challenger,
                                    choice
                                        .class
                                        .map(|class| format!(" / {}", class.name()))
                                        .unwrap_or_default(),
                                    choice.challenger_utility
                                ));
                                ui.small(format!(
                                    "Conditional on the complete-plan scenario; native qualification is required. {}",
                                    choice.condition
                                ));
                            }
                        },
                    );
                }
            }

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
                if let Some(components) = crate::engine::role_evidence(row, seat) {
                    egui::Grid::new(("competence_grid", seat, tier))
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Capability");
                            ui.strong("Score");
                            ui.strong("Fixed weight");
                            ui.strong("Evidence");
                            ui.end_row();
                            for component in components {
                                ui.label(component.name);
                                ui.label(competence_text(component.value));
                                ui.label(format!("{:.1}%", component.weight * 100.0));
                                ui.label(evidence_level_text(component.level))
                                    .on_hover_text(format!(
                                        "{}\n{}\n{}\n{}",
                                        component.basis,
                                        component
                                            .benchmark
                                            .map(|benchmark| benchmark.name())
                                            .unwrap_or("Composite model evidence"),
                                        component
                                            .source_id
                                            .as_deref()
                                            .unwrap_or("No linked source"),
                                        component
                                            .observation_id
                                            .as_deref()
                                            .unwrap_or("No linked observation")
                                    ));
                                ui.end_row();
                            }
                        });
                    ui.small(
                        "The role score is the fixed-weight geometric mean of these components. Unknown components do not become zero-valued evidence.",
                    );
                }
                if self.settings.best_in_house_mode == BestInHouseMode::PerPlan {
                    let selected_capability = pick.competence.unwrap_or(0.0);
                    let incomplete = table
                        .rows
                        .iter()
                        .enumerate()
                        .filter_map(|(index, candidate)| {
                            let relevant = index != pick.row_index
                                && crate::aa::vendor_key(&candidate.harness, &candidate.model)
                                    .is_some_and(|provider| {
                                        self.settings.subscriptions.contains_key(provider)
                                    })
                                && crate::engine::role_evidence(candidate, seat).is_some_and(
                                    |components| {
                                        components.iter().any(|component| component.value.is_none())
                                    },
                                );
                            let sensitivity = relevant.then(|| {
                                crate::engine::competence_sensitivity(
                                    candidate,
                                    seat,
                                    &self.settings,
                                )
                            })?;
                            sensitivity
                                .is_none_or(|scenario| {
                                    scenario.high.is_none_or(|high| high >= selected_capability)
                                })
                                .then_some((candidate, sensitivity))
                        })
                        .collect::<Vec<_>>();
                    if !incomplete.is_empty() {
                        egui::CollapsingHeader::new(format!(
                            "Subscribed routes with missing role evidence ({})",
                            incomplete.len()
                        ))
                        .show(ui, |ui| {
                            for (candidate, sensitivity) in incomplete {
                                ui.label(format!(
                                    "{} · Unranked—missing evidence",
                                    role_model_name(candidate)
                                ));
                                ui.small(sensitivity.map_or_else(
                                    || {
                                        "No score range is available because the role component weights or task counts are unavailable."
                                            .to_owned()
                                    },
                                    |scenario| match (scenario.low, scenario.high) {
                                        (Some(low), Some(high)) => format!(
                                            "Declared capability scenario {low:.3}–{high:.3}; this is not a confidence interval. {}",
                                            role_evidence_summary(candidate, seat)
                                        ),
                                        _ => "No score range is available because the role component weights or task counts are unavailable."
                                            .to_owned(),
                                    },
                                ));
                            }
                        });
                    }
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
                        ui.strong("Dataset / tasks");
                        ui.strong("Reference weight");
                        ui.strong("Pass");
                        ui.strong("Time/attempt");
                        ui.strong("Cost/attempt");
                        ui.end_row();
                        for (benchmark, weight) in
                            crate::engine::reference_workload(row, seat).unwrap_or_default()
                        {
                            let metric = row
                                .task_metrics
                                .iter()
                                .find(|metric| metric.benchmark == benchmark);
                            ui.add_sized(
                                [88.0, ui.spacing().interact_size.y],
                                egui::Label::new(benchmark.name()).wrap(),
                            );
                            ui.label(format!("{:.1}%", weight * 100.0));
                            ui.label(metric.map_or_else(
                                || "Unknown".to_owned(),
                                |metric| format!("{} / {}", metric.dataset_id, metric.task_count),
                            ));
                            let pass = ui.label(
                                metric
                                    .map(|metric| {
                                        format!(
                                            "{:.1}%",
                                            row.retry.adjusted_pass(benchmark, metric.pass) * 100.0
                                        )
                                    })
                                    .unwrap_or_else(|| "Unknown".into()),
                            );
                            if let Some(evidence) = row.retry.repeat_evidence(benchmark) {
                                pass.on_hover_text(format!(
                                    "Retry dependence: {:.3} / {} / {}",
                                    evidence.rho,
                                    evidence.layer.name(),
                                    evidence.evidence_dataset
                                ));
                            }
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
                    "Normalized Intelligence Index (AA score / 100). Orchestrator competence is this index. It is not a success probability. LCR and HLE are diagnostics only.",
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
                                .map(role_model_name)
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

        selected_floor
    }

    fn why_content(&self, ui: &mut egui::Ui, seat: Seat, tier: Tier) {
        let table = self.selected_table();
        if seat == Seat::NetResearch {
            let Some(primary) = table
                .research_tiers
                .iter()
                .find(|pick| pick.tier == tier)
                .and_then(|pick| pick.primary.as_ref())
            else {
                ui.label("No eligible research route for these filters.");
                return;
            };
            let Some(row) = table.rows.get(primary.row_index) else {
                return;
            };
            ui.heading(research_route_name(row, &primary.native_harness));
            ui.label("Highest qualified research score among eligible routes for this tier.");
            ui.separator();
            ui.label(format!("Research score: {:.3}", primary.score));
            ui.label(format!(
                "Provider: {} · {}",
                primary.provider_id, primary.plan_id
            ));
            ui.label(format!("Native harness: {}", primary.native_harness));
            return;
        }
        let Some(pick) = table.get_pick(seat, tier) else {
            ui.label("No qualified recommendation for these filters.");
            return;
        };
        let Some(row) = table.rows.get(pick.row_index) else {
            return;
        };
        ui.heading(role_model_name(row));
        ui.add(
            egui::Label::new(
                "Best nominal reference capacity after the selected competence and scenario constraints.",
            )
            .wrap(),
        );
        ui.separator();
        ui.label(format!("Competence: {}", competence_text(pick.competence)));
        ui.label(format!("Reference tasks/week: {:.1}", pick.tasks_per_week));
        ui.label(format!(
            "Scenario band: {:.1}–{:.1}",
            pick.tasks_low, pick.tasks_high
        ));
        ui.label(format!(
            "Modeled cost/reference: ${:.2}",
            pick.cost_per_task
        ));
        ui.label(format!("Retry cap: {}", pick.attempt_limit));
    }

    fn research_alternatives_content(&self, ui: &mut egui::Ui, tier: Tier) {
        let table = self.selected_table();
        let Some(pick) = table.research_tiers.iter().find(|pick| pick.tier == tier) else {
            ui.label("No research alternatives for this tier.");
            return;
        };
        if pick.alternatives.is_empty() {
            ui.label("No other eligible research routes in the current evidence.");
            return;
        }
        for candidate in &pick.alternatives {
            let Some(row) = table.rows.get(candidate.row_index) else {
                continue;
            };
            ui.strong(research_route_name(row, &candidate.native_harness));
            ui.label(format!(
                "{} · {} · score {:.3}",
                candidate.provider_id, candidate.plan_id, candidate.score
            ));
            ui.label(egui::RichText::new(&candidate.eligibility).color(crate::theme::MUTED));
            ui.separator();
        }
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
            .map(role_model_name)
            .unwrap_or_else(|| format!("Row #{}", *choice));
        egui::ComboBox::from_id_salt(("comparison_choice", seat, tier))
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for row_index in alternatives {
                    let name = table
                        .rows
                        .get(row_index)
                        .map(role_model_name)
                        .unwrap_or_else(|| format!("Row #{row_index}"));
                    ui.selectable_value(choice, row_index, name);
                }
            });

        let request = (self.comparison_generation, seat, tier, *choice);
        let pending = self.comparison_pending.is_some();
        let score_is_current = self
            .scored_settings
            .as_ref()
            .is_some_and(|scored| scored.scoring_matches(&self.settings));
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

    fn source_refresh_delay(&self) -> Option<Duration> {
        let fetched = OffsetDateTime::parse(&self.table.source_fetched_at, &Rfc3339).ok()?;
        let deadline = fetched + time::Duration::hours(i64::from(self.settings.cache_hours));
        let remaining = deadline - OffsetDateTime::now_utc();
        (remaining > time::Duration::ZERO)
            .then(|| Duration::from_nanos(remaining.whole_nanoseconds() as u64))
    }

    fn start_refresh(&mut self) {
        if self.refresh_in_flight {
            return;
        }
        if self.settings_tx.send((self.settings.clone(), true)).is_ok() {
            self.invalidate_comparisons();
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
        while let Ok(response) = self.table_rx.try_recv() {
            let (new_table, fetched_rows, scored_settings) = match response {
                TableResponse::Primary(table, fetched_rows, settings) => {
                    (table, fetched_rows, settings)
                }
                TableResponse::Vendors(vendor_tables, settings) => {
                    if settings.scoring_matches(&self.settings) {
                        self.vendor_tables = *vendor_tables;
                        if !self.refresh_in_flight {
                            self.engine_status.clear();
                        }
                    }
                    continue;
                }
            };
            let new_table = *new_table;
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
            self.table_revision = self.table_revision.wrapping_add(1);
            self.table = new_table;
            self.vendor_tables = std::array::from_fn(|_| Table::empty());
            self.conductor_comparison.clear();
            self.conductor_comparison_pending = false;
            self.comparison_choice.clear();
            self.scored_settings = Some(scored_settings);
            self.engine_status = if self.refresh_in_flight {
                "Refreshing vendor views…".to_owned()
            } else {
                "Scoring vendor views…".to_owned()
            };
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

    fn apply_settings_change(&mut self) {
        self.settings = self.settings.clone().normalize();
        let _ = save_settings(&self.settings);
        self.invalidate_comparisons();
        self.conductor_comparison.clear();
        self.table_revision = self.table_revision.wrapping_add(1);
        let _ = self.settings_tx.send((self.settings.clone(), false));
        self.engine_status = "Scoring…".to_owned();
    }

    fn save_desktop_settings(&mut self) -> Result<(), String> {
        self.settings = self.settings.clone().normalize();
        save_settings(&self.settings).map_err(|error| format!("Saving settings failed: {error:#}"))
    }

    fn team_content(&mut self, ui: &mut egui::Ui) {
        Self::section_heading(
            ui,
            "Your team",
            "One coordinated plan across the AI subscriptions you already use.",
        );
        if self.settings.subscriptions.is_empty() {
            self.edit_plan = true;
        }
        if self.edit_plan {
            egui::Frame::group(ui.style())
                .fill(crate::theme::SURFACE)
                .corner_radius(10)
                .inner_margin(18)
                .show(ui, |ui| {
                    let mut changed = self.subscription_controls(ui);
                    ui.horizontal(|ui| {
                        ui.label("Hours per account");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.agent_hours)
                                    .range(1.0..=1680.0),
                            )
                            .changed();
                        ui.label("Concurrent projects");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.orchestrators)
                                    .range(1..=64),
                            )
                            .changed();
                    });
                    ui.horizontal_wrapped(|ui| {
                        let mut reserve_usd = self.settings.orchestrator_headroom_usd.is_some();
                        if ui
                            .checkbox(&mut reserve_usd, "Reserve Orchestrator USD headroom")
                            .changed()
                        {
                            self.settings.orchestrator_headroom_usd = reserve_usd.then_some(0.0);
                            changed = true;
                        }
                        if let Some(value) = &mut self.settings.orchestrator_headroom_usd {
                            changed |= ui
                                .add(egui::DragValue::new(value).range(0.0..=1_000_000.0).prefix("$"))
                                .changed();
                        }
                        let mut reserve_hours =
                            self.settings.orchestrator_headroom_hours.is_some();
                        if ui
                            .checkbox(&mut reserve_hours, "Reserve Orchestrator hours")
                            .changed()
                        {
                            self.settings.orchestrator_headroom_hours =
                                reserve_hours.then_some(0.0);
                            changed = true;
                        }
                        if let Some(value) = &mut self.settings.orchestrator_headroom_hours {
                            changed |= ui
                                .add(egui::DragValue::new(value).range(0.0..=1680.0).suffix(" h"))
                                .changed();
                        }
                    });
                    ui.small(
                        "Optional fixed Orchestrator headroom is reserved once for persistent coordination and does not scale with worker jobs.",
                    );
                    if changed {
                        self.apply_settings_change();
                    }
                    if !self.settings.subscriptions.is_empty()
                        && ui.button("Done editing").clicked()
                    {
                        self.edit_plan = false;
                    }
                });
            return;
        }
        if !self
            .scored_settings
            .as_ref()
            .is_some_and(|scored| scored.scoring_matches(&self.settings))
        {
            if !self.refresh_warning.is_empty() {
                ui.colored_label(crate::theme::ERROR, &self.refresh_warning);
                if ui.button("Retry refresh").clicked() {
                    self.start_refresh();
                }
            } else {
                ui.spinner();
                ui.label(if self.engine_status.is_empty() {
                    "Recalculating your team…"
                } else {
                    &self.engine_status
                });
            }
            return;
        }
        if let Ok((revision, points)) = self.conductor_comparison_rx.try_recv()
            && revision == self.table_revision
        {
            self.conductor_comparison = points;
            self.conductor_comparison_pending = false;
        }
        let Some(portfolio) = self.table.portfolio.as_ref() else {
            ui.spinner();
            ui.label("Building your team from the current evidence…");
            return;
        };
        let dispatch = &portfolio.dispatch;
        let estimated = portfolio.pools.iter().any(|pool| pool.price_is_estimate);
        egui::Frame::group(ui.style())
            .fill(crate::theme::SURFACE)
            .corner_radius(10)
            .inner_margin(16)
            .show(ui, |ui| {
                let summary_width = (ui.available_width() - 128.0).max(180.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(summary_width);
                        let plans = portfolio
                            .pools
                            .iter()
                            .map(|pool| format!("{} {}", pool.provider_name, pool.plan_name))
                            .collect::<Vec<_>>()
                            .join(" · ");
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "{plans} · {} concurrent projects",
                                    portfolio.orchestrators
                                ))
                                .strong(),
                            )
                            .wrap(),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Shared allowance and working time across all roles",
                            )
                            .color(crate::theme::MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Edit plan").clicked() {
                            self.edit_plan = true;
                        }
                    });
                });
            });
        ui.add_space(12.0);
        let cards = [
            (
                if estimated {
                    "EST. MONTHLY SPEND".to_owned()
                } else {
                    "MONTHLY SPEND".to_owned()
                },
                format!("${:.2}", portfolio.monthly_price),
                "/month · Declared subscriptions".to_owned(),
            ),
            (
                "HOURS / ACCOUNT".to_owned(),
                format!("{:.1} weekly", portfolio.available_hours_per_provider),
                "Shared by worker roles".to_owned(),
            ),
            (
                "MODELED FIT / FORECAST".to_owned(),
                format!(
                    "{} / {}",
                    dispatch.admitted_changes, dispatch.forecast_changes
                ),
                "Allowance and working time".to_owned(),
            ),
        ];
        let card_columns = if ui.available_width() >= 900.0 { 4 } else { 2 };
        for row in cards.chunks(card_columns) {
            ui.columns(card_columns, |columns| {
                for (column, (label, value, note)) in columns.iter_mut().zip(row) {
                    egui::Frame::group(column.style())
                        .fill(crate::theme::SURFACE)
                        .corner_radius(10)
                        .inner_margin(14)
                        .show(column, |ui| {
                            ui.set_min_height(82.0);
                            ui.label(
                                egui::RichText::new(label)
                                    .size(11.0)
                                    .color(crate::theme::MUTED),
                            );
                            ui.label(egui::RichText::new(value).size(18.0).strong());
                            ui.label(
                                egui::RichText::new(note)
                                    .size(11.0)
                                    .color(crate::theme::MUTED),
                            );
                        });
                }
            });
            ui.add_space(8.0);
        }
        let service = &dispatch.service;
        egui::CollapsingHeader::new("Quality-adjusted worker service")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!(
                    "Plan score {:.3} · weighted workload {:.3} · team capability {:.3}",
                    service.quality_adjusted_service,
                    service.workload,
                    service.team_capability
                ))
                .on_hover_text(
                    "Plan score Q is quality-adjusted worker service. W is weighted admitted workload. Team capability is exp(L/W), the workload-weighted geometric capability term.",
                );
                ui.label(format!(
                    "Chosen admitted jobs: {} · largest worker plan found: {} jobs / {} plan score",
                    dispatch.admitted_changes,
                    service.maximum_volume_admitted,
                    service.maximum_volume_service.map_or_else(
                        || "Unknown".to_owned(),
                        |value| format!("{value:.3}")
                    )
                ));
                ui.label(format!(
                    "Worker-service objective bound: {}",
                    service
                        .bound
                        .map_or_else(|| "Unknown".to_owned(), |bound| format!("{bound:.3}"))
                ));
                ui.label(service.relative_gap.map_or_else(
                    || {
                        if service.proven_optimal {
                            "Worker-service objective proved for the declared search.".to_owned()
                        } else {
                            "Worker-service search unresolved; objective gap is unknown."
                                .to_owned()
                        }
                    },
                    |gap| {
                        format!(
                            "Worker-service search gap {:.1}%{}",
                            gap * 100.0,
                            if service.proven_optimal {
                                " · proved"
                            } else {
                                " · unresolved"
                            }
                        )
                    },
                ));
                ui.small(
                    "Quality-adjusted service is a policy utility from the geometric team profile, not a probability of task success. The worker plan is conditional on persistent human-facing coordination.",
                );
                ui.small(&service.message);
            });
        ui.add_space(18.0);
        if !dispatch.executable || portfolio.conductor.is_none() {
            let status = if !portfolio.message.is_empty() {
                &portfolio.message
            } else if !dispatch.solver.message.is_empty() {
                &dispatch.solver.message
            } else {
                "No executable team for this plan"
            };
            ui.colored_label(crate::theme::ERROR, status);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.conductor_comparison_pending,
                    egui::Button::new(if self.conductor_comparison_pending {
                        "Comparing conductor floors…"
                    } else {
                        "Compare conductor floors"
                    }),
                )
                .on_hover_text("Runs bounded full-plan replans at each measured Intelligence Index floor. No balanced point or global frontier is inferred.")
                .clicked()
            {
                self.conductor_comparison_pending = true;
                let rows = self.table.rows.clone();
                let settings = self.settings.clone();
                let sender = self.conductor_comparison_tx.clone();
                let revision = self.table_revision;
                let context = ui.ctx().clone();
                std::thread::spawn(move || {
                    let _ = sender.send((
                        revision,
                        crate::portfolio::compare_conductors(&rows, &settings),
                    ));
                    context.request_repaint();
                });
            }
            ui.colored_label(
                crate::theme::MUTED,
                "Each point is a separate bounded plan, not a proved global Pareto frontier.",
            );
        });
        if !self.conductor_comparison.is_empty() {
            let mut selected_conductor_floor = None;
            egui::Grid::new("conductor_floor_comparison")
                .striped(true)
                .show(ui, |ui| {
                    for label in ["Minimum II", "Conductor", "Admitted", "Utility", "Gap", ""] {
                        ui.strong(label);
                    }
                    ui.end_row();
                    for point in &self.conductor_comparison {
                        ui.label(format!("{:.3}", point.competence_floor));
                        ui.label(
                            self.table
                                .rows
                                .get(point.row_index)
                                .map_or_else(|| "Unknown".to_owned(), role_model_name),
                        );
                        ui.label(point.admitted_changes.to_string());
                        ui.label(format!("{:.3}", point.quality));
                        ui.label(point.relative_gap.map_or_else(
                            || {
                                if point.proven {
                                    "Proved".to_owned()
                                } else {
                                    "Unknown".to_owned()
                                }
                            },
                            |gap| format!("{:.1}%", gap * 100.0),
                        ));
                        if ui.button("Use floor").clicked() {
                            selected_conductor_floor = Some(point.competence_floor);
                        }
                        ui.end_row();
                    }
                });
            if let Some(floor) = selected_conductor_floor {
                self.settings
                    .competence_floors
                    .insert(Seat::Orchestrator.name().to_owned(), Some(floor));
                let _ = save_settings(&self.settings);
                self.comparison_generation = self.comparison_generation.wrapping_add(1);
                self.comparison_pending = None;
                self.comparison_result = None;
                self.conductor_comparison.clear();
                self.table_revision = self.table_revision.wrapping_add(1);
                let _ = self.settings_tx.send((self.settings.clone(), false));
                self.engine_status = "Scoring…".to_owned();
            }
        }
        ui.strong("Role assignment previews");
        ui.label(
            egui::RichText::new(
                "Select a class to preview its seven seat assignments. Generate Orchestrator includes all four class policies and classifies each actual task automatically.",
            )
            .color(crate::theme::MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            for class in WorkClass::ALL {
                ui.selectable_value(&mut self.workflow_class, class, class.name())
                    .on_hover_text(class.condition());
            }
        });
        ui.add_space(8.0);
        egui::ScrollArea::horizontal()
            .id_salt("team_roles_scroll")
            .show(ui, |ui| {
                egui::Grid::new("team_roles")
                    .striped(true)
                    .min_col_width(110.0)
                    .spacing([18.0, 10.0])
                    .show(ui, |ui| {
                        for label in [
                            "Role",
                            "Model / effort",
                            "Subscription",
                            "Competence",
                            "Role evidence",
                            "Visits",
                        ] {
                            ui.label(
                                egui::RichText::new(label)
                                    .size(11.0)
                                    .color(crate::theme::MUTED),
                            );
                        }
                        ui.end_row();
                        if let Some(conductor) = &portfolio.conductor
                            && let Some(row) = self.table.rows.get(conductor.row_index)
                        {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(12.0, 12.0),
                                    egui::Sense::hover(),
                                );
                                let center = rect.center();
                                ui.painter().add(egui::Shape::convex_polygon(
                                    vec![
                                        egui::pos2(center.x, rect.top()),
                                        egui::pos2(rect.right(), center.y),
                                        egui::pos2(center.x, rect.bottom()),
                                        egui::pos2(rect.left(), center.y),
                                    ],
                                    crate::theme::GOLD,
                                    egui::Stroke::NONE,
                                ));
                                ui.label(
                                    egui::RichText::new("Orchestrator")
                                        .color(crate::theme::GOLD)
                                        .strong(),
                                );
                            });
                            ui.label(role_model_name(row));
                            ui.label(&conductor.provider_id)
                                .on_hover_text(&conductor.plan_id);
                            ui.label(competence_text(Some(conductor.competence)));
                            ui.label(format!(
                                "{} · {}",
                                conductor.evidence_profile, conductor.evidence_level
                            ))
                                .on_hover_text(role_evidence_hover(row, Seat::Orchestrator));
                            let usage = conductor.usage_forecast.as_ref().map_or_else(
                                || "Usage forecast: Unknown / on demand".to_owned(),
                                |forecast| {
                                    format!(
                                        "Usage forecast: {} calls · {:.3} USD · {:.3} h",
                                        forecast.visits, forecast.usage, forecast.hours
                                    )
                                },
                            );
                            let headroom = format!(
                                "Fixed headroom: {} USD · {} h",
                                conductor
                                    .headroom
                                    .usd
                                    .map_or_else(|| "unspecified".to_owned(), |value| format!("{value:.3}")),
                                conductor
                                    .headroom
                                    .hours
                                    .map_or_else(|| "unspecified".to_owned(), |value| format!("{value:.3}"))
                            );
                            ui.label(if conductor.persistent {
                                "Persistent / On demand"
                            } else {
                                "On demand"
                            })
                            .on_hover_text(format!(
                                "Human-facing coordination service. Calls are independent of worker-job counts and actual native use is governed by ledger holds.\n{usage}\n{headroom}"
                            ));
                            ui.end_row();
                        }
                        for seat in crate::orchestration::ROLE_ORDER
                            .into_iter()
                            .filter(|seat| *seat != Seat::Orchestrator)
                        {
                            let Some(role) = portfolio.roles.iter().find(|role| role.seat == seat)
                            else {
                                continue;
                            };
                            let Some(rule) = role
                                .rules
                                .iter()
                                .find(|rule| rule.class == self.workflow_class)
                            else {
                                continue;
                            };
                            ui.strong(seat.name()).on_hover_text(format!(
                                "{}\n{}\n{}\n{}",
                                rule.condition,
                                rule.message,
                                rule.fallback_policy,
                                rule.calibration
                            ));
                            let research_primary = rule
                                .research_candidates
                                .iter()
                                .find(|candidate| candidate.primary);
                            if let Some(primary) = research_primary
                                && let Some(row) = self.table.rows.get(primary.row_index)
                            {
                                ui.label(research_route_name(row, &primary.native_harness));
                                ui.label(&primary.provider_id)
                                    .on_hover_text(&primary.plan_id);
                            } else if let Some(row) =
                                rule.row_index.and_then(|index| self.table.rows.get(index))
                            {
                                ui.label(role_model_name(row));
                                ui.label(rule.provider_id.as_deref().unwrap_or("Unknown"));
                            } else {
                                ui.label("No qualified model");
                                ui.label("—");
                            }
                            ui.label(competence_text(
                                research_primary
                                    .map(|candidate| candidate.score)
                                    .or(rule.competence),
                            ));
                            if let Some(primary) = research_primary {
                                let component_count = 3 + usize::from(primary.hle_weight > 0.0);
                                ui.label(format!("{component_count} model-level components"))
                                    .on_hover_text(format!(
                                        "Accuracy {:.3}; non-wrong {:.3}; LCR {:.3}; HLE {}\n{}",
                                        primary.accuracy,
                                        primary.non_wrong,
                                        primary.lcr,
                                        if primary.hle_weight > 0.0 {
                                            primary.hle.map_or_else(
                                                || "selected but unknown".to_owned(),
                                                |value| format!("{value:.3}")
                                            )
                                        } else {
                                            "not used for this class".to_owned()
                                        },
                                        primary.source
                                    ));
                            } else if let Some(row) =
                                rule.row_index.and_then(|index| self.table.rows.get(index))
                            {
                                ui.label(role_evidence_compact(row, seat))
                                    .on_hover_text(role_evidence_hover(row, seat));
                            } else {
                                ui.label("Unknown");
                            }
                            ui.label(if research_primary.is_some() && rule.planned_jobs == 0 {
                                "On demand".to_owned()
                            } else {
                                format!("{:.2}", rule.expected_visits)
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(sensitivity) = portfolio
            .dispatch
            .repair_sensitivity
            .iter()
            .find(|sensitivity| sensitivity.class == self.workflow_class)
        {
            ui.label(format!(
                "{} repair activation: {:.1}% nominal · scenario {:.1}%–{:.1}%",
                self.workflow_class.name(),
                sensitivity.nominal * 100.0,
                sensitivity.lower * 100.0,
                sensitivity.upper * 100.0
            ))
            .on_hover_text(format!(
                "Declared assumption scenario, not a confidence interval. {}",
                sensitivity.basis
            ));
        }
        if let Some(report) = &portfolio.capability_scenario {
            let choices = portfolio
                .close_choices
                .iter()
                .filter(|choice| {
                    choice.class.is_none() || choice.class == Some(self.workflow_class)
                })
                .collect::<Vec<_>>();
            egui::CollapsingHeader::new("Conditional capability scenario")
                .default_open(!choices.is_empty())
                .show(ui, |ui| {
                    ui.label(format!(
                        "Joint complete-plan scenario: {} · plan objective {:.3} versus chosen team under the same assumptions {}",
                        report.status,
                        report.quality,
                        report
                            .baseline_quality
                            .map_or_else(|| "Unknown".to_owned(), |value| format!("{value:.3}"))
                    ));
                    ui.label(report.bound.map_or_else(
                        || "Scenario objective bound: Unknown".to_owned(),
                        |bound| format!("Scenario objective bound: {bound:.3}"),
                    ));
                    ui.label(report.relative_gap.map_or_else(
                        || {
                            if report.proven {
                                "Scenario result proved for the declared search.".to_owned()
                            } else {
                                "Scenario comparison unresolved; no stability claim is available."
                                    .to_owned()
                            }
                        },
                        |gap| {
                            format!(
                                "Scenario search gap {:.1}%{}",
                                gap * 100.0,
                                if report.proven { " · proved" } else { " · unresolved" }
                            )
                        },
                    ));
                    ui.small(&report.message);
                    if choices.is_empty() {
                        ui.label(if report.proven {
                            "No selected-class assignment changes in this declared joint scenario."
                        } else {
                            "No selected-class alternative is established by the unresolved search."
                        });
                    }
                    for choice in choices {
                        let nominal = self
                            .table
                            .rows
                            .get(choice.nominal_row_index)
                            .map(role_model_name)
                            .unwrap_or_else(|| format!("Row #{}", choice.nominal_row_index));
                        let challenger = self
                            .table
                            .rows
                            .get(choice.challenger_row_index)
                            .map(role_model_name)
                            .unwrap_or_else(|| format!("Row #{}", choice.challenger_row_index));
                        ui.label(format!(
                            "{}{}: {nominal} → {challenger} · assignment utility {:.3} → {:.3}",
                            choice.seat.name(),
                            choice
                                .class
                                .map(|class| format!(" / {}", class.name()))
                                .unwrap_or_default(),
                            choice.nominal_utility,
                            choice.challenger_utility
                        ));
                        ui.small(format!(
                            "Conditional joint-plan alternative; native qualification is required for {} / {} / {} with up to {} attempts. Scenario {} · bound {} · {}. {}",
                            choice.challenger_provider_id,
                            choice.challenger_plan_id,
                            choice.challenger_native_harness,
                            choice.challenger_attempt_limit,
                            choice.scenario_status,
                            choice.scenario_bound.map_or_else(
                                || "Unknown".to_owned(),
                                |bound| format!("{bound:.3}")
                            ),
                            choice.scenario_relative_gap.map_or_else(
                                || if choice.scenario_proven { "proved".to_owned() } else { "gap unknown; unresolved".to_owned() },
                                |gap| format!("gap {:.1}%{}", gap * 100.0, if choice.scenario_proven { " · proved" } else { " · unresolved" })
                            ),
                            choice.condition
                        ));
                    }
                    egui::CollapsingHeader::new(format!(
                        "All scenario assignments ({})",
                        report.assignments.len()
                    ))
                    .show(ui, |ui| {
                        for assignment in &report.assignments {
                            let model = self
                                .table
                                .rows
                                .get(assignment.row_index)
                                .map(role_model_name)
                                .unwrap_or_else(|| format!("Row #{}", assignment.row_index));
                            ui.label(format!(
                                "{}{} · {} · {} / {} · up to {} attempts · assignment utility {:.3}",
                                assignment.seat.name(),
                                assignment
                                    .class
                                    .map(|class| format!(" / {}", class.name()))
                                    .unwrap_or_default(),
                                model,
                                assignment.provider_id,
                                assignment.plan_id,
                                assignment.attempt_limit,
                                assignment.utility
                            ))
                            .on_hover_text(&assignment.native_harness);
                        }
                    });
                });
        }
        ui.add_space(12.0);
        egui::CollapsingHeader::new("Budget, timing, and assumptions")
            .default_open(false)
            .show(ui, |ui| {
                if let Some(conductor) = &portfolio.conductor {
                    ui.label(format!(
                        "Orchestrator headroom reserved once: {} USD · {} h",
                        conductor
                            .headroom
                            .usd
                            .map_or_else(|| "unspecified".to_owned(), |value| format!("{value:.3}")),
                        conductor
                            .headroom
                            .hours
                            .map_or_else(|| "unspecified".to_owned(), |value| format!("{value:.3}"))
                    ));
                    ui.small(
                        "This optional fixed reserve is independent of worker-job counts. On-demand native calls use ledger holds; no call volume is forecast when usage is unknown.",
                    );
                }
                ui.label(&portfolio.math_audit.objective);
                ui.label(&portfolio.math_audit.coordination_proxy);
                egui::ScrollArea::horizontal()
                    .id_salt("team_pool_usage_scroll")
                    .show(ui, |ui| {
                        egui::Grid::new("team_pool_usage")
                            .striped(true)
                            .show(ui, |ui| {
                                for label in [
                                    "Account",
                                    "Allowance",
                                    "Expected",
                                    "Reserved",
                                    "Remaining",
                                    "Time used / available",
                                ] {
                                    ui.strong(label);
                                }
                                ui.end_row();
                                for pool in &portfolio.pools {
                                    ui.label(format!(
                                        "{} · {}",
                                        pool.provider_name, pool.plan_name
                                    ));
                                    ui.label(format!("{:.2} {}", pool.weekly_capacity, pool.unit));
                                    ui.label(format!("{:.2}", pool.nominal_usage));
                                    ui.label(format!("{:.2}", pool.reserved_usage));
                                    ui.label(format!("{:.2}", pool.remaining_reserved_capacity));
                                    ui.label(format!(
                                        "{:.1} / {:.1} h",
                                        pool.scheduled_hours, pool.available_hours
                                    ));
                                    ui.end_row();
                                }
                            });
                    });
                for pool in &portfolio.pools {
                    if pool.rate_infeasible {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            format!("{}: rate-infeasible at {} parallel orchestrators; no measured worker route fits.", pool.plan_name, portfolio.orchestrators),
                        );
                    }
                    for window in &pool.rate_windows {
                        let ceiling = window.ceiling_per_hour.map(|rate| format!("{rate:.4} {}/h", window.unit.label())).unwrap_or_else(|| "unknown / reference".to_owned());
                        ui.label(format!("{} · {}{} · {ceiling} · {}", pool.plan_name, window.window_id, if window.binding { " (tightest in scope)" } else { "" }, window.status));
                    }
                }
                for assumption in &portfolio.assumptions {
                    ui.label(assumption);
                }
            });
        let mut generate = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let executable = dispatch.executable && portfolio.conductor.is_some();
                if ui
                    .add_enabled(
                        executable && self.generator.install_rx.is_none(),
                        egui::Button::new("Generate Orchestrator")
                            .fill(crate::theme::BLUE)
                            .stroke(egui::Stroke::new(1.0, crate::theme::BLUE)),
                    )
                    .clicked()
                {
                    generate = true;
                }
            });
        });
        if generate {
            self.generator.prepare(
                self.table.clone(),
                self.settings.clone(),
                self.table_revision,
                ui.ctx().clone(),
            );
            self.section = Section::Setup;
        }
    }

    fn rankings_content(&mut self, ui: &mut egui::Ui) {
        Self::section_heading(
            ui,
            "Rankings",
            "Compare one role and budget tier at a time.",
        );
        let previous_filters = (self.selected_tab, self.ranking_seat, self.ranking_tier);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("ranking_provider")
                .selected_text(if self.selected_tab == 0 {
                    "All providers"
                } else {
                    VENDOR_TABS[self.selected_tab - 1].0
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.selected_tab, 0, "All providers");
                    for (index, (name, _)) in VENDOR_TABS.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_tab, index + 1, *name);
                    }
                });
            egui::ComboBox::from_id_salt("ranking_role")
                .selected_text(self.ranking_seat.name())
                .show_ui(ui, |ui| {
                    for seat in Seat::ALL {
                        ui.selectable_value(&mut self.ranking_seat, seat, seat.name());
                    }
                });
            egui::ComboBox::from_id_salt("ranking_tier")
                .selected_text(self.ranking_tier.name())
                .show_ui(ui, |ui| {
                    for tier in Tier::ALL {
                        ui.selectable_value(&mut self.ranking_tier, tier, tier.name());
                    }
                });
        });
        if previous_filters != (self.selected_tab, self.ranking_seat, self.ranking_tier) {
            self.side_panel_open = false;
            self.selected_seat_tier = None;
            self.invalidate_comparisons();
        }
        ui.add_space(12.0);
        let table = self.selected_table();
        if self.ranking_seat == Seat::NetResearch {
            let Some(research) = table
                .research_tiers
                .iter()
                .find(|research| research.tier == self.ranking_tier)
            else {
                ui.label("No qualified research recommendation for these filters.");
                return;
            };
            let mut clicked_research = None;
            egui::ScrollArea::horizontal()
                .id_salt("research_ranking_table_scroll")
                .show(ui, |ui| {
                    egui::Grid::new("research_ranking_table")
                        .striped(true)
                        .min_col_width(120.0)
                        .spacing([18.0, 12.0])
                        .show(ui, |ui| {
                            for label in [
                                "Rank",
                                "Model",
                                "Provider",
                                "Research score",
                                "Role evidence",
                                "Modeled cost",
                            ] {
                                ui.label(
                                    egui::RichText::new(label)
                                        .size(11.0)
                                        .color(crate::theme::MUTED),
                                );
                            }
                            ui.end_row();
                            for (rank, candidate) in research
                                .primary
                                .iter()
                                .chain(research.alternatives.iter())
                                .enumerate()
                            {
                                let Some(row) = table.rows.get(candidate.row_index) else {
                                    continue;
                                };
                                let rank_label = if rank == 0 {
                                    "1".to_owned()
                                } else {
                                    (rank + 1).to_string()
                                };
                                if ui.selectable_label(false, rank_label).clicked()
                                    || ui
                                        .selectable_label(
                                            false,
                                            research_route_name(row, &candidate.native_harness),
                                        )
                                        .clicked()
                                {
                                    clicked_research = Some((candidate.row_index, rank == 0));
                                }
                                ui.label(&candidate.provider_id);
                                ui.label(format!("{:.3}", candidate.score));
                                let component_count = 3 + usize::from(candidate.hle_weight > 0.0);
                                ui.label(format!("{component_count} model-level components"))
                                    .on_hover_text(format!(
                                        "Accuracy {:.3} × {:.2}; non-wrong {:.3} × {:.2}; LCR {:.3} × {:.2}; HLE {}\n{}",
                                        candidate.accuracy,
                                        candidate.accuracy_weight,
                                        candidate.non_wrong,
                                        candidate.non_wrong_weight,
                                        candidate.lcr,
                                        candidate.lcr_weight,
                                        if candidate.hle_weight > 0.0 {
                                            candidate.hle.map_or_else(
                                                || format!("unknown × {:.2}", candidate.hle_weight),
                                                |value| format!("{value:.3} × {:.2}", candidate.hle_weight)
                                            )
                                        } else {
                                            "diagnostic, not used for this class".to_owned()
                                        },
                                        candidate.source
                                    ));
                                ui.label(candidate.expected_usd.map_or_else(
                                    || "Unknown".to_owned(),
                                    |cost| format!("${cost:.2}"),
                                ));
                                ui.end_row();
                            }
                        });
                });
            if let Some((row_index, primary)) = clicked_research {
                self.selected_seat_tier = Some((self.ranking_seat, self.ranking_tier));
                self.side_panel_open = true;
                self.detail_tab = if primary {
                    DetailTab::Why
                } else {
                    DetailTab::Alternatives
                };
                if !primary {
                    self.comparison_choice
                        .insert((self.ranking_seat, self.ranking_tier), row_index);
                }
            }
            return;
        }
        let Some(pick) = table.get_pick(self.ranking_seat, self.ranking_tier) else {
            ui.label("No qualified recommendation for these filters.");
            return;
        };
        let mut clicked_candidate = None;
        egui::ScrollArea::horizontal()
            .id_salt("ranking_table_scroll")
            .show(ui, |ui| {
                egui::Grid::new("ranking_table")
                    .striped(true)
                    .min_col_width(110.0)
                    .spacing([18.0, 12.0])
                    .show(ui, |ui| {
                        for label in [
                            "Rank",
                            "Model",
                            "Provider",
                            "Competence",
                            "Role evidence",
                            "Ref. tasks/week",
                            "Cost/reference",
                        ] {
                            ui.label(
                                egui::RichText::new(label)
                                    .size(11.0)
                                    .color(crate::theme::MUTED),
                            );
                        }
                        ui.end_row();
                        for (rank, candidate) in pick.top.iter().enumerate() {
                            let Some(row) = table.rows.get(candidate.row_index) else {
                                continue;
                            };
                            let rank_label = if rank == 0 {
                                "1".to_owned()
                            } else {
                                (rank + 1).to_string()
                            };
                            if ui.selectable_label(false, rank_label).clicked()
                                || ui.selectable_label(false, role_model_name(row)).clicked()
                            {
                                clicked_candidate = Some((candidate.row_index, rank == 0));
                            }
                            ui.label(&row.vendor);
                            ui.label(competence_text(candidate.competence));
                            ui.label(role_evidence_compact(row, self.ranking_seat))
                                .on_hover_text(role_evidence_hover(row, self.ranking_seat));
                            ui.label(format!("{:.1}", candidate.tasks_per_week))
                                .on_hover_text(
                                    "Modeled reference-task throughput, not forecast job demand.",
                                );
                            ui.label(format!("${:.2}", candidate.cost_per_task));
                            ui.end_row();
                        }
                    });
            });
        if let Some((row_index, primary)) = clicked_candidate {
            self.selected_seat_tier = Some((self.ranking_seat, self.ranking_tier));
            self.side_panel_open = true;
            self.detail_tab = if primary {
                DetailTab::Why
            } else {
                DetailTab::Alternatives
            };
            if !primary {
                self.comparison_choice
                    .insert((self.ranking_seat, self.ranking_tier), row_index);
            }
        }
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        let width = if root.max_rect().width() < 1000.0 {
            160.0
        } else {
            210.0
        };
        egui::Panel::left("workspace_sidebar")
            .exact_size(width)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::SURFACE)
                    .stroke(egui::Stroke::new(1.0, crate::theme::BORDER))
                    .inner_margin(egui::Margin::symmetric(14, 18)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.image((self.brand.id(), egui::vec2(38.0, 38.0)));
                    ui.strong("AI Tier List");
                });
                ui.add_space(22.0);
                for (section, label) in [
                    (Section::Team, "Your team"),
                    (Section::Rankings, "Rankings"),
                    (Section::Setup, "Setup"),
                ] {
                    if ui
                        .add_sized(
                            [ui.available_width(), 40.0],
                            egui::Button::selectable(self.section == section, label),
                        )
                        .clicked()
                    {
                        self.section = section;
                        self.side_panel_open = false;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    for (section, label) in
                        [(Section::About, "About"), (Section::Settings, "Settings")]
                    {
                        if ui
                            .add_sized(
                                [ui.available_width(), 40.0],
                                egui::Button::selectable(self.section == section, label),
                            )
                            .clicked()
                        {
                            self.section = section;
                            self.side_panel_open = false;
                        }
                    }
                });
            });
    }

    fn section_heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.label(egui::RichText::new(title).size(28.0).strong());
        if !subtitle.is_empty() {
            ui.label(egui::RichText::new(subtitle).color(crate::theme::MUTED));
        }
        ui.add_space(14.0);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_channels();
        self.native_connections.pump();
        let ctx = ui.ctx().clone();
        let mut reveal_window = false;
        while let Some(event) = self
            .desktop
            .as_ref()
            .and_then(|desktop| desktop.next_event())
        {
            match event {
                crate::desktop::DesktopEvent::Open => {
                    self.window_visible = true;
                    self.hide_command_sent = false;
                    reveal_window = true;
                }
                crate::desktop::DesktopEvent::Subscriptions => {
                    self.window_visible = true;
                    self.hide_command_sent = false;
                    reveal_window = true;
                    self.section = Section::Settings;
                    self.settings_tab = 0;
                }
                crate::desktop::DesktopEvent::Quit => {
                    self.quit_requested = true;
                }
            }
        }
        if let Some(guard) = self.health_guard.take()
            && let Err(error) = guard.confirm_healthy()
        {
            self.update_status = format!("Update health confirmation failed: {error:#}");
            self.close_after_handoff = true;
        }
        if self.close_after_handoff || self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|input| input.viewport().close_requested())
            && self.desktop.is_some()
            && self.settings.close_to_tray
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_visible = false;
            self.hide_command_sent = true;
        }
        if !self.window_visible {
            if !self.hide_command_sent {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.hide_command_sent = true;
            }
            return;
        }
        reveal_window |= !self.scoring_started;
        if reveal_window {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if !self.scoring_started {
            self.scoring_started = self
                .settings_tx
                .send((self.settings.clone(), false))
                .is_ok();
        }
        self.sidebar(ui);
        if !self.refresh_in_flight
            && self.source_expired()
            && self
                .retry_after
                .is_none_or(|deadline| Instant::now() >= deadline)
        {
            self.start_refresh();
        }
        if let Some(deadline) = self.retry_after
            && deadline > Instant::now()
        {
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(clicked_at) = self.refresh_clicked_at {
            let deadline = clicked_at + Duration::from_secs(REFRESH_COOLDOWN_SECONDS);
            if deadline > Instant::now() {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        if !self.refresh_in_flight
            && let Some(delay) = self.source_refresh_delay()
        {
            ctx.request_repaint_after(delay);
        }

        egui::Panel::bottom("source_status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let state = if self.refresh_in_flight {
                    "refreshing"
                } else if self.source_expired() {
                    "stale"
                } else if !self.engine_status.is_empty() {
                    "scoring"
                } else {
                    match self.table.cache_state {
                        CacheState::Live => "live",
                        CacheState::Disk => "cached",
                        CacheState::Baked => "bundled",
                    }
                };
                ui.label(format!("Artificial Analysis · {state}"))
                    .on_hover_text(format!("Last pull: {}", self.table.source_fetched_at));
                let cooldown = self.refresh_clicked_at.map_or(0, |clicked_at| {
                    REFRESH_COOLDOWN_SECONDS.saturating_sub(clicked_at.elapsed().as_secs())
                });
                if ui
                    .add_enabled(
                        cooldown == 0 && !self.refresh_in_flight,
                        egui::Button::new(if cooldown == 0 {
                            "Refresh".to_owned()
                        } else {
                            format!("Refresh ({cooldown}s)")
                        }),
                    )
                    .clicked()
                {
                    self.start_refresh();
                }
                if !self.refresh_warning.is_empty() {
                    ui.colored_label(crate::theme::ERROR, "Refresh issue")
                        .on_hover_text(&self.refresh_warning);
                }
            });
        });

        let panel_open = self.section == Section::Rankings
            && self.side_panel_open
            && self.selected_seat_tier.is_some()
            && !(self.selected_tab == 0
                && self.settings.best_in_house_mode == BestInHouseMode::PerPlan);
        let t = ctx.animate_bool(egui::Id::new("side_panel_anim"), panel_open);
        if t > 0.001
            && let Some((seat, tier)) = self.selected_seat_tier
        {
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
                            let title_width = (ui.available_width() - 52.0).max(120.0);
                            ui.add_sized(
                                [title_width, 44.0],
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "{} - {}",
                                        seat.name(),
                                        tier.name()
                                    ))
                                    .heading(),
                                )
                                .wrap(),
                            );
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
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.detail_tab,
                                DetailTab::Why,
                                "Why this pick",
                            );
                            ui.selectable_value(
                                &mut self.detail_tab,
                                DetailTab::Alternatives,
                                "Alternatives",
                            );
                            ui.selectable_value(
                                &mut self.detail_tab,
                                DetailTab::Evidence,
                                "Evidence",
                            );
                        });
                        ui.separator();

                        egui::ScrollArea::vertical()
                            .id_salt(("detail_scroll", seat, tier))
                            .auto_shrink([false, false])
                            .max_height(ui.available_height())
                            .show(ui, |ui| match self.detail_tab {
                                DetailTab::Why => {
                                    self.why_content(ui, seat, tier);
                                }
                                DetailTab::Alternatives if seat != Seat::NetResearch => {
                                    self.counterfactual_content(ui, seat, tier);
                                }
                                DetailTab::Alternatives => {
                                    self.research_alternatives_content(ui, tier);
                                }
                                DetailTab::Evidence => {
                                    let table = self.selected_table();
                                    let row = if seat == Seat::NetResearch {
                                        table
                                            .research_tiers
                                            .iter()
                                            .find(|pick| pick.tier == tier)
                                            .and_then(|pick| pick.primary.as_ref())
                                            .and_then(|candidate| {
                                                table.rows.get(candidate.row_index)
                                            })
                                    } else {
                                        table
                                            .get_pick(seat, tier)
                                            .and_then(|pick| table.rows.get(pick.row_index))
                                    };
                                    evidence_view::show(ui, table, row);
                                }
                            });
                    }
                });
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::CANVAS)
                    .inner_margin(24),
            )
            .show(ui, |ui| {
                if self.section == Section::Settings {
                    egui::ScrollArea::vertical()
                        .id_salt("settings_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width().min(1400.0));
                            Self::section_heading(ui, "Settings", "");
                            if self.settings_content(ui) {
                                self.apply_settings_change();
                            }
                        });
                    return;
                }
                if self.section == Section::About {
                    egui::ScrollArea::vertical()
                        .id_salt("about_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width().min(1400.0));
                            self.about_content(ui, &ctx);
                        });
                    return;
                }
                if self.section == Section::Setup {
                    Self::section_heading(ui, "Setup", "");
                    let score_current = self
                        .scored_settings
                        .as_ref()
                        .is_some_and(|scored| scored.scoring_matches(&self.settings));
                    if !self.generator.open {
                        let executable = self.table.portfolio.as_ref().is_some_and(|portfolio| {
                            portfolio.dispatch.executable && portfolio.conductor.is_some()
                        });
                        if self.settings.subscriptions.is_empty() {
                            ui.label("Choose subscriptions to build your team.");
                            if ui.button("Choose subscriptions").clicked() {
                                self.edit_plan = true;
                                self.section = Section::Team;
                            }
                        } else if !score_current || !executable {
                            ui.label(if score_current {
                                "No executable team is available for this plan."
                            } else {
                                "Your team is still scoring or needs a refresh."
                            });
                            if ui.button("Back to your team").clicked() {
                                self.section = Section::Team;
                            }
                        } else if ui.button("Prepare setup").clicked() {
                            self.generator.prepare(
                                self.table.clone(),
                                self.settings.clone(),
                                self.table_revision,
                                ui.ctx().clone(),
                            );
                        }
                    } else if self.generator.show(
                        ui,
                        &self.table,
                        &self.settings,
                        self.table_revision,
                        score_current,
                    ) {
                        self.section = Section::Team;
                    }
                    return;
                }
                if self.section == Section::Team {
                    self.settings.best_in_house_mode = BestInHouseMode::PerPlan;
                    egui::ScrollArea::vertical()
                        .id_salt("team_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width().min(1400.0));
                            self.team_content(ui);
                        });
                } else {
                    if self.settings.best_in_house_mode == BestInHouseMode::PerPlan {
                        self.settings.best_in_house_mode = BestInHouseMode::Absolute;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("rankings_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width().min(1400.0));
                            self.rankings_content(ui);
                        });
                }
            });
    }
}
