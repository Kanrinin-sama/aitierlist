# Allocation-refinement audit — 12 findings

## 1. Audit identity and scope

- Client: Muse Code native CLI; model: Meta Muse Spark; requested reasoning effort: high.
- Repo revision named by packet: `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0` (not re-verified here; all line cites are to the working tree as read).
- Saved case audited: `H=112` hours, `N=4` concurrent projects, one subscription per provider, 112 forecast proxy jobs.
- Current-tree artifact observed: [target/debug/audit-current-production.json](/projects/aitierlist/target/debug/audit-current-production.json:57821) (`admittedChanges: 19`, `forecastChanges: 112`, `deferredChanges: 93`); [target/debug/audit-current-production.json](/projects/aitierlist/target/debug/audit-current-production.json:60111) (`quality: 67.1672`, `relativeGap: 0.11640`, `status: feasible_search_limit`).
- Version drift warning: [research/team-allocation.md](/projects/aitierlist/research/team-allocation.md:29) describes a different baseline (112 admitted, quality 324.88, gap 11.04%). That is **not** the current 19-admitted artifact. Any claim that mixes the two numbers is invalid.
- Client/model/effort for the audited decision:
  - Current production conductor: `rowIndex 9`, Codex/OpenAI binding `model-95e6962b9d6651f4cad62a70`, competence = utility = `0.3380`, cap 1 ([audit lines 57772–57819](file:///projects/aitierlist/target/debug/audit-current-production.json)). Packet names it GPT-5.6 Sol low; the JSON confirms the Codex lane, II-cost basis, and 38/38 expected/reserved visits, but the display-name table was not in the JSON slice read, so the marketing name is taken from the packet, not independently re-resolved.
  - Floor comparison: `audit-orch-0.500.json` conductor competence `0.5251`, `rowIndex 7`, 34 visits, admits 17; `audit-orch-0.527.json` admits 16. Packet names these Astra xhigh / Astra max; JSON confirms the counts and the competence step, not the marketing names.
  - Effort semantics: effort is a harness/billing argument, not a calibrated capability level. Do not read “low beats max” as a capability ranking.

## 2. Web tools used and success

| Tool | Target | Result |
|---|---|---|
| `web_fetch` | https://artificialanalysis.ai/methodology/coding-agents-benchmarking | Success. Confirms Coding Agent Index v1.5 = equal-weight DeepSWE v1.1 + Terminal-Bench 4.0 + SWE-Atlas-QnA; per-page table: 113 / 66 / 124 tasks, 3 attempts each, pass@1. |
| `web_fetch` | https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3 | Success. Confirms Intelligence Index v4.3 = 10 evaluations incl. Terminal-Bench v4.0, AA-LCR v1.1, AA-Omniscience, HLE, GDP.pdf, CritPt; Astra max and Fable 5.1 max lead at 53. |
| `web_fetch` | https://artificialanalysis.ai/methodology/intelligence-benchmarking | Success (truncated but sufficient). Confirms v4.3 composition, legacy status of GPQA Diamond / Terminal-Bench v2.1 / τ³-Banking, and version-history hazard. |
| `web_fetch` | https://proceedings.mlr.press/v267/kim25e.html | Success. Confirms Kim et al., ICML 2025, PMLR 267:30038–30066: 350+ LLMs, ~60% error agreement when both err on one leaderboard dataset; larger/more accurate models stay correlated across architectures/providers. |
| `web_search` | Intelligence Index methodology weights; Kim correlated errors; DeepSWE/Terminal-Bench task counts; Bacchelli/Bird review outcomes; DRF; Bertsimas–Sim | Success. Primary-source snippets retrieved for each; see URLs below. |
| `web_fetch` (not attempted further) | Paywalled journal PDFs (JPE, QJE, Operations Research) | Not fetched; citations below use the packet’s own links plus public abstracts/DOI records. No theorem claim depends on full text. |

Primary URLs relied on:

- Coding-agent methodology: [AA Coding Agent Index v1.5 methodology](https://artificialanalysis.ai/methodology/coding-agents-benchmarking)
- Intelligence Index: [AA Intelligence Index v4.3 announcement](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3) and [AA intelligence benchmarking methodology](https://artificialanalysis.ai/methodology/intelligence-benchmarking)
- Correlated errors: [PMLR v267 kim25e](https://proceedings.mlr.press/v267/kim25e.html) and [arXiv 2506.07962](https://arxiv.org/abs/2506.07962v1)
- DRF (analogy only): Ghodsi et al., NSDI 2011, via [USENIX legacy record](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf) named in-tree
- Robust budgets (analogy only): Bertsimas–Sim 2004, doi `10.1287/opre.1030.0065` (public record confirmed via search; full text paywalled)
- Review practice: Bacchelli–Bird, ICSE 2013 (public record confirmed; most durable link is the [Microsoft Research PDF](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/ICSE202013-codereview.pdf) already cited in-tree)

Theorem-applicability rule applied throughout: DRF, O-ring, Garicano hierarchies, job-shop scheduling, robust-optimization budgets, bandits-with-knapsacks, RouteLLM, and ordinal-regression citations in-tree are **design analogies**, not proofs about this allocator. None transfers without its own assumptions (divisible demands, truthful reporting, calibrated sets, measured outcomes).

## 3. Verdicts per finding

### F1 — `ceil(H·N/4)` scales demand, not capacity; N=1→4 can keep the same admitted prefix — SUBSTANTIATED, with one qualification

Verified in code:

- Forecast: [src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1157) `forecast = ceil(agent_hours · orchestrators / 4)`.
- Declared convention: [src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1186) and [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:13): capacity scenario, not backlog/forecast/quota; orchestrator count “does not multiply quota or approved concurrency.”
- Prefix admission with monotone feasibility: [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:26) and scheduler [src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:645).

Qualification (counterexample to the strongest reading): the “same 19-job prefix” holds only conditional on the same candidate set, allowances, and calendars. Changing N changes forecast length (28→112) and Hamilton class counts/sequence, and conductor load `2·Σ N[c]·f[c]` grows with admission, so the admitted prefix composition and makespan need not be identical. The mechanism is real; the exact-number claim needs the ceteris-paribus rider.

- Severity: High (demand illusion drives over-admission framing).
- Refinement: replace `H·N/4` with authorized ready-task IDs plus duty-factor/arrival telemetry; keep any scenario forecast labeled “capacity scenario, zero native admission.” Gate N increases on measured context/coordination cost, not quota.

### F2 — Lexicographic admission precedes route quality; a stronger conductor loses if it displaces one job — SUBSTANTIATED

Verified:

- Search first attempts full forecast, then binary-searches max feasible count ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1500)).
- LP relaxation objectives are ordered coverage → quality → −time → −cost ([src/solver.rs](/projects/aitierlist/src/solver.rs:209)); tie-breaks in `better()` ([src/solver.rs](/projects/aitierlist/src/solver.rs:248)); [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:26) states admission-first explicitly.

No counterexample: this is the intended lexicographic policy. The defect is normative, not a code bug: quantity compensates for missing assurance.

- Severity: High.
- Refinement: implement epsilon-constraint / Pareto reporting — maximize quality subject to a policy-set admission floor, and publish the admission–quality frontier plus the displaced-job shadow explanation instead of a single scalar “utility.” Never let count outrank mandatory safety/evidence gates.

### F3 — Baseline conductor is low effort while creation/assurance routes use max — MECHANISM SUBSTANTIATED, interpretation corrected

Verified mechanism:

- Per-plan conductor utility = `row.smart` (normalized Intelligence Index) with common first-party II task cost + canonical decode proxy ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1441), [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:117)); workers use capped reference completion mixed with static skills ([src/engine.rs](/projects/aitierlist/src/engine.rs:391)).
- A cheap-per-II-unit low-effort binding can therefore win jointly when worker-quality sum dominates and the conductor term is small — even when Implementer/Debugger caps are 3 and several routes are max effort ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:15)).

