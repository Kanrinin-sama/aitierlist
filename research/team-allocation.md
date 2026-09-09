# Reliable multi-agent team allocation under shared quotas

The current For-you policy selects one persistent conductor jointly with worker routes because every visit consumes shared account time and capacity. The proxy forecast is `ceil(H × N / 4)`, where `H` is the elapsed weekly working horizon and `N` is concurrent project demand; increasing `N` does not multiply subscription capacity. Focused, Standard, and Complex or Extensive work use three, eight, and nine stages respectively, with an explicit default-off research insertion for each class. Orchestrator competence is the normalized Intelligence Index in both views, and its common coordination resource cycle has cap one. [METHODOLOGY.md](../METHODOLOGY.md) defines the current equations, and [the allocation refinement assessment](allocation-refinement/assessment.md) states their evidence and optimization limits.

This report distinguishes three claims. A mathematical plan can be feasible under declared coefficients. A runtime plan can be admissible under current native balances and deadlines. A team can be effective on actual work. None implies the next. Benchmark scores help estimate particular capabilities, but they are neither independent probabilities nor guarantees of real-job success. Subscription prices do not reveal native quota. A solver bound describes its stated objective, not overall software quality.

## Current proxy and its limits

The forecast is a declared demand and capacity scenario, not an authorized backlog or arrival model. It distributes proxy demand into four risk-selected workflow classes and selects one route for every required worker-role/class pair while one fixed conductor spans the plan. Provider accounts share modeled API-equivalent allowances and account calendars.

Expected usage estimates ordinary proxy consumption, while reserved usage protects admission under the declared stress assumptions. Actual usage comes from native meter observations and can use provider-specific units. Benchmark utilities are role evidence rather than probabilities of detecting or completing real work. A retry cap reserves a bounded conditional chain and does not launch parallel replicas. Any route substitution changes shared feasibility and requires a complete re-solve. Solver bounds apply only to the modeled objective and candidate set and are not quality confidence intervals.

The practical table should retain one Whole-plan Orchestrator row and four rows per worker seat. Each row shows one default eligible binding, planned integer job volume, account claim, and explicit fallback condition as an expandable conditional policy. It does not present several routes for a user to round-robin. The runtime selects individual jobs within a class using job features, readiness, remaining quota, and the same manifest's admissibility rules. UI, Markdown, and dispatcher share the same version and stable IDs. A model version or evidence snapshot change invalidates affected decisions and triggers a joint replan.

## Decision principle: qualify risk before maximizing output

A production allocator should solve a hierarchy of decisions. First determine whether a job is sufficiently specified and whether required capabilities are available. Next choose a workflow whose assurance level matches consequence and uncertainty. Then maximize useful admitted work under native resource and deadline constraints. Finally optimize expected value, latency, and spend among qualified plans. Quantity must not compensate for missing assurance.

