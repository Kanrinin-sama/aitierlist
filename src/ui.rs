use crate::settings::{Settings, load_settings, save_settings};
use crate::types::{CacheState, Seat, Table, Tier};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const REFRESH_COOLDOWN_SECONDS: u64 = 16;

pub struct App {
    table: Table,
    panel_widths: std::collections::HashMap<(Seat, Tier), f32>,
    agent_hours_text: String,
    agent_hours_invalid: bool,
    refresh_in_flight: bool,
    retry_after: Option<Instant>,
    refresh_warning: String,
    table_rx: Receiver<(Table, bool)>,
    settings_tx: Sender<(Settings, bool)>,
    engine_error_rx: Receiver<String>,
    engine_status: String,
    refresh_clicked_at: Option<Instant>,
    settings: Settings,
    settings_open: bool,
    selected_seat_tier: Option<(Seat, Tier)>,
    side_panel_open: bool,
    update_status: String,
    update_rx: Option<Receiver<Result<crate::update::Checked, String>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (table_tx, table_rx) = channel();
        let settings = load_settings();
        let table = Table::empty();
        let (settings_tx, settings_rx) = channel::<(Settings, bool)>();
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
                    if table_tx.send((table, fetched_rows)).is_err() {
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
            settings_tx,
            engine_error_rx,
            engine_status: "Loading rows…".to_string(),
            refresh_clicked_at: None,
            settings,
            settings_open: false,
            selected_seat_tier: None,
            side_panel_open: false,
            update_status: "Checking for updates…".to_string(),
            update_rx: None,
        };

