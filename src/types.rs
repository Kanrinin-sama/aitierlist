use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Seat {
    Implementer,
    Debugger,
    Reviewer,
    Orchestrator,
    Sanity,
    Comprehension,
    NetResearch,
}

impl Seat {
    pub const ALL: [Seat; 7] = [
        Seat::Implementer,
        Seat::Debugger,
        Seat::Reviewer,
        Seat::Orchestrator,
        Seat::Sanity,
        Seat::Comprehension,
        Seat::NetResearch,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Seat::Implementer => "Implementer",
            Seat::Debugger => "Debugger",
            Seat::Reviewer => "Reviewer",
            Seat::Orchestrator => "Orchestrator",
            Seat::Sanity => "Sanity",
            Seat::Comprehension => "Comprehension",
            Seat::NetResearch => "Net Research",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Tier {
    Api,
    T200,
    T100,
    T20,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::Api, Tier::T200, Tier::T100, Tier::T20];

    pub fn name(self) -> &'static str {
        match self {
            Tier::Api => "API (spend unbounded)",
            Tier::T200 => "$200",
            Tier::T100 => "$100",
            Tier::T20 => "$20",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Benchmark {
    Swe,
    Terminal,
    Qna,
    Gpqa,
    Hle,
    Lcr,
    Omniscience,
}

impl Benchmark {
    pub fn name(self) -> &'static str {
        match self {
            Benchmark::Swe => "DeepSWE",
            Benchmark::Terminal => "Terminal-Bench",
            Benchmark::Qna => "Repository Q&A",
            Benchmark::Gpqa => "GPQA reasoning",
            Benchmark::Hle => "Humanity's Last Exam",
            Benchmark::Lcr => "Long-context reasoning",
            Benchmark::Omniscience => "Omniscience",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMetric {
    pub benchmark: Benchmark,
    #[serde(default)]
    pub dataset_id: String,
    #[serde(default)]
    pub task_count: usize,
    pub pass: f64,
    pub seconds: f64,
    #[serde(default)]
    pub pooled_seconds: f64,
    pub usd: f64,
    pub time_basis: String,
    pub cost_basis: String,
    #[serde(default)]
    pub canonical_resources: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Baked,
    Disk,
    Live,
}

impl CacheState {
    pub fn name(self) -> &'static str {
        match self {
            CacheState::Baked => "baked",
            CacheState::Disk => "disk",
            CacheState::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Row {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    pub vendor: String,
    #[serde(rename = "index")]
    pub smart: Option<f64>,
    pub logic: Option<f64>,
    pub swe: Option<f64>,
    pub qna: Option<f64>,
    pub lcr: Option<f64>,
    pub omniscience_accuracy: Option<f64>,
    pub omniscience_attempt_rate: Option<f64>,
    #[serde(rename = "hallucination")]
    pub halluc: Option<f64>,
    pub gdp_pdf: Option<f64>,
    pub wait_seconds: f64,
    pub read_seconds: f64,
    pub pooled_seconds: f64,
    pub attempt_usd: f64,
    pub read_usd: f64,
    pub model_key: String,
    pub family: String,
    #[serde(rename = "name")]
    pub display_name: String,
    pub pass: Option<f64>,
    pub term: Option<f64>,
    pub gpqa: Option<f64>,
    pub hle: Option<f64>,
    pub task_metrics: Vec<TaskMetric>,
    pub retry: crate::retry::RowRetry,
    pub usd_per_step: Option<f64>,
    pub speed: Option<f64>,
    pub orchestrator_usd: Option<f64>,
    pub orchestrator_seconds: Option<f64>,
    pub orchestrator_cost_basis: Option<String>,
    pub orchestrator_time_basis: Option<String>,
    pub benchmark_observation_ids: Vec<String>,
    pub benchmark_evidence: Vec<crate::evidence::EvidenceProjection>,
}

impl Row {
    pub fn task_metric(&self, benchmark: Benchmark) -> Option<&TaskMetric> {
        self.task_metrics
            .iter()
            .find(|metric| metric.benchmark == benchmark)
    }

    pub fn benchmark_tasks(&self, benchmark: Benchmark) -> Option<usize> {
        self.task_metric(benchmark)
            .map(|metric| metric.task_count)
            .filter(|count| *count > 0)
            .or_else(|| {
                self.benchmark_evidence
                    .iter()
                    .find(|projection| projection.family == benchmark_key(benchmark))
                    .and_then(|projection| projection.series.task_count)
                    .filter(|count| *count > 0)
            })
    }

    pub fn benchmark_dataset(&self, benchmark: Benchmark) -> Option<&str> {
        self.task_metric(benchmark)
            .map(|metric| metric.dataset_id.as_str())
            .filter(|dataset| !dataset.is_empty())
    }

    pub fn display_name(&self) -> String {
        let name = if !self.display_name.is_empty() {
            self.display_name.clone()
        } else {
            match &self.effort {
                Some(effort) if !effort.is_empty() => format!("{} ({})", self.model, effort),
                _ => self.model.clone(),
            }
        };
        if self.harness.to_ascii_lowercase().starts_with("antigravity")
            && name.to_ascii_lowercase().starts_with("antigravity")
        {
            return name
                .split_once(" - ")
                .map_or_else(|| "AGY".to_owned(), |(_, model)| format!("AGY - {model}"));
        }
        name
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePick {
    pub row_index: usize,
    pub competence: Option<f64>,
    pub competence_floor: Option<f64>,
    pub capacity_shortfall: f64,
    pub minutes_per_task: f64,
    pub cost_per_task: f64,
    pub attempt_limit: usize,
    pub model_cost_per_task: f64,
    pub escalation_cost_per_task: f64,
    pub escalation_rate: f64,
    pub cycles_per_week: f64,
    pub assisted_tasks_per_week: f64,
    pub escalation_hours_per_week: f64,
    pub agent_hours_per_week: f64,
    pub tasks_per_week: f64,
    pub tasks_low: f64,
    pub tasks_high: f64,
    pub streams_star: Option<f64>,
    pub a_star_hours: Option<f64>,
    pub util_pct: Option<f64>,
    pub monthly_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioPick {
    pub name: String,
    pub row_index: usize,
    pub tasks_per_week: f64,
    pub selected_tasks_per_week: f64,
    pub attempt_limit: usize,
    pub capacity_shortfall: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub row_index: usize,
    pub competence: Option<f64>,
    pub competence_floor: Option<f64>,
    pub capacity_shortfall: f64,
    pub minutes_per_task: f64,
    pub cost_per_task: f64,
    pub attempt_limit: usize,
    pub model_cost_per_task: f64,
    pub escalation_cost_per_task: f64,
    pub escalation_rate: f64,
    pub cycles_per_week: f64,
    pub assisted_tasks_per_week: f64,
    pub escalation_hours_per_week: f64,
    pub agent_hours_per_week: f64,
    pub tasks_per_week: f64,
    pub tasks_low: f64,
    pub tasks_high: f64,
    pub streams_star: Option<f64>,
    pub a_star_hours: Option<f64>,
    pub util_pct: Option<f64>,
    pub monthly_price: Option<f64>,
    pub scenarios: Vec<ScenarioPick>,
    pub top: Vec<CandidatePick>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatTierPick {
    pub seat: Seat,
    pub tier: Tier,
    pub pick: Option<Pick>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontierPoint {
    pub competence_floor: f64,
    pub row_index: usize,
    pub competence: Option<f64>,
    pub attempt_limit: usize,
    pub tasks_per_week: f64,
    pub assisted_tasks_per_week: f64,
    pub escalation_hours_per_week: f64,
    pub tasks_low: f64,
    pub tasks_high: f64,
    pub capacity_shortfall: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedCandidate {
    pub row_index: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatTierFrontier {
    pub seat: Seat,
    pub tier: Tier,
    pub points: Vec<FrontierPoint>,
    pub excluded: Vec<ExcludedCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    #[serde(default)]
    pub portfolio: Option<crate::portfolio::Portfolio>,
    pub picks: Vec<SeatTierPick>,
    #[serde(default)]
    pub frontiers: Vec<SeatTierFrontier>,
    #[serde(default)]
    pub plan_comparisons: Vec<crate::comparison::PlanComparison>,
    pub rows: Vec<Row>,
    #[serde(default)]
    pub research_tiers: Vec<crate::portfolio::ResearchTierPick>,
    #[serde(default)]
    pub evidence_catalog: crate::evidence::EvidenceCatalog,
    pub generated_at: String,
    pub source_fetched_at: String,
    pub cache_state: CacheState,
}

impl Table {
    pub fn get_pick(&self, seat: Seat, tier: Tier) -> Option<&Pick> {
        self.picks
            .iter()
            .find(|p| p.seat == seat && p.tier == tier)
            .and_then(|p| p.pick.as_ref())
    }

    pub fn empty() -> Self {
        Self {
            portfolio: None,
            picks: Vec::new(),
            frontiers: Vec::new(),
            plan_comparisons: Vec::new(),
            rows: Vec::new(),
            research_tiers: Vec::new(),
            evidence_catalog: crate::evidence::EvidenceCatalog::default(),
            generated_at: String::new(),
            source_fetched_at: String::new(),
            cache_state: CacheState::Baked,
        }
    }
}

fn benchmark_key(benchmark: Benchmark) -> &'static str {
    match benchmark {
        Benchmark::Swe => "swe",
        Benchmark::Terminal => "terminal",
        Benchmark::Qna => "qna",
        Benchmark::Gpqa => "gpqa",
        Benchmark::Hle => "hle",
        Benchmark::Lcr => "lcr",
        Benchmark::Omniscience => "omniscience",
    }
}
