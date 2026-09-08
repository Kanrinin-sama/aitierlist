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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Resolved {
    pub rho: f64,
    pub layer: Layer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowRetry {
    pub deepswe: Resolved,
    pub terminal_bench: Resolved,
    pub qna: Resolved,
    pub difficulty_ratio: f64,
}

impl RowRetry {
    pub fn adjusted_pass(&self, benchmark: Benchmark, pass: f64) -> f64 {
        if benchmark == Benchmark::Swe {
            (pass * self.difficulty_ratio).clamp(0.0, 1.0)
        } else {
            pass
        }
    }

    pub fn description(&self) -> String {
        format!(
            "DeepSWE rho: {:.6} ({})\nTerminal-Bench 2.1 rho: {:.6} ({})\nRepository Q&A rho: {:.6} ({})\nDeepSWE difficulty multiplier: {:.6}",
            self.deepswe.rho,
            self.deepswe.layer.name(),
            self.terminal_bench.rho,
            self.terminal_bench.layer.name(),
            self.qna.rho,
            self.qna.layer.name(),
            self.difficulty_ratio,
        )
    }
}

impl Default for RowRetry {
    fn default() -> Self {
        let table = table();
        let deepswe = Resolved {
            rho: table.deepswe.pool_rho,
            layer: Layer::SourcePool,
        };
        Self {
            deepswe,
            terminal_bench: Resolved {
                rho: table.terminal_bench.pool_rho,
                layer: Layer::SourcePool,
            },
            qna: deepswe,
            difficulty_ratio: 1.0,
        }
    }
}

#[derive(Deserialize)]
struct RetryTable {
    deepswe: DeepSwe,
    terminal_bench: TerminalBench,
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
    pool_rho: f64,
    harnesses: Vec<Harness>,
}

#[derive(Deserialize)]
struct Harness {
    harness_label: String,
    rho: f64,
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
        if let Some(model) = identity.deepswe.and_then(|name| {
            table
                .deepswe
                .models
                .iter()
                .find(|model| model.model == name)
        }) {
            resolved.deepswe = Resolved {
                rho: model.rho,
                layer: Layer::ModelPool,
            };
            if let Some(config) = model
                .configs
                .iter()
                .find(|config| config.reasoning_effort.as_deref() == identity.effort)
            {
                resolved.deepswe = Resolved {
                    rho: config.rho_shrunk,
                    layer: Layer::Exact,
                };
                resolved.difficulty_ratio = config.difficulty_ratio;
            }
        }
        if let Some(harness) = identity.terminal_harness.and_then(|name| {
            table
                .terminal_bench
                .harnesses
                .iter()
                .find(|harness| harness.harness_label == name)
        }) {
            resolved.terminal_bench = Resolved {
                rho: harness.rho,
                layer: Layer::HarnessPool,
            };
            if let Some(submission) = harness.submissions.iter().find(|submission| {
                submission.model_display == identity.terminal_model
                    && submission.reasoning_effort.as_deref() == identity.effort
            }) {
                resolved.terminal_bench = Resolved {
                    rho: submission.rho,
                    layer: Layer::Exact,
                };
            }
        }
    }
    if let Some(rho) = rho_override {
        let value = Resolved {
            rho,
            layer: Layer::Override,
        };
        resolved.deepswe = value;
        resolved.terminal_bench = value;
        resolved.qna = value;
    }
    resolved
}
