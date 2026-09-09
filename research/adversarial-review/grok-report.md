I have the source, the scheduler math, and the primary papers. The report below is the full audit.

# Adversarial audit: allocation fairness, entitlements, starvation, strategy, deadlines, cross-project accounting

## Provenance

- **Client / model / effort:** Grok 4.6 (xAI), dispatched on kaltsit. Independent adversarial audit of the allocation policy as a mathematical/operational decision system. High effort: full read of every named source, then primary literature on DRF, weighted DRF, indivisibilities, bottleneck fairness, dynamic/stateful DRF, price of fairness, and EDF.
- **Named sources read in full:** `research/adversarial-review/snapshot.md`, `research/team-allocation.md`, `METHODOLOGY.md`, `src/native_scheduler.rs`, `src/team_policy.rs`, `src/route_qualification.rs`, `src/native_reconciliation.rs`. No other file under `research/adversarial-review` was read. Application code, UI, and serialization were not reviewed except as policy math in those files.
- **Web tools:** `web_search` (successful: Ghodsi, Parkes–Procaccia–Shah, Dolev et al., Joe-Wong et al., Kash–Procaccia–Shah, Sadok et al., Bertsimas–Farias–Trichakis, Liu–Layland). `web_fetch` (successful: Ghodsi NSDI 2011 PDF from USENIX; Parkes et al. TEAC 2015 PDF; Dolev et al. arXiv:1106.2673 PDF). Brave MCP was not used. No tool failed in a way that blocked a citation; where a page was a landing page rather than the paper body, the USENIX/TEAC/arXiv PDFs were used.

**Theorem vs analogy (global):** Ghodsi DRF theorems (sharing incentive, strategy-proofness, envy-freeness, Pareto efficiency) apply to *progressive filling that equalizes dominant shares* $s_i=\max_j(u_{ij}/R_j)$ of *divisible* Leontief demands, with users reporting demand vectors, over *total* endowments. They do **not** apply to this allocator. The implementation is at most DRF-inspired: a later lexicographic term that minimizes the *maximum* entitlement-weighted residual share. Parkes et al. further show that with *indivisible* tasks, no mechanism can be Pareto efficient, sharing-incentive, and strategy-proof together. Those caveats are used below; they are not a claim that the code “implements DRF incorrectly.” The snapshot already says fairness is inspiration, not inherited theorems. The audit asks whether the *implemented* rule still does the operational work the snapshot and `team-allocation.md` describe.

Empirical coefficients, real meter distributions, and team success rates are **unknown**. Counterexamples are constructed from the declared math.

---

## Conclusion

The implemented policy does **not** provide dominant-resource-fair entitlements, isolation, or strategy-proofness. Fairness is a fourth lexicographic term after an unbounded, project-declared “useful” score, so a project can be starved whenever another project can post more `1 + priority + consequence + deadline-risk` points. The quantity called a dominant share is not Ghodsi’s share: the denominator is *remaining* window capacity, started holds sit in the numerator, zero remaining maps to share `0`, and wall-clock / host / exclusive / human resources are omitted. Once any project has large attributable started holds relative to residual capacity, fairness among *new* work is inert. Declared `priority`, `entitlement_weight`, `artifact_value`, risk-vector fields, `human_seconds`, and generation membership are strategic. Deadline protection uses a user-set risk ordinal and priority-first serialization, which can miss a feasible earliest-deadline schedule. Cross-project accounting mixes reset instances, lets omitted never-started holds squat on residual capacity, and reports a different dominant-share formula than the one the search minimizes.

Those are conceptual defects in the decision rule, not implementation bugs in the sense of the snapshot.

---

## Findings (severity order)

### 1. Critical — “Useful” lexicographically dominates fairness; sharing incentive and isolation fail

**What the policy says.** Snapshot lex intent: qualifications; then high-consequence and expiring deadlines; then “maximize priority-weighted useful admitted tasks”; then “entitlement-weighted dominant-resource-share fairness including attributable started holds.” `team-allocation.md` lines 104–113 put “weighted useful admitted tasks with project fairness” in one numbered step and cites Ghodsi DRF as the fairness principle.

