# Upstream benchmark ingestion

The source review used four native clients:

- Claude Opus 4.6, high effort
- Grok 4.6, xhigh effort
- Gemini 3.8 Flash through Antigravity, high effort
- Muse Spark through Muse Code; the client did not expose an exact model version

## Machine-readable sources

| Series | Published source | Identity and available evidence |
|---|---|---|
| DeepSWE v1.1 | [aggregate](https://deepswe.datacurve.ai/artifacts/v1.1/leaderboard-live.json), [trials](https://deepswe.datacurve.ai/artifacts/v1.1/trials.json), [tasks](https://deepswe.datacurve.ai/artifacts/v1.1/tasks.json) | 113 tasks. Aggregate rows identify model, mini-swe-agent harness, configuration, reasoning effort, pass@1/pass@4, confidence interval, and mean/median cost, token, duration, and step fields. Trial and task files remain raw provenance; individual trial outcomes are not model-level scores. |
| DeepSWE v1 | [aggregate](https://deepswe.datacurve.ai/artifacts/v1/leaderboard-live.json), [trials](https://deepswe.datacurve.ai/artifacts/v1/trials.json), [tasks](https://deepswe.datacurve.ai/artifacts/v1/tasks.json) | Separate dataset series. Scores and trials are retained independently from v1.1. |
| Terminal-Bench v4 | [current leaderboard](https://github.com/harbor-framework/terminal-bench/tree/main/leaderboard), [pinned example](https://github.com/harbor-framework/terminal-bench/tree/83c7a6172d629c6575b785ab12c8db787bb2e323/leaderboard) | 66 tasks. Submission records contain exact agent, model, effort, accuracy, repeated-attempt metrics, trial counts, cost, token, duration, run, and source-filter metadata. |
| Terminal-Bench v2.1 | [current leaderboard](https://github.com/harbor-framework/terminal-bench-2-1/tree/main/leaderboard), [pinned example](https://github.com/harbor-framework/terminal-bench-2-1/tree/7131e4375048a0e408a8fb404b5f499d726b695b/leaderboard) | 89 tasks. It remains a separate series from v4 because the task set and protocol version differ. |
| Terminal-Bench Science v0.1 | [benchmark](https://www.terminal-bench-science.ai/), [Harbor repository](https://github.com/harbor-framework/terminal-bench-science) | Science workflow tasks form the independent `terminal-science` family; worker Terminal-Bench scores do not substitute for it. A repository without the published leaderboard layout produces an unavailable source status. |
| SWE-Atlas Q&A | [repository and harness](https://github.com/scaleapi/SWE-Atlas), [dataset](https://huggingface.co/datasets/ScaleAI/SWE-Atlas-QnA), [leaderboard](https://labs.scale.com/leaderboard/sweatlas-qna) | 124 repository-comprehension tasks. The tasks and Harbor-format harness are public. The public leaderboard supplies model-level results; no public per-trial repeat feed was confirmed. |
| MCP Atlas | [leaderboard](https://labs.scale.com/leaderboard/mcp_atlas) | Scale's MCP leaderboard is ingested as the independent `mcp-atlas` family. Published model, harness, protocol, grader, effort, and score fields remain attached to the source observation. |
| Humanity's Last Exam | [repository](https://github.com/centerforaisafety/hle), [project](https://lastexam.ai/) | The app's Artificial Analysis observation is the 2,158-question text-only evaluation. It is distinct from the complete multimodal benchmark. |
| HLE-Rolling | [project](https://agi.safe.ai/) | No confirmed public model-results feed is available. Refresh records an explicit unavailable status and does not substitute original HLE results. |
| GPQA | [repository](https://github.com/idavidrein/gpqa), [paper](https://arxiv.org/abs/2311.12022) | The app uses the 198-question Diamond subset. Dataset access is gated; upstream does not publish a current model-results feed. |
| AA-LCR | [dataset](https://huggingface.co/datasets/ArtificialAnalysis/AA-LCR), [methodology](https://artificialanalysis.ai/methodology/intelligence-benchmarking) | 100 long-context questions. Dataset versions remain distinct; model results arrive through Artificial Analysis. |
| AA Omniscience | [evaluation](https://artificialanalysis.ai/evaluations/omniscience), [public subset](https://huggingface.co/datasets/ArtificialAnalysis/AA-Omniscience-Public) | Accuracy, hallucination rate, attempt rate, and the maintainer index are separate fields. The public subset has 600 questions; the evaluated set has 6,000. |
| GDP.pdf | [Surge leaderboard](https://surgehq.ai/benchmarks/gdp-pdf), [repository](https://github.com/surge-ai/gdp-pdf) | 100 document tasks, five runs per task, no tools, Gemini 3.5 Flash judge. Surge's raw-PDF evaluation remains distinct from Artificial Analysis's LiteParse/image implementation and GPT-5.6 Luna judge. |
| Epoch benchmark data | [public archive](https://epoch.ai/data/benchmark_data.zip) | The ZIP supplies `benchmark_metadata.csv` plus benchmark result CSVs. Metadata selects each source file and score column; observations retain the published model, organization, effort, harness, protocol, grader, benchmark version, task count, source file, and raw row. Score values retain their native published units. GPQA and HLE mirrors declare a 0–1 scale; other families do not receive an inferred scale. Epoch-run or mirrored observations retain Epoch provenance and do not become maintainer-native results. |

## Source rules

`SourceRef.revision` identifies the fetched artifact revision. Benchmark versions live in `BenchmarkSeries`; a fetch timestamp is not a benchmark version or measurement date. Observation identities include the source revision and canonical model, effort, harness, and configuration identity.

Scores and resource bundles remain atomic to one observation. Missing cost, tokens, or duration stay unknown. A newer score without a matching resource bundle does not inherit resources from another harness or dataset version.

DeepSWE v1 and v1.1, Terminal-Bench v2.1 and v4, Terminal-Bench Science, Surge GDP.pdf, and the Artificial Analysis GDP.pdf implementation are independent series. Model display names, execution hosts, model vendors, harness IDs, and reasoning effort are retained as distinct provenance fields.

Published repeated-attempt evidence is eligible only when dataset version, model, effort, and harness match. SWE-Atlas Q&A, HLE, GPQA, LCR, and Omniscience have no confirmed public model-level repeat feed for automatic retry fitting.

Harbor refresh resolves each repository's current default branch to a commit SHA, then fetches the root leaderboard submissions, run metadata, and leaderboard configuration at that pinned revision. Nested archive leaderboards are not current observations.

The established coding, repository-Q&A, conductor, and research benchmark families continue to drive their matching app roles. Newly ingested sibling families and extra score columns remain provenance-rich diagnostics until a role projection explicitly names their series, harness, metric, and transfer policy.

## Available sibling feeds

- [SWE-Atlas Test Writing](https://labs.scale.com/leaderboard/sweatlas-tw) and [Refactoring](https://labs.scale.com/leaderboard/sweatlas-refactoring) publish separate leaderboard series and public tasks through the SWE-Atlas repository.
- [Surge Chartography](https://surgehq.ai/benchmarks/chartography) and [Hemingway Bench](https://surgehq.ai/benchmarks/hemingway-bench) publish separate HTML leaderboard tables. Their scores are not GDP.pdf observations.
- Artificial Analysis publishes model-level diagnostics for HLE, LCR, Omniscience, GDP.pdf, GPQA, Terminal-Bench, DeepSWE, SWE-Atlas Q&A, SciCode, CritPt, Briefcase, GDPval-AA, and AutomationBench-AA through its model/evaluation payloads. Each field retains its own benchmark and harness identity.

## Unavailable evidence

- No public per-trial repeat feed was confirmed for SWE-Atlas Q&A, HLE, GPQA, AA-LCR, or AA Omniscience.
- The complete AA Omniscience evaluation set and several Artificial Analysis agentic benchmarks are private.
- Scale leaderboard scores are web-published; a stable bulk results JSON endpoint was not confirmed.
- Surge leaderboard pages publish scores without matching per-model cost, token, or duration bundles.
