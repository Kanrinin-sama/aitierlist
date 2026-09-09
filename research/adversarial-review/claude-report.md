# Adversarial audit of the allocation policy

Provenance: Claude Opus 4.6 (`claude-opus-4-6`), high effort. Client: Claude Code session. Web tools used: WebSearch (successful, 2 queries on DRF indivisibility impossibility and Bertsimas-Sim integer extensions), WebFetch (not used directly; three parallel research subagents used WebSearch/WebFetch successfully across DRF/Bertsimas-Sim, Bandits with Knapsacks/correlated errors/O-Ring/Garicano, and code completeness verification). All cited papers were verified against published abstracts or full text where accessible.

---

## Findings ordered by severity

### 1. DRF sharing incentive is not preserved under lexicographic quantity-first ordering (HIGH)

The native scheduler (`native_scheduler.rs:104-113`) implements fairness as the fourth lexicographic level, subordinated to high-consequence coverage, deadline coverage, and weighted useful task count. `Score::compare` maximizes `useful` (level 3) before minimizing `fairness_cost` (level 4). The `useful` counter at line 1058 sums `1 + priority + consequence + deadline` per admitted task, so total admitted count dominates fairness.

**Counterexample.** Two projects each offer 5 ready tasks against one account with capacity for 5 total tasks. Project A tasks have priority=1; project B tasks have priority=0. Admitting 5 from A scores useful=10 (5 * (1+1+0+0)). The fairest mixed allocation (3A + 2B) scores useful=8. The solver selects all-A, giving project B zero allocation. DRF's sharing incentive guarantees each user at least 1/n of every resource; this property is violated.

