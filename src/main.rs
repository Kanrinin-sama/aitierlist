#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![recursion_limit = "256"]

mod aa;
mod agent_setup;
mod cache;
mod comparison;
mod engine;
mod file_tx;
mod host_install;
mod isolation;
mod native_reconciliation;
mod native_scheduler;
mod orchestration;
mod portfolio;
mod research_packet;
mod retry;
mod route_qualification;
mod settings;
mod snapshot;
mod solver;
mod subscriptions;
mod team_policy;
mod theme;
mod types;
mod ui;
mod update;
mod workspace;

#[used]
static VERSION_TAG: &str = concat!("AITIERLIST_VERSION=", env!("CARGO_PKG_VERSION"), "\0");

fn main() -> eframe::Result<()> {
    let install_config = match update::install_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    };
    let startup = match blockitall_update::install::initialize(&install_config) {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    };
    let health_guard = match startup {
        blockitall_update::Startup::HelperExit(code) => std::process::exit(code as i32),
        blockitall_update::Startup::Application(guard) => guard,
    };
    let application_arguments = health_guard
        .application_args()
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let fleet_arguments = application_arguments.clone();
    if fleet_arguments
        .first()
        .is_some_and(|flag| flag == "--fleet" || flag == "--fleet-worker")
    {
        let outcome = match fleet_arguments.as_slice() {
            [flag, manifest, task] if flag == "--fleet-worker" => {
                isolation::worker(std::path::Path::new(manifest), task)
            }
            [flag, manifest, operation, id] if flag == "--fleet" => {
                isolation::dispatch(std::path::Path::new(manifest), operation, id, None)
            }
            [flag, manifest, operation, id, request] if flag == "--fleet" => isolation::dispatch(
                std::path::Path::new(manifest),
                operation,
                id,
                Some(std::path::Path::new(request)),
            ),
            _ => Err(anyhow::anyhow!(
                "Use --fleet <manifest> admit task <task.json>, run <visit-id>, settle <visit-id> <settlement.json>, reconcile meter <observation.json>, replan queue, update <task-id> <task.json>, status <task-or-visit-id>, or cancel <task-id>"
            )),
        };
        if let Err(error) = outcome {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let mut launch_arguments = application_arguments.iter().cloned();
    while let Some(argument) = launch_arguments.next() {
        let path = if argument == "--launch-orchestrator" {
            Some(launch_arguments.next().unwrap_or_default())
        } else {
            argument
                .strip_prefix("--launch-orchestrator=")
                .map(ToOwned::to_owned)
        };
        if let Some(path) = path {
            if let Err(error) = host_install::launch(std::path::Path::new(&path)) {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
            return Ok(());
        }
    }

    if application_arguments.iter().any(|argument| {
        argument == "--dump-table"
            || argument.starts_with("--export-policy=")
            || argument.starts_with("--export-orchestrator=")
            || argument.starts_with("--compare-conductors=")
    }) {
        let outcome = (|| -> anyhow::Result<()> {
            let arguments = application_arguments.clone();
            let settings_file = arguments
                .iter()
                .find_map(|argument| argument.strip_prefix("--settings=").map(ToOwned::to_owned));
            let comparison_request = arguments
                .iter()
                .find_map(|argument| argument.strip_prefix("--compare=").map(ToOwned::to_owned));
            let conductor_comparison_path = arguments.iter().find_map(|argument| {
                argument
                    .strip_prefix("--compare-conductors=")
                    .map(ToOwned::to_owned)
            });
            let export_path = arguments.iter().find_map(|argument| {
                argument
                    .strip_prefix("--export-policy=")
                    .or_else(|| argument.strip_prefix("--export-orchestrator="))
                    .map(ToOwned::to_owned)
            });
            if export_path.is_some() && comparison_request.is_some() {
                anyhow::bail!("Orchestrator export cannot be combined with --compare");
            }
            let mut settings = if let Some(path) = settings_file {
                serde_json::from_slice::<settings::Settings>(&std::fs::read(path)?)?
            } else {
                settings::load_settings()
            };
            let mut cache_path = None;
            let mut input_table_path = None;
            for argument in arguments {
                if let Some((key, value)) = argument.split_once('=') {
                    match key {
                        "--agent-hours" => settings.agent_hours = value.parse()?,
                        "--orchestrators" => settings.orchestrators = value.parse()?,
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
                        "--input-table" => input_table_path = Some(value.to_owned()),
                        "--settings" => {}
                        _ => {}
                    }
                }
            }
            settings = settings.normalize();
            if cache_path.is_some() && input_table_path.is_some() {
                anyhow::bail!("--input-table cannot be combined with --cache");
            }
            let (rows, state, fetched) = if let Some(path) = input_table_path {
                let payload: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
                let rows = serde_json::from_value::<Vec<types::Row>>(
                    payload
                        .get("rows")
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Input table is missing rows"))?,
                )?;
                let fetched = payload
                    .get("sourceFetchedAt")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Input table is missing sourceFetchedAt"))?
                    .to_owned();
                (rows, types::CacheState::Disk, fetched)
            } else if let Some(path) = cache_path {
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
            if let Some(path) = conductor_comparison_path {
                std::fs::write(
                    &path,
                    serde_json::to_vec_pretty(&portfolio::compare_conductors(
                        &table.rows,
                        &settings,
                    ))?,
                )?;
                println!("{path}");
                return Ok(());
            }
            if let Some(path) = export_path {
                let path = std::path::Path::new(&path);
                orchestration::write_policy(&table, path)?;
                println!("{}", path.display());
                return Ok(());
            }
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
    if application_arguments
        .iter()
        .any(|argument| argument == "--dump-rows")
    {
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
    std::hint::black_box(VERSION_TAG);
    std::hint::black_box(update::COMMIT_TAG);
    std::hint::black_box(update::TREE_TAG);

    let handoff_status = match blockitall_update::install::take_last_status(&install_config) {
        Ok(Some(status)) => format!(
            "Update {}: {} — {}",
            status.version, status.outcome, status.detail
        ),
        Ok(None) => String::new(),
        Err(error) => format!("Update status unavailable: {error:#}"),
    };
    let updater = match update::updater(install_config) {
        Ok(updater) => updater,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    };

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/aitierlist.png"))
        .expect("embedded application icon must be a valid PNG");
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("AI Tier List")
        .with_icon(icon)
        .with_inner_size([1440.0, 940.0])
        .with_min_inner_size([900.0, 650.0]);

    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "aitierlist",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(ui::App::new(
                cc,
                updater,
                health_guard,
                handoff_status,
            )))
        }),
    )
}
