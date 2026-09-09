# Grok allocation refinement audit

**Client:** Grok Build native CLI; **model:** Grok 4.6; **requested effort:** xhigh.

**Conclusion.** At revision `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0` (HEAD equals `origin/main`), the current saved proxy case is a feasible API-equivalent plan, not a native operating commitment. It admits **19 of 112** forecast jobs, status `feasible_search_limit`, quality **67.1672**, bound **74.9857**, relative gap **11.64%** after **1024** nodes. OpenAI, Anthropic, Google, and xAI reserved utilization are **99.89% / 98.68% / 92.95% / 97.96%** of declared weekly API-equivalent capacity; Muse is **81.14%**. Native meters and authorization are a separate path. Findings **1–12** are all **supported** by current source and artifacts; several are design properties rather than accidental bugs. The live incentive problem is lexicographic **count-before-quality** on a demand that scales with `N` while subscription capacity does not.

`research/team-allocation.md` describes a different, older solve (112 admitted, GPT-6 Astra low, ~11.04% gap). It is not the current production artifact.

---

**Review metadata**

| Field | Value |
|---|---|
| Client | Grok Build |
| Model | Grok 4.6 |
| Requested effort | xhigh |
| Tree | `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0` (even with `origin/main`) |
| Named rust/markdown sources | read in full |
| Audit JSON | keys and named fields only |
| Product launch / tests / edits / subagents | none |

**Web tools (all succeeded unless noted)**

- `web_search` / `web_fetch`: AA Coding Agent Index v1.5 methodology; AA Intelligence Index v4.3 (7 Sep 2026); tbench.ai TB 4.0; GitHub `harbor-framework/terminal-bench` commit `83c7a617…` and all 13 submission JSONs; Codex shared-usage pricing (fetched 2026-09-09); Grok consumer FAQ; Antigravity plans; Ghodsi et al. NSDI 2011 PDF; Kim et al. ICML 2025; Google Research agent-scaling post + arXiv:2512.08296.
- Exact name **Terminal-BenchNormal**: **not found** as a public dataset, repo path, or Harbor slug. The named GitHub commit is the official TB v4 leaderboard JSON set.

---

## Finding 1 — `ceil(H*N/4)` scales demand, not capacity

**Verdict: confirmed. High.**

```1157:1186:src/portfolio.rs
let forecast = (settings.agent_hours * settings.orchestrators as f64 / 4.0).ceil() as usize;
...
"The proxy demand volume is ceil(elapsed weekly hours * concurrent project orchestrators / 4). ... Orchestrators share every subscription allowance; the count does not multiply quota or approved worker concurrency."
```

Pool hours are `agent_hours * subscription_count`, not `* orchestrators`. Search capacities use that same `hours`.

**Artifact.** `audit-n1.json` vs `audit-n4.json` / `audit-current-production.json`:

| | N=1 | N=4 |
|---|---:|---:|
| Forecast | 28 | 112 |
| Admitted | **19** | **19** |
| Quality / bound / gap / nodes | 67.1672 / 74.9857 / 11.64% / 1024 | identical |
| Admitted sequence | identical 19-class prefix | identical |
| Pool reserved USD | identical | identical |

**Counterexample.** If forecast were below the capacity-feasible prefix (small `H` or `N`), raising `N` would increase admitted jobs. In this 112-hour case the bind is shared allowance/calendar, so `N=1→4` only inflates deferred work (9→93).

**Incentive.** Inflating concurrent projects does not buy more proxy admission once the 19-job prefix is capacity-limited. It does overstate unserved demand. Native `N` still multiplies conductor sessions on one binding (`native_scheduler.rs` fixed conductor; `METHODOLOGY.md` L12).

**Refinement.** Keep `N` as session count only. Forecast authorized ready jobs, or show a sensitivity table of admitted prefix vs `H` and vs `N` separately. Do not invent a duty factor.

---

## Finding 2 — lexicographic admission beats route quality

**Verdict: confirmed. High.**

Outer search maximizes prefix length, then quality:

```1516:1536:src/portfolio.rs
if result.outcome.solution.is_some() {
    lower = count;
    best = Some(result);
} else {
    ...
    upper = count;
}
```

Inner LP is coverage, then quality, then time, then cost (`solver.rs` 209–223). Any feasible 19-job assignment beats every 17-job assignment, including higher quality.

