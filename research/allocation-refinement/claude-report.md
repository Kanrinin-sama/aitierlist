# Allocation Refinement Audit Report

**Client**: Claude Code native CLI
**Model**: Claude Opus 4.6 (`claude-opus-4-6`)
**Requested effort**: high — full source read of 8 named source files, 4 audit JSONs, 2 design documents
**Review-source revision**: `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0`
**Web provenance**: The native review used WebFetch and WebSearch for the cited primary sources; a source-verification follow-up checked the report's claims against the named repository revision and cited literature.

**Case parameters**: H=112, N=4, forecast=112, five selected subscriptions (OpenAI Codex Pro 200, Anthropic Claude Max 20×, Muse Power 10×, Google AI Ultra 20×, xAI SuperGrok Heavy).

---

## Finding 1 — Proxy demand scales by N without scaling capacity

**Verdict: CONFIRMED**

`portfolio.rs:1157`: `forecast = (settings.agent_hours * settings.orchestrators as f64 / 4.0).ceil() as usize`. Pool capacity at `portfolio.rs:1176-1178` uses `capacity(provider, plan) * settings.subscription_count(provider.id) as f64`, which is independent of `orchestrators`. Setting N=4 forecasts 112 jobs against the same pool capacities that would serve N=1's 28-job forecast. The 19/112 admission ratio is the diagnostic signal.

**Counterexample**: None. The code path is unambiguous.

**Severity**: High.

**Refinement**: Display the admitted/forecast ratio prominently. When a user increases N, show the admission ceiling at N=1 alongside the multi-project forecast. The threshold at which to warn is a user preference, not derivable from the model — present it as a configurable setting with no default.

---

## Finding 2 — Lexicographic admission precedes route quality

**Verdict: CONFIRMED**

`solver.rs:209-223`: The simplex objectives are lexicographically ordered as `[coverage, quality, -time, -cost]`. The admission search at `portfolio.rs:1498-1543` binary-searches on admitted count, maximizing it before the solver optimizes quality within each count. A stronger conductor consuming more of the shared pool can reduce the admissible prefix. The solver correctly prefers the weaker conductor that admits 19 over one that admits 17, because admission is lexicographically prior.

Note: this is the root cause of Finding 3's observed behavior. The conductor's quality *does* enter the solver objective (see Finding 3 correction), but it is subordinate to admission count.

**Severity**: Medium. Correct per specification, but the Pareto tradeoff between admission count and route quality is invisible.

**Refinement**: Solve for multiple conductor choices and report the admission-count vs. aggregate-quality Pareto frontier so the user can choose their operating point.

---

## Finding 3 — Baseline conductor is the cheapest that maximizes admission

**Verdict: CONFIRMED as observed behavior; CORRECTED on mechanism**

The original report claims "The current formulation charges the conductor's resources but gives zero quality credit for conductor intelligence." This is wrong. At `portfolio.rs:1463`, `utility: competence` sets the orchestrator policy's utility to its competence score. At `portfolio.rs:866`, `Choice { quality: expected_factor * policy.utility, ... }` includes this utility in the solver's quality objective. Conductor quality already enters every search `Choice`.

The reason a low-competence conductor wins is Finding 2: admission count is lexicographically prior. A cheap conductor admits more jobs, saturating the first objective. Only within the same admission count does quality (including conductor quality) optimize. The report's proposed refinement to "add a small quality term for the conductor" is unnecessary — the term already exists.

**Severity**: Low for the mechanism (it works as designed). Medium for the user-facing effect (unintuitive that upgrading the conductor can reduce admitted work).

**Refinement**: The Pareto frontier from Finding 2 already addresses this. No additional quality term is needed. Label each operating point with its conductor identity and competence so the user sees the tradeoff.

---

## Finding 4 — Floor comparisons confound composition changes

**Verdict: CONFIRMED**

Each orchestrator floor selects a different conductor, which changes shared account consumption, which changes feasible worker routes, which changes the entire allocation. The audit data:

| Floor | Conductor | Admitted | Quality | Bound | Gap |
|---|---|---:|---:|---:|---:|
| none | Sol low (0.338) | 19 | 67.17 | 74.99 | 11.64% |
| 0.338 | (same or next) | 17 | 69.66 | 70.90 | 1.77% |
| 0.500 | Astra xhigh | 17 | 70.07 | 70.90 | 1.18% |
| 0.527 | Astra max | 16 | 63.23 | 63.76 | 0.85% |

These are four complete team plans, not four conductor choices with workers held constant.

**Severity**: Medium.

**Refinement**: Label each floor comparison as a complete replan. Report the Pareto frontier of (admitted, quality, conductor competence) triples.

---

## Finding 5 — Conductor competence definitions diverge between Absolute and portfolio views

**Verdict: CONFIRMED**

`engine.rs:68-74`: Absolute Orchestrator competence = `(GPQA, 0.5) + (Intelligence Index, 0.5)`.
`portfolio.rs:1441-1442`: Portfolio conductor competence = `row.smart` (Intelligence Index alone).

These measure different things. A model scoring well on GPQA but poorly on agentic benchmarks ranks high in Absolute but low in portfolio.

**Severity**: High.

**Refinement**: Unify the conductor competence definition. Since GPQA is no longer in the Intelligence Index (removed from II v4.2+ as saturated), either: (a) use II alone for both views, or (b) define a dedicated conductor competence with explicit, documented weights used in both views. The choice is a policy preference; document it as such.

---

## Finding 6 — Conductor resource model uses 2 benchmark-task-equivalent units, not 2 full suites

**Verdict: CONFIRMED**

`portfolio.rs:825-826`: Orchestrator visits = 2.0 per change. `engine.rs:470-486` returns per-call costs from `orchestrator_usd` and `orchestrator_seconds` — one benchmark-task-equivalent unit. The methodology documentation is already explicit.

**Severity**: Low. Already documented.

**Refinement**: No code change needed. Consider a tooltip in the UI clarifying "benchmark-task-equivalent units."

---

## Finding 7 — Static nine-stage proxy reserves every seat for every admitted job

**Verdict: CONFIRMED**

`portfolio.rs:65-129`: All 9 stages with fixed dependencies. `RESERVED_VISITS = [1, 1, 2, 2, 2, 1, 0]` applies uniformly regardless of workflow class.

Meanwhile, `team_policy.rs:120-235` generates visits dynamically. Focused work (`team_policy.rs:125`): `let checks = !matches!(template, WorkClass::Focused)` — skips Reviewer, Sanity, and Debugger. Comprehension is only added for Complex/Extensive (`team_policy.rs:124`). A Focused job needs Orchestrator(2) + Implementer(1) = 3 visits; the proxy reserves 9.

**Severity**: Medium. The proxy overstates resource demand for Focused work (50% of the forecast under the default 50/30/15/5 mix).

**Refinement**: Parameterize the proxy visit template by workflow class, mirroring the native scheduler's actual structure:
- Focused: Orch(2), Impl(1) = 3 reserved visits
- Standard: Orch(2), Impl(1), Reviewer(2), Sanity(2), Debugger(1) = 8
- Complex/Extensive: add Comprehension(1) = 9

---

## Finding 8 — Net Research budget is outside the proxy allocation

**Verdict: CONFIRMED**

`portfolio.rs:8-9`: `EXPECTED_VISITS[6] = 0.0`, `RESERVED_VISITS[6] = 0`. Net Research has no solver group and no account capacity is reserved. The native scheduler (`team_policy.rs:122-123`) generates Net Research visits when `risk.evidence_need > 0`, consuming real account budget at runtime.

**Severity**: Medium.

**Refinement**: Add a configurable expected-evidence fraction to the proxy's account constraints. The fraction should come from measured task-level evidence demand when telemetry exists, or be presented as an explicit user-set parameter with no hardcoded default. Do not invent a percentage.

---

## Finding 9 — No monotonic competence enforcement across workflow classes

**Verdict: CONFIRMED**

`portfolio.rs:787-800`: Independent groups per (seat, class) pair. No constraint links competence across classes. Resource pressure can force a weaker route onto a higher class.

**Counterexample**: In practice, the same route often wins multiple classes because the solver maximizes aggregate quality. A non-monotonic assignment is chosen only when it is the best feasible allocation. Adding a hard monotonicity constraint could reduce total quality.

