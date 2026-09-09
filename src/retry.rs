use crate::types::{Benchmark, Row};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Layer {
    Exact,
    ModelPool,
    HarnessPool,
    SourcePool,
    Override,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Self::Exact => "Exact",
            Self::ModelPool => "Model pool",
            Self::HarnessPool => "Harness pool",
            Self::SourcePool => "Source pool",
            Self::Override => "Override",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolved {
    pub rho: f64,
    pub layer: Layer,
    pub evidence_dataset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowRetry {
    pub deepswe: Option<Resolved>,
    pub terminal_bench: Option<Resolved>,
    pub qna: Option<Resolved>,
    pub difficulty_ratio: f64,
}

impl RowRetry {
    pub fn repeat_evidence(&self, benchmark: Benchmark) -> Option<&Resolved> {
        match benchmark {
            Benchmark::Swe => self.deepswe.as_ref(),
            Benchmark::Terminal => self.terminal_bench.as_ref(),
            Benchmark::Qna => self.qna.as_ref(),
            _ => None,
        }
    }

    pub fn adjusted_pass(&self, benchmark: Benchmark, pass: f64) -> f64 {
        if benchmark == Benchmark::Swe {
            (pass * self.difficulty_ratio).clamp(0.0, 1.0)
        } else {
            pass
        }
    }

    pub fn description(&self) -> String {
        format!(
            "DeepSWE rho: {}\nTerminal-Bench rho: {}\nRepository Q&A rho: {}\nDeepSWE difficulty multiplier: {:.6}",
            self.deepswe
                .as_ref()
                .map(|value| format!(
                    "{:.6} ({}, {})",
                    value.rho,
                    value.layer.name(),
                    value.evidence_dataset
                ))
                .unwrap_or_else(|| "No matching published repeat evidence".to_string()),
            self.terminal_bench
                .as_ref()
                .map(|value| format!(
                    "{:.6} ({}, {})",
                    value.rho,
                    value.layer.name(),
                    value.evidence_dataset
                ))
                .unwrap_or_else(|| "No matching published repeat evidence".to_string()),
            self.qna
                .as_ref()
                .map(|value| format!(
                    "{:.6} ({}, {})",
                    value.rho,
                    value.layer.name(),
                    value.evidence_dataset
                ))
                .unwrap_or_else(|| "No published repeat evidence".to_string()),
            self.difficulty_ratio,
        )
    }
}

impl Default for RowRetry {
    fn default() -> Self {
        Self {
            deepswe: None,
            terminal_bench: None,
            qna: None,
            difficulty_ratio: 1.0,
        }
    }
}

#[derive(Deserialize)]
struct RetryTable {
    deepswe: DeepSwe,
    terminal_bench: TerminalBench,
    terminal_bench_v4: TerminalBench,
}

#[derive(Deserialize)]
struct DeepSwe {
    pool_rho: f64,
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    model: String,
    rho: f64,
    configs: Vec<Config>,
}

#[derive(Deserialize)]
struct Config {
    reasoning_effort: Option<String>,
    rho_shrunk: f64,
    difficulty_ratio: f64,
}

#[derive(Deserialize)]
struct TerminalBench {
    harnesses: Vec<Harness>,
}

#[derive(Deserialize)]
struct Harness {
    harness_label: String,
    submissions: Vec<Submission>,
}

#[derive(Deserialize)]
struct Submission {
    model_display: String,
    reasoning_effort: Option<String>,
    rho: f64,
}

fn table() -> &'static RetryTable {
    static TABLE: OnceLock<RetryTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/retry-correlation.json"))
            .expect("embedded retry correlation table")
    })
}

struct Identity<'a> {
    deepswe: Option<&'static str>,
    terminal_model: &'a str,
    terminal_harness: Option<&'static str>,
    effort: Option<&'a str>,
}