Counterexample to “low is wrong”: under the stated objective this can be the correct joint pick. Effort labels are not comparable across families/harnesses, and conductor work (brief/close, reconciliation) is not implementation work. The finding is valid as objective-mismatch evidence, not as proof the conductor is incapable.

- Severity: Medium (misleading effort comparison; real issue is role-evidence mismatch).
- Refinement: qualify conductors on orchestration-relevant evidence (repo synthesis, planning, reconciliation, tool-control) with provisional-vs-calibrated gates per [src/route_qualification.rs](/projects/aitierlist/src/route_qualification.rs:167); report conductor choice with its displaced-worker opportunity cost, not effort rank.

### F4 — Floor 0.50→0.527 comparison (17 vs 16 admitted, whole-team change) is not causal evidence of dominance — SUBSTANTIATED

Verified counts: 17 admitted at 0.500, 16 at 0.527 ([audit-orch-0.500.json](file:///projects/aitierlist/target/debug/audit-orch-0.527.json) slices read); conductor competence jumps to `0.5251` in the 0.500 case. Admission count, class mix, shared budgets, calendars, and the LP bound all move together. Aggregate utility differences across different feasible sets cannot identify a conductor’s marginal effect.

- Severity: High (blocks any “Astra max dominates xhigh” claim from totals alone).
- Refinement: require joint re-solve counterfactuals — hold admission set fixed or report feasible integer substitutions with fresh holds; publish per-window shadow prices as local, relaxation-specific diagnostics, never as stable prices.

### F5 — Conductor competence definition differs between portfolio and Absolute views — SUBSTANTIATED

Verified split:

- Absolute/engine Orchestrator: `(GPQA + Intelligence Index)/2` ([src/engine.rs](/projects/aitierlist/src/engine.rs:68), [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:82)).
- Per-plan conductor candidate: `row.smart` only, `utility = competence` ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1441)).