        app.trigger_update_check();
        app
    }

    fn detail_content(&self, ui: &mut egui::Ui, seat: Seat, tier: Tier) {
        if let Some(pick) = self.table.get_pick(seat, tier) {
            let selected_row = self.table.rows.get(pick.row_index);

            ui.heading("Top Pick");
            if let Some(r) = selected_row {
                ui.label(egui::RichText::new(r.display_name()).strong().size(15.0));
                ui.label(format!("Vendor: {} | Harness: {}", r.vendor, r.harness));
            }
            ui.add_space(4.0);
            ui.label(format!(
                "Win Rate: {:.1}% | Family Win: {:.1}% (N={})",
                pick.win_rate * 100.0,
                pick.family_win_rate * 100.0,
                pick.n
            ));
            ui.label(format!(
                "Speed: {:.2} min/task | Cost: ${:.2}/task",
                pick.minutes_per_task, pick.cost_per_task
            ));
            ui.label(format!("Throughput: {:.1} tasks/wk", pick.tasks_per_week));
            if let Some(s) = pick.streams_star {
                ui.label(format!("Streams*: {:.2}", s));
            }

            ui.add_space(10.0);
            ui.heading("Quality Breakdown");
            if let Some(r) = selected_row {
                egui::Grid::new("quality_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Metric");
                        ui.strong("Value");
                        ui.end_row();

                        ui.label("Smart");
                        ui.label(
                            r.smart
                                .map(|v| format!("{:.1}%", v * 100.0))
                                .unwrap_or_else(|| "-".into()),
                        );
                        ui.end_row();

                        ui.label("Logic");
                        ui.label(
                            r.logic
                                .map(|v| format!("{:.1}%", v * 100.0))
                                .unwrap_or_else(|| "-".into()),
                        );
                        ui.end_row();

                        ui.label("SWE");
                        ui.label(
                            r.swe
                                .map(|v| format!("{:.1}%", v * 100.0))
                                .unwrap_or_else(|| "-".into()),
                        );
                        ui.end_row();

                        ui.label("QnA");
                        ui.label(
                            r.qna
                                .map(|v| format!("{:.1}%", v * 100.0))
                                .unwrap_or_else(|| "-".into()),
                        );
                        ui.end_row();

                        ui.label("LCR");
                        ui.label(
                            r.lcr
                                .map(|v| format!("{:.1}%", v * 100.0))
                                .unwrap_or_else(|| "-".into()),
                        );
                        ui.end_row();

                        if self.settings.show_hallucination || r.halluc.is_some() {
                            ui.label("Hallucination");
                            ui.label(
                                r.halluc
                                    .map(|v| format!("{:.1}%", v * 100.0))
                                    .unwrap_or_else(|| "-".into()),
                            );
                            ui.end_row();
                        }

                        ui.label("Wait Seconds");
                        ui.label(format!("{:.1}s", r.wait_seconds));
                        ui.end_row();

                        ui.label("Read Seconds");
                        ui.label(format!("{:.1}s", r.read_seconds));
                        ui.end_row();

                        ui.label("Attempt USD");
                        ui.label(format!("${:.3}", r.attempt_usd));
                        ui.end_row();

                        ui.label("Read USD");
                        ui.label(format!("${:.3}", r.read_usd));
                        ui.end_row();

                        ui.label("Estimated");
                        ui.label(if r.estimated { "Yes (~)" } else { "No" });
                        ui.end_row();
                    });
            }

            ui.add_space(10.0);
            ui.heading(format!("Top-{} Candidates", pick.top.len().min(4)));
            for (idx, cand) in pick.top.iter().take(4).enumerate() {
                ui.group(|ui| {
                    let cand_row = self.table.rows.get(cand.row_index);
                    let name = cand_row
                        .map(|r| r.display_name())
                        .unwrap_or_else(|| format!("Row #{}", cand.row_index));
                    ui.strong(format!("#{}: {}", idx + 1, name));
                    ui.label(format!(
                        "Win: {:.1}% | Fam Win: {:.1}% | N: {}",
                        cand.win_rate * 100.0,
                        cand.family_win_rate * 100.0,
                        cand.n
                    ));
                    ui.label(format!(
                        "{:.1} tasks/wk | {:.2} min/task | ${:.2}/task",
                        cand.tasks_per_week, cand.minutes_per_task, cand.cost_per_task
                    ));
                    ui.label(format!(
                        "Streams*: {}",
                        cand.streams_star
                            .map(|s| format!("{:.2}", s))
                            .unwrap_or_else(|| "-".into())
                    ));
                    if let Some(cr) = cand_row {
                        ui.horizontal_wrapped(|ui| {
                            ui.small(format!(
                                "Smart: {} | SWE: {} | QnA: {}",
                                cr.smart
                                    .map(|v| format!("{:.1}%", v * 100.0))
                                    .unwrap_or_else(|| "-".into()),
                                cr.swe
                                    .map(|v| format!("{:.1}%", v * 100.0))
                                    .unwrap_or_else(|| "-".into()),
                                cr.qna
                                    .map(|v| format!("{:.1}%", v * 100.0))
                                    .unwrap_or_else(|| "-".into()),
                            ));
                        });
                    }
                });
            }

            ui.add_space(10.0);
            ui.heading("Source & Timestamps");
            ui.label("Source: Artificial Analysis");
            ui.label(format!("Source Fetched: {}", self.table.source_fetched_at));
            ui.label(format!("Generated At: {}", self.table.generated_at));
            ui.label(format!("Cache State: {}", self.table.cache_state.name()));
        } else {
            ui.label("No candidate pick available for this seat and tier.");
        }
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
        self.detail_content(&mut measure, seat, tier);
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
            self.refresh_in_flight = true;
            self.refresh_clicked_at = Some(Instant::now());
            self.engine_status = "Refreshing…".to_string();
        }
    }

    pub fn trigger_update_check(&mut self) {
        let (tx, rx) = channel();
        self.update_rx = Some(rx);
        self.update_status = "Checking for updates…".to_string();
        std::thread::spawn(move || {
            let outcome =
                crate::update::check(env!("CARGO_PKG_VERSION")).map_err(|e| format!("{e:#}"));
            let _ = tx.send(outcome);
        });
    }

    fn pump_channels(&mut self) {
        while let Ok((new_table, fetched_rows)) = self.table_rx.try_recv() {
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
            self.table = new_table;
            if !self.refresh_in_flight {
                self.engine_status.clear();
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
            match res {
                Ok(checked) => match checked.outcome {
                    crate::update::Outcome::Available(av) => {
                        self.update_status = format!("Update {} available", av.version);
                        crate::update::record_check(Some(&av.version));
                    }
                    crate::update::Outcome::UpToDate { latest } => {
                        self.update_status = format!("Up to date ({latest})");
                        crate::update::record_check(Some(&latest));
                    }
                },
                Err(e) => {
                    self.update_status = format!("Update check: {e}");
                    crate::update::record_check(None);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_channels();
        let ctx = ui.ctx().clone();
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
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                if ui.button("Check for updates").clicked() {
                    self.trigger_update_check();
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
                    if !self.update_status.is_empty() {
                        ui.add(egui::Label::new(&self.update_status).truncate())
                            .on_hover_text(&self.update_status);
                    }
                });
            });
        });

        let panel_open = self.side_panel_open && self.selected_seat_tier.is_some();
        let t = ctx.animate_bool(egui::Id::new("side_panel_anim"), panel_open);
        if t > 0.001 {
            let (seat, tier) = self.selected_seat_tier.unwrap();
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

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            self.detail_content(ui, seat, tier);
                        });
                    }
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Hours per week");
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
            ui.label("hours you work x agents running in parallel");
            ui.add_space(8.0);
            egui::ScrollArea::both()
                .id_salt("roles_table_scroll")
                .auto_shrink([false, false])
                .max_height(ui.available_height())
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.strong("Coding Agent Tier List");
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
                            ui.strong("Seat / Tier");
                            ui.strong("Agent");
                            ui.strong("Win");
                            ui.strong("Streams*");
                            ui.strong("Min/task");
                            ui.strong("$/task");
                            ui.strong("Tasks/wk");
                            for heading in ["Util%", "A*h"] {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.strong(heading);
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
                                        win,
                                        streams_star,
                                        min_task,
                                        cost_task,
                                        tasks_wk,
                                        a_star_hours,
                                    ) = if let Some(pick) = maybe_pick {
                                        let name = self
                                            .table
                                            .rows
                                            .get(pick.row_index)
                                            .map(|r| r.display_name())
                                            .unwrap_or_else(|| "-".into());
                                        let win = format!("{:.0}%", pick.win_rate * 100.0);
                                        let streams_star = pick
                                            .streams_star
                                            .map(|s| format!("{:.2}", s))
                                            .unwrap_or_else(|| "-".into());
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
                                            win,
                                            streams_star,
                                            min_task,
                                            cost_task,
                                            tasks_wk,
                                            a_star_hours,
                                        )
                                    } else {
                                        (
                                            "-".into(),
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
                                    if ui.selectable_label(is_selected, &win).clicked() {
                                        clicked = true;
                                    }
                                    if ui.selectable_label(is_selected, &streams_star).clicked() {
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
                                                    .map(|value| format!("{value:.1}"))
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
                    for seat in Seat::ALL {
                        let picks: Vec<_> = [
                            (Tier::T200, self.settings.plan_prices.t200),
                            (Tier::T100, self.settings.plan_prices.t100),
                            (Tier::T20, self.settings.plan_prices.t20),
                        ]
                        .into_iter()
                        .filter_map(|(tier, price)| {
                            self.table
                                .get_pick(seat, tier)
                                .map(|pick| (tier, price, pick))
                        })
                        .collect();
                        let Some(best) = picks.iter().max_by(|left, right| {
                            left.2.tasks_per_week.total_cmp(&right.2.tasks_per_week)
                        }) else {
                            continue;
                        };
                        if let Some(enough) = picks
                            .iter()
                            .filter(|(_, _, pick)| {
                                pick.tasks_per_week >= best.2.tasks_per_week * 0.95
                            })
                            .min_by(|left, right| left.1.total_cmp(&right.1))
                        {
                            let agent = self.table.rows[enough.2.row_index].display_name();
                            ui.label(if enough.0 == best.0 {
                                format!(
                                    "{}: {} - {} at {:.1}/wk",
                                    seat.name(),
                                    enough.0.name(),
                                    agent,
                                    enough.2.tasks_per_week
                                )
                            } else {
                                format!(
                                    "{}: {} is enough - {} at {:.1}/wk ({} gives {:.1})",
                                    seat.name(),
                                    enough.0.name(),
                                    agent,
                                    enough.2.tasks_per_week,
                                    best.0.name(),
                                    best.2.tasks_per_week
                                )
                            });
                        }
                    }
                    for (heading, bare_model, columns) in [
                        (
                            "Coding Agents",
                            false,
                            [
                                "rank", "Agent", "SWE", "Term", "QnA", "Vendor", "Wait", "Cost",
                                "Tasks/wk",
                            ],
                        ),
                        (
                            "Models (bare API)",
                            true,
                            [
                                "rank", "Model", "Term", "Logic", "Tok/s", "Vendor", "Wait",
                                "Cost", "Tasks/wk",
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
                                        ui.strong(*heading);
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
                                    let wall = crate::engine::cycle(
                                        row.pass,
                                        row.wait_seconds.round().max(1.0) as u32,
                                        row.attempt_usd,
                                        0.0,
                                        &self.settings,
                                    )
                                    .wall;
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
                                        format!("{:.1}", self.settings.agent_hours * 3600.0 / wall),
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
                    let mut changed = false;
                    ui.heading("Engine & Cache Configuration");
                    ui.horizontal(|ui| {
                        ui.label("Escalation minutes:");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.escalation_minutes)
                                    .range(0.0..=10080.0),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Escalation USD:");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.escalation_usd)
                                    .range(0.0..=1000000.0),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Rho:");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.rho)
                                    .speed(0.001)
                                    .range(0.0..=0.999),
                            )
                            .changed();
                    });

                    ui.horizontal(|ui| {
                        ui.label("Monte Carlo Draws (1000..20000):");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.settings.draws, 1000..=20000)
                                    .step_by(100.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
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
                            "Show Hallucination rate in quality view",
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
                    ui.label("Leave blank for model default estimation");
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for (vendor, val) in self.settings.vendor_overrides.iter_mut() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{vendor}:"));
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
                        let _ = save_settings(&self.settings);
                        let _ = self.settings_tx.send((self.settings.clone(), false));
                        self.engine_status = "Scoring…".to_string();
                    }
                });
            self.settings_open = open;
        }
    }
}