**Severity**: Low.

**Refinement**: Add an optional post-solve diagnostic. Flag any class where the selected route has lower competence than a lower class for the same seat. If the user enables a monotonicity preference, add linking constraints per seat. This is an optional policy setting, not a default.

---

## Finding 10 — Review burden uses fixed visit counts

**Verdict: CONFIRMED, but the original report's refinement is redundant**

`portfolio.rs:8`: `EXPECTED_VISITS = [1.0, 0.25, 1.25, 2.0, 1.25, 1.0, 0.0]`. These are fixed policy parameters from the declared 25% repair incidence.

The original report's refinement suggests "use the class resource factors (0.25/1/2/4) to scale review expected visits." However, class factors already scale everything. At `portfolio.rs:830-834`:
```
let expected_factor: f64 = demands.iter()
    .map(|(changes, factor)| changes * factor * visits)
    .sum();
```
where `factor = class.resource_factor()` and `visits = EXPECTED_VISITS[group.role]`. The expected factor, nominal cost, reserved resources, and quality are all already class-factor-scaled.

What the fixed visit counts lack is calibration from real gate outcomes — the 0.25 repair incidence is a policy constant, not a measured distribution.

**Severity**: Low.

**Refinement**: When telemetry from real gate outcomes accumulates, replace the fixed 0.25 repair incidence with per-class conditional distributions derived from measured data. No code change is needed now beyond the telemetry collection infrastructure.

---

## Finding 11 — Coding reference mix and AA Coding Agents index are distinct

**Verdict: CONFIRMED**

`engine.rs:25-28`: 113 DeepSWE + 89 Terminal-Bench = 202 tasks, weights 113/202 and 89/202.

AA Coding Agents Index v1.5: three equally weighted benchmarks — DeepSWE v1.1 (113 tasks), Terminal-Bench v4.0 (66 tasks), SWE-Atlas Q&A (124 tasks).

Key discrepancies:
1. The app excludes SWE-Atlas Q&A from coding competence (it appears as the separate Repository Q&A reference workload — `engine.rs:42`, `engine.rs:98-101`).
2. The app uses 89 Terminal-Bench tasks (from v2.1 retry data) vs. AA's 66 (v4.0).
3. The app's weights are task-count proportional vs. AA's equal benchmark weights.

**Severity**: Medium. The app's coding reference is a legitimate policy choice, but users comparing to AA's rankings will see different orderings.

**Refinement**: Document the version and task-set differences in the methodology. When Terminal-Bench v4.0 retry data becomes available, add it as a configurable retry source. The coding reference weights are a policy choice that should remain explicitly labeled as such.

---

## Finding 12 — Modeled API-equivalent quotas do not prove native capacity

**Verdict: CONFIRMED**

`portfolio.rs:1180`: `unit: "estimated API-equivalent USD"`. Pool capacities derive from `monthly_allowance_low * 12/52` (`engine.rs:499`). Native meters — premium requests, token quotas, session limits, rolling windows — are completely separate. The native scheduler correctly uses `native_capacity` from actual ledger snapshots, not the proxy's estimates. The two accounting systems are intentionally separate.

**Severity**: High for a user who treats the proxy plan as a runtime guarantee. Low for the system design, which correctly separates the two.

**Refinement**: In the UI, label the proxy allocation table with a persistent qualifier: "Modeled API-equivalent proxy — native admission and holds operate in native units via the ledger." Flag tightly reserved accounts by their remaining fraction rather than against an arbitrary percentage threshold — the tightness is visible in the data and the appropriate concern level is a user judgment.

---

## Cross-cutting observations

### Bertsimas-Sim uncertainty model — PARTIALLY CORRECTED

The original report correctly identifies that the current proxy uses a uniform `assumption_span_pct` stress factor (`engine.rs:278-299`) producing 66 scenarios as a fixed-endpoint grid, not a Γ-budgeted uncertainty set.