Knowledge hierarchies are valuable when difficult problems can be escalated to agents with broader knowledge while routine work stays local [1](https://www.journals.uchicago.edu/doi/10.1086/317671). Software work also has complementarity: one weak critical contribution can reduce the value of otherwise strong contributions, an application of the O-ring production concept rather than a literal calibrated production function [2](https://econintel.org/Cl705/KremerORing.pdf). These ideas support role qualification and escalation. They do not justify multiplying raw benchmark scores into a fabricated “team success probability.”

Qualification must be evidence-based and role-specific. A route is eligible when its model, effort, harness, tools, permissions, network scope, context capacity, and billing source are verified for the job, and when available evidence covers the role’s demands. The system should store evidence dimensions rather than collapse everything into one universal score or take the minimum of incomparable raw utilities. Relevant dimensions can include repository comprehension, patch generation, terminal execution, fault localization, adversarial review, long-context synthesis, source retrieval, citation accuracy, and tool-control reliability. If calibrated role outcomes exist, policy can use a conservative lower confidence bound. Without them, a provisional categorical rule must say which evidence is accepted and why; it cannot masquerade as a calibrated threshold.

Thresholds should come from operational policy or calibrated outcomes. A security-sensitive migration may require a reviewer with demonstrated security analysis and a separate execution identity. A documentation correction may need source verification but no coding benchmark. An unfamiliar build-system failure may require terminal competence and local tool access. The system may enforce a categorical requirement such as “verified repository read plus shell in this workspace” or a calibrated lower confidence bound on a measured role outcome. It should not apply an arbitrary 0.7 floor to every role, nor translate a benchmark score into an unsupported probability.

Cold start must remain useful. Verified access, billing, tool, permission, and isolation facts are hard eligibility conditions. Competence can begin as a provisional task-relevant prior drawn from a named benchmark and transfer assumption. The dispatcher can immediately route bounded, real, low-consequence work whose acceptance criteria are observable, marking confidence and harness transfer explicitly. It must not create synthetic calibration tasks. High-consequence work or a route missing essential access follows the required stronger workflow or defers. One accepted first job supplies one cost and outcome observation; it does not prove reliability. As evidence accumulates, provisional rules can become calibrated lower bounds without blocking every route on day one.

Workflow risk should be a vector with explicit triggers:

| Dimension | Example observable | Effect on workflow |
|---|---|---|
| Consequence | auth, billing, deletion, public API, production data | independent review, stronger evidence, narrower permissions |
| Uncertainty | unknown root cause, novel subsystem, missing reproduction | comprehension/research before implementation |
| Coupling | number and criticality of dependent components | integration review and coordinated scheduling |
| Reversibility | rollback cost and state mutation | reserve recovery path and human gate |
| Evidence need | external facts, changing APIs, citations | Net Research seat and source packet |
| Tool risk | network, shell, browser, credentials, deployment | exact capability and permission bundle |
| Correlation | same family/harness across creator and checker | require meaningfully distinct checker where valuable |
| Deadline | slack relative to critical path and reset windows | reserve buffer, reduce scope, or defer |

The four work classes are workflow templates selected from this vector rather than from changed-file counts. File count is useful load information, but a one-line authorization defect can be high risk and a twenty-file mechanical rename can be low risk. Focused covers a bounded question with known sources and reversible output. Standard covers a well-specified task needing one implementation and independent checks. Complex covers interacting decisions, uncertain diagnosis, or consequential interfaces. Extensive covers open-ended research, broad system design, high consequence, or unresolved scope. The selected template declares its required stages and escalation rules.

## Native allocation specification

The native planning unit is an integer task count rather than an indivisible whole class. Let projects be `p`, ready task types be `t`, workflow templates be `w`, seats be `s`, route bundles be `r`, accounts be `a`, meter windows be `k`, and time buckets be `b`. A route bundle is an exact model, effort, harness, tool set, permission profile, network/source scope, and billing binding. Let `x[p,t,w,s,r]` be the integer number of worker visits assigned to a route, `z[p,t,w]` the integer number of admitted tasks, `y[r]` the selected conductor binding, and `z_o[p,t,w,r]` the tasks assigned to that binding. Each admitted task expands into its template DAG, so visit counts follow from `z` and conditional reserve policy.

The fixed conductor constraint is:

```text
sum(r in qualified_conductors) y[r] = 1
sum(w) z[p,t,w] <= D[p,t]
0 <= z_o[p,t,w,r] <= M[p,t,w] * y[r]
sum(r) z_o[p,t,w,r] = z[p,t,w]
conductor_visits[r] = sum(p,t,w) nu[p,t,w] * z_o[p,t,w,r]
sum(r) x[p,t,w,s,r] = required_visits[p,t,w,s] * z[p,t,w]
```

This linear formulation keeps one binding across project sessions while accounting for it inside every shared resource. `D` is bounded authorized backlog or an explicit forecast count; capacity forecasts never authorize invented jobs. Each actual job ID expands exactly once, and dispatch requires an authorized ready job. `M` is a valid task-count bound. `required_visits` and `nu` are integer required or reserved stage visits from an explicit workflow and scenario. Stochastic repair incidence informs expected cost and scenarios, not fractional job identities. The conductor cannot be picked after worker allocation. If its binding, account, or availability changes, the system performs a joint replan.

Task flow constraints enforce the DAG. A task can enter implementation only after required comprehension or research. Review and sanity depend on an immutable implementation artifact. Repair depends on a rejection or blocked implementation. Rechecks depend on a repair artifact. Acceptance depends on all mandatory gates. The planning model reserves conditional stages; runtime releases reservations only when the condition becomes impossible. Standard job-shop formulations illustrate the relationships: precedence delays dependent operations, while no-overlap prevents simultaneous use of one machine [25](https://developers.google.com/optimization/scheduling/job_shop?hl=en). This supports the constraint form, not a guarantee about agent work or a required solver library.

Each `x` consumes several resources simultaneously. The model must distinguish provider-allowed request concurrency and rate meters, token or native quota windows, local CPU/memory/process capacity, worker elapsed time, DAG critical-path time, handoff and feedback delay, and human attention. A persistent conductor session can be idle for much of the wall-clock window while still retaining context and incurring startup, resume, or compaction calls. The current one-lane-per-provider calendar is a conservative planning rule, not a hardware or provider truth.

- native account-meter units in every overlapping reset window;
- elapsed execution time on a route and account lane;
- exclusive workspace or repository write locks;
- host concurrency slots and tool licenses;
- deadline slack along the DAG critical path;
- monetary cost when billing is metered;
- human approval or escalation capacity where required;
- evidence collection and verification capacity for research work.

For account `a` and window `k`, robust admission requires:

```text
sum(x * worker_reserve[a,k,r,t,w])
  + C_orch[a,k](z_o, y, active_projects)
  + unreflected_residual_holds[a,k]
  + unreflected_external_use_or_reserve[a,k]
  <= observed_remaining[a,k]
```

Here `y` is binary and `z`, `z_o`, and `x` are nonnegative integers. `x` contains only new, unheld future work. `C_orch` covers planned conductor visits plus only future startup, resume, and compaction consumption not already held or reflected by the meter. Residual running holds enter only while their consumption is not yet reflected in the authoritative remaining amount. Started work and settled provider usage are never charged again. Native units must remain native. API dollars cannot be subtracted from premium requests unless a calibrated conversion with provenance and uncertainty exists. When conversion is unavailable, the route remains uncalibrated and receives only a bounded pilot admission with manual meter observation.

Robust optimization should protect against estimation error without pretending to eliminate it. Bertsimas and Sim show how budgets of uncertainty can interpolate between nominal and worst-case conservatism [3](https://pubsonline.informs.org/doi/10.1287/opre.1030.0065). Here, uncertainty sets should be fitted separately for route/task duration and native consumption. A route with five observations should carry wider uncertainty than one with hundreds. Correlated account shocks, such as a vendor throttle affecting all routes, belong in shared scenarios. The model should expose the uncertainty budget and sensitivity rather than label a plan guaranteed.

The objective is lexicographic:

1. satisfy mandatory safety, evidence, access, and workflow constraints;
2. minimize high-consequence unserved demand and expired deadlines;
3. maximize weighted useful admitted tasks with project fairness;
4. maximize calibrated expected decision value or artifact value;
5. minimize rework, critical-path time, uncertain consumption, and monetary cost;
6. retain operational slack where its value exceeds additional work.

Project fairness should borrow the principle of dominant resource fairness: compare projects by their largest share of any scarce resource, so a project cannot dominate simply because it consumes a different mix [4](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf). This is inspiration rather than a demand to equalize provider spending. A project using Anthropic and another using OpenAI can be treated fairly by dominant shares of the resources each needs. Priority, deadline, and consequence can weight entitlements. Unused vendor capacity is valid when no qualified ready task can consume it.

Comparative advantage asks where losing a route hurts the joint plan most. If Grok and Claude were equally effective for a small task but Grok were uniquely effective for debugging, the solver should place Claude on the small task and reserve Grok for debugging. That is an illustration, not a measured claim about either model. The decision comes from feasible joint substitutions and current shadow prices, not fixed provider order.

The LP relaxation’s dual values can report local shadow prices. If the OpenAI meter constraint has a high dual value, one additional unit would locally improve the relaxed objective more than an idle unit elsewhere. This helps explain displacement: “this conductor route consumes the resource currently limiting qualified implementation.” Shadow prices are local approximations for the current relaxation. They can change after an integer substitution, workflow change, reset, or new task, and must not be presented as stable market prices. With incomplete user preferences, retain alternatives compatible with the stated ordering instead of inventing neutral weights; robust ordinal regression offers one way to report necessary and possible preference relations [24](https://www.lamsade.dauphine.fr/mcda/biblio/PDF/GMS-EJOR2008.pdf).

Online learning is necessary because route quality and consumption are uncertain. Bandits with knapsacks formalize learning while resources deplete [5](https://arxiv.org/abs/1305.2545), and RouteLLM learns cost-quality routing [6](https://arxiv.org/abs/2406.18665). Exploration remains bounded to qualified, low-consequence, reversible work; high-risk jobs are not experiments.

## Receding-horizon dispatcher

The weekly static plan is an initial horizon rather than runtime truth. Runtime replans on each material event: job arrival, scope or risk change, stage completion, rejection, consumption settlement, native meter snapshot, reset, deadline change, route failure, host loss, or verified capability change. Work already started and immutable outputs already produced remain fixed while the remaining ready queue, holds, conditional stages, and future capacity are reconsidered.

The plan is a predetermined conditional policy: for a given job and observed state, it chooses one action. The readable table and generated Markdown are versioned views of that policy with the same stable IDs. They are not free-choice menus or round-robin suggestions. Future jobs, costs, meter states, and availability remain unknown, so the policy relies on receding-horizon replanning rather than an omniscient weekly timeline.

At each dispatch transaction the executable dispatcher, rather than the language model, performs these steps:

1. acquire the app-owned ledger lock, load a versioned profile/ledger snapshot, reconcile authoritative observations, then release the lock;
2. compute available capacity and the ready queue from that immutable snapshot;
3. solve the horizon without holding the ledger lock;
4. reacquire the lock, reload state, and compare the snapshot version;
5. if relevant state changed, release and retry from the new snapshot;
6. otherwise persist task, attempt, route, account-window, and hold identities atomically, then release;
7. launch through the exact isolated route bundle;
8. settle usage and artifact outcome idempotently, then trigger replanning.

Expected, reserve, and actual quantities must never overwrite one another. Expected values drive ordinary planning and value estimates. Reserve values protect admission and may be scenario-specific. Actual provider usage is an authoritative aggregate that may be delayed or quantized and may include unrelated calls on the shared account. Per-attempt attribution is therefore an estimate unless the provider supplies exact identifiers. Settlement clears each hold once, carries unresolved residual consumption forward, and never subtracts spend already included in a remaining-balance snapshot. A call crossing a reset retains a conservative hold and attribution estimate until the aggregate meter resolves it.

Budget pacing aims to preserve useful optionality over the remaining active window. It cannot guarantee 100% consumption. The dispatcher must not invent work, duplicate a task, assign sequential padding, or run a weaker seat merely to exhaust quota. When surplus exists, it can move eligible, ready, disjoint work to other qualified seats or start bounded evidence collection with declared decision value. When deficit exists, it can lower effort only if the exact lower-effort binding remains qualified, choose a qualified substitute on another account, reduce scope with the user, or defer. Every change is evaluated jointly because it changes opportunity cost and potentially the critical path.

For provider `v` and native window `k`, define `A[v,k]` as the provider-reported current remaining amount minus residual holds or external-use reserve not yet reflected in that observation. Define `D[v,k]` as forecast consumption for qualified, authorized future work in the joint plan and calendar. Then `S[v,k] = A[v,k] - D[v,k]`. Compare `S` with the calibrated uncertainty band for that meter and workload rather than a universal percentage. Its sign and interval inform the local opportunity price and trigger replanning. A burn trajectory follows ready useful work, active periods, deadlines, and provider reset windows; it is not linear wall-clock spend or equal utilization across vendors.

Positive excess funds already-ready, disjoint, qualified work only when it cannot delay critical work through a shared account, host, workspace, handoff, or human-attention resource. Deficit leads to a still-qualified lower effort, a joint substitution, scope reduction, or deferral. Avoidably expiring subscription capacity is a final tie-break among equivalent useful quality and time outcomes. Subscription price is sunk during the active billing period, while incremental API cash is a separate marginal cost. The planner should use paid capacity promptly when valuable work is ready, without turning spending itself into output or allowing a dollar proxy to leave every paid pool idle automatically.

The conductor remains persistent across project horizons. It owns plan decomposition, evidence requests, conflict reconciliation, and user-facing decisions. It does not own arithmetic or ledger mutation. Its own startup, resume, compaction, planning, and reconciliation tokens consume the shared account even when they occur outside worker dispatch. The launcher needs a meter hook and reserved allowance for these calls; Markdown cannot intercept or settle them. Multiple project sessions use the same selected binding while their contexts and jobs remain independently identified and fairly scheduled. `N` increases total context and coordination demand; it does not multiply subscription capacity.

Agent-system scaling research finds that more agents help decomposable work but can harm through coordination, duplication, and sequential dependencies [8](https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/). Admit parallel visits only for disjoint artifacts or independently valuable checks. Project conductors do not multiply quota or native concurrency.

## Net Research as a seventh seat

Net Research is a first-class seat rather than an incidental tool available to whichever worker happens to browse. Its route bundle includes exact model, effort, harness, tools, authorization, network policy, source scope, browser or fetch capability, citation requirements, and billing account. The seat produces a compact evidence packet consumed by comprehension, implementation, review, or the conductor. When it resolves facts already needed by comprehension or planning, its consumption replaces that unknown-resolution labor; it is not a blanket additive visit for every change. A one-line external documentation correction can require research, while a large internal refactor with authoritative local context may require none.

Research complexity should be based on the question, not file count:

| Class | Question shape | Required behavior |
|---|---|---|
| Focused | one stable fact or one known primary source | fetch, extract, cite, record date and scope |
| Standard | several factual claims or two-source comparison | structured search, primary-source preference, contradiction check |
| Complex | changing product facts, cross-provider comparison, or technical synthesis | query decomposition, source-quality assessment, uncertainty register |
| Extensive | open-ended landscape, contested evidence, or high-consequence decision | bounded research plan, multiple independent searches, synthesis review |

Demand should be triggered by decision value. Research is warranted when an uncertain fact could change route eligibility, risk, cost, schedule, or user choice. It is not warranted merely to decorate a report or maximize information entropy. Decision-theoretic work on imperfect forecasts frames information value through how it changes the decision and payoff [26](https://pubsonline.informs.org/doi/10.1287/mnsc.13.3.233). Search results should become stable evidence records containing claim, source URL, publisher, retrieval time, direct support span or structured extraction, source type, freshness requirement, uncertainty, and consuming decision. Source uncertainty remains separate from model ability: an excellent model cannot make a weak source authoritative.

Human code-review research finds that understanding a change and its context is central to review, while review also transfers knowledge and develops alternatives [27](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/ICSE202013-codereview.pdf). Specialists should own bounded deliverables; handoffs carry requirements, code state, and evidence; exceptions escalate to the conductor. Context compression must preserve those inputs and explicit decisions rather than an author's confidence. This observational evidence does not imply that one expensive reviewer dominates two inexpensive reviewers.

Benchmarks illustrate distinct research demands. BrowseComp contains 1,266 hard-to-find factual questions and is designed to test browsing persistence [9](https://openai.com/index/browsecomp/). FutureSearch Deep Research Bench contains 89 multi-step open-web tasks across eight categories [10](https://arxiv.org/abs/2506.06287). Deep Research Bench II uses 132 tasks and 9,430 report rubrics; its evaluated systems remain below 50% rubric satisfaction, which motivates careful report evaluation but does not rank the exact current routes [11](https://arxiv.org/abs/2601.08536). BrowseComp-Plus studies stronger source and answer verification [12](https://aclanthology.org/2026.acl-long.1023/). None alone proves repository-aware orchestration, tool authorization, or current product entitlement. This evidence audit found no comparable primary result covering the exact Astra, Luna, Opus, Grok, Gemini, and Muse bundles in this table. A global model winner from a 2025 benchmark must not be asserted for a 2026 tool bundle.

Local capability evidence must be represented precisely. Claude safe mode preserves provider-native built-ins subject to permissions and explicitly identifies WebFetch, while disabling custom skills, plugins, hooks, MCP, commands, and agents. Grok help identifies built-in web search and fetch unless disabled, but capability still depends on sandbox and permission. Muse exposes a web-tools disable flag that the shipped isolated route does not pass; exact operations and permission outcomes remain unknown. Codex’s inspected help does not establish whether a native web feature is enabled. Antigravity exposes plugin and MCP management but does not prove a usable research tool in the clean generated home. Multi-provider harness branding proves neither provider access nor research capability.

The selected user's Grok subscription is confirmed by the user to include X access. This is an entitlement fact for that Grok setup, not independent verification for every plan, binding, tool surface, or isolated profile. The xAI X Search tool is also a documented API capability [13](https://docs.x.ai/developers/tools/x-search). X publishes an MCP server usable by MCP-compatible clients subject to X API permissions and a developer application [14](https://docs.x.com/tools/mcp), and its search API is nonexclusive [15](https://docs.x.com/x-api/posts/search/introduction). These separately paid API or MCP connectors are not equivalent to the included Grok capability. A subscription-only route excludes them unless they are explicitly configured, authorized, and funded. Other users still require onboarding of their exact binding, included tools, isolation behavior, quota, and billing. Grok's consumer FAQ describes a shared weekly pool across Chat, Imagine, Voice, and Build with workload-dependent consumption and a displayed reset [16](https://docs.x.ai/grok/faq); API billing must not be inferred from that consumer account.

The Antigravity executable must not be equated with Gemini CLI. Antigravity CLI documentation identifies subagents and web-search tools [17](https://antigravity.google/docs/cli/features/), while its permissions documentation makes `read_url` and `execute_url` approval-sensitive [18](https://antigravity.google/docs/cli/permissions/). Its plans describe five-hour and weekly limits with workload-dependent consumption [19](https://antigravity.google/docs/plans). Exact isolated binding, headless permission outcome, entitlement, and meter still require onboarding. Gemini CLI's tool inventory belongs to that different product and version [20](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/tools.md). Muse's apparent web-tool support establishes potential capability, not quality. Codex evidence must state whether it came from cached material or a live retrieval. Claude's current tool reference documents distinct WebSearch and WebFetch behavior and a shared per-session search limit [21](https://code.claude.com/docs/en/tools-reference); permissions remain governed by its permission system [22](https://code.claude.com/docs/en/permissions). Codex configuration claims should be grounded in current configuration documentation [23](https://learn.chatgpt.com/docs/config-file/config-basic).

The initial Net Research candidate table should therefore show conditional eligibility rather than a winner. Claude is eligible when WebSearch/WebFetch and permissions survive the exact isolated bundle; Codex is provisional until its exact search/fetch tools and cached-versus-live source behavior are verified; Grok is eligible for general web work when its built-ins survive isolation and for X-specific work only with verified X capability and billing; Antigravity is eligible when URL/search permissions work headlessly; Muse is eligible only after its exact web tools, evidence quality, and billing are observed. General-purpose models may qualify without a coding benchmark. Technical research requires relevant domain understanding and source handling, not a blanket SWE weight. FutureSearch's current board provides adjacent researched-forecasting evidence for Opus 5 xhigh and GPT-5.6 Sol high on frozen web snapshots [28](https://drb.futuresearch.ai/); that is not research-report accuracy, an exact CLI-effort comparison, or evidence for Astra, Luna, Grok, Gemini, or Muse.

## Runtime profile and ledger specification

The application already owns a local orchestrator directory below its platform-specific application data directory. Preserve its exact filenames: `setup.v1.json`, `ledger.v1.json`, `setup.v1.schema.json`, `ledger.v1.schema.json`, `ledger.v1.lock`, and `generations/`. Generated fleet jobs remain below a generation directory’s `jobs/<task-id>/`. Markdown may name these app-resolved paths at generation time, but it must not contain credentials or copy a previous machine’s path as an instruction on another machine.

The profile stores stable, approved facts:

- schema version and computer identity;
- host ID, host kind, executable identity, version evidence, isolation capability, and provenance;
- exact model binding ID, benchmark provenance, native model identifier, effort, harness, and invocation arguments;
- tool inventory, permission mode, network policy, allowed source scope, filesystem roots, exclusions, and exclusive resources;
- provider and billing binding, connector, account alias, subscription or API mode, plan, and verification provenance;
- native meter names, units, capacity rules, reset semantics, timezone, and observation source;
- workflow calendar, active periods, concurrency capability, approval policy, and lifecycle controls;
- capability and billing facts with source, observed time, freshness, confidence state, and user-approved overrides.

The ledger stores live state:

- schema version, computer ID, monotonic update identity, and timestamp;
- account-window IDs, meter IDs, start/reset/end times, observed capacity and remaining amount;
- snapshot source, observation cutoff, cumulative external usage, and uncertainty state;
- project ID, priority, entitlement weight, deadline, and dominant-resource share;
- task ID, idempotency key, workflow template, role, risk vector, evidence requirements, dependencies, ready state, and immutable artifact IDs;
- attempt ID, exact binding, host process identity, start/end state, reservation IDs, and retry lineage;
- per-window expected usage, robust reserve, actual settlement, outstanding hold, and straddling-call attribution;
- outcome category, artifact acceptance, rejection reason, rework cause, human escalation, and released conditional reservations.

Stable IDs should derive from canonical identities where possible and remain readable in compact Markdown references. JSON field order and minification can reduce prompt size, but compactness must not erase units, provenance, or distinctions between expected, reserve, actual aggregate, and attribution estimates. A compact policy should point to schemas on disk rather than embedding repeated schema prose in every routine prompt. The executable launcher and dispatcher enforce state and boundaries. Markdown teaches the policy and supports portability, but cannot guarantee arbitrary model compliance.

Onboarding must verify exact bindings before admission: executable and version; authentication without reading or recording secrets; exact model and effort availability; native invocation identifier; subscription connector and billing account; shared aliases across harnesses; meter unit and reset behavior; remaining quota observation method; work calendar; actual concurrency; filesystem boundaries; network and research tools; permission and approval behavior; cancellation and status controls. A host mismatch is a user decision. The system should present the measured harness, proposed native harness, evidence-transfer assumption, resource impact, and replan result, then ask once. It must not silently treat model-family similarity as exact entitlement.

Native isolation should compile from the verified bundle. A clean generated home reduces ambient configuration but does not itself prove network denial or tool safety. Per-host launch arguments must enforce the approved customization, plugin, MCP, subagent, permission, and network policy. If the CLI cannot enforce a required boundary, the route is ineligible for isolated fleet work. Nonisolated interactive use can remain available when explicitly approved, but it is a distinct capability and risk class.

## Telemetry and calibration

The system needs real-work telemetry, not a test suite, to learn useful coefficients. Every completed attempt should record task and risk descriptors, exact bundle, input/context size buckets, native meter deltas, wall and active time, tool calls by class, artifact outcome, reviewer disposition, rework, and whether the attempt changed the final accepted result. It should not record secrets, full prompts by default, or sensitive repository content merely for analytics.

Calibration should estimate distributions by route, role, workflow, risk, and task family. Sparse groups borrow only through explicit hierarchical models and retain wide intervals. Native quota conversion uses aggregate meter changes over controlled intervals, with concurrent external use represented separately; it must not claim exact per-attempt attribution from a shared delayed meter. Conductor overhead is calibrated from its own real planning and reconciliation calls, not directly from Intelligence Index. Intelligence Index includes agentic evidence, but it does not directly calibrate this application's conductor, router, or Net Research bundle. Conditional repair incidence is learned from actual gate outcomes. Correlation is estimated across creator/checker bundles and conditional retries rather than assumed away.

Outcomes must distinguish inability, tool failure, permission denial, quota denial, timeout, incorrect artifact, incomplete artifact, review disagreement, and user preference. Rework that merely changes style is not the same as correction of a defect. Artifact acceptance and later rollback are stronger outcome signals than self-reported success. Telemetry can inform decisions only after its observation process and missingness are understood.

The planner should publish calibration provenance with each coefficient: sample count, time range, workload definition, route identity, estimator, interval, and last observation. Stale or transferred coefficients receive explicit uncertainty. A benchmark-to-native-harness transfer remains a proxy until real tasks support it. No coefficient should be re-read solely to prove the program wrote it; observations matter because they change admission, route qualification, reserve size, or replanning.

## Practical rollout

The first rollout should improve truthfulness before sophistication.

| Ship first | Remains empirical |
|---|---|
| exact binding, billing, tool, permission, network, root, and isolation gates | role effectiveness on each real task family |
| integer ready jobs, risk-selected workflow templates, and fixed conductor binding | active-project duty factors and arrival distributions |
| native-window ledger with versioned short-lock transactions | native consumption and duration distributions |
| deterministic conditional policy shared by UI, Markdown, and runtime | conductor startup, resume, compaction, and reconciliation overhead |
| provisional benchmark priors with source and transfer flags | creator/checker and conditional-retry error correlation |
| expected, reserve, aggregate actual, and attribution-estimate separation | calibrated uncertainty bands and role lower confidence bounds |
| staged admission and joint replanning | long-run subscription value from qualified ready demand |

1. Add integer ready jobs, risk-selected workflow templates, and the Net Research bundle with verified tools and billing; retain the fixed conductor and DAG.
2. Split API-equivalent forecasts from native account windows. Uncalibrated accounts receive bounded pilot admission.
3. Put qualification and workflow constraints before throughput, with provisional evidence rules until calibrated outcomes exist.
4. Add dominant-share fairness, robust coefficients, receding-horizon triggers, frozen-started work, atomic holds, and idempotent settlement.
5. Record real-work resource, outcome, and rework telemetry; expose local shadow prices and feasible integer substitutions.
6. Permit bounded exploration only on qualified, low-consequence work, and reassess subscriptions from qualified useful demand. Idle Muse capacity may remain idle or be removed at renewal.

Acceptance should be artifact based. Given a fixed profile, ledger snapshot, project queue, and evidence catalog, the dispatcher emits a deterministic conditional policy with stable IDs. Every scheduled visit maps to an exact qualified binding and required dependency. Conductor and worker holds sum exactly into the same native account windows. Expected, reserve, provider-reported actual aggregate, and attribution estimates reconcile without double counting. Started tasks remain fixed across a replan. A reset-spanning call preserves its hold and attribution estimate. Removing a capability makes dependent routes ineligible. Raising task risk selects the required workflow or defers. A deficit produces only qualified lower effort, qualified substitution, scope reduction, or deferral. Surplus never creates duplicate or fake tasks and never delays required work through a shared resource. Parallel Reviewer and Sanity visits are legitimate only when they apply distinct explicit criteria and have scheduling as well as DAG independence. A moved artifact requests rebased paths and binding verification. These are observable artifacts and state transitions; they do not require claims of guaranteed quality.

## Operating boundary

Proxy allocation does not authorize execution. Runtime admission verifies exact model and effort bindings, billing, native meters and reset windows, permissions, concurrency, deadlines, dependencies, and fresh shared-account holds. It admits ready task IDs with risk-qualified workflows, settles observed native usage, and replans after material changes. The conductor remains one accountable identity, the dispatcher owns arithmetic, and research is invoked only for decisions it can change. `H`, `N`, and subscription IDs do not identify backlog readiness, consequence, tool needs, deadlines, account state, or error correlation. Empirical coefficients remain provisional until real-work telemetry supports them.

## Sources

1. Luis Garicano, [“Hierarchies and the Organization of Knowledge in Production”](https://www.journals.uchicago.edu/doi/10.1086/317671), *Journal of Political Economy* 108(5), 2000.
2. Michael Kremer, [“The O-Ring Theory of Economic Development”](https://econintel.org/Cl705/KremerORing.pdf), *Quarterly Journal of Economics* 108(3), 1993.
3. Dimitris Bertsimas and Melvyn Sim, [“The Price of Robustness”](https://pubsonline.informs.org/doi/10.1287/opre.1030.0065), *Operations Research* 52(1), 2004.
4. Ali Ghodsi et al., [“Dominant Resource Fairness: Fair Allocation of Multiple Resource Types”](https://www.usenix.org/legacy/events/nsdi11/tech/full_papers/Ghodsi.pdf), NSDI 2011.
5. Ashwinkumar Badanidiyuru, Robert Kleinberg, and Aleksandrs Slivkins, [“Bandits with Knapsacks”](https://arxiv.org/abs/1305.2545), 2013.
6. Isaac Ong et al., [“RouteLLM: Learning to Route LLMs with Preference Data”](https://arxiv.org/abs/2406.18665), 2024.
7. Kim et al., [“Correlated Errors in Large Language Models”](https://proceedings.mlr.press/v267/kim25e.html), ICML 2025 proceedings.
8. Google Research, [“Towards a science of scaling agent systems: When and why agent systems work”](https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/), 2026.
9. OpenAI, [“BrowseComp: a benchmark for browsing agents”](https://openai.com/index/browsecomp/), 2025.
10. FutureSearch, [“Deep Research Bench”](https://arxiv.org/abs/2506.06287), 2025.
11. Deep Research Bench II, [132-task and 9,430-rubric report benchmark](https://arxiv.org/abs/2601.08536), 2026.
12. BrowseComp-Plus, [ACL 2026 paper](https://aclanthology.org/2026.acl-long.1023/), 2026.
13. xAI, [X Search tool documentation](https://docs.x.ai/developers/tools/x-search), accessed 2026-09-09.
14. X Developer Platform, [X MCP documentation](https://docs.x.com/tools/mcp), accessed 2026-09-09.
15. X Developer Platform, [posts search introduction](https://docs.x.com/x-api/posts/search/introduction), accessed 2026-09-09.
16. xAI, [Grok consumer FAQ](https://docs.x.ai/grok/faq), accessed 2026-09-09.
17. Google Antigravity, [CLI features](https://antigravity.google/docs/cli/features/), accessed 2026-09-09.
18. Google Antigravity, [CLI permissions](https://antigravity.google/docs/cli/permissions/), accessed 2026-09-09.
19. Google Antigravity, [plans and limits](https://antigravity.google/docs/plans), accessed 2026-09-09.
20. Google, [Gemini CLI tools reference](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/tools.md), accessed 2026-09-09.
21. Anthropic, [Claude Code tools reference](https://code.claude.com/docs/en/tools-reference), accessed 2026-09-09.
22. Anthropic, [Claude Code permissions](https://code.claude.com/docs/en/permissions), accessed 2026-09-09.
23. OpenAI, [Codex basic configuration](https://learn.chatgpt.com/docs/config-file/config-basic), accessed 2026-09-09.
24. Salvatore Greco, Vincent Mousseau, and Roman Słowiński, [robust ordinal regression for value functions](https://www.lamsade.dauphine.fr/mcda/biblio/PDF/GMS-EJOR2008.pdf), *European Journal of Operational Research*, 2008.
25. Google for Developers, [job-shop scheduling example](https://developers.google.com/optimization/scheduling/job_shop?hl=en), accessed 2026-09-09.
26. Howard E. Thompson and William Beranek, [“The Efficient Use of an Imperfect Forecast”](https://pubsonline.informs.org/doi/10.1287/mnsc.13.3.233), *Management Science*, 1966.
27. Alberto Bacchelli and Christian Bird, [“Expectations, Outcomes, and Challenges of Modern Code Review”](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/ICSE202013-codereview.pdf), ICSE 2013.
28. FutureSearch, [BTF-3 researched-forecasting leaderboard](https://drb.futuresearch.ai/), accessed 2026-09-09.

Local source inventory: application files `portfolio.rs`, `solver.rs`, `engine.rs`, `subscriptions.rs`, `types.rs`, `settings.rs`, `aa.rs`, `orchestration.rs`, `host_install.rs`, `agent_setup.rs`, and `isolation.rs`, with `METHODOLOGY.md` and `local-harness-capabilities.md`. These are a 2026-09-09 research snapshot of a dirty 0.2.1 working tree based on commit `492cbd8b04ef6b9d8b69e3809250f61ec251cf3d`; they are source provenance, not a deployed-binary claim.
