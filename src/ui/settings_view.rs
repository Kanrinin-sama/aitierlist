use crate::theme;
use crate::types::Seat;
use eframe::egui;
use std::sync::mpsc::channel;

const SUBSCRIPTIONS_TAB: usize = 0;
const WORKSPACE_TAB: usize = 1;
const MODELING_TAB: usize = 2;

impl super::App {
    pub(super) fn settings_content(&mut self, ui: &mut egui::Ui) -> bool {
        let tab_id = ui.id().with("settings_active_tab");
        let mut active_tab = ui
            .ctx()
            .data_mut(|data| data.get_temp::<usize>(tab_id))
            .unwrap_or(SUBSCRIPTIONS_TAB);
        ui.horizontal(|ui| {
            for (index, label) in ["Subscriptions", "Workspace", "Modeling & data"]
                .into_iter()
                .enumerate()
            {
                if ui.selectable_label(active_tab == index, label).clicked() {
                    active_tab = index;
                }
            }
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(tab_id, active_tab));
        ui.add_space(16.0);

        match active_tab {
            WORKSPACE_TAB => self.workspace_controls(ui),
            MODELING_TAB => self.modeling_controls(ui),
            _ => self.subscription_controls(ui),
        }
    }

    pub(super) fn subscription_controls(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(
            egui::RichText::new("Your subscriptions")
                .size(20.0)
                .color(theme::TEXT),
        );
        ui.colored_label(
            theme::MUTED,
            "Choose each plan and the number of accounts sharing work across your team.",
        );
        ui.add_space(12.0);

        let mut changed = false;
        let mut monthly_spend = 0.0;
        let mut estimated_price = false;
        for provider in crate::subscriptions::PROVIDERS {
            let mut selected = self
                .settings
                .subscriptions
                .get(provider.id)
                .cloned()
                .unwrap_or_default();
            let previous = selected.clone();
            let label = provider
                .plans
                .iter()
                .find(|plan| plan.id == selected)
                .map(plan_label)
                .unwrap_or_else(|| "No plan".to_owned());
            let saved_count = self.settings.subscription_count(provider.id);
            let count_text = self
                .subscription_count_text
                .entry(provider.id.to_owned())
                .or_insert_with(|| saved_count.to_string());
            let mut count_changed = false;

            egui::Frame::new()
                .fill(theme::SURFACE)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(10)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(provider.name)
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            count_changed = ui
                                .add(
                                    egui::TextEdit::singleline(count_text)
                                        .desired_width(44.0)
                                        .horizontal_align(egui::Align::Center),
                                )
                                .changed();
                            ui.colored_label(theme::MUTED, "accounts");
                            egui::ComboBox::from_id_salt(("subscription", provider.id))
                                .width(210.0)
                                .selected_text(label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut selected, String::new(), "No plan");
                                    for plan in provider.plans {
                                        ui.selectable_value(
                                            &mut selected,
                                            plan.id.to_owned(),
                                            plan_label(plan),
                                        );
                                    }
                                });
                        });
                    });
                });
            ui.add_space(8.0);

            let parsed_count = count_text.parse::<usize>().ok().filter(|count| *count > 0);
            if count_changed && let Some(count) = parsed_count {
                self.settings
                    .subscription_counts
                    .insert(provider.id.to_owned(), count);
                changed = true;
            }
            if parsed_count.is_none() {
                ui.colored_label(
                    theme::ERROR,
                    "Account count must be a positive whole number.",
                );
            }
            if selected != previous {
                if selected.is_empty() {
                    self.settings.subscriptions.remove(provider.id);
                } else {
                    self.settings
                        .subscriptions
                        .insert(provider.id.to_owned(), selected.clone());
                }
                changed = true;
            }
            if let Some(plan) = provider.plans.iter().find(|plan| plan.id == selected) {
                monthly_spend +=
                    plan.monthly_price * self.settings.subscription_count(provider.id) as f64;
                estimated_price |= plan.price_is_estimate;
            }
        }

        egui::Frame::new()
            .fill(theme::RAISED)
            .corner_radius(10)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Monthly total").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let total = if estimated_price {
                            format!("est. ${monthly_spend:.2}")
                        } else {
                            format!("${monthly_spend:.2}")
                        };
                        ui.label(egui::RichText::new(total).strong().color(theme::CYAN));
                    });
                });
            });
        changed
    }

    fn workspace_controls(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(
            egui::RichText::new("Collaboration workspace")
                .size(20.0)
                .color(theme::TEXT),
        );
        ui.colored_label(
            theme::MUTED,
            "Projects, briefs, research, decisions, and deliverables live in this folder.",
        );
        ui.add_space(12.0);

        let mut changed = false;
        ui.label(egui::RichText::new("Folder").strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.collaboration_root_text)
                    .desired_width(ui.available_width() - 100.0),
            );
            if ui
                .add_enabled(
                    self.workspace_picker.is_none(),
                    egui::Button::new("Browse…"),
                )
                .clicked()
            {
                let (sender, receiver) = channel();
                self.workspace_picker = Some(receiver);
                let context = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = crate::workspace::pick_root()
                        .map_err(|error| format!("Browse failed: {error:#}"));
                    let _ = sender.send(result);
                    context.request_repaint();
                });
            }
        });
        ui.colored_label(
            theme::MUTED,
            "Profiles, authentication, ledgers, and generated runtimes remain in app data.",
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Use default").clicked() {
                match crate::workspace::default_root() {
                    Ok(path) => {
                        self.collaboration_root_text = path.display().to_string();
                        self.settings.collaboration_root = None;
                        self.workspace_status.clear();
                        changed = true;
                    }
                    Err(error) => {
                        self.workspace_status = format!("Default unavailable: {error:#}");
                    }
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
                            self.settings.collaboration_root = (default.as_ref()
                                != Some(&paths.root))
                            .then_some(paths.root.clone());
                            self.collaboration_root_text = paths.root.display().to_string();
                            self.workspace_status = format!("Prepared {}", paths.root.display());
                            changed = true;
                        }
                        Err(error) => {
                            self.workspace_status = format!("Workspace setup failed: {error:#}");
                        }
                    }
                }
            }
        });
        if !self.workspace_status.is_empty() {
            let color = if self.workspace_status.starts_with("Prepared") {
                theme::SUCCESS
            } else {
                theme::ERROR
            };
            ui.colored_label(color, &self.workspace_status);
        }
        changed
    }

    fn modeling_controls(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label(
            egui::RichText::new("Modeling & data")
                .size(20.0)
                .color(theme::TEXT),
        );
        ui.colored_label(
            theme::MUTED,
            "Tune rescue behavior and the assumptions behind rankings.",
        );
        ui.add_space(12.0);

        let mut changed = false;
        egui::Frame::new()
            .fill(theme::SURFACE)
            .corner_radius(10)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Rescue defaults").strong());
                ui.add_space(8.0);
                egui::Grid::new("rescue_defaults")
                    .num_columns(2)
                    .spacing([24.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Time per unfinished cycle").on_hover_text(
                            "Additional rescue time for an unfinished cycle. Assisted completions are not credited to the agent.",
                        );
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.escalation_minutes)
                                    .range(0.0..=10080.0)
                                    .suffix(" min"),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("External cost per unfinished cycle").on_hover_text(
                            "Additional external rescue cost applied to every role.",
                        );
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.escalation_usd)
                                    .range(0.0..=1000000.0)
                                    .prefix("$"),
                            )
                            .changed();
                        ui.end_row();
                    });
            });
        ui.add_space(12.0);

        egui::CollapsingHeader::new("Scoring assumptions")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Retry correlation").on_hover_text(
                        "Leave blank to derive it by row and suite. Use 0–0.999 to override every suite.",
                    );
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.rho_override_text)
                                .hint_text("derived")
                                .desired_width(80.0),
                        )
                        .changed()
                    {
                        if self.rho_override_text.trim().is_empty() {
                            self.settings.rho_override = None;
                            changed = true;
                        } else if let Ok(value) = self.rho_override_text.trim().parse::<f64>()
                            && value.is_finite()
                            && (0.0..=0.999).contains(&value)
                        {
                            self.settings.rho_override = Some(value);
                            changed = true;
                        }
                    }
                    if !self.rho_override_text.trim().is_empty()
                        && !self
                            .rho_override_text
                            .trim()
                            .parse::<f64>()
                            .is_ok_and(|value| value.is_finite() && (0.0..=0.999).contains(&value))
                    {
                        ui.colored_label(theme::ERROR, "Use 0–0.999");
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Scenario distance").on_hover_text(
                        "Assumed distance for joint low and high stress scenarios; it is not measured error.",
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
                changed |= ui
                    .checkbox(
                        &mut self.settings.show_hallucination,
                        "Show hallucination reliability",
                    )
                    .changed();
            });

        egui::CollapsingHeader::new("Competence minimums")
            .default_open(false)
            .show(ui, |ui| {
                ui.colored_label(
                    theme::MUTED,
                    "Enable a role to require a minimum qualification score.",
                );
                for seat in Seat::ALL {
                    let floor = self
                        .settings
                        .competence_floors
                        .entry(seat.name().to_owned())
                        .or_insert(None);
                    ui.horizontal(|ui| {
                        let mut enabled = floor.is_some();
                        let response = ui.checkbox(&mut enabled, seat.name());
                        if seat == Seat::Orchestrator {
                            response.clone().on_hover_text("Per-plan Orchestrator competence uses the AA Intelligence Index. Absolute rankings blend GPQA and the Intelligence Index equally.");
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
                            ui.colored_label(theme::MUTED, "Automatic");
                        }
                    });
                }
            });

        egui::CollapsingHeader::new("Team workflow")
            .default_open(false)
            .show(ui, |ui| {
                ui.colored_label(
                    theme::MUTED,
                    "The base forecast covers coding work. Enable a class to add one declared AA research-reference visit to every forecast job in that class.",
                );
                for class in crate::portfolio::WorkClass::ALL {
                    changed |= ui
                        .checkbox(
                            self.settings
                                .research_classes
                                .entry(class.name().to_owned())
                                .or_insert(false),
                            format!("{} includes research", class.name()),
                        )
                        .changed();
                }
                changed |= ui
                    .checkbox(
                        &mut self.settings.monotonic_class_competence,
                        "Require first-attempt competence to rise with funded work class",
                    )
                    .on_hover_text("Applies separately to each ordinary role. It compares measured first-attempt role competence, not retry-adjusted utility or research scores.")
                    .changed();
            });

        egui::CollapsingHeader::new("Tier prices")
            .default_open(false)
            .show(ui, |ui| {
                ui.colored_label(theme::MUTED, "Monthly USD used for ranking budget tiers.");
                egui::Grid::new("tier_prices").show(ui, |ui| {
                    for (label, value) in [
                        ("$200 tier", &mut self.settings.plan_prices.t200),
                        ("$100 tier", &mut self.settings.plan_prices.t100),
                        ("$20 tier", &mut self.settings.plan_prices.t20),
                    ] {
                        ui.label(label);
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix("$").range(1.0..=1000.0))
                            .changed();
                        ui.end_row();
                    }
                });
            });

        egui::CollapsingHeader::new("Data cache")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Cache validity");
                    changed |= ui
                        .add(egui::Slider::new(&mut self.settings.cache_hours, 1..=48).suffix(" h"))
                        .changed();
                });
            });

        egui::CollapsingHeader::new("Vendor allowance overrides")
            .default_open(false)
            .show(ui, |ui| {
                ui.colored_label(
                    theme::MUTED,
                    "Weekly USD. Leave blank to use the plan default.",
                );
                egui::Grid::new("vendor_overrides")
                    .num_columns(4)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        for (vendor, value) in &mut self.settings.vendor_overrides {
                            ui.label(vendor.as_str());
                            if crate::engine::plan_for(vendor, f64::INFINITY).is_none() {
                                ui.colored_label(theme::MUTED, "API only");
                            } else {
                                ui.label("");
                            }
                            let editor_id = ui.id().with(("vendor_override", vendor.as_str()));
                            let focused = ui.memory(|memory| memory.has_focus(editor_id));
                            let saved_text =
                                value.map(|amount| amount.to_string()).unwrap_or_default();
                            let mut text = if focused {
                                ui.ctx()
                                    .data_mut(|data| data.get_temp::<String>(editor_id))
                                    .unwrap_or(saved_text)
                            } else {
                                saved_text
                            };
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .id(editor_id)
                                    .hint_text("default")
                                    .desired_width(96.0),
                            );
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(editor_id, text.clone());
                            });
                            let trimmed = text.trim();
                            let parsed = trimmed
                                .parse::<f64>()
                                .ok()
                                .filter(|amount| amount.is_finite() && *amount >= 0.0);
                            let invalid = !trimmed.is_empty() && parsed.is_none();
                            let commit = response.lost_focus()
                                || response.has_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                            if commit && !invalid {
                                let next = if trimmed.is_empty() { None } else { parsed };
                                if *value != next {
                                    *value = next;
                                    changed = true;
                                }
                            }
                            if invalid {
                                ui.colored_label(theme::ERROR, "Use zero or more");
                            } else {
                                ui.label("");
                            }
                            ui.end_row();
                        }
                    });
            });
        changed
    }
}

fn plan_label(plan: &crate::subscriptions::Plan) -> String {
    let price = if plan.price_is_estimate {
        format!("est. ${:.2}", plan.monthly_price)
    } else {
        format!("${:.2}", plan.monthly_price)
    };
    let name = match plan.id {
        "codex-pro-100" | "codex-pro-200" => "Pro",
        _ => plan.name,
    };
    format!("{name} · {price}/month")
}
