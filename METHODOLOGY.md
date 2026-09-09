# Methodology

Best in house offers Absolute best and Per plan. Absolute best and the vendor tabs make an independent recommendation for each role and plan tier, selecting the candidate and retry cap with the most nominal verified reference tasks per week. Their scenario bands describe each independent pick.

Per plan produces a deterministic API-equivalent proxy forecast for the selected subscriptions. Every worker seat has four conditional workflow policies, each naming one default binding, integer proxy job volume, exact account claim, and ordered fallback condition. One persistent conductor binding owns planning and reconciliation across the whole plan. The native planner separately admits authorized ready task IDs against verified bindings, native account windows, dependencies, and holds. UI, generated Markdown, and dispatcher use the same versioned policy and stable IDs. Subscription choices default to none and the mode defaults to Absolute best.

Both modes separate role competence from operating capacity and apply optional competence minimums as hard qualification requirements.

## Per-plan dispatch policy

Record consequence, uncertainty, coupling, reversibility, evidence need, tool risk, correlation, and deadline risk before paid delegation. Extensive covers open-ended research, broad system design, high consequence, or unresolved scope. Complex covers interacting decisions, uncertain diagnosis, consequential interfaces, or costly recovery. Standard covers a well-specified task requiring an independent check. Focused covers a bounded question with known sources, observable acceptance, and reversible output. File and component counts inform load but do not select the workflow. A material risk change requires workflow reselection and joint replanning.

The proxy forecast is `ceil(elapsed weekly working hours * concurrently active project orchestrators / 4)` jobs, apportioned deterministically into approximately 50% Focused, 30% Standard, 15% Complex, and 5% Extensive using Hamilton counts. Resource factors are `0.25`, `1`, `2`, and `4`. This is a declared capacity scenario, not an authorized backlog, demand forecast, native quota conversion, or executable admission. Native admission starts at zero and is derived only from authorized ready task IDs. The orchestrator count defaults to one and aggregates offered project demand without multiplying shared quota or approved runtime concurrency. Expected proxy figures use a 25% repair incidence and make no cross-role independence claim.

Each admitted class selects one model and effort with a same-model attempt cap from 1 through 3 for each worker role. A single whole-plan conductor choice has cap one. Its utility is the normalized Artificial Analysis Intelligence Index, and its resource coefficients require the common first-party Intelligence Index task cost and canonical decode-time proxy. Known competence and any configured competence minimum qualify the routes. Shared provider budgets, nested limits, and time constrain the conductor and worker choices jointly. Role utility is an explicit preference score; it is not a calibrated probability or a guarantee of completed real-world changes. Unsupported or unfunded classes explicitly defer.

For worker role `r`, class `c`, and eligible model/effort/cap policy `p`, the binary decision `x[r,c,p]` selects exactly one policy for each required worker role-class group. A separate binary `y[p]` selects exactly one conductor policy for the whole admitted plan. Let `N[c]` be admitted class demand, `e[r]` the expected visit count under the declared repair incidence, `m[r]` the full bounded visit count, `f[c]` the class resource factor, `w[r,b]` the fixed competence-component weight, and `v[r,b,p]` its policy-specific evidence value. For a worker reference-workload benchmark, `v` is that benchmark's capped retry completion probability; non-reference GPQA, HLE, and LCR components remain static. The worker utility and primary objective are:

```text
U[r,p] = sum_b(w[r,b] * v[r,b,p])
sum(N[c] * e[r] * f[c] * U[r,p] * x[r,c,p])
```

