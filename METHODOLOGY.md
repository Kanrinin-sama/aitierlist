# Methodology

aitierlist makes an independent recommendation for each role and plan tier. It separates role competence from operating capacity and automatically selects the candidate and retry cap with the most nominal verified reference tasks per week, with a band across a declared finite set of operating scenarios. Optional competence minimums enforce hard qualification requirements. Agent hours and vendor allowances are not allocated across a shared roster.

## Role competence

Competence is a fixed weighted utility on a zero-to-one scale. It is not a probability that a task succeeds and is never passed into the retry calculation.

Let `coding = (113 DeepSWE + 89 Terminal-Bench) / 202`, `reasoning = (GPQA + HLE) / 2`, and `index` be the normalized intelligence index. The role utilities are:

```text
Implementer   = coding
Debugger      = (coding + reasoning) / 2
Reviewer      = (Repository Q&A + reasoning) / 2
Orchestrator  = (GPQA + index) / 2
Sanity        = Repository Q&A
Comprehension = (LCR + Repository Q&A) / 2
```

These fixed profiles encode the role judgment. The engine does not fit a preference function from the current candidate pool. A candidate with a missing required capability has no competence value for that role.

Each role also has an optional competence minimum in settings. When set, candidates with unknown competence or a score below the minimum are excluded before automatic selection. When unset, missing competence does not exclude a candidate. Competence is displayed and breaks ties after nominal throughput and worst scenario capacity shortfall. The optional competence-minimum choices show the selected policy at each available score threshold; the list is a threshold decision curve rather than a claim that every displayed row is strictly Pareto-efficient.

