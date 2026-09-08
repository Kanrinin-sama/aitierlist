#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aa;
mod cache;
mod comparison;
mod engine;
mod file_tx;
mod handoff;
mod retry;
mod settings;
mod types;
mod ui;
mod update;

#[used]
static VERSION_TAG: &str = concat!("AITIERLIST_VERSION=", env!("CARGO_PKG_VERSION"), "\0");

fn main() -> eframe::Result<()> {
    if std::env::args().any(|argument| argument == "--dump-table") {
        let outcome = (|| -> anyhow::Result<()> {
            let arguments = std::env::args().skip(1).collect::<Vec<_>>();
            let settings_file = arguments
                .iter()
                .find_map(|argument| argument.strip_prefix("--settings=").map(ToOwned::to_owned));
            let comparison_request = arguments
                .iter()
                .find_map(|argument| argument.strip_prefix("--compare=").map(ToOwned::to_owned));
            let mut settings = if let Some(path) = settings_file {
                serde_json::from_slice::<settings::Settings>(&std::fs::read(path)?)?
            } else {
                settings::load_settings()
            };
            let mut cache_path = None;
            for argument in arguments {
                if let Some((key, value)) = argument.split_once('=') {
                    match key {
                        "--agent-hours" => settings.agent_hours = value.parse()?,
                        "--draws" => settings.draws = value.parse()?,
                        "--assumption-span" => settings.assumption_span_pct = value.parse()?,
                        "--rho-override" => {
                            settings.rho_override = if value.is_empty() {
                                None
                            } else {
                                Some(value.parse()?)
                            }
                        }
                        "--escalation-minutes" => settings.escalation_minutes = value.parse()?,
                        "--escalation-usd" => settings.escalation_usd = value.parse()?,
                        "--competence-floor" => {
                            let (seat_name, floor) = value.split_once(':').ok_or_else(|| {
                                anyhow::anyhow!(
                                    "competence floor must be Seat:value, for example Reviewer:0.7"
                                )
                            })?;
                            let seat = types::Seat::ALL
                                .into_iter()
                                .find(|seat| seat.name().eq_ignore_ascii_case(seat_name))
                                .ok_or_else(|| anyhow::anyhow!("unknown seat {seat_name}"))?;
                            settings
                                .competence_floors
                                .insert(seat.name().to_string(), Some(floor.parse()?));
                        }
                        "--cache" => cache_path = Some(value.to_owned()),
                        "--settings" => {}
                        _ => {}
                    }
                }
            }
            settings = settings.normalize();
            let (rows, state, fetched) = if let Some(path) = cache_path {
                let payload: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
                (
                    aa::rows_from_payloads(&payload)?,
                    types::CacheState::Disk,
                    payload["fetchedAt"].as_str().unwrap_or_default().to_owned(),
                )
            } else {
                aa::load_rows(false, f64::INFINITY)?
            };
            let table = engine::score(rows, &settings, state, fetched, None);
            let output = if let Some(request) = comparison_request {
                let mut parts = request.split(':');
                let seat_name = parts.next().unwrap_or_default();
                let tier_name = parts.next().unwrap_or_default();
                let row_index = parts
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("comparison must be Seat:Tier:rowIndex"))?
                    .parse::<usize>()?;
                if parts.next().is_some() {
                    anyhow::bail!("comparison must be Seat:Tier:rowIndex");
                }
                let seat = types::Seat::ALL
                    .into_iter()
                    .find(|seat| seat.name().eq_ignore_ascii_case(seat_name))
                    .ok_or_else(|| anyhow::anyhow!("unknown seat {seat_name}"))?;
                let tier = types::Tier::ALL
                    .into_iter()
                    .find(|tier| {
                        tier.name().eq_ignore_ascii_case(tier_name)
                            || match tier {
                                types::Tier::Api => tier_name.eq_ignore_ascii_case("api"),
                                types::Tier::T200 => tier_name.eq_ignore_ascii_case("t200"),
                                types::Tier::T100 => tier_name.eq_ignore_ascii_case("t100"),
                                types::Tier::T20 => tier_name.eq_ignore_ascii_case("t20"),
                            }
                    })
                    .ok_or_else(|| anyhow::anyhow!("unknown tier {tier_name}"))?;
                serde_json::to_value(comparison::compare_candidate(
                    &table.rows,
                    &settings,
                    seat,
                    tier,
                    row_index,
                ))?
            } else {
                serde_json::to_value(table)?
            };
            println!("{}", serde_json::to_string_pretty(&output)?);
            Ok(())
        })();
        if let Err(error) = outcome {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--dump-rows") {
        match aa::load_rows(false, f64::INFINITY) {
            Ok((mut rows, _, _)) => {
                rows.sort_by_key(|row| row.display_name().to_lowercase());
                for row in rows {
                    println!(
                        "{}|{}|{}|{}|{}|{}|{}|{}",
                        row.harness,
                        row.model,
                        row.effort.as_deref().unwrap_or(""),
                        row.pass
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "Unknown".to_string()),
                        row.wait_seconds,
                        row.attempt_usd,
                        row.qna.map(|value| value.to_string()).unwrap_or_default(),
                        row.vendor
                    );
                }
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
    }
    if handoff::helper_launch_requested() {
        std::process::exit(handoff::run_helper() as i32);
    }

    std::hint::black_box(VERSION_TAG);
    std::hint::black_box(update::COMMIT_TAG);
    std::hint::black_box(update::TREE_TAG);

    update::cleanup_previous_update();

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/aitierlist.png"))
        .expect("embedded application icon must be a valid PNG");
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("aitierlist")
        .with_icon(icon)
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([900.0, 600.0]);

    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "aitierlist",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(eframe::egui::Visuals::dark());
            Ok(Box::new(ui::App::new(cc)))
        }),
    )
}
