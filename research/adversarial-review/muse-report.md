# Adversarial review: allocation policy as a mathematical and operational decision system

## Provenance

- Client: Muse Code, powered by Meta Muse Spark.
- Model: Meta Muse Spark (Muse Code agent).
- Effort: default; no explicit effort setting was supplied for this review.
- Scope read: `research/adversarial-review/snapshot.md`, `research/team-allocation.md`, `METHODOLOGY.md`, `src/native_scheduler.rs`, `src/team_policy.rs`, `src/route_qualification.rs`, `src/native_reconciliation.rs` (including overflow windows). No other file under `research/adversarial-review/` was read; no peer report was read.
- Web tools used:
  - `web_search`: successful. Five queries: DRF NSDI 2011; Bertsimas–Sim Price of Robustness; Bandits with Knapsacks; correlated LLM errors / ensemble diversity; OR-Tools job-shop precedence/no-overlap. All returned usable primary-source URLs.
  - `web_fetch`: successful. Fetched the USENIX DRF page (200 OK, full) and the Google OR-Tools job-shop page (200 OK, truncated by the tool but the constraint description was legible). No other fetches were needed.

## Review stance

The documents are unusually honest about the feasible/admissible/effective distinction, about benchmark scores not being probabilities, about DRF-as-inspiration, about shadow prices being local, and about the scheduler being conservative rather than optimal. The code largely matches that honesty: it reports `Exhaustive` vs `FeasibleSearchLimit`, never fabricates an optimum proof or gap (`feasible_gap` is always `None`), keeps expected/reserve/aggregate-actual/attribution separate, and freezes started work. The findings below are therefore not "the system claims optimality." They are places where the mathematics, the code logic, or a plausible reader inference still overreaches, plus two logic defects that break the highest-risk path the policy claims to protect.

Findings are ordered by severity. Each has a counterexample and a conceptual correction. No synthetic calibration tasks or software tests are proposed.

## Finding 1 (High): high-risk repair path deadlocks in `checker_correlated`

Location: `src/native_scheduler.rs`, `candidate_plans` + `checker_correlated` + `assemble_plan`.

Logic:

- `frozen` includes `Running | Completed | Blocked` jobs, so a `Blocked` implementer visit is excluded from replanning.
- `frozen_creator` only matches `Running | Completed` implementers.
- `checker_correlated` returns `true` (reject every candidate) when no creator is found: `let Some(creator) = selected_creator.or(frozen_creator) else { return true; }`.
- It only looks for an `Implementer` in the current prefix, never for the `Debugger` whose repair the rechecks actually review.

Counterexample: a task with `consequence >= 2` whose implementer is `Blocked`. The implementer visit is frozen out of `task_visits`. `selected_creator` is `None`, `frozen_creator` is `None`, so every `Reviewer`/`Sanity` choice (including the conditional repair rechecks) is skipped as "correlated." `choices[index]` is empty, `assemble_plan` returns immediately with zero plans, and the task defers with "No exact qualified route bundle fits." The tasks that most need the repair workflow are exactly the ones that can never schedule it. Low-risk tasks are unaffected because the function early-returns `false` when `consequence < 2 && correlation < 2`, which makes the defect perverse: protection triggers the deadlock.

Correction: compare each checker against the attempt it actually checks (original implementer for first checks; debugger repair attempt for rechecks; frozen `Blocked` binding counts as a creator for this purpose), and fail closed only when the lineage is genuinely unknown. At minimum, include `Blocked` implementer bindings in `frozen_creator` and document that rechecks of a repair are diversified against the repairer, not only the original implementer.

## Finding 2 (High): qualification and pilot gates trust caller-supplied risk and thresholds

Locations: `src/route_qualification.rs` (`Provisional` vs `Calibrated` branches, `task.role_thresholds` lookup); `src/native_scheduler.rs` (`coefficient_bundle`, pilot keys); `src/team_policy.rs` (`RiskVector::template`).