**Artifact.** Production 19 jobs, Sol low, quality **67.17**. Floor 0.50: 17 jobs, Astra xhigh, quality **70.07**. Floor 0.338: 17 jobs, Astra medium, quality **69.66**. Count-first therefore selects the **lower** modeled utility.

**Counterexample.** A stronger conductor that is also cheaper, or that does not displace the 19th job, still wins at 19. The claim is the lex rule, not that every better model loses.

**Incentive.** The objective pays for an extra proxy job even when total modeled utility falls. That is the opposite of a quality-qualified allocator.

**Refinement.** Emit the feasible front at several admitted counts (at least 16/17/19 in this case) with quality, reserved USD, and makespan. Let the user pick; do not invent a scalar that mixes jobs and utility. This is an epsilon-constraint / Pareto report, matching the methodology’s own Mavrotas citation.

---

## Finding 3 — conductor GPT-5.6 Sol low vs max-effort workers

**Verdict: confirmed. Medium (mechanism High).**

Production conductor: row 9, **GPT-5.6 Sol (low)**, Codex, cap 1, utility = Intelligence Index **0.337979**. Workers on the same 19 jobs include Sol **max**, Astra **max**, Opus **max**, Grok **xhigh**, Muse **max**.

Orchestrator utility is `row.smart` with cap 1 and II cost/time (`portfolio.rs` 1441–1464, `engine.rs` 470–485). Workers use capped retry completion on reference workloads, so max effort can raise utility. A low-II cheap conductor preserves the 19-job prefix; a high-effort Astra conductor does not (Finding 2/4).

**Counterexample.** This is not a coding-competence ranking of Sol vs Astra. It is the joint resource consequence of putting the conductor on the same OpenAI pool as Sol-max implementation.

**Refinement.** Report conductor displacement: for each eligible conductor, the max feasible prefix and the worker set that survives. Do not treat Sol-low as “the best conductor.”

---

## Finding 4 — Orchestrator floors change the whole team

**Verdict: confirmed. High.**

| Floor file | Conductor | Admitted | Quality | Implementer Focused | Debugger Focused |
|---|---|---:|---:|---|---|
| production (Sol 0.338 II) | Sol low | 19 | 67.17 | Sol max | Gemini 3.8 Flash high |
| `audit-orch-0.338.json` | Astra **medium** | 17 | 69.66 | Astra max | Gemini Flash high, cap 2 |
| `audit-orch-0.500.json` | Astra **xhigh** | 17 | 70.07 | Sol max on Focused, Astra max elsewhere | Gemini Flash high, cap 2 |
| `audit-orch-0.527.json` | Astra **max** | 16 | 63.23 | Astra max | Sol max on Focused |

Finding text matches 0.50 and 0.527. Floor 0.338 is additional evidence that a small floor change rewrites workers, caps, and Muse/Anthropic/xAI claims. Aggregate utility is not a causal ranking of conductors.

**Refinement.** Publish the four (or more) complete tables as a sensitivity panel. Do not pick a winner from quality alone.

---

## Finding 5 — two conductor competence definitions

**Verdict: confirmed. Medium.**

Per-plan conductor: `row.smart` only (`portfolio.rs` 1441–1463).

Absolute Orchestrator: `(GPQA + Intelligence Index) / 2` (`engine.rs` 68–74; `METHODOLOGY.md` 82–83, 117).

Sol low: II **0.338**, GPQA **0.898** → Absolute **0.618**. Astra max: II **0.528**, GPQA **0.961** → Absolute **0.744**. Same model, different views.

**Primary source, 7 Sep 2026.** Intelligence Index v4.3 does **not** include GPQA. GPQA Diamond was dropped in v4.2 as saturated ([AA v4.3](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3), [v4.2](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-2)). II v4.3 is ten evals (Agents 30 / Coding 20 / General 30 / Scientific 20), including Terminal-Bench v4.0 at 10%. Mixing leftover GPQA with that composite is a policy choice, not “the” AA conductor score.

**Refinement.** One named conductor score per mode, with the formula on the row. If Absolute keeps GPQA, label it a standalone diagnostic, not Index membership.

---

## Finding 6 — conductor charges two class-weighted II task-equivalents

**Verdict: confirmed. Medium.**

Workflow visits: 2 per admitted job → **38** reported visits.

Resource load: `2 * Σ N[c] f[c]` = `2 * (9·0.25 + 6·1 + 3·2 + 1·4)` = **36.5** BTE. Reserved usage `36.5 × 0.32580243 = 11.8918`, matching the conductor row.