**What it does.**

```104:113:src/native_scheduler.rs
        self.high_consequence
            .cmp(&other.high_consequence)
            .then_with(|| self.deadline.cmp(&other.deadline))
            .then_with(|| self.useful.cmp(&other.useful))
            .then_with(|| other.fairness_cost.total_cmp(&self.fairness_cost))
            .then_with(|| self.artifact_value.total_cmp(&other.artifact_value))
            .then_with(|| other.resource_cost.total_cmp(&self.resource_cost))
            .then_with(|| other.seconds.total_cmp(&self.seconds))
```

```1085:1088:src/native_scheduler.rs
            score.useful += 1
                + u64::from(task.priority)
                + u64::from(task.risk.consequence)
                + u64::from(task.risk.deadline);
```

Fairness never trades against `useful`. `priority` is an unconstrained `u32`. Ghodsi’s sharing incentive is: with $n$ users, no user is worse off than a static $1/n$ partition of *every* resource ([Ghodsi et al., NSDI 2011, §3 and Thm. on sharing incentive](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)). Isolation is the same guarantee against others’ demand. This search has **no** per-project floor.

`_project_counts` is accumulated in `walk` and passed into `score`, then ignored:

```1076:1076:src/native_scheduler.rs
    fn score(&self, selection: &[Option<usize>], _project_counts: &BTreeMap<String, u32>) -> Score {
```

There is no max-min on per-project task counts, no progressive filling, no leximin on the share vector.