Observations:

- The calibrated gate reads its threshold from `task.role_thresholds`: `task.role_thresholds.get(visit.seat.name())`. A task author who sets threshold `0.0` passes any finite `lower_bound`. There is no policy-side floor in code. Authorization (`task.authorized`) is necessary but not sufficient: an authorized but optimistic caller understates the bar for consequential work.
- Provisional-vs-calibrated, pilot-vs-full, and checker-diversity enforcement all key off the self-reported `RiskVector` (`consequence`, `correlation`, template). Nothing independently validates the vector against artifacts, diffs, permissions, or destinations.
- `RiskVector::template` computes `maximum` over consequence, uncertainty, coupling, reversibility, tool risk, correlation, deadline — deliberately excluding `evidence_need`. A task with `evidence_need = 255` and all other dims `0` is `Focused`, so provisional evidence and pilot admission apply to arbitrarily research-heavy work. `visits()` adds a research visit for any `evidence_need > 0` but does not escalate the template.

Counterexample: submit two identical high-stakes migrations, one labeled `consequence = 3`, one labeled `consequence = 1`. The second qualifies provisional routes, pilot coefficients, same-family creator/checker, and no mandatory calibrated reviewer — same work, weaker assurance, by label alone.

Correction: treat the risk vector as a claim, not a measurement. Require the dispatcher (not the task submitter) to derive or floor consequence/reversibility/correlation from observable task features (auth/billing/deletion/public-API/production-data flags, permission scope, deployment targets), enforce policy-minimum calibrated thresholds server-side per seat, and include `evidence_need` in the escalation function or give research-heavy work its own template floor. Document that current thresholds are caller-asserted until that lands.

## Finding 3 (High): fairness is throughput-first instantaneous minimax, not DRF

Locations: `Score::compare`, `fairness_cost`, `resource_cost`, `existing_project_usage` in `src/native_scheduler.rs`; DRF analogy in snapshot and `research/team-allocation.md`.

What the code does: `useful` (count + priority + consequence + deadline) strictly dominates `fairness_cost`; fairness only breaks ties at equal `(high_consequence, deadline, useful)`. `fairness_cost` is the max over projects of dominant reserve-share (`amount / capacity / weight`), seeded with started holds. Minimizing it never sacrifices one useful task.