This is not 38 native turns, not two Intelligence Index suites, and not measured conductor overhead (`METHODOLOGY.md` 23; `portfolio.rs` 1191, 1199). Native ledger is supposed to charge each real call once (`native_reconciliation.rs`).

**Refinement.** Keep 38 as visit count and 36.5 BTE as the proxy cost. Calibrate conductor startup/resume/compaction from native meter deltas when those exist. Do not invent a turn multiplier.

---

## Finding 7 — static nine-stage proxy vs risk DAGs

**Verdict: confirmed. Medium.**

Proxy `STAGES` (`portfolio.rs` 65–129): Orchestrator, Comprehension, Implementer, Reviewer, Sanity, conditional Debugger, two rechecks, Orchestrator close — **every** admitted job.

Native `visits()` (`team_policy.rs` 120–234): Focused skips Reviewer/Sanity/Debugger; Complex/Extensive add Comprehension; Net Research only if `evidence_need > 0`.

Google’s agent-scaling results are **analogy**, not a theorem for this DAG: multi-agent help on decomposable work and hurt on sequential work ([Google Research, 28 Jan 2026](https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/), arXiv:2512.08296). They do not calibrate nine reserved stages.

**Refinement.** Use the proxy nine-stage template only as a stress envelope. Runtime already expands from the risk vector; the static solve should not claim those seats are required.

---

## Finding 8 — Net Research is ranked, not allocated

**Verdict: confirmed. Medium.**

`EXPECTED_VISITS[NetResearch]=0`, `RESERVED_VISITS=0`; Net Research is skipped in `search()` groups. Production: **0 jobs**, “Conditional candidates; runtime qualification and native admission remain required.” Ranked AA class weights are present (Focused primary Astra high 0.631; Standard Grok 4.6 xhigh 0.681). `qualify_research` still requires fetch, and search unless `known_source_fetch` (`route_qualification.rs` 318–347).

Those weights are declared policy (`METHODOLOGY.md` 87–96), not success probabilities. They never enter the reserved USD that is 92%+ consumed.

**Refinement.** When a ready task has `evidence_need > 0`, put one research visit into the same native-window solve. Do not add a research seat to every proxy job.

---

## Finding 9 — no monotonic competence floors by class

**Verdict: confirmed. Medium.**

Production utilities, same role:

- Implementer Extensive **0.452** (Gemini 3.8 Flash high) < Focused/Standard/Complex **0.570** (Sol max).
- Sanity Extensive **0.583** (Grok xhigh) < Focused **0.594** (Muse max).
- Comprehension Complex **0.690** (Sol max) < Focused **0.697** (Grok xhigh).

Floors are a single per-seat setting, not per class (`portfolio.rs` 1426–1448). Native `minimum_workflow` only raises `uncertainty` (`native_scheduler.rs` 457–477).

**Counterexample.** Extensive Reviewer Opus max **0.681** > Focused Sol max **0.629**. Non-monotonicity is not universal.

**Refinement.** If policy wants class-monotonic competence, enforce it as an explicit constraint. Do not invent a 0.7 gate.

---

## Finding 10 — review burden is uncalibrated

**Verdict: confirmed. Medium.**

Proxy Reviewer/Sanity: two reserved visits, expected 1.25 at 25% repair. Utility is Repository Q&A ± reasoning, **not** defect detection. Caps 1–3 change completion probability on that reference workload, not assurance value.

