# Allocation Refinement Audit Report

- **Client / Model / Effort:** Antigravity CLI (`agy`) / Gemini 3.8 Flash (High effort)
- **Repository Revision:** `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0` (checked out, clean read-only audit)
- **Baseline Case Context:** 112 working hours ($H=112$), 4 concurrent project orchestrators ($N=4$), 1 owned subscription account per selected provider, 112 forecast proxy jobs. Solver status: `feasible_search_limit`, 19 admitted, 93 deferred, utility $67.1672$, upper bound $74.9857$, relative optimality gap $11.6402\%$ (1,024 nodes).
- **Web Tools and Execution Success:**
  - `read_url_content`: `https://artificialanalysis.ai/methodology/coding-agents-benchmarking` — **Success** (Retrieved live AA Coding Agent Index v1.5 methodology, task composition, and internet isolation criteria).
  - `read_url_content`: `https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3` — **Success** (Retrieved live Intelligence Index v4.3 release, Terminal-Bench v4.0 upgrade [66 tasks], and cost-per-task data).
  - `search_web`: Google Research agent scaling literature (`arXiv:2512.08296`), Stochastic RCPSP rework & proactive buffer literature, Bertsimas & Sim (2004) robust optimization, Ghodsi et al. (2011) Dominant Resource Fairness, Thompson & Beranek (1966) imperfect forecast value theory, Claude Code & Antigravity rolling rate limit documentation — **Success** (All queries returned authoritative primary citations).

---

## Systematic Audit of All Twelve Findings

