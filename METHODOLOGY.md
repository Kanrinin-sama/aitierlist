# Methodology

aitierlist makes an independent recommendation for each role and plan tier. It separates role competence from operating capacity and automatically selects the candidate and retry cap with the smallest worst proportional shortfall across competence and a declared finite set of operating scenarios. Optional competence minimums enforce hard qualification requirements. Agent hours and vendor allowances are not allocated across a shared roster.

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

For each role and tier, the best available competence is the maximum fixed competence among the feasible candidates. A policy's proportional competence shortfall is

```text
competence shortfall = 1 - policy competence / best available competence
```

A zero best competence produces zero competence shortfall. The zero point is fixed by the source scale; the engine does not normalize scores between the worst and best candidates in the current pool.

Each role also has an optional competence minimum in settings. When set, candidates below the minimum are excluded before automatic selection. When unset, competence and capacity both participate in the automatic choice. The optional competence-minimum choices show the selected policy at each available score threshold; the list is a threshold decision curve rather than a claim that every displayed row is strictly Pareto-efficient.

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

## Automatic competence–capacity balance

The engine evaluates every eligible candidate and every cap from 1 through 64 under the nominal assumptions and 66 deterministic operating scenarios formed from:

- low and high runtime;
- low and high model cost;
- low and high retry failure correlation;
- low and high escalation time;
- both endpoints of a ranged allowance;
- both reference resource-time bases.

The configured assumption distance sets the low and high stress values for runtime, cost, retry correlation, and rescue time. It is a modeling choice, not measured uncertainty or a confidence interval. The 66 scenarios are the declared finite set, not measurements, a causal model, or a guarantee over every value in a continuous uncertainty region.

For policy `a`, consisting of a candidate and cap fixed before the scenario is known, and scenario `s`, proportional capacity shortfall is

```text
best capacity(s)       = max_b autonomous capacity(b, s)
capacity shortfall(a,s)= 1 - autonomous capacity(a,s) / best capacity(s)
worst capacity loss(a) = max_s capacity shortfall(a,s)
combined loss(a)       = max(competence shortfall(a), worst capacity loss(a))
selected policy        = arg min_a combined loss(a)
```

If a scenario's best capacity is zero, every policy has zero capacity shortfall in that scenario. Best competence and best capacity are computed separately and can be attained by different candidates. The scenario comparator `b` may choose its hindsight-best eligible candidate and cap.

The selected policy protects competence and capacity equally in percentage terms. This equal proportional protection is a chosen decision rule, not an empirical fact. It is a discrete adaptation of proportional compromise ideas associated with bargaining solutions, not a claim that the model-selection problem is a classical bargaining problem: [European Central Bank Working Paper 1359](https://www.ecb.europa.eu/pub/pdf/scpwps/ecbwp1359.pdf). The finite-scenario worst-loss treatment follows minimax-regret decision methods for imprecise utility: [Boutilier et al. (2006)](https://www.cs.toronto.edu/~cebly/Papers/_download_/BPPS-aij06.pdf).

Ties prefer the smaller combined loss, then the smaller sum of competence shortfall and worst capacity shortfall, higher nominal reference capacity, higher competence, the shorter cap, then canonical display name, harness, and effort order.

## Plan and counterfactual comparisons

Plan comparisons use only the selected subscription policy at each tier. Every pair reports the actual configured monthly prices, raw competence values, and autonomous production over the same 66 scenarios. A cheaper plan is called dominant only when its price is known and it is no worse in competence or any scenario. An override without a known monthly subscription price remains price unknown; it is never treated as free or cheaper.

The counterfactual report changes one input for one chosen model configuration at a time while every competitor remains unchanged. It evaluates 24 bounded, logarithmically spaced changes to agent-attempt runtime, vendor-usage cost, and configuration-specific allowance, recomputing retry caps, ideal competence and capacity points, and the full 66-scenario decision each time. When a sampled point wins, it refines the first sampled losing-to-winning bracket and returns a verified sufficient winning change. This is not proof of the globally smallest winning change because candidate, retry-cap, and ideal-point changes can make winning regions non-monotonic. If no sampled point wins, the report says no winning change was found at the sampled reductions or allowances; it does not claim that winning is impossible. These results are not measurements, causal estimates, or predictions that changing a shared vendor plan would leave competitors unchanged.

## Inputs

DeepSWE, Terminal-Bench v2.1, and SWE-Atlas Repository Q&A outcomes are raw per-evaluation means. Missing or invalid required raw outcomes exclude the candidate. Direct cost partitions gross input tokens into uncached, cache-read, and cache-write categories, prices them separately, and anchors the weighted suite to pooled observed cost. When applicable model prices are absent, cost uses the pooled observed value. A zero-output suite uses pooled observed time. The interface labels the resource basis.

Matched GPQA, HLE, LCR, and intelligence-index values contribute only to competence. Artificial Analysis normalization divides canonical totals by unique task counts where task-level display values are needed. Hallucination is an optional informational indicator and does not alter competence, retries, or capacity. Missing hallucination data remains Unknown.