Kim et al., ICML 2025: models agree ~60% of the time when both err; larger/more accurate models remain correlated across providers ([PMLR 267:30038](https://proceedings.mlr.press/v267/kim25e.html)). **Analogy** for creator/checker correlation. Native `checker_correlated` only blocks same host or same `display_model` when consequence and correlation are high (`native_scheduler.rs` 1023–1069). Retry ρ is fitted on DeepSWE / TB 2.1, not on review catches.

**Refinement.** Keep visit counts as policy. Do not report Reviewer utility as P(catch). Optional: require a distinct checker identity for high-consequence work (already sketched natively).

---

## Finding 11 — Coding Agents mix, TB version, retry provenance

**Verdict: confirmed. High for interpretation; do not silently swap TB 2.1 ρ onto v4.**

**AA Coding Agent Index v1.5** (current methodology, Sep 2026): equal weight **DeepSWE v1.1 (113) + Terminal-Bench 4.0 (66) + SWE-Atlas-QnA (124)**, three attempts, pass@1. [Methodology](https://artificialanalysis.ai/methodology/coding-agents-benchmarking).

**App coding competence** is still `113/202` DeepSWE + `89/202` Terminal-Bench (`engine.rs` 25–28; `aa.rs` 16–17). That 89 is TB **2.1** task count. Agent-row **time/cost** pooling is dataset-aware (`terminal-bench-v4` → 66 in `aa.rs` 511–519). Competence weights are not.

Production audit, `sourceFetchedAt` 2026-09-09 21:06: **exactly 13** non-`model` rows, all `deep-swe-v1.1, terminal-bench-v4, swe-atlas-qna`. Codex Sol max TB v4 pass **0.374**; model-row Sol low TB pass **0.768** on canonical decode resources (v2.1 pairing). Snapshot line on agent-row v4 vs model-row v2.1 is correct.

Retry table (`assets/retry-correlation.json`): DeepSWE v1.1 + **Terminal-Bench 2.1** at `7131e437…`, 20 submissions, `n_trials` ≈ 445 (= 89×5), pool ρ **0.5779**. Q&A uses the DeepSWE source pool. Agent-row v4 pass rates still consume that TB 2.1 ρ if harness/model/effort match.

### Terminal-Bench v4 pass-at-2..5 at `83c7a6172d629c6575b785ab12c8db787bb2e323`

**Verified, with limits.** Commit exists (2026-09-03T02:43:32Z, “Promote submission from #1892”). `leaderboard/submissions/` has **13** JSON files. **All 13** expose `pass_at_2`…`pass_at_5`, `n_trials=330` (= 66×5). `main` still has the same 13 names.

| Agent | Model | Effort | acc% | p@2 | p@3 | p@4 | p@5 |
|---|---|---|---:|---:|---:|---:|---:|
| Claude Code | Fable 5.1 | max | 57.88 | 0.700 | 0.747 | 0.773 | 0.788 |
| Claude Code | Opus 5 | max | 51.82 | 0.617 | 0.658 | 0.682 | 0.697 |
| Claude Code | Fable 5 | max | 44.55 | 0.573 | 0.632 | 0.664 | 0.682 |
| Claude Code | GLM-5.3 | max | 41.82 | 0.508 | 0.547 | 0.567 | 0.576 |
| Codex | GPT-5.6 Sol | max | 37.27 | 0.496 | 0.550 | 0.582 | 0.606 |
| Claude Code | Opus 4.8 | max | 23.64 | 0.346 | 0.405 | 0.442 | 0.470 |
| Codex | GPT-5.6 Terra | max | 21.52 | 0.306 | 0.364 | 0.406 | 0.439 |
| Grok Build | Grok 4.6 | **none** | 20.30 | 0.285 | 0.330 | 0.364 | 0.394 |
| mini-SWE-agent | Gemini 3.8 Flash | high | 19.09 | 0.288 | 0.353 | 0.400 | 0.439 |
| Codex | GPT-5.6 Luna | max | 17.27 | 0.242 | 0.285 | 0.312 | 0.333 |
| Claude Code | Sonnet 5 | max | 12.42 | 0.205 | 0.262 | 0.309 | 0.349 |
| Grok Build | Grok 4.5 | **none** | 12.42 | 0.183 | 0.223 | 0.252 | 0.273 |
| mini-SWE-agent | Gemini 3.7 Flash | high | 11.21 | 0.164 | 0.200 | 0.230 | 0.258 |

**Do not treat this commit as the current complete official board or as a drop-in for the app’s 13 coding-agent rows.**

- [tbench.ai](https://tbench.ai/) currently lists **GPT-6 Astra (max) Codex 58.2% ± 2.8% (Sep 3, 2026)**. That row is **absent** from this GitHub tree and from current `main` submissions.
- AA’s 13 agent rows include Astra max, Muse Spark 1.3, Kimi K3, DeepSeek V4, Opencode GLM-5.3, Qwen3.8 Max, Antigravity Gemini 3.8 Flash. Official TB JSON uses **mini-SWE-agent** for Gemini, **Claude Code** for GLM-5.3, Grok effort **none**, and has no Muse/Kimi/DeepSeek/Astra.
- AA Coding Agents and AA II v4.3 run TB v4 **three** times, pass@1. Official TB JSON is **five** trials, pass@k. Different estimator.

**Refinement.** If retry ρ is refit, fit it on this v4 JSON **only for those exact agent/model/effort keys**, keep TB 2.1 ρ for model-row v2.1 scores, and do not average the two. Competence weights should follow the dataset that produced `row.term` (66 vs 89). Equal-weight AA Index is a third, separate number.

---

## Finding 12 — modeled quotas, fairness, lanes, and search limits are not native optimality

**Verdict: confirmed. High.**

**Quotas.** Declared API-equivalent USD (`12/52` of modeled monthly allowance). Native docs are different meters:

- Codex: ChatGPT Work and Codex **share usage**; model tables are ranges, not USD ([learn.chatgpt.com/docs/pricing](https://learn.chatgpt.com/docs/pricing), 2026-09-09). App-server rate limits are percent-of-window, 15–60 min.
- Grok: one **weekly pool** across Chat, Imagine, Voice, Build; workload-dependent ([docs.x.ai/grok/faq](https://docs.x.ai/grok/faq)).
- Antigravity: five-hour refresh plus weekly; consumption tracks work done, not prompts ([antigravity.google/docs/plans](https://antigravity.google/docs/plans)).

99% reserved OpenAI USD does not prove Codex remaining percent.

**Shared accounts.** Proxy: one calendar per provider, `subscription_count` lanes, conductor locked to account ordinal 1 (`portfolio.rs` 692–693, 1187). Native: remaining = snapshot − unreflected holds − external (`native_scheduler.rs` 509–552). Production is one account per selected provider.

**Fairness / incentives.** Native `fairness_cost` is max entitlement-weighted dominant share, **minimized after** high-consequence count, deadline urgency, and useful-task count (`native_scheduler.rs` 103–113, 1252–1289). Ghodsi et al. NSDI 2011 DRF is strategy-proof for **lying about resource demand vectors** in progressive filling ([paper](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)). That theorem does **not** apply to:

- user-declared `consequence`, `deadline`, `entitlement_weight`;
- lexicographic extra jobs before fairness;
- splitting `project_id` to shrink apparent share;
- proxy mode, which has **no** project fairness.

**Deadlines.** Proxy: reserved makespan **36.01 / 112** hours; no per-job deadline. Native: expired deadline defers; plan duration must fit deadline; `calendar_fits` rejects `cursor > deadline`. Deadline **score** increases as slack shrinks, so tight deadlines jump the queue.

**Search limits.** Proxy: 1024 global, 256 per probe; this case is `feasible_search_limit`, **11.64%** gap, not proven. Native: `feasible_gap` is **always `None`**; DFS `walk` plus `assemble_plan` truncation can miss substitutions. Bound is the LP quality bound for the declared scheduler, not software-quality uncertainty.

**Calendar mismatch.** Proxy account calendars may run concurrently. Native `schedule` / `calendar_fits` advance a **single global cursor** (`native_scheduler.rs` 1307–1368, 1442–1530). Four project conductors do not get four parallel native timelines even when host concurrency is >1.

**Refinement.** Stage native admission from authorized jobs and observed remaining windows. Report proxy 92%+ figures only as modeled USD. Expose dominant shares without claiming DRF. Prefer Pareto of feasible native plans over a single lex incumbent when the node limit binds.

---

## Implementable refinements (no new coefficients)

1. **Objectives.** Replace count-then-quality with an explicit front: admitted prefix × quality × reserved USD × makespan, for the counts already observed (16, 17, 19). Same for native: quantity vs fairness_cost vs deadline misses.
2. **Constraints.** `N` must not scale subscription capacity. Class-monotonic floors only if configured. High-consequence creator/checker distinctness is already in native; do not silently drop it in the proxy.
3. **Metrics.** One conductor formula per mode. Agent-row TB v4 pass uses 66-task weight; model-row TB 2.1 stays 89. Retry ρ stays source-paired. Conductor 36.5 BTE ≠ 38 visits ≠ native turns.
4. **Reporting.** Label `feasible_search_limit` as unproven. Show N=1 and N=4 as the same 19-job capacity case. Show Orchestrator-floor tables as sensitivity, not winners. Net Research remains unallocated until a ready task needs it.
5. **TB v4 retry.** The GitHub commit is a valid **v4 pass-at-k** table for its 13 official JSON rows. Use it only for those keys. Do not call it Terminal-BenchNormal (name unknown). Do not claim it covers Astra or the current AA 13. Prefer a newer official dump if Astra’s pass-at-2..5 is required.

Bertsimas–Sim robustness, RouteLLM, and bandits-with-knapsacks remain **citations for future calibrated learning**, not licenses to invent ρ, repair incidence, or floors.