### Finding 1: Demand Scaling vs. Subscription Capacity Decoupling
* **Summary:** The proxy demand $\lceil H \cdot N / 4 \rceil$ scales arrival volume with project count $N$ but does not scale subscription capacity. Changing concurrent projects from 1 to 4 leaves the identical 19-job admitted prefix.
* **Verified Claims:**
  - In [`src/portfolio.rs:1157-1161`](file:///C:/projects/aitierlist/src/portfolio.rs#L1157-L1161), proxy forecast is strictly computed as `let forecast = (settings.agent_hours * settings.orchestrators as f64 / 4.0).ceil() as usize`. At $H=112, N=1$, `forecast = 28`; at $H=112, N=4$, `forecast = 112`.
  - In [`src/portfolio.rs:552-582`](file:///C:/projects/aitierlist/src/portfolio.rs#L552-L582), `sequence(total)` generates a deterministic sequence via Hamilton class quotas and remainder sorting. Evaluation confirms that `sequence(28)` and `sequence(112)` generate the exact same initial 28-job sequence: `[0,1,0,2,1,0,0,3,1,0,2,0,1,0,1,0,2,0,1,0,0,1,0,2,1,0,0,1]`.
  - In [`src/portfolio.rs:1180-1184`](file:///C:/projects/aitierlist/src/portfolio.rs#L1180-L1184), pool capacity is strictly `capacity * settings.subscription_count(provider.id)`. Concurrency $N$ does not scale subscription allowances.
  - In [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), OpenAI capacity is $99.886\%$ reserved at 19 jobs. The prefix-preserving solver binary searches over the admitted prefix length; because the 19-job prefix saturates the binding account allowances under both $N=1$ and $N=4$, changing $N$ from 1 to 4 leaves the exact same 19-job admitted prefix (9 Focused, 6 Standard, 3 Complex, 1 Extensive) and merely shifts deferred jobs from 9 to 93.
* **Counterexamples & Nuances:**
  - If a user configures multiple subscription seats per provider proportional to $N$ (`subscription_count = N`), capacity would scale linearly.
  - If conductor overhead scaled with $N$ (e.g., holding 4 persistent orchestrator sessions in native memory and token context), increasing $N$ under a fixed subscription budget would actually *reduce* admissible worker jobs below 19.
* **Severity:** **High** (Operational and planning model mismatch).
* **Primary Sources:**
  - Codebase: [`src/portfolio.rs:552-582`](file:///C:/projects/aitierlist/src/portfolio.rs#L552-L582), [`src/portfolio.rs:1157-1184`](file:///C:/projects/aitierlist/src/portfolio.rs#L1157-L1184).
  - Academic: Yubin Kim et al., *"Towards a science of scaling agent systems: When and why agent systems work"*, Google Research / MIT, arXiv:2512.08296 (2025/2026).
* **Implementable Refinements:**
  1. Replace the heuristic $\lceil H \cdot N / 4 \rceil$ with an explicit Poisson arrival model $\lambda_p$ per active project $p$ and active project duty factor $u_p \in (0, 1]$.
  2. Model multi-project conductor overhead as $C_{orch}(N, z) = N \cdot S_{session} + \sum_{j=1}^z \nu(w_j)$ to reflect persistent concurrent sessions.

---

### Finding 2: Lexicographic Admission Preceding Route Quality
* **Summary:** The solver enforces lexicographic maximization of admitted job count before optimizing route quality, causing a superior conductor to be discarded whenever it displaces even one low-value proxy job.
* **Verified Claims:**
  - In [`src/portfolio.rs:1500-1544`](file:///C:/projects/aitierlist/src/portfolio.rs#L1500-L1544), the outer loop performs binary search over candidate admitted counts (`count`). For each count, the solver executes `search(count, ...)` which solves an LP-relaxed branch-and-bound optimization.
  - In [`src/solver.rs:209-223`](file:///C:/projects/aitierlist/src/solver.rs#L209-L223), the multi-criteria simplex objective maximizes required group coverage before quality, nominal time, and nominal cost.
  - If Conductor $A$ yields an orchestrator competence of $0.528$ (Astra max) but consumes enough quota that only 16 changes can be admitted, while Conductor $B$ yields competence $0.338$ (Sol low) and allows 19 changes to fit, the binary search selects 19 changes and permanently discards Conductor $A$.
* **Counterexamples & Nuances:**
  - When operational policy strictly prioritizes clearing backlog volume over quality (e.g., high-volume mechanical migration tasks), lexicographic admission is rational.
  - In low-backlog scenarios ($D < \text{capacity}$), all tasks are admitted, allowing route quality to become the active selection objective.
* **Severity:** **Critical** (Systemic decision pathology under scarce quotas).
* **Primary Sources:**
  - Codebase: [`src/portfolio.rs:1500-1544`](file:///C:/projects/aitierlist/src/portfolio.rs#L1500-L1544), [`src/solver.rs:209-223`](file:///C:/projects/aitierlist/src/solver.rs#L209-L223).
  - Academic: G. Mavrotas, *"Effective implementation of the $\epsilon$-constraint method in multi-objective mathematical programming problems"*, *Applied Mathematics and Computation*, 213(2):455–465, 2009.
* **Implementable Refinements:**
  1. Replace strict lexicographic ordering with an $\epsilon$-constraint or weighted augmented objective:
     $$\max \sum_{j \in \text{admitted}} V(t_j) + \lambda \sum_{(r,c)} U(r, c, p) \cdot x_{r,c,p}$$
     where $V(t_j)$ represents task decision value and $\lambda$ is a user-configurable quality trade-off coefficient.
  2. Compute and present the Pareto frontier of $(z, U_{total})$ pairs directly in the UI and report.

---

### Finding 3: Baseline Conductor Inversion (GPT-5.6 Sol Low vs. Max-Effort Workers)
* **Summary:** The baseline optimization selects GPT-5.6 Sol at low effort as the whole-plan conductor while creation and assurance worker seats are assigned maximum reasoning effort.
* **Verified Claims:**
  - In [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), the selected conductor is Row 9 (`GPT-5.6 Sol (low)`), with competence $0.337979$ and utility $0.337979$.
  - Simultaneously, worker assignments are allocated at maximum and high effort:
    - Implementer (Focused, Standard, Complex): `Codex - GPT-5.6 Sol (max)`, utility $0.569874$.
    - Reviewer (Focused, Complex): `Codex - GPT-5.6 Sol (max)`; Reviewer (Standard): `Grok 4.6 (xhigh)`; Reviewer (Extensive): `Claude Code - Opus 5 (max)`, utility $0.680729$.
    - Sanity (Standard): `Codex - GPT-6 Astra (max)`; Sanity (Complex): `Claude Code - Opus 5 (max)`.
    - Comprehension (Complex): `Codex - GPT-5.6 Sol (max)`; Comprehension (Extensive): `Muse Code - Muse Spark 1.3 (max)`.
  - In [`src/aa.rs:404-418`](file:///C:/projects/aitierlist/src/aa.rs#L404-L418), Sol low has an Intelligence Index task cost of $\$0.260642$ and output decode time of $68.19$ seconds, compared to Astra max at $\$0.518621$ and $114.73$ seconds. Because OpenAI quota has only $0.4050$ modeled USD of remaining headroom ($99.886\%$ reserved), selecting any higher-effort conductor violates OpenAI capacity at 19 admitted jobs.
* **Counterexamples & Nuances:**
  - If the conductor's functional role is restricted to mechanical CLI tool invocation and brief forwarding, low reasoning effort could suffice.
  - However, in [`src/team_policy.rs:143, 232`](file:///C:/projects/aitierlist/src/team_policy.rs#L143) and [`METHODOLOGY.md:43-44`](file:///C:/projects/aitierlist/METHODOLOGY.md#L43-L44), the conductor is tasked with plan decomposition, scope/risk classification, and cross-worker artifact reconciliation, which are cognitively demanding.
* **Severity:** **High** (Capability hierarchy inversion).
* **Primary Sources:**
  - Codebase: [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), [`src/aa.rs:404-418`](file:///C:/projects/aitierlist/src/aa.rs#L404-L418).
  - Academic: L. Garicano, *"Hierarchies and the Organization of Knowledge in Production"*, *Journal of Political Economy*, 108(5):874–904, 2000.
* **Implementable Refinements:**
  1. Enforce an operational competence floor on the Orchestrator seat ($\text{floor}_{orch} \ge 0.45$).
  2. Implement hierarchical routing where routine worker dispatch is automated by deterministic runtime code, reserving conductor model invocations for escalation and reconciliation.

---

### Finding 4: Conductor Floor Sensitivity and Whole-Team Composition Confounding
* **Summary:** Raising the Orchestrator floor to 0.50 selects GPT-6 Astra xhigh (admitting 17); raising it to 0.527 selects Astra max (admitting 16). Aggregate utility shifts are confounded by wholesale team reconfiguration rather than isolating conductor impact.
* **Verified Claims:**
  - Audits across the three floor configurations reveal:
    - Floor $0.338$ ([`audit-orch-0.338.json`](file:///C:/projects/aitierlist/target/debug/audit-orch-0.338.json)): Admitted 17, Conductor Row 1 (`GPT-6 Astra medium`, comp $0.4967$), Quality $69.6648$.
    - Floor $0.500$ ([`audit-orch-0.500.json`](file:///C:/projects/aitierlist/target/debug/audit-orch-0.500.json)): Admitted 17, Conductor Row 7 (`GPT-6 Astra xhigh`, comp $0.5251$), Quality $70.0692$.
    - Floor $0.527$ ([`audit-orch-0.527.json`](file:///C:/projects/aitierlist/target/debug/audit-orch-0.527.json)): Admitted 16, Conductor Row 16 (`GPT-6 Astra max`, comp $0.5281$), Quality $63.2276$.
  - Comparing floor $0.500$ to $0.527$ demonstrates that selecting Astra max consumes additional OpenAI quota, triggering downstream substitutions:
    - Focused Implementer switches from `Codex Sol max` to `Codex Astra max`.
    - Focused Debugger switches from `Antigravity Gemini 3.8 Flash high (2x)` to `Codex Sol max (1x)`.
    - Focused Reviewer switches from `Grok 4.6 xhigh` to `Claude Opus 5 max`.
    - Sanity and Comprehension seats across Focused, Standard, Complex, and Extensive undergo complete cross-vendor reshuffling.
  - The aggregate quality drop from $70.0692$ to $63.2276$ reflects the loss of 1 admitted change (a penalty of $\approx 3.5$ utility points) combined with budget-starved worker downgrades, not an inherent defect of Astra max.
* **Counterexamples & Nuances:**
  - In a joint constrained optimization problem under coupled knapsack constraints, ceteris paribus substitution is mathematically impossible; any change in resource consumption by one role must induce substitutions elsewhere.
  - The audit finding is verified in that aggregate portfolio utility cannot be cited as causal evidence of individual conductor effectiveness.
* **Severity:** **Medium** (Causal attribution error in reporting).
* **Primary Sources:**
  - Audit Data: [`target/debug/audit-orch-0.338.json`](file:///C:/projects/aitierlist/target/debug/audit-orch-0.338.json), [`target/debug/audit-orch-0.500.json`](file:///C:/projects/aitierlist/target/debug/audit-orch-0.500.json), [`target/debug/audit-orch-0.527.json`](file:///C:/projects/aitierlist/target/debug/audit-orch-0.527.json).
* **Implementable Refinements:**
  1. Decompose reported portfolio utility into additive components:
     $$U_{total} = U_{orch} + U_{workers} + \text{Penalty}(D - z)$$
  2. Compute and display the LP shadow prices (dual values $\pi_a$) for each account constraint to explicitly show the opportunity cost of conductor upgrades:
     $$\Delta \text{Value} = \Delta U_{orch} - \sum_a \pi_a \cdot \Delta \text{Usage}_{orch, a}$$

---

### Finding 5: Conductor Competence Metric Inconsistency Across Views
* **Summary:** The portfolio conductor optimization uses normalized Intelligence Index, whereas Absolute Best and METHODOLOGY.md define Orchestrator competence as the average of GPQA and Intelligence Index.
* **Verified Claims:**
  - In [`src/portfolio.rs:1441-1445, 1463`](file:///C:/projects/aitierlist/src/portfolio.rs#L1441-L1445):
    ```rust
    let competence = if seat == Seat::Orchestrator {
        row.smart
    } else {
        engine::competence(row, seat)
    };
    ...
    utility: competence,
    ```
    Here, conductor competence and utility in `portfolio.rs` are assigned directly from `row.smart` (the normalized Artificial Analysis Intelligence Index).
  - In [`src/engine.rs:68-75`](file:///C:/projects/aitierlist/src/engine.rs#L68-L75):
    ```rust
    if seat == Seat::Orchestrator {
        let smart = row.smart?;
        let gpqa = pass(row, Benchmark::Gpqa)?;
        return (smart.is_finite() && (0.0..=1.0).contains(&smart)).then_some(vec![
            ("GPQA reasoning", gpqa, 0.5),
            ("Intelligence index", smart, 0.5),
        ]);
    }
    ```
  - In [`METHODOLOGY.md:82`](file:///C:/projects/aitierlist/METHODOLOGY.md#L82), the formula is documented as `Orchestrator = (GPQA + index) / 2`.
  - For GPT-5.6 Sol low, `row.smart = 0.337979`, while `gpqa = 0.550505`, producing an `engine::competence` of $0.444242$. The portfolio solver excludes GPQA entirely.
* **Counterexamples & Nuances:**
  - As documented in the live AA Intelligence Index v4.3 methodology (`https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3`), the Intelligence Index already incorporates scientific reasoning and complex benchmark tasks. Averaging GPQA with the Intelligence Index double-counts scientific multiple-choice capability.
  - While using pure Intelligence Index in `portfolio.rs` is technically sound, having conflicting definitions between views violates transparency and breaks score comparability.
* **Severity:** **Medium** (Metric misalignment across engine components).
* **Primary Sources:**
  - Codebase: [`src/portfolio.rs:1441-1463`](file:///C:/projects/aitierlist/src/portfolio.rs#L1441-L1463), [`src/engine.rs:68-75`](file:///C:/projects/aitierlist/src/engine.rs#L68-L75), [`METHODOLOGY.md:82`](file:///C:/projects/aitierlist/METHODOLOGY.md#L82).
  - Benchmark: Artificial Analysis Intelligence Index v4.3 (September 7, 2026).
* **Implementable Refinements:**
  1. Standardize conductor competence across all files to a single canonical definition:
     $$\text{Competence}_{orch} = \text{IntelligenceIndex}_{norm}$$
  2. Deprecate the legacy `(GPQA + index) / 2` formula in `engine.rs` and update `METHODOLOGY.md` accordingly.

---

### Finding 6: Conductor Resource Proxy Units vs. Native Operational Turns
* **Summary:** Conductor resource accounting charges two class-weighted Intelligence Index benchmark-task-equivalent visits per admitted change rather than modeling native conductor session turns or benchmark suites.
* **Verified Claims:**
  - In [`src/aa.rs:404-418`](file:///C:/projects/aitierlist/src/aa.rs#L404-L418), `orchestrator_usd` and `orchestrator_seconds` are extracted from `host["intelligenceIndexCostPerTask"]["cost"]["total"]` and output decode speed. This represents the average cost of *one single task* in the AA benchmark suite.
  - In [`src/portfolio.rs:1191`](file:///C:/projects/aitierlist/src/portfolio.rs#L1191) and [`src/portfolio.rs:1725-1728`](file:///C:/projects/aitierlist/src/portfolio.rs#L1725-L1728), orchestrator load is charged as:
    $$\text{Units} = 2 \times \sum_{c} N_c \cdot f_c$$
    where $f_c \in \{0.25, 1.0, 2.0, 4.0\}$.
  - In [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), the conductor charges 38 expected visits across the 19 admitted jobs, consuming $\$9.5134$ expected and $\$11.8918$ reserved USD. This reflects 38 synthetic task equivalents, not 38 native turns or two complete benchmark evaluation runs.
* **Counterexamples & Nuances:**
  - The codebase explicitly states this abstraction in `METHODOLOGY.md:24`: "This models coordination demand rather than literal native turns; the runtime ledger charges every actual native call exactly once."
  - Using benchmark-task units provides a standardized cross-model cost baseline when native harness execution logs are unavailable.
* **Severity:** **High** (Risk of significant divergence from real native token burn).
* **Primary Sources:**
  - Codebase: [`src/aa.rs:398-432`](file:///C:/projects/aitierlist/src/aa.rs#L398-L432), [`src/portfolio.rs:1191-1201`](file:///C:/projects/aitierlist/src/portfolio.rs#L1191-L1201).
  - Benchmark: Artificial Analysis Intelligence Index Cost per Task documentation (2026).
* **Implementable Refinements:**
  1. Define conductor resource consumption as a two-part tariff: a fixed session context maintenance fee $C_{base}$ plus per-handoff marginal token usage $C_{visit} \cdot \text{tokens}$.
  2. Implement an empirical calibration bridge that updates the proxy multiplier using historical turn-count observations from `ledger.v1.json`.

---

### Finding 7: Static Nine-Stage Workflow Uniformity vs. Dynamic Risk Needs
* **Summary:** The static proxy simulates a rigid nine-stage workflow reserving every worker seat and conditional repair/recheck for every admitted job, whereas actual task risk requires fewer or different stages.
* **Verified Claims:**
  - In [`src/portfolio.rs:65-129`](file:///C:/projects/aitierlist/src/portfolio.rs#L65-L129), `STAGES` hardcodes exactly 9 sequential stages:
    1. Orchestrator 1 (Brief)
    2. Comprehension 1
    3. Implementer 1
    4. Reviewer 1
    5. Sanity 1
    6. Debugger 1 (Conditional repair)
    7. Reviewer 2 (Conditional recheck)
    8. Sanity 2 (Conditional recheck)
    9. Orchestrator 2 (Close)
  - In [`src/portfolio.rs:773-800`](file:///C:/projects/aitierlist/src/portfolio.rs#L773-L800), every admitted change simulates and reserves quota for all 9 stages regardless of whether the change is Focused, Standard, Complex, or Extensive.
  - Conversely, the native runtime policy in [`src/team_policy.rs:120-235`](file:///C:/projects/aitierlist/src/team_policy.rs#L120-235) generates stages dynamically based on `RiskVector`:
    - `Focused` tasks omit Comprehension, omit Reviewer 1 & 2, omit Sanity 1 & 2, and omit Debugger (only 3 stages: Orchestrator 1, Implementer 1, Orchestrator 2).
    - `Standard` tasks include Reviewer and Sanity, but omit Comprehension.
    - `Complex` and `Extensive` tasks include Comprehension.
    - Net Research is invoked dynamically only when `evidence_need > 0`.
  - Consequently, the static proxy reserves 9 stages for Focused tasks when only 3 are required, unnecessarily throttling admission to 19 jobs.
* **Counterexamples & Nuances:**
  - Full nine-stage stress reservation guarantees that if unexpected repair or verification is triggered at runtime, the allocated budget will not be exceeded.
  - However, because Focused jobs constitute $50\%$ of the Hamilton workload ($9/19$ admitted jobs), reserving 9 stages for all Focused tasks creates extreme artificial scarcity.
* **Severity:** **Critical** (Major source of capacity under-utilization).
* **Primary Sources:**
  - Codebase: [`src/portfolio.rs:65-129`](file:///C:/projects/aitierlist/src/portfolio.rs#L65-L129), [`src/team_policy.rs:120-235`](file:///C:/projects/aitierlist/src/team_policy.rs#L120-235).
  - Academic: W. Herroelen & R. Leus, *"Robust and proactive approaches to project scheduling"* (RCPSP buffer management), *Computers & Operations Research*, 31(14):2347–2366, 2004.
* **Implementable Refinements:**
  1. Restructure the portfolio proxy to generate stage sets tailored to each work class template matching `team_policy.rs`:
     - Focused: 3 stages (Orchestrator, Implementer, Orchestrator).
     - Standard: 7 stages (Adds Reviewer 1, Sanity 1, Debugger, Rechecks).
     - Complex / Extensive: 8–9 stages (Adds Comprehension and Net Research).
  2. Scale worker group reservation multipliers in `portfolio.rs` by template-specific stage counts rather than a blanket 9-stage multiplier.

---

### Finding 8: Net Research Exclusion from Static Proxy Allocation
* **Summary:** Net Research candidates are evaluated and filtered based on class-weighted AA metrics, but static research demand and quota interactions are omitted from the solver and schedule.
* **Verified Claims:**
  - In [`src/portfolio.rs:1214-1395`](file:///C:/projects/aitierlist/src/portfolio.rs#L1214-L1395), Net Research candidates are scored against the four class weight vectors ($A, R, L, E$) and filtered against competence floors.
  - In [`src/portfolio.rs:790`](file:///C:/projects/aitierlist/src/portfolio.rs#L790), Net Research is explicitly excluded from the solver candidate groups:
    ```rust
    if role == orchestrator_role || Seat::ALL[role] == Seat::NetResearch {
        continue;
    }
    ```
  - In [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), the Net Research seat reports `plannedJobs = 0`, `allocatedHours = 0.0`, `reservedUsage = 0.0`, and `utility = 0.0`.
  - When runtime tasks trigger `evidence_need > 0` ([`src/team_policy.rs:123`](file:///C:/projects/aitierlist/src/team_policy.rs#L123)), the spawned Net Research visits consume quota and calendar time that were never reserved in the static plan, risking unmodeled budget exhaustion.
* **Counterexamples & Nuances:**
  - For purely internal codebase modifications (e.g., standard refactoring, local bug fixes), research is unnecessary; omitting it from the baseline prevents idle budget holding.
  - However, for Extensive tasks (defined as open-ended research, system design, and unresolved scope), web research is mandatory.
* **Severity:** **High** (Unmodeled resource consumption leakage).
* **Primary Sources:**
  - Codebase: [`src/portfolio.rs:790`](file:///C:/projects/aitierlist/src/portfolio.rs#L790), [`src/portfolio.rs:1214-1395`](file:///C:/projects/aitierlist/src/portfolio.rs#L1214-L1395), [`src/team_policy.rs:123, 145-153`](file:///C:/projects/aitierlist/src/team_policy.rs#L123).
  - Academic: H. E. Thompson & W. Beranek, *"The Efficient Use of an Imperfect Forecast"*, *Management Science*, 13(3):233–243, 1966.
* **Implementable Refinements:**
  1. Define an expected research probability $p_{res}(c)$ for each template: $p_{res}(\text{Focused})=0.0$, $p_{res}(\text{Standard})=0.1$, $p_{res}(\text{Complex})=0.4$, $p_{res}(\text{Extensive})=1.0$.
  2. Include Net Research in the solver group assignments with expected resource usage $N_c \cdot p_{res}(c) \cdot \text{Cost}_{res}$.

---

### Finding 9: Non-Monotonic Competence Floors Across Workflow Classes
* **Summary:** The optimizer does not enforce monotonic competence floors across workflow classes, resulting in higher-risk Extensive tasks receiving lower-competence assignments than Focused tasks.
* **Verified Claims:**
  - Examination of [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json) reveals several non-monotonic assignments:
    - **Implementer:** Focused, Standard, and Complex select `Codex - GPT-5.6 Sol (max)` (utility $0.569874$), whereas Extensive selects `Antigravity SDK - Gemini 3.8 Flash (high)` (utility $0.452444$).
    - **Sanity:** Standard selects `Codex - GPT-6 Astra (max)` (utility $0.618280$), whereas Extensive selects `Grok Build - Grok 4.6 (xhigh)` (utility $0.583333$).
    - **Comprehension:** Focused and Standard select `Grok Build - Grok 4.6 (xhigh)` (utility $0.696667$), whereas Complex selects `Codex - GPT-5.6 Sol (max)` (utility $0.690161$).
  - Neither [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs) nor [`src/solver.rs`](file:///C:/projects/aitierlist/src/solver.rs) enforces constraints of the form $U(r, c+1) \ge U(r, c)$.
  - Because Extensive tasks have a resource factor of $4.0\times$, the solver substitutes cheaper models into Extensive slots to fit account budget constraints, directly violating the principle that high-consequence work requires stronger competence.
* **Counterexamples & Nuances:**
  - Some models feature large context windows (e.g., Gemini 3.8 Flash) that qualify them for Extensive codebase exploration even if their raw code generation score is lower than Sol max.
  - Nevertheless, inverting competence on the critical implementation role for high-consequence work violates the O-ring theory of software production.
* **Severity:** **High** (Systemic risk posture inversion).
* **Primary Sources:**
  - Codebase: [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), [`src/portfolio.rs:1418-1492`](file:///C:/projects/aitierlist/src/portfolio.rs#L1418-L1492).
  - Academic: M. Kremer, *"The O-Ring Theory of Economic Development"*, *Quarterly Journal of Economics*, 108(3):551–575, 1993.
* **Implementable Refinements:**
  1. Add linear constraints enforcing monotonic role competence across classes:
     $$\sum_p \text{competence}(p) \cdot x_{r, c+1, p} \ge \sum_p \text{competence}(p) \cdot x_{r, c, p} - \delta$$
  2. Implement tiered competence minimums in settings: $\text{floor}(r, \text{Extensive}) \ge \text{floor}(r, \text{Complex}) \ge \text{floor}(r, \text{Standard})$.

---

### Finding 10: Uncalibrated Defect Exposure and Review Value Modeling
* **Summary:** Review burden is modeled via static visit counts, caps, and benchmark utility, without calibrated defect generation, review complexity scaling, or marginal assurance value.
* **Verified Claims:**
  - In [`src/portfolio.rs:65-129`](file:///C:/projects/aitierlist/src/portfolio.rs#L65-L129), Reviewer visits are fixed at 1 initial and 1 conditional recheck visit across all classes.
  - In [`src/engine.rs:35-39`](file:///C:/projects/aitierlist/src/engine.rs#L35-L39), Reviewer competence is statically defined as `0.5 * QnA + 0.25 * GPQA + 0.25 * HLE`.
  - The model does not incorporate:
    - Base defect rate $P_{defect}$ of the implementer.
    - True positive defect detection rate (sensitivity) or false discovery rate.
    - Code change complexity or AST diff size.
    - Error correlation between the creator and reviewer (e.g., Sol reviewing Sol).
  - In [`target/debug/audit-current-production.json`](file:///C:/projects/aitierlist/target/debug/audit-current-production.json), Focused and Complex tasks use `Codex - GPT-5.6 Sol (max)` for *both* Implementer and Reviewer, maximizing correlated failure risk.
* **Counterexamples & Nuances:**
  - Benchmarking real-world LLM code review efficacy is an open research challenge; using a standardized composite (SWE-Atlas Q&A + reasoning) provides a non-arbitrary baseline proxy.
  - However, treating reviewer utility as purely additive without modeling bug interception rate provides a distorted estimate of true software delivery reliability.
* **Severity:** **Medium** (Assurance model deficiency).
* **Primary Sources:**
  - Codebase: [`src/portfolio.rs:65-129`](file:///C:/projects/aitierlist/src/portfolio.rs#L65-L129), [`src/engine.rs:35-39`](file:///C:/projects/aitierlist/src/engine.rs#L35-L39).
  - Academic:
    - A. Bacchelli & C. Bird, *"Expectations, Outcomes, and Challenges of Modern Code Review"*, ICSE 2013.
    - Y. Kim et al., *"Correlated Errors in Large Language Models"*, ICML 2025 (PMLR 267).
* **Implementable Refinements:**
  1. Add an error correlation discount factor $\rho_{family} \in [0.5, 0.8]$ applied to Reviewer utility when the Implementer and Reviewer share the same model family:
     $$U_{rev, eff} = U_{rev} \cdot (1 - \rho_{family} \cdot \mathbf{1}_{\{family_{impl} = family_{rev}\}})$$
  2. Scale review resource duration proportionally with code churn/token volume: $T_{rev} = T_{base} \cdot (1 + \beta \cdot \text{churn})$.

---

### Finding 11: Benchmark Provenance, Version Pairing, and Task Weighting
* **Summary:** The Coding Agents source exposes 13 configurations. AA Coding Agents v1.5 equally weights DeepSWE v1.1 (113 tasks), Terminal-Bench 4.0 (66 tasks), and SWE-Atlas Q&A (124 tasks); the app's 113:89 mix is a proprietary task-weighted policy. Terminal-Bench versions and resource pairings require precise interpretation.
* **Verified Claims:**
  - Official AA documentation (`https://artificialanalysis.ai/methodology/coding-agents-benchmarking`) establishes:
    - Coding Agent Index v1.5 is the **equal-weight average** (33.3% each) of DeepSWE v1.1 (113 tasks), Terminal-Bench 4.0 (66 tasks), and SWE-Atlas-QnA (124 tasks).
  - Official AA release article (`https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3`) confirms:
    - Terminal-Bench upgraded from v2.1 (89 tasks) to v4.0 (66 tasks, 3 runs).
  - App source inspection confirms:
    - Exactly 13 agent configurations exist in `audit-current-production.json`.
    - In [`src/aa.rs:17`](file:///C:/projects/aitierlist/src/aa.rs#L17), `const TERMINAL_TASKS: f64 = 89.0;`.
    - In [`src/engine.rs:25-28`](file:///C:/projects/aitierlist/src/engine.rs#L25-L28), `CODING = [(Benchmark::Swe, 113.0 / 202.0), (Benchmark::Terminal, 89.0 / 202.0)]`.
    - Agent rows calculate resource means from the AA Coding Agent table (which uses TB v4.0), whereas model rows calculate resources from `canonicalEvalTokenCounts["terminalbenchV21"]` paired with TB v2.1 scores.
    - Retry correlation in `assets/retry-correlation.json` fits $\rho$ from DeepSWE v1.1 trial rollouts and Terminal-Bench 2.1 submissions.
* **Counterexamples & Nuances:**
  - Weighting by total task count ($113/202$ and $89/202$) is a mathematically legitimate pooled estimation technique, but calling it an "Artificial Analysis benchmark index" is inaccurate.
  - Pairing TB v2.1 on model rows was necessary because canonical token counts for TB v4.0 were not initially available in early 2026 model manifests.
* **Severity:** **Medium** (Provenance and comparability discrepancy).
* **Primary Sources:**
  - Web: Artificial Analysis Coding Agent Index v1.5 Methodology (`https://artificialanalysis.ai/methodology/coding-agents-benchmarking`).
  - Web: Artificial Analysis Intelligence Index v4.3 Announcement (`https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3`).
  - Codebase: [`src/engine.rs:25-28`](file:///C:/projects/aitierlist/src/engine.rs#L25-L28), [`src/aa.rs:17, 514-520, 682-712`](file:///C:/projects/aitierlist/src/aa.rs#L17).
* **Implementable Refinements:**
  1. Update reporting to explicitly distinguish the "App 113:89 Task-Weighted Reference Workload" from the "AA Coding Agent Index v1.5 (Equal Weights)".
  2. Upgrade model rows to Terminal-Bench v4.0 (66 tasks) once canonical token metrics are published, or freeze both rows to explicit historical versions with provenance tags.

---

### Finding 12: Operational Divergence (API Quotas, DRF, Serial Lanes, Bounded Limits)
* **Summary:** Modeled API-equivalent quotas, entitlement-weighted DRF, per-account serial lanes, and bounded solver limits do not guarantee native execution feasibility, global project fairness, or optimal runtime dispatch.
* **Verified Claims:**
  - **Quota Divergence:** Modeled budgets divide monthly allowances by $52/12$. Real provider subscriptions enforce short rolling windows (e.g., Claude Code enforces a rolling 5-hour session reset triggered by first request; Antigravity enforces 5-hour and weekly limits; Grok shares a rolling pool). A weekly dollar proxy cannot prevent mid-week 5-hour session lockouts.
  - **DRF Approximations:** In [`src/native_scheduler.rs:30-34, 119-130`](file:///C:/projects/aitierlist/src/native_scheduler.rs#L30-L34), dominant resource shares are computed per project across candidate plans within a single horizon. True DRF (Ghodsi et al. 2011) assumes continuous, divisible resource allocations across all active demands; with discrete integer jobs, shared delayed meters, and uncertain task arrivals, local dominant shares do not guarantee global max-min fairness.
  - **Single-Lane Serialization:** In [`src/portfolio.rs:699-730`](file:///C:/projects/aitierlist/src/portfolio.rs#L699-L730), account calendars enforce strictly non-overlapping intervals (one visit at a time per provider account). While safe against basic concurrency limits, this understates throughput when native provider plans permit concurrent CLI sessions or subagents.
  - **Bounded Search Limits:** In [`src/solver.rs:484`](file:///C:/projects/aitierlist/src/solver.rs#L484) and [`src/portfolio.rs:1545-1554`](file:///C:/projects/aitierlist/src/portfolio.rs#L1545-L1554), the branch-and-bound search halts at 1,024 nodes, returning `feasible_search_limit` with an unproven $11.6402\%$ optimality gap.
* **Counterexamples & Nuances:**
  - The codebase explicitly differentiates planning models from native execution: [`src/native_reconciliation.rs`](file:///C:/projects/aitierlist/src/native_reconciliation.rs) enforces short compare-and-swap ledger locks, atomic holds, and observation reconciliations against actual native meters.
  - The static schedule is intended as an admissibility proof and diagnostic, not a rigid real-time execution script.
* **Severity:** **High** (Operational boundary risk).
* **Primary Sources:**
  - Codebase: [`src/native_scheduler.rs:148-230`](file:///C:/projects/aitierlist/src/native_scheduler.rs#L148-L230), [`src/native_reconciliation.rs:68-180`](file:///C:/projects/aitierlist/src/native_reconciliation.rs#L68-L180).
  - Academic:
    - A. Ghodsi et al., *"Dominant Resource Fairness: Fair Allocation of Multiple Resource Types"*, NSDI 2011.
    - D. Bertsimas & M. Sim, *"The Price of Robustness"*, *Operations Research*, 52(1):35–53, 2004.
  - Official Docs: Anthropic Claude Code Rate Limits and Rolling Windows (2026); Google Antigravity Plans and Limits (2026).
* **Implementable Refinements:**
  1. Add a secondary rolling 5-hour rate limit constraint into `native_scheduler.rs` alongside the weekly budget constraint.
  2. Allow configurable account concurrency lanes ($K_{lanes} \ge 1$) per account when native provider subscriptions permit concurrent headless executions.
  3. Formally report solver optimality gaps as upper-bound limits rather than quality confidence intervals.

---

## Synthesis and Implementable Refinement Roadmap

| Refinement Area | Mathematical & Structural Formulation | Target Source Files | Operational Impact |
|---|---|---|---|
| **Objective Reformulation** | Replace rigid lexicographic count search with $\epsilon$-constraint Pareto optimization: $\max \sum V(t_j) + \lambda \sum U_{r,c,p} x_{r,c,p}$. Expose the $(z, U)$ trade-off curve. | [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs), [`src/solver.rs`](file:///C:/projects/aitierlist/src/solver.rs) | Eliminates conductor quality inversion where Sol low is chosen over Astra max to gain 1 low-value task. |
| **Dynamic Proxy Workflow Stages** | Replace uniform 9-stage proxy with template DAGs matching `team_policy.rs` (Focused: 3 stages, Standard: 7, Complex/Extensive: 8–9). | [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs) | Unlocks $30\%\text{--}60\%$ additional admitted capacity by removing over-reservation on Focused jobs. |
| **Net Research Integration** | Include Net Research in solver candidate groups with class-dependent invocation probabilities: $p_{res}(c) \in \{0.0, 0.1, 0.4, 1.0\}$. | [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs) | Prevents unmodeled quota exhaustion when research-heavy Extensive tasks execute. |
| **Competence Monotonicity** | Enforce linear constraints: $\sum_p \text{comp}(p) x_{r, c+1, p} \ge \sum_p \text{comp}(p) x_{r, c, p} - \delta$. Add class-tiered competence floors. | [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs), [`src/solver.rs`](file:///C:/projects/aitierlist/src/solver.rs) | Eliminates risk inversions where Extensive tasks receive lower-competence assignments than Focused tasks. |
| **Conductor Competence Harmonization** | Standardize conductor competence to normalized Intelligence Index across `engine.rs`, `portfolio.rs`, and `METHODOLOGY.md`. | [`src/engine.rs`](file:///C:/projects/aitierlist/src/engine.rs), [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs), [`METHODOLOGY.md`](file:///C:/projects/aitierlist/METHODOLOGY.md) | Resolves conflicting model rankings and GPQA double-counting across application views. |
| **Correlated Checker Penalty** | Apply error correlation discount $\rho_{family} \in [0.5, 0.8]$ to Reviewer/Sanity utility when creator and checker share the same model family. | [`src/engine.rs`](file:///C:/projects/aitierlist/src/engine.rs), [`src/portfolio.rs`](file:///C:/projects/aitierlist/src/portfolio.rs) | Prevents blind-spot accumulation from homogeneous model assignments (e.g., Sol reviewing Sol). |
| **Native Rolling Window Constraints** | Model 5-hour rolling session constraints alongside weekly allowances in the scheduler. | [`src/native_scheduler.rs`](file:///C:/projects/aitierlist/src/native_scheduler.rs) | Prevents mid-week rate limit lockouts on Claude Code and Antigravity accounts. |

---
*Audit completed strictly in read-only mode at revision `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0`. No files created, modified, or executed beyond read-only queries.*
