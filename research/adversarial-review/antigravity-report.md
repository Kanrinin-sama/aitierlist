**Provenance**
* **Client/Model/Effort**: Gemini 3.1 Pro (High)
* **Web tools used**: `search_web`
* **Web tools successful**: Yes. Successfully retrieved primary citations for "Dominant Resource Fairness" (Ghodsi et al., 2011), "Bandits with Knapsacks" (Badanidiyuru et al., 2013), and "The Price of Robustness" (Bertsimas and Sim, 2004).

***

**Findings (Ordered by Severity)**

**1. Critical: Fluid Time Approximation in Resource-Calendar Interactions**
* **Audit Area**: Resource-calendar interactions & scheduling (`src/native_scheduler.rs`)
* **Finding**: The scheduler treats remaining active time as a fluid volume by taking the sum of disjoint active window fragments (`horizon_seconds`, [native_scheduler.rs:217-220](file:///C:/projects/aitierlist/src/native_scheduler.rs#L217-L220)). It then checks if the overall plan duration fits within this aggregated total. This fails to verify contiguous interval availability.
* **Counterexample**: A task requires an indivisible 2-hour visit. The active calendar has a 1-hour gap before lunch and a 1-hour gap after lunch. The sum is 2 hours. The fluid time-check passes, but the visit cannot actually be executed without interruption.
* **Actionable Conceptual Correction**: Time slots must be mathematically modeled as contiguous intervals. The scheduler must solve a discrete interval-packing or bin-packing problem where visits are placed into specific gaps that are sufficiently long, rather than collapsing time into a liquid budget.

**2. High: Incomplete Robust Optimization (Bertsimas & Sim Mismatch)**
* **Audit Area**: Robust optimization & feasibility under uncertainty (`METHODOLOGY.md` / `team-allocation.md`)
* **Finding**: The documentation correctly cites Bertsimas and Sim (2004) to justify protecting against estimation errors using budgets of uncertainty. However, the actual planning logic evaluates either 66 deterministic discrete scenarios or sums static, full-cap worst-case multipliers. Checking edge-case scenarios or statically buffering everything is conceptually different from the tractable linear robust counterpart established by Bertsimas and Sim.
* **Counterexample**: A plan delegates 10 tasks, each having a worst-case duration double the nominal. Scenario/fixed-reserve logic provisions for all 10 simultaneously hitting their absolute worst-case. True robust optimization using a budget of uncertainty ($\Gamma$) bounds the number of parameters that can simultaneously deviate, recognizing that it is probabilistically improbable for all 10 to fail at once, and thereby provisions efficiently for only (e.g.) 3 concurrent worst-case deviations while still maintaining feasibility guarantees.
* **Actionable Conceptual Correction**: Implement the true robust counterpart by dualizing the uncertainty set. Introduce a tunable $\Gamma$ budget parameter to limit the degree of conservatism, avoiding both exhaustive discrete scenario enumeration and overly pessimistic full-plan static reserves.

**3. High: Terminal State Blindness in Receding Horizons**
* **Audit Area**: Receding horizons & conditional reserves (`src/native_scheduler.rs` / `team-allocation.md`)
* **Finding**: The receding horizon framework strictly "stops at observed native reset episodes; future capacity is not invented." This formulation acts as a finite-horizon MPC (Model Predictive Control) without a terminal state constraint or visibility into known future replenishments, creating temporal blindness.
* **Counterexample**: A complex integration job takes 3 days to execute across its DAG. The current AI vendor API resets in 2 hours. Because the horizon refuses to acknowledge any capacity past the 2-hour mark, it either rejects the 3-day job entirely for exceeding the visible quota, or admits the first stage without any guarantee that the required capacity for the final stages will be available after the reset.
* **Actionable Conceptual Correction**: The horizon must distinguish between *known* deterministic replenishments (e.g., standard subscription replenishments) and *unknown/invented* capacity. Apply strict hard constraints for the current active window, but apply nominal or chance constraints across known future windows to safely admit long-DAG tasks.

**4. Medium: Reset-Spanning Hold Misattribution**
* **Audit Area**: Reset episodes (`team-allocation.md`)
* **Finding**: The policy states: "A call crossing a reset retains a conservative hold and attribution estimate until the aggregate meter resolves it." Retaining the hold against the *old* window's ledger creates an exposure vulnerability if the vendor ultimately attributes the spanning cost to the *new* window.
* **Counterexample**: A large code generation spans from 11:59 PM to 12:05 AM, crossing a midnight quota reset. The local system holds the cost strictly against the old window's ledger. The vendor, however, deducts the quota from the new window upon completion. The local dispatcher perceives the new window as completely unused, over-admits jobs, and unexpectedly hits a vendor rate limit.
* **Actionable Conceptual Correction**: Holds that span a reset boundary must be conservatively duplicated against *both* adjacent windows (or strictly allocated to the later window, depending on specific vendor billing documentation) until an authoritative meter observation completely resolves the attribution ambiguity.