However, the report adds: "which violates the paper's principle that fewer observations should carry wider uncertainty." This is unsupported. [Bertsimas and Sim (2004)](https://pubsonline.informs.org/doi/10.1287/opre.1030.0065) define uncertainty sets with intervals [a_j - â_j, a_j + â_j] where â_j is the uncertainty width, and Γ bounds how many parameters simultaneously deviate. The paper leaves â_j as a modeling choice. It does not require â_j to be an inverse function of observation count. Making width observation-dependent is a reasonable modeling decision, but it is a design preference, not a paper requirement.

**Refinement**: To apply Bertsimas-Sim, fit per-route uncertainty intervals from the best available evidence (sample variance when telemetry exists, or wider declared intervals for uncalibrated routes), then apply the Γ-budget to select how many routes simultaneously hit their worst case. Present the Γ parameter as a user-controllable conservatism knob.

### Correlated errors — CONFIRMED

[Kim et al. (ICML 2025)](https://proceedings.mlr.press/v267/kim25e.html) found 60% error agreement between models and highly correlated errors across larger models.

`native_scheduler.rs:1023-1054`: `checker_correlated` enforces creator/checker separation only when `consequence >= 2 || correlation >= 2`. For Standard work with both dimensions below 2, the same model family can create and check. The proxy model has no correlation constraint.

**Refinement**: When the same model family serves both Implementer and Reviewer/Sanity for the same class in the proxy, consider a quality discount proportional to estimated error correlation. The discount magnitude should be set from measured creator/checker agreement rates when available, or presented as an explicit user parameter.

### Dominant Resource Fairness — CORRECTED

The original report claims "DRF is correctly implemented in the native scheduler's `fairness_cost`." This overstates.

`native_scheduler.rs:1252-1289`: The `fairness_cost` function computes each project's dominant share (max usage/endowment/weight across resources), then takes the max across projects. The `Score::compare` method (`native_scheduler.rs:104-113`) minimizes this cost lexicographically after admission count and deadlines. This is a dominant-share-*inspired* cost that penalizes the largest project share — not a full DRF allocation. DRF's theoretical properties (strategy-proofness, envy-freeness, sharing incentive, Pareto optimality from [Ghodsi et al., NSDI 2011](https://www.usenix.org/conference/nsdi11/dominant-resource-fairness-fair-allocation-multiple-resource-types)) are not claimed and do not necessarily hold.

The design document (`team-allocation.md`, line 113) itself states: "This is inspiration rather than a demand to equalize provider spending."

The proxy allocation has no fairness constraint — all projects share a single forecast pool.

**Refinement**: When N > 1, partition the proxy allocation by project entitlement weights using a dominant-share-inspired partition. Label it as "DRF-inspired" rather than "DRF" and do not claim DRF theorems.

### Google agent scaling research — CONFIRMED

[Google Research (2026)](https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/) found multi-agent coordination improves parallelizable tasks but can degrade sequential ones. The current N parameter multiplies demand without distinguishing parallelizable from sequential work.

**Refinement**: Separate the N parameter into concurrent parallelizable and sequential project counts, or document N as an upper bound on concurrent conductor sessions and not a demand multiplier for sequential work.

---

## Summary of severity and prioritized refinements

| # | Finding | Severity | Primary refinement |
|---|---|---|---|
| 1 | N scales demand, not capacity | High | Show admitted/forecast ratio; show N=1 ceiling |
| 5 | Conductor competence diverges between views | High | Unify to one definition; label as policy |
| 12 | Proxy quotas ≠ native capacity | High | Persistent UI qualifier; show remaining fraction |
| 2 | Lexicographic admission hides quality tradeoff | Medium | Report admission-vs-quality Pareto frontier |
| 3 | Cheapest conductor wins by admission priority | Low (corrected) | Addressed by Finding 2's Pareto frontier |
| 4 | Floor comparisons confound team composition | Medium | Label as complete replans |
| 7 | Static 9-stage proxy for all classes | Medium | Parameterize visits by workflow class |
| 8 | Net Research outside proxy budget | Medium | User-set evidence fraction; no default |
| 11 | Coding mix differs from AA index | Medium | Document version gap |
| 6 | 2 units ≠ 2 full suites | Low | Already documented |
| 9 | No monotonic competence across classes | Low | Optional post-solve diagnostic |
| 10 | Fixed review visit counts | Low | Calibrate from real gate telemetry |