This is an inconsistency only if the two views are compared or jointly interpreted. If they are intentionally different roles (analytical ranking vs. proxy cost driver), the code is correct but the label “Orchestrator competence” is overloaded.

- Severity: Medium.
- Refinement: one name per definition — e.g., `conductor_proxy_utility (II-only)` vs. `orchestrator_competence (GPQA+II)` — or unify with a dated rationale; block cross-view competence comparisons in UI/Markdown.

### F6 — Conductor accounting (2 II-units per weighted job) is a coordination proxy, not measured overhead — SUBSTANTIATED

Verified: [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:24), [src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1609) (`visits = 2`, `expected = changes·2`), and both audit `coordinationProxy` strings state II benchmark-task-equivalent units plus “every actual native call is also charged once.” Current artifact: 38/38 visits, expected 9.51 / reserved 11.89 API-equiv dollars, 0.69/0.86 h — proxy dollars/hours, not native turns.

- Severity: Medium (truthfully labeled in JSON, but easily misread as “224 turns” or “two benchmark suites”).
- Refinement: split the conductor ledger into brief/close visits vs. startup/resume/compaction/reconciliation calls; calibrate the latter from real planning calls in native units; keep II cost strictly as a provisional prior with transfer flag.

### F7 — Nine-stage proxy reserves every seat + repair/rechecks per admitted job; real risk needs fewer/different stages — SUBSTANTIATED

Verified:

- Proxy: 9 `STAGES` ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:65)), `EXPECTED_VISITS = [1.0, 0.25, 1.25, 2.0, 1.25, 1.0, 0.0]`, `RESERVED_VISITS = [1,1,2,2,2,1,0]` ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:8)), 25% repair incidence ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:13)).
- Native path already risk-selects: [src/team_policy.rs](/projects/aitierlist/src/team_policy.rs:120) (`research` only if `evidence_need>0`, `comprehension` only Complex/Extensive, `checks` skipped for Focused).