The engine keeps the fixed weights because it has no preference observations from which to fit a robust ordinal regression family. Robust ordinal regression describes how a compatible preference family could be added when such observations exist: [Greco, Mousseau, and Słowiński (2008)](https://www.lamsade.dauphine.fr/mcda/biblio/PDF/GMS-EJOR2008.pdf). Treating competence as a constraint on an economic objective implements the epsilon-constraint method: [Mavrotas (2009)](https://www.sciencedirect.com/science/article/pii/S0096300309002574).

## Reference workloads

Operating capacity is measured in reference tasks completed autonomously by the agent per week. Implementer and Debugger use the coding reference workload:

```text
DeepSWE        113 / 202
Terminal-Bench  89 / 202
```

Reviewer, Orchestrator, Sanity, and Comprehension use Repository Q&A as their reference workload. Autonomous reference capacity is a common economic output unit supported by the observed agent data. It is not an estimate of completed real-world reviewer, orchestrator, sanity, or comprehension jobs.

The reference workload supplies first-attempt pass rate, tokens, vendor usage, and time. Direct resource time is token-proportional and anchored to pooled observed suite wall time. A second resource-time basis assigns the pooled suite time to each task. Both bases are evaluated as declared operating scenarios. Aggregated competence utility never supplies retry probabilities; raw reference-benchmark outcomes do.

## Retries and escalation

For first-attempt reference success probability `p`, retry failure correlation `rho`, and attempt `k`, conditional success is

```text
p_k = p / (1 + (k - 1) rho / (1 - rho))
```

`rho = 0` gives independent attempts. Let `F_0 = 1`, `F_k = F_(k-1) (1 - p_k)`, and `A_n = sum(F_0 ... F_(n-1))`. At attempt cap `n`:

```text
wall time     = A_n t + F_n E_time
vendor usage  = A_n c
external cost = F_n E_usd
```

Attempts stop on success. A reference task unfinished at the cap is completed through the configured rescue time and external cost. That assisted completion consumes workflow time but is not credited as an agent completion. Caps from 1 through 64 are considered.

For weekly workflow seconds `H`, vendor allowance `B`, expected combined agent-and-rescue seconds `T`, and vendor usage `C` per cycle, production is

```text
cycle capacity       = min(H / T, B / C)
agent completions    = cycle capacity (1 - F_n)
assisted completions = cycle capacity F_n
```

Zero vendor usage leaves cycles time-limited. API tiers have unbounded vendor allowance. A ranged built-in allowance uses its midpoint for the nominal result and both endpoints in the scenario set. Direct USD vendor budgets pay vendor usage; external rescue cost is shown separately and does not consume the vendor allowance. Fixed call allowances are converted with pooled observed USD per step. A fixed USD override replaces the built-in allowance. Time and cost figures shown per cycle include the expected rescue branch; the interface separately reports agent completions, assisted completions, agent hours, and rescue hours.

## Automatic nominal throughput selection

The engine evaluates every eligible candidate and every cap from 1 through 64 under the nominal assumptions and 66 deterministic operating scenarios formed from:

- low and high runtime;
- low and high model cost;
- low and high retry failure correlation;
- low and high escalation time;
- both endpoints of a ranged allowance;
- both reference resource-time bases.

The configured assumption distance sets the low and high stress values for runtime, cost, retry correlation, and rescue time. It is a modeling choice, not measured uncertainty or a confidence interval. The 66 scenarios are the declared finite set, not measurements, a causal model, or a guarantee over every value in a continuous uncertainty region.

For policy `a`, consisting of an eligible candidate and retry cap from 1 through 64, the nominal result uses token-proportional reference time, the configured retry and rescue assumptions, and the midpoint of any allowance range:

```text
nominal tasks/week(a) = min(H / T, B / C) (1 - F_n)
selected policy      = arg max_a nominal tasks/week(a)
```

For API, cycle capacity is `H / T`. Zero vendor usage leaves capacity time-limited. The low/high band is the minimum and maximum autonomous capacity of the same policy across all 66 scenarios: two baseline time bases plus 32 joint endpoint combinations for each basis. The per-scenario list remains available.

An exact nominal tie prefers smaller worst-case proportional capacity shortfall across the scenarios, then higher known competence, the shorter cap, and the existing canonical display name, harness, and effort order. Unknown competence sorts below known competence at that tie-break. Per-scenario capacity shortfall is `1 - policy capacity / best eligible capacity` (zero when the best is zero). These per-scenario comparator values are displayed diagnostics; only their maximum is used to break an exact nominal tie. Competence shortfall and combined loss are not selection objectives. Competence is displayed and an optional per-seat minimum excludes unknown or insufficient competence before selection.

Seat eligibility requires the reference-workload rewards, tokens, and time consumed by the capacity calculation: DeepSWE and Terminal-Bench for Implementer and Debugger, and Repository Q&A for Reviewer, Orchestrator, Sanity, and Comprehension. Missing GPQA, HLE, LCR, or intelligence index leaves any dependent competence unknown, without excluding the row unless that seat has a competence minimum. A zero or missing coding pass excludes Implementer and Debugger but does not exclude the Q&A-workload seats. No missing competence value is imputed. The pooled resource anchors also require token inputs from all three suites; missing non-workload rewards alone do not exclude a row.

Rows with `(none)` effort are excluded. Claude Code configurations for GLM-5.1, GLM-5.2, and Qwen3.8 Max are excluded from subscription plans. The built-in allowance dollar values in `engine.rs` are declared modeling inputs, not vendor-published figures.

## Plan and counterfactual comparisons

Plan comparisons use only the selected subscription policy at each tier. Every pair reports the actual configured monthly prices, raw competence values, and autonomous production over the same 66 scenarios. A cheaper plan is called dominant only when its price is known and it is no worse in competence or any scenario. An override without a known monthly subscription price remains price unknown; it is never treated as free or cheaper.

The counterfactual report changes one input for one chosen model configuration at a time while every competitor remains unchanged. It evaluates 24 bounded, logarithmically spaced changes to agent-attempt runtime, vendor-usage cost, and configuration-specific allowance, recomputing retry caps, nominal throughput, and the full 66-scenario band and tie-break each time. When a sampled point wins, it refines the first sampled losing-to-winning bracket and returns a verified sufficient winning change. This is not proof of the globally smallest winning change because candidate, retry-cap, and scenario tie-break changes can make winning regions non-monotonic. If no sampled point wins, the report says no winning change was found at the sampled reductions or allowances; it does not claim that winning is impossible. These results are not measurements, causal estimates, or predictions that changing a shared vendor plan would leave competitors unchanged.

## Inputs

DeepSWE, Terminal-Bench v2.1, and SWE-Atlas Repository Q&A outcomes are raw per-evaluation means. Missing or invalid reference-workload outcomes exclude the candidate from the seats that require them. Direct cost partitions gross input tokens into uncached, cache-read, and cache-write categories, prices them separately, and anchors the weighted suite to pooled observed cost. When applicable model prices are absent, cost uses the pooled observed value. A zero-output suite uses pooled observed time. The interface labels the resource basis.

Matched GPQA, HLE, LCR, and intelligence-index values contribute only to competence. Artificial Analysis normalization divides canonical totals by unique task counts where task-level display values are needed. Hallucination is an optional informational indicator and does not alter competence, retries, or capacity. Missing hallucination data remains Unknown.
