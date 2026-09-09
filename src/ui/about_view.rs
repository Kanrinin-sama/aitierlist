use crate::theme;
use eframe::egui;

impl super::App {
    pub(super) fn about_content(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.label(
            egui::RichText::new("About")
                .size(28.0)
                .strong()
                .color(theme::TEXT),
        );
        ui.add_space(14.0);

        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.image((self.brand.id(), egui::vec2(58.0, 58.0)));
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("AI Tier List")
                                .size(20.0)
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.colored_label(
                            theme::MUTED,
                            format!("Version {}", env!("CARGO_PKG_VERSION")),
                        );
                    });
                });
            });

        ui.add_space(16.0);
        egui::CollapsingHeader::new(
            egui::RichText::new("How recommendations work")
                .size(20.0)
                .color(theme::TEXT),
        )
        .default_open(true)
        .show(ui, |ui| {
            methodology_section(
                ui,
                "Qualification",
                "Role competence is a fixed benchmark-weighted utility used to qualify and compare candidates. It is not a task-success probability and never becomes a retry probability.",
            );
            methodology_section(
                ui,
                "Reference throughput",
                "Capacity uses reference-workload outcomes under declared runtime, cost, retry, allowance, and rescue scenarios. The scenario distance is a modeling choice, not measured uncertainty, a confidence interval, or empirical real-world output.",
            );
            methodology_section(
                ui,
                "Team forecast",
                "Selected subscriptions contribute shared modeled API-equivalent allowances and account working-time lanes. Forecast jobs are declared planning demand; they are not provider-published native quotas, an authorized backlog, or executable admission.",
            );
            methodology_section(
                ui,
                "Native admission",
                "The native scheduler begins from actual authorized ready task IDs and admits them against verified bindings, account windows, dependencies, holds, deadlines, and measured native units.",
            );
        });

        ui.add_space(12.0);
        ui.hyperlink_to(
            "Artificial Analysis methodology",
            super::AA_INTELLIGENCE_METHODOLOGY_URL,
        );

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Updates").size(20.0).color(theme::TEXT));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Check for updates").clicked() {
                self.handoff_status.clear();
                self.trigger_update_check(context.clone());
            }
            if self.staged_update.is_some() || self.available_update.is_some() {
                let label = if self.staged_update.is_some() {
                    "Retry install"
                } else {
                    "Install update"
                };
                if ui
                    .add_enabled(
                        self.install_rx.is_none()
                            && !self.refresh_in_flight
                            && self.comparison_pending.is_none(),
                        egui::Button::new(label),
                    )
                    .clicked()
                {
                    self.start_install(context.clone());
                }
            }
        });
        if !self.handoff_status.is_empty() {
            ui.label(&self.handoff_status);
        }
        if !self.update_status.is_empty() {
            ui.label(&self.update_status);
        }
    }
}

fn methodology_section(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(title).strong().color(theme::CYAN));
    ui.add(egui::Label::new(egui::RichText::new(body).color(theme::MUTED)).wrap());
}
