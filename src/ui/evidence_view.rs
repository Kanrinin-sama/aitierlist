use crate::evidence::{
    BenchmarkObservation, EvidenceCatalog, EvidenceProjection, ScoreObservation, TransferKind,
};
use crate::theme;
use crate::types::{Row, Table};
use eframe::egui;
use std::collections::BTreeSet;

#[derive(Clone, Default)]
struct EvidenceViewState {
    search: String,
    source: String,
    family: String,
    page: usize,
    catalog_key: String,
    cache_key: String,
    sources: BTreeSet<String>,
    families: BTreeSet<String>,
    visible: Vec<usize>,
    latest: BTreeSet<String>,
}

pub fn show(ui: &mut egui::Ui, table: &Table, row: Option<&Row>) {
    let state_id = egui::Id::new("evidence_view_state");
    let mut state = ui.ctx().data_mut(|data| {
        data.get_temp::<EvidenceViewState>(state_id)
            .unwrap_or_default()
    });
    ui.label(
        egui::RichText::new("Benchmark evidence")
            .size(28.0)
            .strong()
            .color(theme::TEXT),
    );
    ui.colored_label(
        theme::MUTED,
        "Published model and agent observations, including evidence outside recommendation scores.",
    );
    ui.add_space(12.0);
    if let Some(row) = row {
        row_details(ui, row, &table.evidence_catalog);
        ui.add_space(12.0);
    }
    source_status(ui, &table.evidence_catalog);
    ui.add_space(12.0);

    if state.catalog_key != table.generated_at {
        state.catalog_key.clone_from(&table.generated_at);
        state.sources = table
            .evidence_catalog
            .observations
            .iter()
            .map(|observation| observation.source.source_id.clone())
            .collect();
        state.families = table
            .evidence_catalog
            .observations
            .iter()
            .map(|observation| observation.series.family.clone())
            .collect();
        state.latest = latest_ids(&table.evidence_catalog.observations);
        state.cache_key.clear();
    }
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text("Search model, benchmark, harness, source"),
        );
        filter(
            ui,
            "evidence_source",
            "All sources",
            &state.sources,
            &mut state.source,
        );
        filter(
            ui,
            "evidence_family",
            "All benchmarks",
            &state.families,
            &mut state.family,
        );
    });

    let selected: BTreeSet<_> = table
        .rows
        .iter()
        .flat_map(|row| row.benchmark_evidence.iter())
        .map(|projection| projection.observation_id.as_str())
        .collect();
    let search = state.search.trim().to_ascii_lowercase();
    let cache_key = format!(
        "{}\0{}\0{}\0{}",
        table.generated_at, state.search, state.source, state.family
    );
    if state.cache_key != cache_key {
        state.cache_key = cache_key;
        state.page = 0;
        state.visible = table
            .evidence_catalog
            .observations
            .iter()
            .enumerate()
            .filter_map(|(index, observation)| {
                ((state.source.is_empty() || observation.source.source_id == state.source)
                    && (state.family.is_empty() || observation.series.family == state.family)
                    && (search.is_empty() || searchable(observation).contains(&search)))
                .then_some(index)
            })
            .collect();
    }
    ui.add_space(8.0);
    let pages = state.visible.len().div_ceil(50).max(1);
    state.page = state.page.min(pages - 1);
    ui.horizontal(|ui| {
        ui.colored_label(
            theme::MUTED,
            format!(
                "{} observations · page {} of {pages}",
                state.visible.len(),
                state.page + 1
            ),
        );
        if ui
            .add_enabled(state.page > 0, egui::Button::new("Previous"))
            .clicked()
        {
            state.page -= 1;
        }
        if ui
            .add_enabled(state.page + 1 < pages, egui::Button::new("Next"))
            .clicked()
        {
            state.page += 1;
        }
    });
    let start = state.page * 50;
    let end = (start + 50).min(state.visible.len());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for index in &state.visible[start..end] {
                let observation = &table.evidence_catalog.observations[*index];
                let selected_here = selected.contains(observation.observation_id.as_str());
                let latest = state.latest.contains(&observation.observation_id);
                ui.push_id(&observation.observation_id, |ui| {
                    observation_card(ui, observation, selected_here, latest);
                });
                ui.add_space(8.0);
            }
        });
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
}

