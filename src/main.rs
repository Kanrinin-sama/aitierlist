#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod aa;
mod cache;
mod engine;
mod file_tx;
mod handoff;
mod settings;
mod types;
mod ui;
mod update;

#[used]
static VERSION_TAG: &str = concat!("AITIERLIST_VERSION=", env!("CARGO_PKG_VERSION"));

fn main() -> eframe::Result<()> {
    if std::env::args().any(|argument| argument == "--dump-table") {
        let outcome = (|| -> anyhow::Result<()> {
            let mut settings = settings::Settings::default();
            let mut cache_path = None;
            for argument in std::env::args().skip(1) {
                if let Some((key, value)) = argument.split_once('=') {
                    match key {
                        "--agent-hours" => settings.agent_hours = value.parse()?,
                        "--draws" => settings.draws = value.parse()?,
                        "--rho" => settings.rho = value.parse()?,
                        "--escalation-minutes" => settings.escalation_minutes = value.parse()?,
                        "--escalation-usd" => settings.escalation_usd = value.parse()?,
                        "--cache" => cache_path = Some(value.to_owned()),
                        _ => {}
                    }
                }
            }
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
            println!(
                "{}",
                serde_json::to_string_pretty(&engine::score(rows, &settings, state, fetched))?
            );
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
                        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                        row.harness,
                        row.model,
                        row.effort.as_deref().unwrap_or(""),
                        row.pass,
                        row.wait_seconds,
                        row.attempt_usd,
                        row.qna.map(|value| value.to_string()).unwrap_or_default(),
                        row.vendor,
                        row.estimated
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

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("aitierlist")
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