**Theorem applicability.** Not an analogy failure at the margin: the DRF guarantee is simply not present. Bertsimas, Farias, and Trichakis define the *price of fairness* as efficiency loss from imposing max-min or proportional fairness ([Operations Research 59(1):17–31, 2011](https://doi.org/10.1287/opre.1100.0865)). Here the price of fairness is designed to be zero: efficiency-like `useful` is unconstrained by fairness. The dual cost is unbounded unfairness.

**Counterexample (starvation).** One native window, remaining $10$, two projects, entitlement $1$, no started holds, all tasks qualified and feasible alone.

| Project | Tasks | Reserve each | `priority` | `useful` each |
|---|---:|---:|---:|---:|
| A | 3 | 3 | 10 | $1+10=11$ |
| B | 1 | 3 | 1 | $2$ |

Admitting A’s three tasks: `useful=33`, usage $9$. Any set that includes B has `useful≤24`. Lex comparison never reads `fairness_cost`. B is deferred with “Not admitted within the current native-window, time, and fairness allocation” (`native_scheduler.rs` 297–298) even though fairness is not why. Repeat arrivals of A-like work starve B forever: there is no aging.

Weighted DRF would equalize $s_A/w_A$ and $s_B/w_B$ by progressive filling ([Ghodsi §4.3](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)): next task goes to the project with smaller weighted dominant share, so B receives work until shares match. Dolev et al.’s bottleneck-based fairness (“no justified complaints”) says a user may complain unless they are fully served or they hold their entitlement on some bottleneck ([arXiv:1106.2673](https://arxiv.org/pdf/1106.2673), displayed eq. (2)). B has a justified complaint: the bottleneck is not saturated by B’s entitlement; A took it for `useful` points.

---

### 2. Critical — The “dominant share” is not a share of endowment; started holds make fairness inert or inverted

**Implemented fairness cost:**

```1097:1128:src/native_scheduler.rs
    fn fairness_cost(&self, selection: &[Option<usize>]) -> f64 {
        let mut project_usage = self.existing_project_usage.clone();
        ...
                        self.capacity
                            .get(&resource)
                            .filter(|value| **value > 0.0)
                            .map_or(0.0, |value| amount / value / weight)
                    })
                    .fold(0.0, f64::max)
            })
            .fold(0.0, f64::max)
    }
```

Ghodsi dominant share is $s_i=\max_j(u_{ij}/R_j)$ with $R_j$ **total** capacity; Algorithm 1 updates $s_i=\max_j(u_{ij}/r_j)$ after adding to the user’s allocation vector ([Ghodsi, Alg. 1 and §4.1](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)). Weighted form: $s_i=\max_j(u_{ij}/w_{ij})$ (§4.3).

Here:

1. **Denominator is residual** `native_capacity` (remaining − unreflected holds − external − `spent_since_snapshot`), not $R_j$.
2. **Numerator includes started holds** via `existing_project_usage` (jobs not Queued/Ready/Held/Released).
3. **If residual $\le 0$, share is defined as $0$** (`map_or(0.0)` / `filter(|value| **value > 0.0)`).
4. The objective is $\min \max_i s_i$, not leximin (maximize the smallest $s_i$, then the second, …). Ghodsi: “maximize the smallest dominant share … then the second-smallest, and so on.” Wikipedia/leximin statement of DRF is the same.

**Counterexample (inversion at saturation).** Window total $100$. Project A started holds $100$, residual $0$. A’s fairness share is **$0$**. A new project B using residual $0$ cannot fit, but A is treated as the fairest possible user while owning the meter. DRF would report $s_A=1$.

**Counterexample (inert fairness).** A started $90$, residual $10$, $s_A=90/10=9$. Any new bundle with residual-share $\le 9$ does not change $\max_i s_i$. Fairness cannot distinguish packing the residual onto B vs C vs a fourth A-task that stays under the ceiling. `useful` decides. Attributable started holds, which the snapshot says are *included so fairness sees them*, instead **raise a ceiling that turns fairness off** for typical new allocations.

Joe-Wong et al. treat DRF as $\max \min_i \mu_i x_i$ over *job counts* with resource-ratio constraints, using **total** $C_i$ ([IEEE/ACM ToN 21(6):1785–1798, 2013](https://doi.org/10.1109/TNET.2012.2233213); [INFOCOM 2012 PDF](https://www2.seas.gwu.edu/~tlan/papers/fairness_multi_resources.pdf) eq. (4)). Residual-normalized shares are a different object: they behave like a bottleneck *price of leftover slack*, not an entitlement against endowment.

Parkes, Procaccia, and Shah warn that even the “intuitive” $\max\min$ dominant share, and variants that score shares *before* rather than *after* a bundle, fail the indivisible-task properties; their SEQUENTIALMINMAX assigns the next bundle to the agent that **minimizes MaxDom after the assignment** ([TEAC 3(1), 2015, §5 and Thm. 5.4](https://www.cs.toronto.edu/~nisarg/papers/beyondDRF.teac.pdf); [Harvard DASH](http://nrs.harvard.edu/urn-3:HUL.InstRepos:11956916)). This search does compute MaxDom after a *full* selection, but only as lex-4, and it does not choose the next task that way. Under a node limit it is prefix-greedy in consequence/priority order (`walk` 1023–1044, sort 182–190).

**Reported shares ≠ optimized shares.** `dominant_shares` (1230–1263) uses only *new* plan reserves, divides by residual capacity, and **does not divide by entitlement**. Operators reading `NativeHorizon.dominant_shares` do not see the quantity `fairness_cost` minimized, nor started holds.

---

### 3. High — Priority and deadline-risk beat actual deadlines; EDF-feasible work can be dropped

Three different “deadline” objects exist:

| Object | Role in the decision |
|---|---|
| `task.deadline` timestamp | Eligibility (`>` now), plan duration vs slack, `calendar_fits` end vs deadline |
| `task.risk.deadline` `u8` | Workflow template, **search sort**, `score.deadline` **maximized**, `useful` **increased** |
| Calendar order | Consequence desc, **priority desc**, then timestamp |

```182:190:src/native_scheduler.rs
    eligible.sort_by(|left, right| {
        right
            .risk
            .consequence
            .cmp(&left.risk.consequence)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| right.risk.deadline.cmp(&left.risk.deadline))
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
```

```1161:1168:src/native_scheduler.rs
        work.sort_by(|(left, _, _), (right, _, _)| {
            right
                .risk
                .consequence
                .cmp(&left.risk.consequence)
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| deadline_key(left).cmp(&deadline_key(right)))
        });
```

Lex goal 2 in `team-allocation.md` is *minimize unserved high-consequence demand and expired deadlines*. The score **maximizes admitted deadline-risk mass** and **maximizes admitted consequence mass** (`high_consequence` only if `consequence >= 2`). A far-away task with `risk.deadline = 3` outranks an imminent task with `risk.deadline = 0`. Splitting one consequence-3 task into two consequence-2 tasks raises `high_consequence` from 3 to 4.

The calendar is a **single serial cursor** (snapshot: “conservatively serializes selected visits”). Liu and Layland: on one processor, the deadline-driven (EDF) rule is optimal for feasibility — if any feasible schedule exists, EDF finds one ([J. ACM 20(1):46–61, 1973](https://doi.org/10.1145/321738.321743)). Priority-then-deadline is not EDF.

**Counterexample (priority inversion against time).** Horizon 8h. Task H: priority 10, duration 3h, deadline 8h. Task D: priority 1, duration 2h, deadline 2h. EDF: D at $[0,2]$, H at $[2,5]$, both meet deadlines. Implemented calendar: H first, D starts at $t=3>2$, joint leaf fails; only-H leaf has `useful=11` vs only-D `useful=2`. D is dropped. The snapshot’s “protect … expiring deadlines” is false for this pair.

`score.deadline` does not use slack. `RiskVector.template()` promotes `deadline >= 2` into Complex/Extensive (`team_policy.rs` 25–47), which *adds* visits and makes tight deadlines harder to fit — the opposite of reducing scope as `team-allocation.md`’s deadline row recommends.

---

### 4. High — The mechanism is not strategy-proof; several declared fields are profitable lies

Ghodsi strategy-proofness (Thm. 12): a user cannot raise *dominant share* by misreporting the **demand vector**. That theorem is about $D_i$, not about priority labels. Parkes et al. Thm. 5.1: with indivisible tasks, **no** mechanism is simultaneously Pareto efficient, sharing-incentive, and strategy-proof ([TEAC 2015, §5](https://www.cs.toronto.edu/~nisarg/papers/beyondDRF.teac.pdf)). This allocator has indivisible tasks (`z` is 0/1 admission of whole workflows). Claiming DRF incentive-compatibility would be a category error; the snapshot does not. The operational problem is that the *declared* fields below move the actual objective.

| Lever | Effect | Incentive |
|---|---|---|
| `priority: u32` | Adds to `useful` (lex 3); sort key for search and calendar | Inflate without bound |
| `entitlement_weight` | Divides fairness share | Inflate to hide consumption (only bites when `useful` ties) |
| `risk.consequence` | Lex 1 if $\ge 2$; `useful`; sort; heavier template; calibrated-evidence gate | Inflate to 2+ if calibrated evidence exists; else stay at 1 to avoid the gate while still scoring `useful` |
| `risk.deadline` | Lex 2; `useful`; sort; template at $\ge 2$ | Inflate the ordinal independently of `task.deadline` |
| `artifact_value` | Lex 5 | Inflate when useful and fairness tie |
| `human_seconds` | Last-visit duration + human capacity | Under-declare |
| `required_resources` | Exclusive seconds = **full task** duration against each name | Under-declare exclusivity |
| Split into N ready authorized IDs | `useful += N·(1+p+…)` | Split |
| Omit from generation / `ready=false` / `authorized=false` | Never-started holds stay charged (`native_capacity` 453–461) | Squat |

**Counterexample (split).** One authorized job as one task: `useful = 1+p`. As ten tasks with the same `p`: `useful = 10(1+p)`. Lex 3 prefers the split until resources bind. DRF progressive filling is invariant to splitting *demand vectors into identical tasks* in the divisible model; here the **objective itself** pays for extra IDs.

**Counterexample (entitlement).** Residual 10. A weight 100 uses 9 → share $0.009$. B weight 1 uses 1 → share $0.1$. Min-max prefers loading A. Weighted DRF *intends* $w_A:w_B$ if weights are exogenous cluster entitlements ([Ghodsi §4.3](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)). Here `entitlement_weight` lives on `TaskRequest` (`team_policy.rs` 66) and `or_insert` keeps the **first admitted task’s** weight (`fairness_cost` 1103–1105). Two tasks of one project can disagree; only one weight counts. A project with **only started holds** (no new admission) gets `unwrap_or(1.0)` regardless of its true weight.

Ghodsi’s anecdotal manipulations (map/reduce slot gaming; infinite loops to inflate utilization, §3) are the same class: users game the *score the scheduler actually uses*. Here that score is `useful`, not $D_i$.

Dynamic-demand DRF is only approximately incentive-compatible: $(1+\rho)$-IC where $\rho$ measures cross-user resource importance ([arXiv:2109.12401](https://arxiv.org/pdf/2109.12401)). This policy is not in that family: the profitable misreport is not the demand ratio but the lex keys.

---

### 5. High — Generation-scoped replaceability is a cross-project squat

```453:461:src/native_scheduler.rs
            let holds: f64 = ledger
                .jobs
                .iter()
                .filter(|job| job.status != JobStatus::Released)
                .filter(|job| {
                    !matches!(
                        job.status,
                        JobStatus::Queued | JobStatus::Ready | JobStatus::Held
                    ) || !replaceable.contains(job.parent_task_id.as_str())
                })
```

`replaceable` is **eligible tasks in this call only** (192–193). Never-started holds are released for joint reassignment only if the parent is in that set. Snapshot: “holds outside the generation and workspace replacement scope stay fixed.”

**Counterexample.** Window 100. Project A has Held/Queued reservations totaling 80 and is omitted from the supplied set, or is supplied with `ready=false` / `authorized=false` (ineligible, hence not replaceable; `task_eligibility` 389–423). Residual 20. Project B’s ready work can take at most 20. A occupies 80 without competing on `useful`, consequence, or fairness. That is not “frozen started work”; it is unstarted inventory used as a capacity claim.

Kash, Procaccia, and Shah’s Dynamic DRF makes allocations **irrevocable** and still water-fills remaining agents toward equal dominant shares under a $k/n$ cap ([JAIR 51:579–603, 2014](https://www.cs.toronto.edu/~nisarg/papers/DynamicFairDivision.JAIR.pdf), Thm. 4: SI, DEF, DPO, SP). Irrevocability is for *already granted* allocations. Treating *never-started* out-of-scope holds as irrevocable violates the sharing incentive those dynamic mechanisms preserve.

---

### 6. High — Reset-instance mixing: old holds debit new remaining

Capacity holds match `account_id` + `window_id` only (`native_scheduler.rs` 464–466). They do **not** match `window_start` / `reset_at`.

`apply_observation` *does* distinguish instances (`native_reconciliation.rs` 390–402, 575–581). `unique_coverage` for holds requires matching start and reset (622–638). Settlement is instance-aware (`current_instance`, 197–198). Methodology: “never moves settled use from an old reset instance into a new one”; “Calls spanning a reset retain their holds and measured window attribution.”

**Counterexample.** Hold $H$ on instance $[T_0, R_0)$ amount 40, unresolved. Observation advances the ledger window to $[R_0, R_1)$, remaining 100 of the new grant. `native_capacity` still subtracts 40 because `window_id` matches. New-episode residual is reported as 60. Cross-project: every project in the new episode pays A’s previous-episode residual. That is not straddling attribution; it is charging a new endowment for an old instance.

`unreflected` is string inequality on `observed_at` (499–501). After `apply_observation`, cumulative coverage rewrites `reflected_through` to the new cutoff for still-listed holds, so same-instance re-snapshots are consistent. The break is the **new instance** path, where old holds are not in the new coverage set and remain charged against the new `window_id`.

---

### 7. Medium — Resource vector for fairness omits the resources that actually serialize work

`fits` constrains native meters, **per-account seconds**, **per-host seconds**, **exclusive resources**, **human seconds**, and **one uncalibrated pilot per account:window** (1047–1074). `fairness_cost` and `dominant_shares` see **only** native meter reserves.

DRF’s point is that the *dominant* resource may differ across users ([Ghodsi §4](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)). If the binding constraints are human attention or an exclusive lock, a project can take $\approx 1$ of those while showing a small quota share.

**Counterexample.** Two projects, quota plentiful. Human capacity $H$. A declares `human_seconds = H` on one high-`useful` task. B’s work needs any human time. A’s quota-dominant share is tiny; B is infeasible. Fairness reports A as light.

Exclusive resources charge the **sum of all visit durations** against each `required_resources` name (`candidate` 979–983), not the visit that needs the lock. Over-declared exclusivity blocks others; under-declared exclusivity is the profitable strategy (finding 4).

`account_seconds_capacity` is the **full horizon per account** (262–263), i.e. one lane per account in `fits`, while `calendar_fits` then **serializes all visits of all accounts onto one cursor** (1169–1202). `fits` can accept parallel account-seconds that the declared calendar cannot place. Nodes are spent on combinations that die only at the leaf. Host concurrency (`horizon * concurrency`) has the same mismatch. Snapshot correctly says the calendar is not a job-shop optimum; the fairness/capacity accounting still pretends those parallel resources exist.

---

### 8. Medium — Prefix-greedy bounded search + sort order encodes first-mover privilege

```1006:1044:src/native_scheduler.rs
        if self.nodes >= self.node_limit {
            self.limited = true;
            return;
        }
        ...
        for choice in 0..self.candidates[index].len() {
            ...
            if self.limited {
                return;
            }
        }
        self.walk(index + 1, selection, state);
```

Skip of task $i$ is explored only after every candidate subtree of $i$. Combined with consequence/priority sort, a node limit makes early tasks **sticky**: their skip may never run. `candidate_plans` also truncates route assemblies at the same `node_limit`, in `binding_id` order (733–749, 856–884). Comparative advantage / shadow prices in `team-allocation.md` 115–117 are not implemented (`feasible_gap` is always `None`, line 325).

Parkes SEQUENTIALMINMAX picks, among remaining feasible agents, the one whose next bundle minimizes post-allocation MaxDom. Progressive filling picks the current lowest dominant share ([Ghodsi Alg. 1](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf)). This walk picks the next task in a **static risk/priority permutation**. Under `FeasibleSearchLimit`, that permutation is the policy.

Sadok et al. Stateful DRF schedules the user with **lower historical average usage** first and keep DRF’s SP/SI/efficiency claims in the long run ([IEEE 2020 / GTA technical report](https://www.gta.ufrj.br/ftp/gta/TechReports/SCC18.pdf)). `existing_project_usage` is a fragment of that idea, but it is not the scheduling key.

---

### 9. Medium — One conductor binding couples every project in the supplied set

`allocate` searches conductors, then jointly assigns the whole eligible set (248–287). Orchestrator visits must use that binding (`candidate_plans` 713–714). Snapshot: one persistent conductor across the supplied set.

**Cross-project effect.** If A’s qualified worker routes only exist under conductor $C_A$ and B’s under $C_B$, the better `incumbent_score` wins and the other project’s tasks all defer (“No exact qualified route bundle…” or the conductor they needed was not chosen). That is not DRF sharing of heterogeneous resources; it is a **single discrete mode** for the whole generation. A project can also force co-tenants onto its conductor if its `useful` mass dominates the conductor comparison.

Conductor overhead beyond per-task Orchestrator visits (startup/resume/compaction called out in `team-allocation.md` 146) is not a shared cost in `fairness_cost`. Unknown as a calibrated quantity; conceptually it is a common cost attributed only if it appears as per-task coefficients.

---

### 10. Medium — Uncalibrated pilots are a singleton tournament, not a fair cold start

```810:821:src/native_scheduler.rs
    let pilot = coefficients
        .iter()
        .all(|coefficient| coefficient.sample_count > 0 || coefficient.manual_pilot)
        && task.risk.consequence < 2
        && !matches!(
            task.risk.template(),
            crate::portfolio::WorkClass::Complex | crate::portfolio::WorkClass::Extensive
        );
```

`fits` forbids a second pilot key already in `state.pilots` (1070–1073). Snapshot: provisional transfer is bounded low-consequence real work. The bound is **one pilot per account:window in the whole selection**, awarded by consequence/priority order, not by lowest dominant share or by per-project rotation. Cold-start projects sharing a meter starve each other on the only learning slot.

---

### 11. Medium — `resource_cost` sums unlike fractional shares (asset-fairness structure)

```1131:1143:src/native_scheduler.rs
            .map(|(resource, amount)| {
                self.capacity
                    .get(resource)
                    .filter(|value| **value > 0.0)
                    .map_or(f64::INFINITY, |value| amount / value)
            })
            .sum()
```

Snapshot: “unlike units are never summed.” True for the fairness max; false for lex-6 pressure. Ghodsi **Asset Fairness** equalizes $\sum_j s_{ij}$ and **violates sharing incentive** (Thm. 1, Figure 5). Summing residual fractions is the same dimensional move. As a late tie-break it will prefer a plan that burns $0.9$ of one meter over a plan that burns $0.5+0.5$ of two meters. Missing capacity yields $\infty$, so an unobserved meter in a candidate is infinitely expensive *if the candidate still reached scoring* — usually `fits` already blocked it via `unwrap_or(0.0)`.

---

### 12. Lower — Workflow and correlation rules that change who can consume

`RiskVector.template()` omits `evidence_need` from the max (`team_policy.rs` 26–36) but `visits` adds Net Research when `evidence_need > 0` (88, 110–117). High evidence need does not by itself escalate checks. That is a policy choice; it means a high-evidence Focused task has no Reviewer/Sanity while a `deadline=1` task becomes Standard and consumes more shared quota — another reason inflating `deadline` is not free, but inflating `evidence_need` is a cheaper way to grab a research visit.

Checker identity (`same_identity`, 913–927): same `host_id` **or** same `display_model`. Consequence/correlation $\ge 2$ requires a distinct checker (888–910). Strategic: two display names from one family on two hosts pass; two efforts of one display name fail. Snapshot: “Correlation diversity is required only through observable differences in actual creator/checker bundles.” The observable is weaker than family/harness. Kim et al. correlated-error results ([ICML 2025](https://proceedings.mlr.press/v267/kim25e.html), cited in `team-allocation.md` [7]) are about shared failure modes, not host IDs. Applicability: analogy only; this audit does not re-litigate that paper.

Conditional visits stay reserved until the condition is impossible (`candidate_plans` 669–691; snapshot). Conservatively correct for safety; it lets a high-`useful` project’s unused repair chain block another project’s first visit. DRF with held-but-unused resources is equivalent to inflating $D_i$ — Ghodsi notes excess demand can **hurt** the liar under true DRF; here unused reservations hurt *others* because `useful` already paid for admitting the parent task.

---

### 13. Lower — Qualification vs throughput (in scope only as it changes who is in the fair set)

Route qualification is an eligibility gate (`route_qualification.rs`); provisional evidence cannot cover `consequence >= 2` (168–216). That matches the snapshot. It interacts with fairness: projects lacking calibrated evidence are not in the DRF game at all for high-consequence work. That is intended isolation of risk, not a fairness theorem. No synthetic calibration is proposed.

`evidence_covers` requires this visit’s dimensions to include **all un-prefixed** `task.evidence_requirements` (355–375). A task-level research dimension without a seat prefix becomes a requirement on Implementer evidence, shrinking the feasible set in a seat-asymmetric way. Strategic response: prefix every requirement. Conceptual, not a DRF claim.

---

## What is not wrong (relative to the snapshot)

- Native units are not added across different `(account, window, unit)` keys in `fairness_cost` (the max is well-typed).
- Started Running/Completed/Blocked holds are not released as replaceable; frozen creator/checker and immutable visits are excluded from reassembly.
- Search reports `Exhaustive` vs `FeasibleSearchLimit` and does not fabricate a dual, gap, or optimum proof (`feasible_gap: None`).
- External reserves reduce opportunity capacity without entering project shares (`native_capacity` 472–483 vs `fairness_cost`). That matches “fixed external holds affect opportunity cost without making the solve globally fair.”
- Idle capacity when no qualified task fits is allowed (`walk` always may skip).
- Receding horizon stops at observed resets in the sense that a plan with unknown `reset_at` is dropped (`candidate_plans` 774–784).
- Unlike the per-plan LP in `METHODOLOGY.md`, this native allocator does not treat the 112-hour proxy forecast as authorized demand.

Those do not restore DRF properties.

---

## Actionable conceptual corrections

Not software, not tests, not synthetic calibration jobs. Changes to the decision rule so that “DRF-inspired entitlement” has content.

1. **Put entitlements where DRF puts them.** Either (a) progressive filling / SEQUENTIALMINMAX on weighted dominant shares as the packing rule among qualification-feasible bundles, or (b) a hard sharing-incentive constraint: each project $p$ with weight $w_p$ and remaining eligible demand may claim up to $w_p/\sum w$ of **total** endowment (observed remaining + attributable holds + this-horizon reserves) of each resource before any project exceeds that floor. Do not place fairness after an unbounded `useful` sum.

2. **Define share against endowment, not residual.** $s_{p,j}=(H_{p,j}+x_{p,j})/R_j/w_p$ with $R_j$ the window’s native size for this instance (or remaining+all attributable holds on that instance). If $R_j=0$, treat the share as undefined/infeasible, never as $0$. Include human, exclusive, account-lane, and host resources in the same $\max_j$ with their own $R_j$.

3. **Leximin, not min-max of the first bottleneck.** After equalizing the worst weighted share, equalize the second, or use Parkes SEQUENTIALMINMAX for indivisible bundles. Min-max alone is indifferent between $(0.5,0.5)$ and $(0.5,0.1)$.

4. **Make `priority` and `entitlement_weight` exogenous policy identities**, one per project in the ledger, not per-task declarations that enter `useful`. If priority must exist, it belongs in a *higher* lex tier that is not a project-chosen integer, or as a weight on the DRF share (Ghodsi weighted DRF), not as a count of admitted points.

5. **Stop paying for task IDs.** Replace `useful += 1 + priority + consequence + deadline` with a quantity that does not increase under splitting: admitted artifact value under qualification, or a declared job identity with a unit cap. Consequence and deadline-risk should not add into the same sum.

6. **Deadline lex term = unserved expiring work, using timestamps.** On the serial calendar the snapshot actually runs, order visits by earliest `task.deadline` (Liu–Layland EDF) among admitted work; use consequence as a *filter* (must-admit set) not as a preemption key that can push an imminent task past its deadline. Derive `risk.deadline` from slack if it must exist; do not let it diverge from `task.deadline`.

7. **Never-started out-of-scope holds are not frozen work.** Either they are in the replaceable set, or they are accounted as an owned external reserve that counts in that project’s dominant share against total $R_j$. Omitting a task or setting `ready=false` must not preserve a silent claim on residual capacity.

8. **Bind holds to reset instances** $(window_id, window_start, reset_at)$ in `native_capacity`, matching settlement/observation. Old-instance residual must not debit a new grant except as an explicit straddling attribution the observation covers.

9. **Report the same share the optimizer uses**, including started holds, weights, and the full resource vector.

10. **Bounded search should expand the lowest current weighted dominant share** (or SEQUENTIALMINMAX), not a static consequence/priority prefix. If the node limit remains, the expansion order *is* the fairness policy; write it that way.

11. **Pilot slots:** rotate by lowest historical share / per-project cap, not first in the priority permutation.

12. **Do not attribute DRF theorems** (SI, SP, EF, PE) to the allocator unless 1–3 are adopted and the remaining lex objectives are constraints on a DRF-feasible set. With indivisible tasks, state Parkes: PE+SI+SP is impossible; choose an explicit relaxation (EF1+PE+SI via SEQUENTIALMINMAX, or SP+SI dropping PE).

Uncertainty: whether real projects will exercise these levers is unknown; the math pays them if they do. Whether native remaining is a good proxy for $R_j$ depends on observation coverage, which the reconciliation path tries to keep monotonic but which finding 6 can still mix across instances. Broader stochastic job-shop optimality is, as the snapshot says, not implemented and not assumed.