pub fn row_details(ui: &mut egui::Ui, row: &Row, catalog: &EvidenceCatalog) {
    if row.benchmark_evidence.is_empty() && row.benchmark_observation_ids.is_empty() {
        ui.colored_label(theme::MUTED, "No benchmark evidence is linked to this row.");
        return;
    }
    for projection in &row.benchmark_evidence {
        projection_card(ui, projection);
    }
    let selected: BTreeSet<_> = row
        .benchmark_evidence
        .iter()
        .map(|projection| projection.observation_id.as_str())
        .collect();
    let historical: Vec<_> = row
        .benchmark_observation_ids
        .iter()
        .filter(|id| !selected.contains(id.as_str()))
        .filter_map(|id| {
            catalog
                .observations
                .iter()
                .find(|item| item.observation_id == *id)
        })
        .collect();
    if !historical.is_empty() {
        egui::CollapsingHeader::new(format!("Other observations ({})", historical.len())).show(
            ui,
            |ui| {
                for observation in historical {
                    ui.push_id(&observation.observation_id, |ui| {
                        observation_card(ui, observation, false, false);
                    });
                }
            },
        );
    }
}

fn source_status(ui: &mut egui::Ui, catalog: &EvidenceCatalog) {
    egui::CollapsingHeader::new(format!("Sources ({})", catalog.sources.len()))
        .default_open(true)
        .show(ui, |ui| {
            for status in &catalog.sources {
                ui.horizontal_wrapped(|ui| {
                    let cached =
                        status.fetch_error.is_some() && status.last_good_revision.is_some();
                    let state = if cached {
                        "Cached"
                    } else if status.fetch_error.is_some() {
                        "Unavailable"
                    } else {
                        "Available"
                    };
                    ui.label(egui::RichText::new(state).strong().color(
                        if status.fetch_error.is_some() && !cached {
                            egui::Color32::from_rgb(248, 113, 113)
                        } else {
                            egui::Color32::from_rgb(74, 222, 128)
                        },
                    ));
                    ui.hyperlink_to(&status.source.source_id, &status.source.url);
                    ui.colored_label(
                        theme::MUTED,
                        format!(
                            "revision {} · fetched {}",
                            status.source.revision, status.source.fetched_at
                        ),
                    );
                });
                if let Some(error) = &status.fetch_error {
                    ui.colored_label(theme::MUTED, error);
                }
                if let Some(revision) = &status.last_good_revision {
                    ui.colored_label(
                        theme::MUTED,
                        format!(
                            "Cached revision {revision} · {}",
                            status
                                .last_good_fetched_at
                                .as_deref()
                                .unwrap_or("fetch time unknown")
                        ),
                    );
                }
                if let Some(revision) = &status.last_good_revision {
                    ui.colored_label(
                        theme::MUTED,
                        format!(
                            "Cached revision {revision} · {}",
                            status
                                .last_good_fetched_at
                                .as_deref()
                                .unwrap_or("fetch time unknown")
                        ),
                    );
                }
            }
        });
}

fn filter(
    ui: &mut egui::Ui,
    id: &str,
    all_label: &str,
    values: &BTreeSet<String>,
    selected: &mut String,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(if selected.is_empty() {
            all_label
        } else {
            selected.as_str()
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, String::new(), all_label);
            for value in values {
                ui.selectable_value(selected, value.clone(), value);
            }
        });
}

fn observation_card(
    ui: &mut egui::Ui,
    observation: &BenchmarkObservation,
    selected: bool,
    latest: bool,
) {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(&observation.subject.model)
                        .strong()
                        .color(theme::TEXT),
                );
                if selected {
                    ui.colored_label(theme::CYAN, "Selected");
                } else if latest {
                    ui.colored_label(
                        egui::Color32::from_rgb(74, 222, 128),
                        "Latest compatible variant",
                    );
                } else {
                    ui.colored_label(theme::MUTED, "Other observation");
                }
            });
            ui.colored_label(
                theme::MUTED,
                format!(
                    "{} · {} · {}",
                    observation.subject.provider,
                    observation
                        .subject
                        .effort
                        .as_deref()
                        .unwrap_or("effort unspecified"),
                    observation.execution.harness
                ),
            );
            ui.label(format!(
                "{} / {} · version {} · {}",
                observation.series.family,
                observation.series.variant,
                observation.series.version,
                observation.series.task_count.map_or_else(
                    || "task count unknown".to_owned(),
                    |count| format!("{count} tasks")
                )
            ));
            ui.label(format!(
                "Protocol: {} · Grader: {}",
                observation.execution.protocol, observation.execution.grader
            ));
            if let Some(score) = &observation.score {
                ui.label(format!("{}: {}", score.metric, score_text(score)));
            } else {
                ui.colored_label(
                    theme::MUTED,
                    "Diagnostic record; no authoritative primary score.",
                );
            }
            resource_line(ui, observation.resources.as_ref());
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to("Source", &observation.source.url);
                ui.colored_label(theme::MUTED, dates(observation));
            });
            if let Some(path) = observation
                .extra
                .get("rawCachePath")
                .or_else(|| observation.extra.get("cachePath"))
                .and_then(|value| value.as_str())
            {
                ui.colored_label(theme::MUTED, format!("Raw cache: {path}"));
            }
            if !observation.extra.is_empty() {
                egui::CollapsingHeader::new("Published fields").show(ui, |ui| {
                    let formatted = serde_json::to_string_pretty(&observation.extra)
                        .unwrap_or_else(|_| "Published fields unavailable".to_owned());
                    ui.add(egui::Label::new(egui::RichText::new(formatted).monospace()).wrap());
                });
            }
        });
}