Each worker evidence component enters once. At cap one, Implementer and Sanity utility equals their original pass-based competence, not its square. Competence floors use the original competence values separately. Nominal time and cost break utility ties. Worker reservation coefficients use `N[c] * m[r] * f[c]` and each policy's full-cap stressed cost and time. Conductor expected and reserved load both use `2 * sum_c(N[c] * f[c])` Intelligence Index benchmark-task-equivalent units. This models coordination demand rather than literal native turns; the runtime ledger charges every actual native call exactly once. Each owned subscription contributes one copy of its declared allowance and one account calendar over the same elapsed working horizon. A visit must fit one account's allowance and calendar; Fable work must also fit that account's nested Fable allowance. The conductor remains on one account lane. The complete reserved workflow must fit the working week under the deterministic scheduler. Admission first maximizes the length of the fixed forecast prefix, then optimizes route quality for that admitted prefix. This is a discrete resource-constrained assignment formulation; [Google's assignment modeling example](https://developers.google.com/optimization/assignment/assignment_cp) describes the one-choice and shared-capacity pattern.

The forecast uses full Hamilton class counts followed by a deterministic deficit-based class sequence; admitted work is a prefix of that sequence. Prefix-preserving calendars make feasibility monotone in the admitted count. The solver uses a lexicographic LP relaxation with required-group coverage before quality, nominal time, and nominal cost, then branch-and-bound over the one-hot route choices. Complete assignments that fail reserved makespan feasibility generate no-good cuts. Deterministic coordinate improvement evaluates single-group substitutions against both linear resources and the complete scheduler before and after branch-and-bound. Candidate pruning retains nondominated choices within each provider and requires exactly equal reserved duration for dominance so substitutions preserve the fixed-calendar scheduling constraint.

The finite search allows 1,024 LP nodes globally and 256 per admitted-count probe. Required-group coverage uses a `1e-7` operational tolerance, integrality `1e-7`, LP signs `1e-10`, pivots `1e-12`, and quality/time/cost comparisons `1e-8`. An optimal status means optimal within this declared scheduler and these numerical tolerances, with no unresolved larger admission. A feasible incumbent at a search limit remains executable with its reported quality bound and gap; `feasible_search_limit` does not claim optimality. `infeasible` and `search_limit` without an executable incumbent yield no operating policy. Expected completions are class-factor-scaled reference-equivalent service units, while expected visits count actual invocations.

The API-equivalent proxy uses a nine-visit stress template: Orchestrator brief, Comprehension, Implementer, initial Reviewer and Sanity, one conditional Debugger repair, Reviewer and Sanity rechecks, and Orchestrator close. Native tasks expand from the risk vector, invoke Net Research when evidence is required, and reserve only the required and conditional visits in that task DAG. Reviewer and Sanity inspect immutable results and run concurrently only when their criteria and resources are independent. A failed repair or rejected recheck defers without an unplanned repair cycle.

The prefix-preserving scheduler iterates admitted change ordinal and then topological stage ordinal. Each visit is placed in the earliest eligible owned-account calendar gap after all predecessors finish, subject to that account's allowance and nested limits; earlier reservations never move. Account calendars share the same elapsed working horizon and may operate concurrently. This is a forecast slot, not a fabricated native account identity or meter. A candidate whose complete reserved workflow makespan exceeds the working week is rejected and the solver continues. The exported stage schedule includes dependencies, conditions, provider and account ordinals, and conservative start/end offsets under an all-forecast-work-ready-at-start assumption. Late arrivals or overruns that no longer fit the remaining schedule require replanning or deferral against the actual week deadline. Unused conditional repair and recheck stages can be released after primary acceptance.

Forecast reservations are modeled API-equivalent cost and working-hour estimates, not runtime ledger entries. The dispatcher owns the shared ledger lock, admission arithmetic, reservations, settlement, and replanning. Demand and balances aggregate across all projects; each admitted job reserves against every applicable account window and nested limit. Available capacity subtracts only future work, unreflected residual running holds, and unreflected external use or reserve from the authoritative remaining amount. Reliable actual native usage settles the matching hold once; unknown started usage remains held until authoritative meter reconciliation. Only never-started work can release its hold without a debit. Meter snapshots rebase consumption covered by their observation cutoff without double charging. Worker slots are separate from budget holds. Model-estimate USD, raw tokens, and native quota units are not interchangeable. Actual limits, authorization, deadlines, and measured usage control runtime admission.

## Generated conductor and host setup

The collaboration workspace is a user-selected absolute local root, defaulting to `aitierlist-workspace` below the user's home directory. It contains `projects/`, `docs/briefs/`, `docs/decisions/`, `research/<task-id>/`, `deliverables/`, and Git-ignored `scratch/`, plus a concise workspace guide and machine-readable layout index. Setup preserves existing files and repositories, creates missing paths, and initializes Git only when the root has no repository. It never commits or pushes. Profiles, authentication, ledgers, holds, generation bundles, and host homes remain in private app-managed storage. Isolated routes use the durable collaboration root and project paths in their compiled permission and launch identities.

Accepted evidence packets and readable findings publish together below their task's research directory and enter the shared research index by stable packet ID. The published immutable artifact retains packet digest and producer identity. Publication consumes the packet already accepted by settlement; it does not fetch sources again or reinterpret citation validity.

The four workflow rows for every worker seat, including Net Research, form the proxy policy. The generated `orchestrator.md` records one whole-plan conductor, stable route and binding IDs, integer planned volume, exact provider/plan/account claim, calibration state, and an ordered conditional fallback. Fallbacks are not a free model menu and are never round-robin. Eligibility follows task-relevant categorical evidence and configured calibrated bounds when available; it has no universal percentage gate. The conductor has no adaptive swap: changing its model, effort, native harness, binding, provider, plan, or account requires joint replanning. The four-hour cadence and resource factors describe proxy volume. Full-cap repair reservations are stress safeguards. A fallback requires exact native model, harness, tool, permission, billing, and account verification plus fresh real-meter holds. Comparative-advantage and opportunity-cost values remain heuristic routing evidence rather than LP dual prices or measured displacement costs.

The conductor remains human-facing, delegates distinct implementation briefs, runs disjoint writers or independent review concurrently when useful, reconciles results, and tracks cancellation and resumption. Only roles needed for the actual work are invoked. The static forecast calendar is an illustrative feasibility result; runtime dispatch follows ready work, dependencies, current deadlines, and actual account availability.

Onboarding verifies the conductor's exact native host, model, effort, binding, provider, plan, account aliases, real meter units and resets, environment boundaries, and workflow facts. Material discrepancies are consolidated and require joint replanning before paid work. Confirmed facts and intentional overrides persist in the exact first-party machine profile, embedded in the document with portable schema contracts. Ready profiles generate compact reuse and changed-facts startup instructions. Partial or absent profiles generate the full three-phase onboarding; partial valid profiles preserve confirmed facts. A moved machine always requires rebased onboarding. A separate ledger records live jobs and account-window holds; one shared transaction lock prevents installed hosts from spending duplicated balances while allowing project conductors to operate concurrently between transactions.

The static UI and Markdown label proxy jobs separately from native admission. After runtime admission, the native-plan renderer consumes the actual versioned horizon plan and shows its task and project IDs, workflows, fixed conductor binding, integer visit volumes, visit IDs, route IDs, binding IDs, account IDs, native-window reservations, deferred reasons, and coefficient uncertainty. A policy-version or content-identity mismatch blocks rendering or dispatch and requires regeneration and joint replanning.

For each verified native window, remaining capacity and full-window projected useful demand carry their native units, reset semantics, work calendar, deadline slack, and calibrated uncertainty interval. Each account window supplies its declared operational trigger; the application does not impose linear spending or invent universal pacing percentages. Every binding window applies. A deficit first uses the listed qualified lower effort, then a qualified funded substitution, authorized scope reduction, or deferral. Spare capacity serves distinct already-needed work only when it adds no critical-path wait. Useful demand shortage ends the job with unused budget reported.

Native quota units are distinct from modeled API-equivalent USD. Conservative per-call holds require calibration in the native meter's units; an unknown meter enters explicit uncalibrated/manual-check mode with one bounded job before reassessment. Cumulative meter snapshots reconcile against a saved baseline without double-counting job settlements or external account use. Unknown started usage remains conservatively held and stale until reconciliation. Every remaining job.holds entry counts as outstanding regardless of blocked or cancelled job status; settlement explicitly clears resolved holds. Calls spanning a reset retain their holds and measured window attribution.

Settlement and meter reconciliation are pure state transitions executed inside the dispatcher's short compare-and-swap ledger transaction. A settlement key binds immutably to the hash of its full content. It names every visit hold exactly once with the exact account, reset-window instance, and native unit. Usage without an identified provider event remains an attribution estimate and keeps its conservative hold unless an authoritative observation explicitly covers it. An authoritative remaining-balance observation carries its provider cutoff, window start and reset, evidence, and the exact hold, settlement, provider-event, and external-reserve identities it covers. Observations advance monotonically within a reset instance. Reconciliation retains post-cutoff and unreflected consumption and never moves settled use from an old reset instance into a new one.

Artifact acceptance requires a recorded started attempt or the explicit external-conductor execution hook. Accepted artifacts carry stable IDs, digests, producer attempts, parent artifact identities, and acceptance times. An accepted Net Research visit additionally supplies an evidence packet whose claims name the consuming decision and stable source records. Each source records URL, publisher, source type, retrieval time, direct support or structured extraction, freshness requirement, source uncertainty, and live or cached provenance. Source uncertainty remains separate from optional model confidence. Packet acceptance checks required handoff structure and declared criteria; it does not claim to verify a citation's truth or calibrate research quality.

The native Generate popup discovers hosts asynchronously at startup, prepares the document and exact installation preview asynchronously, and installs only after the user accepts the displayed destinations and launch/PATH impacts. Only the available recommended host is checked by default; the interface names the recommendation's source class. Multiple or different hosts can be selected; no hosts still produces portable Markdown. PATH changes are opt-in. Successful hosts and failed hosts are reported separately, with failed-only retries. Prepared policy settings and table revision must still match the current score before installation. First launch performs onboarding; installation alone does not establish access or complete setup.

Discovery covers OpenCode, Aider, Goose, Cline, OpenHands, Pi, and Droid as multi-provider execution harnesses. A native executable, unresolved shell shim, configuration footprint, and desktop-only installation are distinct states. Known npm native sidecars can resolve OpenCode and Cline shims without evaluating shell text. Discovery does not run authentication or model calls. Buzz Desktop is a desktop footprint, not a Goose CLI or a supported orchestrator launcher; a generic `agent.exe` does not establish a Cursor installation. Missing optional harnesses appear in a compact scanned-name summary.

Interactive launch adapters preserve the caller's directory and configured provider/model: OpenCode uses `--prompt`, Aider loads the policy with `--read` and waits for input, Goose uses `run --instructions ... --interactive`, Cline uses `-i`, OpenHands uses `-f`, Pi attaches the policy with `-- @file`, and Droid uses `--append-system-prompt-file`. These adapters load instructions; onboarding establishes actual tools, delegation, lifecycle, and permissions. No adapter grants new subscription access or background capabilities.

Optional profile extensions `HostProfile.active_binding_id` and `ModelBinding.billing` record the configured model/account and verified billing connector without discarding existing native profiles. Multi-provider routes require confirmed subscription billing before using modeled subscription capacity. Explicit API or unresolved billing is not subscription funding for any host kind. A multi-provider host can become the default only through an approved active binding matching the recommended model, provider, owned account, and plan; compatible native hosts remain the ordinary default. Benchmark harness evidence stays attached to its measured harness, so moving a model to another harness requires explicit onboarding acceptance of that evidence as a proxy.

`--export-orchestrator=<path>` and the compatible `--export-policy=<path>` use the same renderer with common settings/cache options and refuse overwrite. Native launcher definitions reference the whole canonical document rather than a divergent copy. The Markdown remains usable without a running application, and a moved machine must confirm rebased profile/ledger paths. User and actual repository verification instructions take precedence over the artifact.

Plan prices and access rules use native documentation: [Codex shared usage](https://learn.chatgpt.com/docs/pricing), [Claude Fable plan access](https://support.claude.com/en/articles/15424964-claude-fable-models-on-your-plan), [Muse subscriptions](https://ai.developer.meta.com/docs/muse-code/subscriptions.md), [Google Antigravity plans](https://antigravity.google/docs/plans.md), and [Grok subscriptions](https://docs.x.ai/grok/faq). Allowances are declared API-equivalent modeling inputs, not provider-published quotas. The lower monthly allowance estimate converts to a weekly allowance by multiplying by `12 / 52`; a selected per-vendor weekly override replaces it. Rolling session and daily windows can prevent executing a weekly allocation at a desired time; a weekly total is not a scheduling guarantee. Unsupported native configurations and unavailable measurements receive no invented allocation. The application shows each selected plan's source and allowance basis in schedule details. Prices without a verified published amount are labeled estimated; any total containing such a price is estimated. Muse's Everyday, High usage 3x, and Power usage 10x tiers use modeled monthly prices of $5, $15, and $50 because the cited native plan documentation does not publish dollar prices. SuperGrok Heavy uses an estimated $300 monthly price and a modeled $300 monthly API-equivalent allowance. External rescue cost is separate from subscription price and provider allowance.

Native eligibility requires the provider and its supported harness: OpenAI with Codex, Anthropic with Claude Code, Muse with Muse Code, Google Gemini models with Gemini CLI or Antigravity SDK, and Grok Build with xAI or Composer 2.5. Codex Spark has a separate unmodeled quota and is excluded. Claude Pro excludes Fable; Max permits it within the account's shared 50% Fable sublimit. Google's third-party model pool is separate and unmodeled, so those models are excluded. Rows with `(none)` effort are excluded.

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

Net Research has a separate class policy and never enters the retry-throughput calculation. Let `A` be Omniscience accuracy, `R = 1 - H` where `H` is the published Omniscience hallucination rate, `L` be AA-LCR, and `E` be HLE. The declared class utilities are:

```text
Focused  = .50 A + .30 R + .20 L
Standard = .30 A + .30 R + .40 L
Complex  = .20 A + .20 R + .40 L + .20 E
Extensive = .15 A + .25 R + .40 L + .20 E
```

These weights are task-policy defaults rather than fitted research-success probabilities. Every required component must be a finite published value on the zero-to-one scale; missing values are not zero-filled or renormalized. A configured Net Research competence minimum applies to the resulting class utility. GPQA remains a scientific-reasoning diagnostic and GDP.pdf a document-work diagnostic; neither raises the generic research utility.

The primary is the highest-utility complete configuration among selected subscription providers, with the same weighted benchmark mixture's direct canonical token-price and decode estimates breaking exact utility ties when available. Conditional alternatives retain per-account and native-harness quality/resource tradeoffs. Unknown resource evidence remains unknown and does not exclude a complete quality score. API benchmark estimates do not reserve subscription quota or authorize a native visit.

The For-you portfolio applies all four class policies to the user's selected subscriptions. A class with no jobs in the admitted prefix retains an individually feasible evidence-ranked route as an unreserved recommendation: planned jobs and weekly usage remain zero, and an authorized task still requires a complete joint replan. The Absolute view computes a separate research-only comparison for API, $200, $100, and $20 tiers using the Standard weights. API includes every model provider with complete AA evidence as an analytical comparison and asserts no native binding. Paid tiers require a known affordable plan and its model restrictions. The configured Net Research minimum applies to both views. Exact score ties prefer known lower same-mixture API proxy cost and decode time; missing resource data is not imputed. Per-provider score/resource frontiers remain conditional alternatives.

These fixed profiles encode the role judgment. The engine does not fit a preference function from the current candidate pool. A candidate with a missing required capability has no competence value for that role.

Each role also has an optional competence minimum in settings. When set, candidates with unknown competence or a score below the minimum are excluded before automatic selection. In Absolute best and vendor tabs, an unset minimum permits unknown competence. Per plan requires known competence for its quality objective. Competence is displayed and breaks ties after nominal throughput and worst scenario capacity shortfall. The optional competence-minimum choices show the selected policy at each available score threshold; the list is a threshold decision curve rather than a claim that every displayed row is strictly Pareto-efficient.

The engine keeps the fixed weights because it has no preference observations from which to fit a robust ordinal regression family. Robust ordinal regression describes how a compatible preference family could be added when such observations exist: [Greco, Mousseau, and Słowiński (2008)](https://www.lamsade.dauphine.fr/mcda/biblio/PDF/GMS-EJOR2008.pdf). Treating competence as a constraint on an economic objective implements the epsilon-constraint method: [Mavrotas (2009)](https://www.sciencedirect.com/science/article/pii/S0096300309002574).

## Reference workloads

Operating capacity is measured in reference tasks completed autonomously by the agent per week. Implementer and Debugger use the coding reference workload:

```text
DeepSWE        113 / 202
Terminal-Bench  89 / 202
```

Reviewer, Sanity, and Comprehension use Repository Q&A as their reference workload. Absolute-best Orchestrator rankings also use their existing GPQA/intelligence competence and Repository Q&A capacity calculation. Per-plan conductor selection instead uses normalized Intelligence Index utility with the common first-party Intelligence Index benchmark-task cost and canonical decode-time proxy. These reference quantities are economic output units supported by observed data, not estimates of completed real-world role jobs.

The reference workload supplies first-attempt pass rate, tokens, vendor usage, and time. Direct resource time is token-proportional and anchored to pooled observed suite wall time. A second resource-time basis assigns the pooled suite time to each task. Both bases are evaluated as declared operating scenarios. Aggregated competence utility never supplies retry probabilities; reference-benchmark outcomes do, with the DeepSWE difficulty adjustment below.

## Derived retry correlation and task difficulty

The embedded `assets/retry-correlation.json` is generated by `scripts/fit-retry.ps1` from two independent multi-attempt sources. [DeepSWE v1.1](https://deepswe.datacurve.ai/artifacts/v1.1/leaderboard-live.json) supplies mini-swe-agent configurations and [scored rollout trials](https://deepswe.datacurve.ai/artifacts/v1.1/trials.json), with [task identities](https://deepswe.datacurve.ai/artifacts/v1.1/tasks.json). [Terminal-Bench 2.1 submissions](https://github.com/harbor-framework/terminal-bench-2-1/tree/main/leaderboard/submissions) supply harness-specific pass-at-k curves. The asset records the DeepSWE generation timestamp and Terminal-Bench commit. Downloads remain in a system temporary directory; the application reads only the embedded table and does not fetch these sources at runtime. The sources are never averaged together.

DeepSWE uses only `included_in_score=true` trials. For configuration `c` (model and reasoning effort), task successes and attempts give `q_t = s_t/n_t`. With `T` observed tasks, `pbar = mean(q_t)`, sample variance `S2`, `A = mean(1/n_t)`, and `D = pbar(1-pbar)`, the raw beta-binomial method-of-moments estimate is:

```text
tau2 = (S2 - A D) / (1 - A)
rho  = clamp(tau2 / D, 0, below 1)
```

The model pool combines efforts within configuration: `sum((T_c-1) tau2_c) / sum((T_c-1) D_c)`. The source pool uses the same expression across all configurations. Pooling uses the untruncated variance estimates, then clamps the resulting rho. It does not merge the attempts of different efforts into a single task success rate.

Terminal-Bench fixes each submission's first-attempt probability at `accuracy/100` and fits rho by unweighted least squares against pass-at-1 through pass-at-5 using the retry equation below. Pass-at-1 is that fixed probability; the other four observations are the published fractions. Each harness pool jointly fits one rho across its submissions while retaining each submission's own probability. The source pool jointly fits all submissions. These are fitted retry assumptions rather than direct estimates of uncertainty.

DeepSWE resolves rho from exact model and effort, then the model pool, then the DeepSWE source pool. The Exact layer uses the configuration's inverse-variance-shrunk rho, with variances estimated by bootstrap over tasks and the model pool as prior. Terminal-Bench resolves it from exact harness display label, model display label, and effort, then the harness pool, then the Terminal-Bench source pool. Repository Q&A uses the DeepSWE source-pool rho for every model and effort. This is a transferred assumption; no Q&A retry table is loaded. When multiple Terminal-Bench submissions share an exact display key, the newest submission date supplies the exact value; all submissions remain in the pooled fits. Every value records its supplying layer. The explicit identity normalizer strips Fable's `(with fallback)` qualifier and uses exact names and efforts; dated model variants and `(none)` efforts do not receive an exact match. Missing effort is not inferred from a model's available configurations.

For DeepSWE difficulty, each task's `d_t` is the equal-configuration mean of `q_t`. Weights proportional to `1-d_t` are normalized over the configuration's observed tasks. Its weighted pass rate `W = sum(w_t q_t)` and unweighted rate `U = mean(q_t)` give `ratio = W/U`. The ratio is inverse-variance-shrunk toward the model pool using variances estimated by bootstrap over tasks. The stored multiplier is `ratio_shrunk / mean(ratio_shrunk over configurations)`. Its mean over source configurations is one: the shared downward shift is removed and only relative spread remains. This preserves the mean multiplier, not necessarily the mean adjusted pass rate across a different AA row population. Exact configuration matches multiply their DeepSWE pass rate by this value, bounded to zero through one; all other rows use one. This adjustment affects DeepSWE competence inputs and retry pass rates only. It leaves Terminal-Bench and Repository Q&A pass rates unchanged.

Retry correlation settings are blank by default, meaning derived values. A numeric override replaces every suite's nominal rho and records the Override layer. The existing low/high scenario multipliers apply independently to each resolved or overridden rho, bounded below one. The interface shows nominal rho, supplying layer, and the DeepSWE difficulty multiplier.

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

## Absolute best nominal throughput selection

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

Absolute-best seat eligibility requires the reference-workload rewards, tokens, and time consumed by its capacity calculation: DeepSWE and Terminal-Bench for Implementer and Debugger, and Repository Q&A for Reviewer, Orchestrator, Sanity, and Comprehension. Per-plan workers follow their corresponding role evidence. A per-plan conductor requires a normalized Intelligence Index, positive first-party Intelligence Index task cost and canonical decode-time data, a selected subscription plan, and a known native harness mapping for the same provider and base model family. The selected exact effort and native entitlement are verified during onboarding. Missing evidence is not imputed. A zero or missing coding pass excludes Implementer and Debugger without excluding non-coding worker seats or an otherwise qualified per-plan conductor.

Rows with `(none)` effort are excluded. Claude Code configurations for GLM-5.1, GLM-5.2, and Qwen3.8 Max are excluded from subscription plans. The built-in allowance dollar values in `engine.rs` are declared modeling inputs, not vendor-published figures.

## Absolute best plan and counterfactual comparisons

Plan comparisons use only the selected subscription policy at each tier. Every pair reports the actual configured monthly prices, raw competence values, and autonomous production over the same 66 scenarios. A cheaper plan is called dominant only when its price is known and it is no worse in competence or any scenario. An override without a known monthly subscription price remains price unknown; it is never treated as free or cheaper.

The counterfactual report changes one input for one chosen model configuration at a time while every competitor remains unchanged. It evaluates 24 bounded, logarithmically spaced changes to agent-attempt runtime, vendor-usage cost, and configuration-specific allowance, recomputing retry caps, nominal throughput, and the full 66-scenario band and tie-break each time. When a sampled point wins, it refines the first sampled losing-to-winning bracket and returns a verified sufficient winning change. This is not proof of the globally smallest winning change because candidate, retry-cap, and scenario tie-break changes can make winning regions non-monotonic. If no sampled point wins, the report says no winning change was found at the sampled reductions or allowances; it does not claim that winning is impossible. These results are not measurements, causal estimates, or predictions that changing a shared vendor plan would leave competitors unchanged.

## Inputs

DeepSWE, Terminal-Bench v2.1, and SWE-Atlas Repository Q&A inputs are per-evaluation means. The DeepSWE pass rate used for competence and retries receives the difficulty multiplier below; Terminal-Bench and Repository Q&A pass rates remain raw. Missing or invalid reference-workload outcomes exclude the candidate from the seats that require them. Direct cost partitions gross input tokens into uncached, cache-read, and cache-write categories, prices them separately, and anchors the weighted suite to pooled observed cost. When applicable model prices are absent, cost uses the pooled observed value. A zero-output suite uses pooled observed time. The interface labels the resource basis.

Matched GPQA, HLE, LCR, Omniscience breakdown values, GDP.pdf, and intelligence-index values retain their Artificial Analysis snapshot provenance. Artificial Analysis normalization divides canonical totals by unique task counts where task-level display values are needed: AA-LCR 100, Omniscience 6,000, HLE 2,158, and GPQA 198. Net Research uses raw Omniscience accuracy and `1 - hallucination rate` as distinct policy components; the published hallucination denominator is wrong answers plus partial answers plus abstentions, while accuracy uses all questions. Omniscience's composite score is not used. Missing values remain Unknown. Other roles keep hallucination informational and do not use it for competence, retries, or capacity.