So the native dispatcher design agrees with the finding; only the static proxy over-reserves. The Standard Sanity 204.8-unit Anthropic reservation cited in team-allocation.md could not be re-verified in the current 19-admitted slices read (needs the pools table), so carry that number as packet-reported, not re-observed.

- Severity: High (drives Fable/Anthropic stress and false “no-fit” rejections).
- Refinement: make the proxy DAG risk-conditional like `team_policy::visits()`; reserve Debugger/rechecks only under the task’s risk vector; release conditional holds on primary acceptance (already specified in METHODOLOGY — enforce it in reporting so reserved ≠ expected).

### F8 — Net Research is demand-selected from proxy weights with runtime holds; static budget interaction stays outside the proxy — SUBSTANTIATED

Verified:

- Class weights are declared policy defaults, “not fitted research-success probabilities” ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:87)); Standard-tier comparison uses `[0.30, 0.30, 0.40, 0.0]` ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:970)); Net Research never enters retry-throughput ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:87)).
- Runtime gates are strict: fetch + (known-source or search), non-empty source/network scope, citation criteria ([src/route_qualification.rs](/projects/aitierlist/src/route_qualification.rs:318)).

Counterexample to over-claiming: static proxy exclusion is by design (research replaces unknown-resolution labor, is not additive per change). The gap is that deferred research demand never feeds back into static capacity planning.

- Severity: Medium.
- Refinement: trigger research seats from decision value (route eligibility, risk, cost/schedule impact), emit stable evidence packets with URL/publisher/retrieval-time/span ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:56)); include expected research holds in the proxy once question-shape rates are measured.

### F9 — No monotonic competence floors across roles; extensive can score below focused — SUBSTANTIATED AS A DESIGN PROPERTY

Verified: floors are optional per seat, default 0.0 ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:1426), [src/engine.rs](/projects/aitierlist/src/engine.rs:881)); class changes only `resource_factor` (0.25/1/2/4), never the floor; role utilities are fixed weighted mixtures ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:76)). Non-monotonicity is therefore permitted, not an accident.

No per-class utility table was extracted from the JSON in this pass, so the “extensive < focused” instance is accepted from the packet/reports rather than re-observed — the code mechanism alone proves it is possible and unblocked.

- Severity: Medium.
- Refinement: replace any universal 0.7-style floor with role×risk thresholds: categorical evidence rules cold-start, calibrated lower bounds once telemetry exists; enforce `consequence ≥ 2 ⇒ calibrated evidence only` (already in [src/route_qualification.rs](/projects/aitierlist/src/route_qualification.rs:222) — surface it in the proxy so unqualified “extensive” plans cannot outrank qualified focused work).

### F10 — Review burden uses fixed visits/caps/utility with no defect, complexity, or marginal-value calibration — SUBSTANTIATED (strongest evidence-validity failure)

Verified:

- Fixed `EXPECTED/RESERVED_VISITS`, 25% repair incidence, caps 1–3 are modeling choices, not measurements ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:13)).
- Retry correlation is partly transferred: Repository Q&A uses the DeepSWE source-pool rho for every model/effort — explicitly “a transferred assumption; no Q&A retry table is loaded” ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:136)).
- Utilities are “not real-job success probability” and “never passed into the retry calculation” ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:74)).

Primary-source support:

- Kim et al. show correlated errors persist across architectures/providers and grow with model accuracy — same-family creator+checker aggregation gains are overstated under independence assumptions ([PMLR](https://proceedings.mlr.press/v267/kim25e.html)).
- Bacchelli–Bird establish review as understanding/knowledge-transfer/alternatives, not pure defect-finding — fixed visit counts cannot capture review complexity or marginal assurance value ([MSR record](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/ICSE202013-codereview.pdf)).

- Severity: High.
- Refinement: add per-attempt telemetry (task/risk descriptors, bundle, context buckets, native deltas, reviewer disposition, rework cause, whether the attempt changed the accepted artifact); estimate creator/checker and retry correlations instead of assuming independence; report review value as defect exposure × complexity × marginal fix rate with intervals, never as utility points.

### F11 — Coding-agent source: 13 configs, v1.5 weights, 113:89 mix, retry/version pairing — SUBSTANTIATED IN PART; version drift confirmed

Verified from primary source (fetched today):

- AA Coding Agent Index v1.5 = equal-weight **DeepSWE v1.1 (113 tasks) + Terminal-Bench 4.0 (66 tasks) + SWE-Atlas-QnA (124 tasks)**, 3 attempts each ([methodology](https://artificialanalysis.ai/methodology/coding-agents-benchmarking)). The packet’s “66” for Terminal-Bench is therefore correct for v1.5/v4.0.
- App’s coding mix is its own policy: `(113·DeepSWE + 89·Terminal-Bench)/202` ([src/engine.rs](/projects/aitierlist/src/engine.rs:25), [METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:110)). **89 ≠ 66**: the app weight matches Terminal-Bench v2.1-era sizing, now legacy per the [intelligence methodology](https://artificialanalysis.ai/methodology/intelligence-benchmarking) (v4.3 uses TB v4.0). This is a real benchmark-transfer hazard: index weights moved under the app.
- Retry provenance design ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:123)) keeps DeepSWE and Terminal-Bench sources separate (never averaged), exact→pool→source resolution, difficulty multiplier mean-one over source configs — sound, but Q&A borrowing (F10) and harness-label matching remain fragile.
- “13 configurations” could not be re-counted from the slices read (row inventory lives outside the audit JSON windows); carry as packet-reported.

Counterexample to strict “must match AA”: the app is entitled to its own task-weighted policy — but then it must not be labeled or compared as “the AA index.”

- Severity: High.
- Refinement: pin and display benchmark versions + task counts + retrieval date/commit on every score; rename app mix to `coding-reference-113:89 (DeepSWE v1.1 : TB-v2.1-era)` until re-pinned to v4.0/66 with fresh coefficients; keep agent-row v4 vs. model-row v2.1 resource pairing explicit and never impute missing resources.

### F12 — Modeled quotas, fairness, lanes, and bounded search prove nothing about native capacity, global fairness, or optimality — SUBSTANTIATED

Verified:

- Allowances are “declared API-equivalent modeling inputs, not provider-published quotas”; weekly = monthly·12/52; several prices estimated ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:68)); built-in allowance dollars are modeling inputs ([src/engine.rs](/projects/aitierlist/src/engine.rs:196)).
- Native path correctly separates expected / reserve / aggregate-actual / attribution-estimate and uses pilot admission for uncalibrated meters ([METHODOLOGY.md](/projects/aitierlist/METHODOLOGY.md:52)) — but the static proxy cannot inherit that separation.
- Solver `feasible_search_limit` with 256 nodes/probe, 1024 global ([src/portfolio.rs](/projects/aitierlist/src/portfolio.rs:10)) reports a bound on the modeled objective/candidate set only; native horizon reports `feasible_gap: None` by construction ([src/native_scheduler.rs](/projects/aitierlist/src/native_scheduler.rs:344)).
- Fairness/robustness/bandit citations are analogies (see §2): DRF needs divisible goods and reported demands; Bertsimas–Sim needs fitted uncertainty sets per route/task; BwK/RouteLLM need bounded low-consequence exploration. None is satisfied by proxy dollars.

- Severity: High (operational-reliance risk).
- Refinement: staged admission — verify bindings/billing/meters/resets/concurrency, admit bounded ready tasks, observe native-unit consumption, replan per settlement; preserve OpenAI/Anthropic slack instead of normalizing >92% reserved; report native-window pacing with uncertainty bands, never a universal spend percentage.

## 4. Cross-cutting themes (as requested)

- Evidence validity: F10/F11 are the load-bearing failures (transferred Q&A rho; TB version drift; II-proxy conductor costs). F5 adds label overload. Every coefficient needs sample count, time range, workload, route identity, estimator, interval, last observation ([research/team-allocation.md](/projects/aitierlist/research/team-allocation.md:216)).
- Correlated errors: F10. Same-family creator/checker and uncaptured retry correlation overstate assurance. Require meaningfully distinct checkers where consequence warrants it; estimate correlations from gate outcomes.
- Review burden: F7/F10. Fixed visits + 25% incidence + caps model throughput, not defect exposure or marginal value. Move to risk-vector templates with mandatory seats, separation, evidence-packet size, and escalation rules.
- Role-competence monotonicity: F9. By design absent. Add role×risk thresholds; provisional priors only for bounded, reversible, low-consequence work with named benchmark + transfer assumption.
- Benchmark transfer: F3/F5/F6/F11. Harness moves (Codex/Claude/Grok/Antigravity/Muse) break score portability; onboarding must accept evidence as proxy explicitly. Never assert a 2025 benchmark winner for a 2026 tool bundle.
- Real-work calibration: F6/F8/F12. No synthetic calibration jobs (per packet constraints); learn from bounded real low-consequence work with observable acceptance; conductor overhead, durations, consumption, repair incidence, and quota conversion all stay provisional with wide intervals until telemetry supports them.

## 5. Implementable refinements (no invented coefficients)

Objectives:

1. Lexicographic order: safety/evidence/access → high-consequence unserved demand → weighted admitted tasks with dominant-share fairness → calibrated decision value → rework/time/cost → slack retention.
2. Publish the admission–quality Pareto frontier and threshold decision curves; retain alternatives compatible with the stated ordering (robust ordinal regression family) instead of inventing neutral weights.

Constraints:

3. Integer task counts (`z[p,t,w]`, `x[p,t,w,s,r]`), fixed single-conductor binding (`Σy=1`), DAG precedence, native-window holds, workspace/host/human capacities, per-task role thresholds.
4. Risk-vector workflow selection (consequence, uncertainty, coupling, reversibility, evidence/tool/correlation/deadline) before throughput; file counts inform load only.

Metrics/reporting:

5. Separate expected / reserve / provider-actual / attribution-estimate everywhere; native units stay native; API dollars never subtract from premium requests without a calibrated conversion.
6. Per-coefficient provenance (n, range, workload, route, estimator, interval, age); stale/transferred flags; uncertainty bands per meter/workload, not universal percentages.
7. Deterministic conditional policy with stable IDs shared by UI/Markdown/dispatcher; version/identity mismatch blocks dispatch; started work frozen across replans; idempotent settlement; reset-spanning holds preserved.
8. Re-solve, don’t swap: every substitution (e.g., Standard Sanity removal, conductor change, Muse idle-capacity use) requires a complete feasible re-solve with fresh holds.

Rollout: bindings/billing/tools/permissions/network/roots/isolation gates → integer ready jobs + risk templates + Net Research with verified tools → native-window ledger with short-lock transactions → qualification before throughput → dominant-share fairness + robust coefficients + receding-horizon triggers → real-work telemetry with shadow-price/substitution diagnostics → bounded exploration only on qualified low-consequence work.

## 6. Disposition of the 112-hour case

Do not approve as a reliable operating plan. It is a feasible modeled maximum-volume scenario under declared proxy coefficients, with an 11.64% modeled gap and >92% modeled reservation on four provider pools. Operate staged admission with explicit slack, risk-qualified workflows, native-unit observation, and joint replanning until distributions stabilize.