fn projection_card(ui: &mut egui::Ui, projection: &EvidenceProjection) {
    ui.group(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&projection.family).strong());
            ui.colored_label(
                theme::CYAN,
                match projection.transfer {
                    TransferKind::ExactHarness => "Exact benchmark harness",
                    TransferKind::ModelLevel => "Model-level evidence",
                    TransferKind::CrossHarnessProxy => "Cross-harness proxy",
                },
            );
        });
        ui.label(format!(
            "{} · {}",
            projection.series.variant,
            score_text(&projection.score)
        ));
        ui.label(format!(
            "Benchmark harness: {} · protocol {}",
            projection.execution.harness, projection.execution.protocol
        ));
        ui.colored_label(theme::MUTED, &projection.selection_reason);
        resource_line(ui, projection.resources.as_ref());
        if let Some(gap) = &projection.resource_gap {
            ui.colored_label(theme::MUTED, gap);
        }
        ui.hyperlink_to("Source", &projection.source.url);
    });
}

fn resource_line(ui: &mut egui::Ui, resource: Option<&crate::evidence::ResourceObservation>) {
    let Some(resource) = resource else {
        ui.colored_label(
            theme::MUTED,
            "Matching cost and runtime resources are not published.",
        );
        return;
    };
    ui.label(format!(
        "Resources: {} · {} · {}; {}",
        resource
            .usd
            .map_or_else(|| "USD unknown".to_owned(), |value| format!("${value:.4}")),
        resource.seconds.map_or_else(
            || "runtime unknown".to_owned(),
            |value| format!("{value:.1} s")
        ),
        resource.time_basis,
        resource.cost_basis
    ));
}

fn score_text(score: &ScoreObservation) -> String {
    if score.scale_min == Some(0.0) && score.scale_max == Some(1.0) {
        format!("{:.4} ({:.2}%)", score.value, score.value * 100.0)
    } else if score.scale_min == Some(0.0) && score.scale_max == Some(100.0) {
        format!("{:.2}%", score.value)
    } else if score.metric.to_ascii_lowercase().contains("elo") {
        format!("{:.1} Elo", score.value)
    } else {
        format!("{:.4} (published units)", score.value)
    }
}

fn searchable(observation: &BenchmarkObservation) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        observation.subject.model,
        observation.subject.provider,
        observation.subject.effort.as_deref().unwrap_or(""),
        observation.series.family,
        observation.series.variant,
        observation.execution.harness,
        observation.source.source_id
    )
    .to_ascii_lowercase()
}

fn latest_ids(all: &[BenchmarkObservation]) -> BTreeSet<String> {
    let mut latest = std::collections::BTreeMap::new();
    for observation in all {
        let key = format!(
            "{}\0{}\0{}\0{}\0{}",
            crate::evidence::canonical_provider(&observation.subject.provider),
            observation.subject.normalized_model(),
            observation.subject.normalized_effort().unwrap_or_default(),
            observation.series.family,
            observation.series.comparable_series_id
        );
        let replace = latest
            .get(&key)
            .is_none_or(|current: &&BenchmarkObservation| {
                crate::evidence::compare_observations(current, observation).is_lt()
            });
        if replace {
            latest.insert(key, observation);
        }
    }
    latest
        .into_values()
        .map(|observation| observation.observation_id.clone())
        .collect()
}

fn dates(observation: &BenchmarkObservation) -> String {
    format!(
        "observed {} · published {} · fetched {}",
        observation
            .source
            .observed_at
            .as_deref()
            .unwrap_or("unknown"),
        observation
            .source
            .published_at
            .as_deref()
            .unwrap_or("unknown"),
        observation.source.fetched_at
    )
}