fn identity(row: &Row) -> Identity<'_> {
    let model = row
        .model
        .strip_suffix(" (with fallback)")
        .unwrap_or(&row.model);
    let model = row
        .effort
        .as_ref()
        .and_then(|effort| model.strip_suffix(&format!(" ({effort})")))
        .unwrap_or(model);
    let deepswe = match model {
        "GPT-6 Astra" => Some("gpt-6-astra"),
        "GPT-5.6 Sol" => Some("gpt-5-6-sol"),
        "GPT-5.6 Terra" => Some("gpt-5-6-terra"),
        "GPT-5.6 Luna" => Some("gpt-5-6-luna"),
        "GPT-5.5" => Some("gpt-5-5"),
        "GPT-5.4" => Some("gpt-5-4"),
        "Opus 5" => Some("claude-opus-5"),
        "Opus 4.8" => Some("claude-opus-4-8"),
        "Fable 5" => Some("claude-fable-5"),
        "Sonnet 5" => Some("claude-sonnet-5"),
        "Sonnet 4.6" => Some("claude-sonnet-4-6"),
        "Gemini 3.8 Flash" => Some("gemini-3-8-flash"),
        "Gemini 3.7 Flash" => Some("gemini-3-7-flash"),
        "Gemini 3.6 Flash" => Some("gemini-3-6-flash"),
        "Gemini 3.5 Flash" => Some("gemini-3-5-flash"),
        "Gemini 3.1 Pro" => Some("gemini-3-1-pro-preview"),
        "Muse Spark 1.2" => Some("muse-spark-1-2"),
        "Muse Spark 1.1" => Some("muse-spark-1-1"),
        "Grok 4.6" => Some("grok-4-6"),
        "Grok 4.5" => Some("grok-4-5"),
        "Kimi K3" => Some("kimi-k3"),
        "Kimi K2.7 Code" => Some("kimi-k2-7-code"),
        "Qwen3.8 Max" => Some("qwen3-8-max"),
        "GLM-5.3" => Some("glm-5-3"),
        "GLM-5.3 Flash" => Some("glm-5-3-flash"),
        "GLM-5.2" => Some("glm-5-2"),
        "DeepSeek V4 Pro" => Some("deepseek-v4-pro"),
        "DeepSeek V4 Flash" => Some("deepseek-v4-flash"),
        _ => None,
    };
    let terminal_harness = match row.harness.as_str() {
        "Codex" => Some("Codex"),
        "Claude Code" => Some("Claude Code"),
        "Gemini CLI" => Some("Gemini CLI"),
        "Cursor CLI" => Some("Cursor CLI"),
        "mini-SWE-agent" => Some("mini-SWE-agent"),
        "Terminus 2" => Some("Terminus 2"),
        _ => None,
    };
    Identity {
        deepswe,
        terminal_model: model,
        terminal_harness,
        effort: row.effort.as_deref(),
    }
}

pub fn resolve(row: &Row, rho_override: Option<f64>) -> RowRetry {
    let identity = identity(row);
    let table = table();
    let mut resolved = RowRetry::default();
    if identity.effort != Some("none") {
        if row.benchmark_dataset(Benchmark::Swe) == Some("deep-swe-v1.1") {
            resolved.deepswe = Some(Resolved {
                rho: table.deepswe.pool_rho,
                layer: Layer::SourcePool,
                evidence_dataset: "deep-swe-v1.1".to_string(),
            });
        }
        if row.benchmark_dataset(Benchmark::Swe) == Some("deep-swe-v1.1")
            && let Some(model) = identity.deepswe.and_then(|name| {
                table
                    .deepswe
                    .models
                    .iter()
                    .find(|model| model.model == name)
            })
        {
            resolved.deepswe = Some(Resolved {
                rho: model.rho,
                layer: Layer::ModelPool,
                evidence_dataset: "deep-swe-v1.1".to_string(),
            });
            if let Some(config) = model
                .configs
                .iter()
                .find(|config| config.reasoning_effort.as_deref() == identity.effort)
            {
                resolved.deepswe = Some(Resolved {
                    rho: config.rho_shrunk,
                    layer: Layer::Exact,
                    evidence_dataset: "deep-swe-v1.1".to_string(),
                });
                resolved.difficulty_ratio = config.difficulty_ratio;
            }
        }
        let terminal_table = match row.benchmark_dataset(Benchmark::Terminal) {
            Some("terminal-bench-v4") => Some((&table.terminal_bench_v4, "terminal-bench-v4")),
            Some("terminal-bench-v2.1") => Some((&table.terminal_bench, "terminal-bench-v2.1")),
            _ => None,
        };
        if let Some((terminal_table, dataset)) = terminal_table
            && let Some(harness) = identity.terminal_harness.and_then(|name| {
                terminal_table
                    .harnesses
                    .iter()
                    .find(|harness| harness.harness_label == name)
            })
            && let Some(submission) = harness.submissions.iter().find(|submission| {
                submission.model_display == identity.terminal_model
                    && submission.reasoning_effort.as_deref() == identity.effort
            })
        {
            resolved.terminal_bench = Some(Resolved {
                rho: submission.rho,
                layer: Layer::Exact,
                evidence_dataset: dataset.to_string(),
            });
        }
    }
    if let Some(rho) = rho_override {
        let value = Resolved {
            rho,
            layer: Layer::Override,
            evidence_dataset: "user-rho-override".to_string(),
        };
        resolved.deepswe = Some(value.clone());
        resolved.terminal_bench = Some(value.clone());
        resolved.qna = Some(value);
    }
    resolved
}