Why DRF theorems do not transfer: DRF (Ghodsi et al., NSDI 2011; [paper page](https://www.usenix.org/conference/nsdi11/dominant-resource-fairness-fair-allocation-multiple-resource-types), [full paper](https://www.usenix.org/events/nsdi11/tech/full_papers/Ghodsi.pdf)) assumes divisible resources drawn from homogeneous pools, progressive filling toward max-min dominant shares, and properties (strategyproofness, envy-freeness, Pareto efficiency) proved for that mechanism. This setting has indivisible task/visit bundles, heterogeneous non-substitutable native meters ("unlike units are never summed"), lexicographic throughput priority, entitlement weights, and a one-shot reserve model. Minimizing the max dominant reserve-share as a tie-break is a reasonable heuristic; it inherits none of the DRF guarantees.

Counterexamples:

- Starvation: a steady stream of priority-10 project-A tasks permanently outranks project-B tasks at equal consequence/deadline. Fairness never fires because every additional A-task increases `useful`. B waits indefinitely despite near-zero dominant share.
- First-write-wins entitlement: `entitlement.entry(project).or_insert(task.entitlement_weight)` keeps the first task's weight per project. Two tasks in one project with different weights silently use whichever sorts first.
- Instantaneous, not historical: only current ledger holds seed usage. A project that consumed 90% of last week's quota but holds nothing now looks identical to one that consumed nothing.

Correction: keep the "inspired by" framing (the docs already do) and add the missing structure if fairness is to be an effectiveness claim: time-averaged dominant shares with a stated window, consistent per-project weights validated at admission, and either a fairness floor (e.g., constrained max-share) or explicit admission that throughput lexicographically dominates fairness and starvation is possible. Do not present `dominant_shares` as a fairness certificate.

## Finding 4 (High): the calendar is sound but needlessly serial; reset/deadline handling understates throughput

Locations: `calendar_fits`, `schedule`, `candidate` in `src/native_scheduler.rs`.

What the code does honestly: one global serial cursor across all visits, each visit in a single active interval, whole-task completion before the minimum coefficient reset (`cursor <= reset`), per-task `seconds <= timeout` and `<= deadline`. The snapshot correctly disclaims global job-shop optimality.

Gaps:

- The serial cursor rejects feasible parallel executions. Two tasks on disjoint accounts/hosts with disjoint exclusive resources could run concurrently, but `calendar_fits` forces them sequential. `fits()` checks aggregate per-account/host/human capacities, yet the calendar never exploits that independence. Throughput claims therefore rest on a pessimistic scheduler, while the text's "feasible within that declared scheduler" is accurate only because the scheduler is stricter than the hardware.
- DAG structure is collapsed to a visit-duration sum in a fixed topological order. Precedence is respected by serialization, but no-overlap is applied globally instead of per-machine, unlike the standard job-shop form (precedence + per-machine no-overlap; see [OR-Tools job-shop](https://developers.google.com/optimization/scheduling/job_shop)). Lanes, host concurrency, exclusive locks, and human attention never appear as separate machines in the calendar.
- The reset rule forbids straddling although reconciliation explicitly supports it (holds retained across resets with attribution estimates). Near-reset tasks defer even when straddling with a conservative hold would be the documented behavior.
- Per-visit duration is the max over coefficients' `reserve_seconds`, while usage sums across windows. Time/consumption coupling across meters is therefore loose in both directions.

Correction: keep serial placement as an admission-safe default, but label it as such and stop short of any makespan-efficiency reading. If throughput matters, model per-lane calendars (account lane, host slots, exclusive locks, human) with precedence + per-machine no-overlap, allow reset-straddling with retained holds as the reconciliation layer already permits, and reconcile `schedule()` with `calendar_fits()` so the emitted calendar is the checked calendar under the same machine model.

## Finding 5 (Medium-High): joint uncertainty is summed, not modeled

Locations: `window_pacing`, `resource_cost`, `candidate` in `src/native_scheduler.rs`; robust-optimization framing in `research/team-allocation.md` (Bertsimas–Sim).

What the code does: `window_pacing` independently sums `expected`, `reserve`, `uncertainty_lower`, `uncertainty_upper` across visits. Summed interval endpoints are presented alongside `available` and `remaining_slack`.

Problems:

- A sum of lower bounds / upper bounds is a valid joint band only under comonotonicity (all visits at their worst case together). Under independence the joint distribution concentrates; under shared shocks (vendor throttle) it does not. The code has no correlation structure, so the band is neither a confidence interval nor a robust uncertainty set — it is an arithmetic sum with unknown coverage.
- There is no uncertainty budget in the Bertsimas–Sim sense ([Price of Robustness](https://doi.org/10.1287/opre.1030.0065), Operations Research 52(1):35–53). Nothing fits per-route/task duration vs consumption sets separately, widens them for `sample_count == 5` vs `500`, or exposes budget/sensitivity. `reserve` is a point estimate used as a hard cap; `uncertainty_*` rides along for display.
- Conditional branches are all reserved simultaneously (correct conservatism for admission), but the same sum then feeds pacing and "pressure" comparisons as if every branch fires. Expected-case planning and stress-case admission are properly separated elsewhere; the pacing display blurs them.

Correction: either promote uncertainty to real robust sets per meter (fit duration and consumption separately, wider for sparse groups, shared scenarios for vendor-wide shocks, exposed budget + sensitivity) or demote the summed band to a labeled arithmetic total with no coverage claim. Never let `remaining_slack = max(0, available - reserve)` plus a summed band be read as "safe with margin" — the `max(0,·)` clamp already hides overdrawn states (see Finding 9).

## Finding 6 (Medium-High): staleness and "confirmed equals verified" weaken hard gates

Locations: `fresh`, `approved_fact`, `verified_true` in `src/route_qualification.rs`; `planning_time` from `ledger.updated_at`; `active_windows`, `task_eligibility` time parsing.

- `fresh()` is correct relative to `planning_time`, but `planning_time` is the ledger's own timestamp. No check bounds ledger staleness against wall-clock time. A days-old snapshot makes days-old capability bundles and role evidence "fresh" forever. The receding-horizon trigger list assumes fresh observations; the code accepts arbitrarily stale ones.
- `approved_fact` treats `Confirmed` + non-empty evidence as equal to `Verified` across executables, models, effort args, billing, isolation, plans, roots, tools. The snapshot's "verified search/fetch access," "exact binding," and "subscription-funded" language implies verification; the code admits confirmations whose evidence content is never inspected here.
- `task_eligibility` rejects expired deadlines and future `ready_at` relative to ledger time, which is consistent internally but inherits the same staleness: a stale ledger admits tasks whose real deadlines passed.

Correction: bound ledger age at planning time, surface observation age in every admission decision, and either distinguish `Confirmed` from `Verified` in the qualification output or narrow the prose to "approved (verified or confirmed with evidence)." The current prose claims more than the predicate enforces.

## Finding 7 (Medium): `evidence_covers` is simultaneously over-strict and under-strict

Location: `evidence_covers` in `src/route_qualification.rs`.

- Global (unscoped) requirements must appear in *every* visit's dimensions. A global `citation_accuracy` requirement would disqualify the implementer whose evidence legitimately lacks it. The research seat's `citation_accuracy` criterion leaks into non-research qualification whenever it is expressed globally.
- Seat-scoped requirements for *other* seats (`reviewer:adversarial_review` when qualifying the implementer) auto-pass. That is the right per-visit semantics, but no code in the reviewed set performs the complementary global completeness check ("for each scoped requirement, some visit in this plan has covering evidence"). A plan can therefore satisfy each visit locally while missing a required seat's evidence globally — unless that check lives in unreviewed code, in which case the reviewed path does not establish it.
- The `known_source_fetch` carve-outs are special-cased in two places with string matching; any rename or additional retrieval mode silently changes who must prove what.

Correction: define requirement scoping once (global vs per-seat vs retrieval-mode), enforce per-visit necessity *and* per-plan sufficiency, and test the cross-product with a concrete matrix (global citation requirement + implementer without it must still qualify; missing reviewer evidence must fail the plan even when each visit passes locally).

## Finding 8 (Medium): bundle-diversity proxy (`same_identity`) mismatches the correlated-error literature

Locations: `same_identity`, `checker_correlated` in `src/native_scheduler.rs`; correlation discussion in `research/team-allocation.md` citing Kim et al.

What the code does: same `host_id` OR same `display_model` counts as correlated. Missing binding records (`is_none_or`) default to correlated (conservative — good).

Mismatches in both directions:

- Over-rejects: two genuinely different failure modes on one multi-provider host (different models, harnesses, tools) are rejected as "same identity." Same display model on different hosts/harnesses is rejected even when harness/tool differences might matter.
- Under-protects: different model families on different hosts pass as "distinct" even when they share training data, architectures, or systematic blind spots. Large-scale measurement finds substantial error agreement (~60% same-wrong-answer agreement on one leaderboard dataset) and, crucially, that larger/more-accurate models have *highly correlated* errors even across distinct architectures and providers ([Correlated Errors in Large Language Models](https://arxiv.org/abs/2506.07962v1); related ensemble-selection analysis under strong error correlation ([budgeted ensemble view](https://arxiv.org/pdf/2602.08003v1))). Label diversity is not error diversity.
- Effort, harness, tool set, permission profile, and data access — the actual bundle — play no role in the comparison despite the snapshot's "observable differences in actual creator/checker bundles."

Correction: compare full bundles (model, effort, harness, tools, data/network scope), not host-or-model alone; record that even full-bundle difference is a proxy, not a measurement; and calibrate the claim to what the cited literature supports — diversity helps only when it changes errors. The retirement of any "distinct bundle ⇒ independent check" inference should be explicit. Conditional-retry independence is likewise unestablished: the native solve has no retry-correlation structure at all (that machinery lives only in the proxy forecast's `retry-correlation.json` path in `METHODOLOGY.md`).

## Finding 9 (Medium): search, scoring, and reporting gaps that distort effectiveness reads

Locations: `Search::walk`, `fits`, `score`, `resource_cost`, `dominant_shares`, `allocate` conductor loop, `window_pacing` in `src/native_scheduler.rs`.

- Traversal order decides quality under limits: tasks sorted by consequence/priority/deadline, candidates by `binding_id`, admit-before-skip, no value-density heuristic, no lower-bound pruning. Under `FeasibleSearchLimit` the incumbent is an arbitrary order-dependent feasible point. Honestly labeled, but any "the solver prefers X" reading is unfounded.
- `explored_nodes` reports only the winning conductor's search, while total work is up to `conductors × node_limit` plus per-task plan enumeration capped at the same `node_limit`. `limited` conflates global search cutoff with per-task plan-cap truncation. Reproduce or compare runs on node counts with care.
- `deadline` and `consequence` enter `useful` *and* their own higher tiers — double counting that makes the lexicographic order harder to interpret than documented.
- `resource_cost` maps missing/zero capacity to `INFINITY` while `fairness_cost` maps it to `0.0`. A zero-capacity overuse looks maximally bad on one tier and invisible on the other; `fits()` luckily blocks positive overuse first, but the diagnostics disagree by construction.
- `native_capacity` clamps `available` with `.max(0.0)`; `window_pacing.remaining_slack` clamps with `.max(0.0)`. Overdrawn meters render as zero rather than negative — the display cannot distinguish "exactly exhausted" from "overdrawn."
- `dominant_shares` omits entitlement weights that `fairness_cost` includes, so the reported shares and the optimized objective disagree.
- `existing_project_usage` keys units from ledger windows while `capacity` requires profile/ledger unit agreement and skips mismatched windows. A unit mismatch silently drops a window from capacity but may retain its holds in fairness seeding.

Correction: sort candidates by a stated value heuristic (or document binding-id order as arbitrary), report total nodes and the binding that truncated, unclamp or separately flag negative availability, use one capacity/zero-capacity convention across fairness/cost/pacing, validate per-project weight consistency, and align reported shares with the optimized (weighted) definition or label the difference.

## Finding 10 (Medium): pilot scope is one-per-window globally; research-heavy work slips through

Locations: `unresolved_pilots`, `fits`, `add`, `coefficient_bundle` in `src/native_scheduler.rs`.

- Pilot keys are `account:window` strings shared across tasks, bindings, seats, and workflows. One unresolved pilot hold on a window blocks *every* other pilot candidate on that window, even for unrelated bindings. That is safe but coarse: a stuck pilot hold (e.g., settlement delayed across a reset) freezes all cold-start learning on the meter.
- Backtrack removal (`state.pilots.remove(key)`) is correct only because `fits()` prevents ever adding a pre-existing key; the invariant is implicit and fragile. A future change that admits overlapping pilots would silently unblock pre-existing holds on backtrack.
- As noted in Finding 2, the pilot-eligible template excludes `Complex | Extensive` but the template itself excludes `evidence_need`, so open-ended research pipelines enter on pilot coefficients with provisional benchmark priors (see Finding 11).

Correction: scope pilot keys per coefficient/binding (or per task family) with a stated concurrency limit instead of one-per-window, add explicit hold-expiry/reassessment for stuck pilots, and floor research-heavy templates out of pilot eligibility regardless of the file-count-independent template computation.

## Finding 11 (Medium): calibration transfer is labeled honestly but structurally thin

Locations: `METHODOLOGY.md` retry/competence sections; `route_qualification.rs` provisional branch; `native_scheduler.rs` coefficient use; `native_reconciliation.rs` settlement/observation.

Agreements first: never summing unlike units, requiring calibrated lower bounds for consequential work (in prose), separating expected/reserve/actual/attribution, carrying residual holds until covered, monotonic observation cutoffs, idempotent settlement digests, and refusing to invent synthetic calibration work are all sound. The provisional path (bounded low-consequence real work, named benchmark + explicit transfer assumption) is the right cold-start shape.

Gaps:

- The provisional transfer assumption is a non-empty string. Any string passes. "Benchmark X predicts harness Y because both are code" is formally sufficient. There is no check that the benchmark exercises the visit's criteria (e.g., a patch-generation score transferring to `adversarial_review` or `citation_accuracy`), only that *some* fresh role evidence covers the visit's criteria dimensions — and dimensions are strings supplied with the evidence.
- Native coefficients carry `sample_count`, but nothing in the reviewed code widens reserves/intervals for `n = 5` vs `n = 500`, borrows hierarchically with stated shrinkage, or enforces the documented provenance (sample count, range, workload, estimator, interval, last observation) at admission. The prose promises calibration provenance per coefficient; the admission predicate sees `expected/reserve/seconds/sample_count/manual_pilot`.
- Meter reality (delayed, quantized aggregates, shared-account contamination, reset-straddling calls) is handled carefully in reconciliation, but the planner consumes point reserves. Shared vendor-wide shocks have no scenario structure in the native solve. The Bertsimas–Sim budget-of-uncertainty reference therefore describes an aspiration, not a mechanism. (Primary: Bertsimas & Sim, "The Price of Robustness," Operations Research 52(1):35–53, [doi](https://doi.org/10.1287/opre.1030.0065).)
- Bandits-with-knapsacks ([FOCS 2013 extended abstract](https://ieee-focs.org/FOCS-2013-Papers/5135a207.pdf); [author page](https://slivkins.com/work/bandits-svc/)) and RouteLLM-style routing assume arms with learnable reward/consumption distributions and (in the basic form) relatively stationary, attributable feedback. Here feedback is delayed, quantized, externally contaminated, and unattributable without provider event IDs — conditions under which no regret bound transfers. Bounded exploration on qualified low-consequence work is the correct takeaway; any "online learning will converge to efficient routing" claim would need nonstationary, contaminated, partially-attributable machinery that is not present.

Correction: validate transfer relevance (benchmark × criterion matrix, not just non-empty strings), enforce sample-size-aware widening with hierarchical borrowing stated explicitly, add shared-shock scenarios to the native reserve model or drop the robustness-budget language to "point reserves + manual bands," and scope the bandit citation to motivation only.

## Finding 12 (Lower): source-research policy verifies access, not value

Locations: `qualify_research`, `evidence_covers` in `src/route_qualification.rs`; `visits()` research trigger; settlement packet path in `src/native_reconciliation.rs`; `METHODOLOGY.md` packet semantics.

- `qualify_research` checks tool-name substrings (`fetch/webfetch`, `search/websearch`), non-empty source/network scopes, and a `citation_accuracy` criterion string. It does not check retrieval quality, source-type coverage, freshness requirements, or contradiction handling — correctly so for a gate, but the gate should not be read as research competence.
- Packet acceptance (per `METHODOLOGY.md`) checks handoff structure and declared criteria, explicitly not citation truth or research quality. The snapshot's "source uncertainty remains separate from model ability" is the right framing.
- The native objective has no value-of-information term. `evidence_need > 0` toggles a research visit; nothing asks whether the uncertain fact could change route eligibility, risk, cost, schedule, or user choice (the decision-value test the prose states). Research is therefore costed but not valued in the solve — it is overhead to be reserved, never information to be weighed.
- `visits()` makes research depend only on orchestrator output and gates comprehension/implementation behind it whenever present. A failed or thin research visit blocks the chain by DAG order even when local authoritative context would suffice.

Correction: add the missing VOI hinge — trigger research from the decision it can change, record claim/URL/publisher/retrieval/support-span/type/freshness/uncertainty/consuming-decision as the docs require, and allow the DAG to proceed on local context when research is unavailable or low-value rather than hard-blocking. Keep the "no citation-truth certification" disclaimer; it is accurate.

## What would make the effectiveness claims supportable

1. Fix the repair deadlock (Finding 1) and the caller-controlled gates (Finding 2) first; everything else is secondary while high-risk repair is unschedulable and risk labels are self-asserted.
2. Replace summed uncertainty bands with fitted per-meter sets + shared-shock scenarios, or retract coverage language (Finding 5, Finding 11).
3. State fairness as throughput-first with possible starvation, or add historical shares + floors (Finding 3).
4. Model lanes/machines in the calendar or label serial placement as admission-safe pessimism (Finding 4).
5. Bound ledger age, separate `Confirmed` from `Verified`, unclamp negative availability, and align shares/weights/capacity conventions (Findings 6, 9).
6. Compare full bundles for checker diversity and treat even that as proxy pending measured creator/checker error correlation (Finding 8).
7. Scope pilots per binding/family with reassessment, and floor research-heavy work out of pilots (Finding 10).
8. Give research a decision-value trigger instead of an evidence-need flag (Finding 12).

## Primary sources consulted (via search/fetch)

- Ali Ghodsi et al., "Dominant Resource Fairness: Fair Allocation of Multiple Resource Types," NSDI 2011 — [USENIX page](https://www.usenix.org/conference/nsdi11/dominant-resource-fairness-fair-allocation-multiple-resource-types) / [full paper](https://www.usenix.org/events/nsdi11/tech/full_papers/Ghodsi.pdf). Used for Finding 3 (applicability vs analogy).
- Dimitris Bertsimas and Melvyn Sim, "The Price of Robustness," Operations Research 52(1):35–53 — [doi](https://doi.org/10.1287/opre.1030.0065). Used for Finding 5 / Finding 11 (budgets of uncertainty are fitted sets, not summed bands).
- Ashwinkumar Badanidiyuru, Robert Kleinberg, and Aleksandrs Slivkins, "Bandits with Knapsacks" (FOCS 2013) — [extended abstract](https://ieee-focs.org/FOCS-2013-Papers/5135a207.pdf) / [author page](https://slivkins.com/work/bandits-svc/). Used for Finding 11 (regret bounds do not transfer to delayed/contaminated/unattributable meters).
- Correlated LLM errors: [Correlated Errors in Large Language Models](https://arxiv.org/abs/2506.07962v1) (large-scale error-agreement measurement) and budgeted-ensemble analysis under strong correlation ([arXiv overview](https://arxiv.org/pdf/2602.08003v1)). Used for Finding 8.
- Google OR-Tools job-shop modeling (precedence + per-machine no-overlap) — [job-shop page](https://developers.google.com/optimization/scheduling/job_shop). Used for Finding 4 (global serialization vs per-machine no-overlap).
- Document-supplied sources taken as given for scope (Garicano hierarchies, Kremer O-ring, Greco–Mousseau–Słowiński robust ordinal regression, RouteLLM, BrowseComp/FutureSearch benches, provider tool/plan docs) were used to check analogy boundaries but were not re-verified beyond the citations already in `research/team-allocation.md` and `METHODOLOGY.md`.