The documents acknowledge this: "inspired by dominant resource fairness rather than claimed to inherit every DRF theorem." However, the specific lost properties are not enumerated. Parkes, Procaccia, and Shah ([ACM TEAC 2015](https://dl.acm.org/doi/abs/10.1145/2739040)) prove that with indivisible demands, strategy-proofness is incompatible with both Pareto efficiency and envy-freeness. Even relaxed EF1 (envy-free up to one bundle) cannot coexist with strategy-proofness in the integer setting. The system's integer allocation therefore cannot inherit sharing incentive, strategy-proofness, or envy-freeness simultaneously, regardless of where fairness sits in the lexicographic order.

**Correction.** Document which DRF properties are provably lost (strategy-proofness, envy-freeness, sharing incentive) and which are approximately preserved (Pareto efficiency under integer rounding). If sharing incentive matters operationally, the fairness level must be promoted to at least co-primary with useful count, or a minimum per-project allocation floor must be enforced as a hard constraint before optimization. Alternatively, adopt SEQUENTIALMINMAX-style allocation that preserves sharing incentive + EF1 at the cost of strategy-proofness, and document the tradeoff.

---

### 2. Bertsimas-Sim uncertainty model is cited but not implemented (HIGH)

The team-allocation document (line 102) references Bertsimas and Sim ([Operations Research, 2004](https://pubsonline.informs.org/doi/10.1287/opre.1030.0065)) for "budgets of uncertainty" that "interpolate between nominal and worst-case conservatism." The `NativeCoefficient` struct (`agent_setup.rs:335-348`) carries `uncertainty_lower` and `uncertainty_upper` fields, suggesting interval uncertainty.

However, the native scheduler's `fits()` function (`native_scheduler.rs:1020-1047`) checks only `coefficient.reserve` against capacity. The uncertainty bounds are never used during search or admission. They appear only in the informational `window_pacing()` output (`native_scheduler.rs:1354-1396`), which sums them per window for display. There is no uncertainty budget parameter Gamma, no constraint-wise uncertainty set, and no robust counterpart formulation.

**Counterexample.** A route has expected=10, reserve=15, uncertainty_upper=20 native units. Available capacity is 16. The `fits()` check passes (15 <= 16). Actual consumption reaches 20, exhausting the account. Bertsimas-Sim with Gamma=1 would protect against one coefficient reaching its worst case; the simple reserve does not interpolate between nominal and worst case.

Bertsimas-Sim requires: (a) a linear nominal problem, (b) uncertainty entering only in constraint coefficients, not problem structure, (c) symmetric bounded intervals. The native scheduler's backtracking search is not an LP, so the Bertsimas-Sim tractability result (robust counterpart remains LP) does not apply. The 2003 companion paper "Robust Discrete Optimization and Network Flows" (Mathematical Programming 98(1)) addresses integer problems but requires specific combinatorial structure not present here.

**Correction.** Either implement budgeted uncertainty sets fitted per route/task with an explicit Gamma parameter and robust admission constraint, or remove the Bertsimas-Sim citation from the implemented-semantics description and replace it with a description of the actual mechanism: per-coefficient worst-case reserve with no budget interpolation. Retain the citation only in the research-aspirations section.

---

### 3. The `useful` score double-counts consequence across lexicographic levels (MEDIUM)

`Score::compare` at level 1 (`high_consequence`) sums `consequence` for tasks with consequence >= 2. At level 3 (`useful`), consequence is added again for all admitted tasks regardless of threshold (`native_scheduler.rs:1054-1061`):

```
score.high_consequence += consequence  // level 1, only when >= 2
score.useful += 1 + priority + consequence + deadline  // level 3, all tasks
```

A task with consequence=3 contributes 3 at level 1 AND 3 at level 3. Level 1 already guarantees high-consequence tasks are admitted first; re-counting consequence in level 3 gives them a second advantage over low-consequence tasks when deciding among solutions that tie on level 1. The documented intent says level 2 is "minimize high-consequence unserved demand" and level 3 is "maximize priority-weighted useful admitted tasks." Including consequence in the priority weight conflates two conceptually separate concerns.

**Correction.** Either remove `consequence` from the `useful` sum (leaving it as `1 + priority + deadline`) or document that the double-counting is intentional to provide a continuous preference gradient rather than a strict two-level separation.

---

### 4. Exhaustive search is practically unreachable; status term may mislead (MEDIUM)

The native scheduler reports `SearchStatus::Exhaustive` when the DFS tree is fully explored (`native_scheduler.rs:319-322`). Each `walk()` call counts as one node (line 981). For n tasks each with k candidates, the tree has O((k+1)^n) nodes. With 10 tasks and 5 candidates each, this is approximately 60 million nodes. Any realistic node limit (the proxy solver uses 1024, per `portfolio.rs:11`) will produce `FeasibleSearchLimit` for nontrivial instances.

Additionally, `Exhaustive` means "full tree explored," not "feasible solution found." When every leaf fails `calendar_fits` (line 986-994), the search is exhaustive but the incumbent remains all-None (the initial default at line 271), producing zero admitted tasks. A consumer expecting `Exhaustive` to imply "found an optimal assignment" would be misled.

**Counterexample.** 3 tasks, each with 2 candidates; all candidates pass `fits()` but every complete assignment exceeds the calendar makespan. The search exhaustively explores all 27 nodes, reports `Exhaustive`, and admits zero tasks.

**Correction.** Distinguish `Exhaustive` into `ExhaustiveOptimal` (full tree explored, incumbent found) and `ExhaustiveInfeasible` (full tree explored, no feasible assignment). Document the practical search-space size and the conditions under which exhaustive enumeration is feasible (small task counts, few candidates per task).

---

### 5. No quality bound exists for the native scheduler (MEDIUM)

The `feasible_gap` field in `NativeHorizon` is always `None` (`native_scheduler.rs:326`). Unlike the proxy solver (`solver.rs`), which computes LP relaxation bounds and reports gaps, the native scheduler provides no upper bound on the optimal objective. When the search reports `FeasibleSearchLimit`, the user knows the incumbent may be suboptimal but has zero quantitative information about how far.

The proxy solver computes:
- LP relaxation bound via the simplex method (`solver.rs:42-137`)
- Gap as `(bound - quality) / quality` (`portfolio.rs:990-992`)
- Proven-optimal status when all pending nodes are pruned (`solver.rs:597`)

The native scheduler has no LP relaxation, no dual prices, and no bounding. The snapshot document correctly states "reports no fabricated optimum proof, dual price, or numerical gap," but the asymmetry between the two solvers means the native scheduler cannot support the same quality assurance as the proxy solver.

**Correction.** Either implement an LP relaxation for the native scheduler to provide upper bounds (the structure admits one: relax integer task assignments to fractions, solve the resulting multi-resource assignment LP), or document that the native scheduler is a feasibility-focused heuristic without optimality guarantees, and that quality assessment requires comparing multiple horizons or using the proxy solver as a reference.

---

### 6. Bandits with Knapsacks theory does not apply to this setting (MEDIUM)

The team-allocation document (line 119) cites Badanidiyuru, Kleinberg, and Slivkins ([arXiv:1305.2545](https://arxiv.org/abs/1305.2545)) for "learning while resources deplete." The theory requires: (a) stochastic i.i.d. rewards from a fixed distribution, (b) a time horizon T large relative to budgets (regret bounds scale as O(OPT/sqrt(T))), and (c) graceful degradation on constraint violation (the algorithm simply stops, rather than suffering catastrophic loss).

This system has approximately 5 provider accounts, weekly horizons with perhaps 5-20 replan events per week, unknown and changing reward distributions, and catastrophic constraint violations (overspending a subscription quota that cannot be reversed within the reset window). The asymptotic guarantees are vacuous at this scale. No learning loop is implemented; the system replans from scratch at each horizon.

The document frames this as aspiration: "Online learning is necessary because route quality and consumption are uncertain." This is honest, but the citation could imply theoretical grounding that does not exist.

**Correction.** Retain the citation only in the research-aspirations context. Add a sentence stating that the implemented system performs no online learning or exploration-exploitation tradeoff; route quality is assessed through discrete replan events rather than bandit-style arm pulls.

---

### 7. Correlated-error diversity is weaker than implied (MEDIUM)

The `checker_correlated` function (`native_scheduler.rs:861-884`) enforces creator/checker diversity when consequence >= 2 or correlation >= 2. The `same_identity` function (`native_scheduler.rs:886-899`) considers two routes correlated when they share either the same host_id OR the same display_model (line 898: `left.host_id == right.host_id || left.display_model == right.display_model`).

Kim et al. ([ICML 2025](https://proceedings.mlr.press/v267/kim25e.html)) demonstrate that even models from distinct providers and architectures exhibit high error correlation. On tested benchmarks, models agreed on errors approximately 60% of the time. Larger, more accurate models had more correlated errors, not fewer, even across families. This implies that provider diversity buys less decorrelation than the `same_identity` check assumes.

The system correctly notes that "correlated-error research warns that teams gain less from aggregation when members share failure modes." But the implementation's binary same-model-or-same-host gate does not address the empirical finding that different-model-different-host pairs can still be highly correlated on hard tasks.

**Correction.** Document that the `same_identity` check is a necessary but not sufficient condition for decorrelation. The system cannot measure actual error correlation without real-work outcome data. When such data becomes available, the correlation gate should be refined from binary identity matching to an empirical correlation threshold derived from observed creator/checker agreement rates.

---

### 8. O-Ring and Garicano citations are analogies without implementable structure (LOW)

The O-Ring theory (Kremer 1993) uses a strictly multiplicative production function: output = product of q(i) across workers. The document says it uses O-Ring "as an application of the concept rather than a literal calibrated production function." This is defensible as qualitative motivation (weakest-link reasoning), but the system implements no multiplicative complementarity. Role utilities are additive weighted sums (`METHODOLOGY.md:72-82`), not multiplicative.

Garicano's hierarchy model (JPE 2000) assumes exponentially distributed problem difficulty over a continuum and known distribution parameters. With approximately 10 discrete task types whose difficulty distribution is unknown and changing, the continuum assumption fails and escalation thresholds cannot be computed.

Neither citation provides implementable structure for the current system. They motivate design choices (role qualification, escalation) without constraining the optimization.

**Correction.** No implementation change needed. The citations are used appropriately as conceptual motivation. Consider adding a sentence to each citation noting the specific structural assumption that prevents direct application (multiplicative production function for O-Ring; known continuous difficulty distribution for Garicano).

---

### 9. Proxy solver no-good cuts inflate the reported bound (LOW)

In `solver.rs:567-574`, when a rounding candidate passes resource checks but fails the calendar feasibility test (`evaluate` returns `None`), a no-good cut is added and the node is re-queued with `bound: relaxation.quality`. This bound was computed before the cut existed; the post-cut relaxation will be weakly tighter. The stale bound appears in the final bound computation (`solver.rs:598-603`), which takes the maximum across all pending nodes.

This makes the reported bound conservative (too high), which inflates the reported gap. The system never falsely claims optimality because proven status requires `pending.is_empty()`. However, the gap reported to the user overestimates suboptimality, potentially causing unnecessary concern.

**Correction.** Either re-solve the relaxation before storing the bound on the re-queued node, or document that the reported gap is an upper bound on the true gap and may be loose when no-good cuts are active.

---

### 10. The `unreflected()` check is correct but fragile (LOW)

`native_scheduler.rs:499-501`:
```rust
fn unreflected(reflected_through: Option<&str>, snapshot_at: &str) -> bool {
    reflected_through != Some(snapshot_at)
}
```

This uses string equality on RFC3339 timestamps. The logic is correct: a hold is unreflected if it was not covered by the exact observation whose remaining balance is the current snapshot. However, RFC3339 strings can represent the same instant differently (e.g., `2026-09-09T12:00:00Z` vs `2026-09-09T12:00:00+00:00`). If any code path produces a different RFC3339 serialization for the same instant, a hold could be double-counted (conservatively) or, worse, considered reflected when it is not.

The `time` crate's `Rfc3339` formatter produces canonical output, so the risk is limited to external inputs (meter observations, settlement timestamps). The reconciliation module (`native_reconciliation.rs`) sets `reflected_through` to `input.observed_at` verbatim (line 365), and native capacity checks against `snapshot.observed_at` verbatim (line 469). As long as both originate from the same string, the comparison is safe.

**Correction.** Consider parsing both timestamps and comparing `OffsetDateTime` values rather than strings, to be robust against non-canonical RFC3339 inputs. Alternatively, document the canonicalization invariant required of all external timestamp inputs.

---

## Summary of theorem applicability

| Citation | Status | Key structural gap |
|---|---|---|
| DRF (Ghodsi et al. 2011) | Analogy; explicit disclaimer | Integer allocation provably loses strategy-proofness (Parkes et al. 2015); lexicographic subordination loses sharing incentive |
| Bertsimas-Sim (2004) | Cited but not implemented | No uncertainty budget Gamma, no robust counterpart; reserves are simple worst-case holds |
| Bandits with Knapsacks (2013) | Aspiration only | No learning loop; horizon too short for asymptotic guarantees |
| RouteLLM (2024) | Aspiration only | No preference-data-based routing is implemented |
| O-Ring (Kremer 1993) | Conceptual motivation | Multiplicative production function not implemented; role utilities are additive |
| Garicano (2000) | Conceptual motivation | Known continuous difficulty distribution not available |
| Kim et al. correlated errors (2025) | Partially implemented | Binary identity gate is necessary but not sufficient; empirical correlation is higher than the gate captures |
| Job-shop scheduling (Google OR-Tools) | Structural analogy | Implemented scheduler is conservative serial placement, not a job-shop solver |
| Robust ordinal regression (Greco et al. 2008) | Acknowledged absent | No preference observations from which to fit a function family |
| Epsilon-constraint (Mavrotas 2009) | Implemented | Competence floors as constraints on an economic objective match the method |

## Sources

- [Ghodsi et al., DRF (NSDI 2011)](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)
- [Parkes, Procaccia, Shah, Beyond DRF (ACM TEAC 2015)](https://dl.acm.org/doi/abs/10.1145/2739040)
- [Bertsimas and Sim, The Price of Robustness (Operations Research, 2004)](https://pubsonline.informs.org/doi/10.1287/opre.1030.0065)
- [Bertsimas and Sim, Robust Discrete Optimization (Mathematical Programming, 2003)](https://link.springer.com/article/10.1007/s10107-003-0396-4)
- [Badanidiyuru, Kleinberg, Slivkins, Bandits with Knapsacks (2013)](https://arxiv.org/abs/1305.2545)
- [Kim et al., Correlated Errors in LLMs (ICML 2025)](https://proceedings.mlr.press/v267/kim25e.html)
- [Kremer, The O-Ring Theory (QJE, 1993)](https://econintel.org/Cl705/KremerORing.pdf)
- [Garicano, Hierarchies and Knowledge (JPE, 2000)](https://www.journals.uchicago.edu/doi/10.1086/317671)
- [Greco, Mousseau, Słowiński, Robust Ordinal Regression (EJOR, 2008)](https://www.lamsade.dauphine.fr/mcda/biblio/PDF/GMS-EJOR2008.pdf)
- [Mavrotas, Effective Implementation of the Epsilon-Constraint Method (Applied Mathematics and Computation, 2009)](https://www.sciencedirect.com/science/article/pii/S0096300309002574)
- [Google OR-Tools, Job-Shop Scheduling](https://developers.google.com/optimization/scheduling/job_shop?hl=en)
- [RouteLLM (2024)](https://arxiv.org/abs/2406.18665)
