# Allocation refinement review packet

Review the allocation policy as a mathematical and operational decision system at repository revision `eb18a671c6fb9aa217e0a22f799cbe43e3c326e0`. The review is read-only. Do not edit files, run tests, create synthetic calibration jobs, launch subagents, or delegate implementation. Use web search and retrieval for current primary scholarly papers and official technical documentation. Cite direct URLs and distinguish theorem applicability from analogy.

Read these current sources in full:

- `research/team-allocation.md`
- `METHODOLOGY.md`
- `src/engine.rs`
- `src/portfolio.rs`
- `src/solver.rs`
- `src/native_scheduler.rs`
- `src/team_policy.rs`
- `src/route_qualification.rs`
- `src/native_reconciliation.rs`
- `target/debug/audit-current-production.json`
- `target/debug/audit-orch-0.338.json`
- `target/debug/audit-orch-0.500.json`
- `target/debug/audit-orch-0.527.json`

The current saved case uses 112 working hours, four concurrent projects, one subscription account per selected provider, and 112 forecast proxy jobs. The static proxy admits 19 and defers 93. It reports `feasible_search_limit`, utility 67.1672, bound 74.9857, and an 11.64% gap for the admitted workload under its declared scheduler. OpenAI, Anthropic, Google, and xAI reserves exceed 92% of their modeled API-equivalent allowances. Native meters and authorization remain separate.

Audit these twelve findings:

1. The proxy demand `ceil(H*N/4)` scales arrival demand but does not scale subscription capacity; changing concurrent projects from one to four can leave the same 19-job admitted prefix.
2. Lexicographic admission precedes route quality, so a stronger conductor loses whenever it displaces one proxy job.
3. The baseline selects GPT-5.6 Sol low as conductor while many creation and assurance routes use max effort.
4. An Orchestrator floor of 0.50 selects GPT-6 Astra xhigh and admits 17; floor 0.527 selects Astra max and admits 16. The comparison changes whole-team composition, so aggregate utility is not causal evidence of real-world dominance.
5. The portfolio conductor utility uses normalized Intelligence Index while the Absolute view uses GPQA plus Intelligence Index; one common conductor competence definition may be needed.
6. Conductor resource accounting charges two class-weighted Intelligence Index benchmark-task-equivalent visits per admitted proxy job. This is not two full benchmark suites or measured native conductor overhead.
7. The static nine-stage proxy reserves every worker seat plus conditional repair/rechecks for every admitted job. Real task risk, research need, and acceptance criteria can require fewer or different stages.
8. Net Research is selected on demand from declared AA proxy weights and requires runtime capability/native holds, but static demand and budget interaction remain outside the proxy allocation.
9. Higher workflow classes do not enforce monotonic competence floors across every role, and some extensive assignments have lower utility than focused assignments.
10. Review burden is modeled through fixed visit counts, caps, and utility, without calibrated defect exposure, review complexity, or marginal assurance value.
11. The current Coding Agents source exposes 13 configurations. AA Coding Agents v1.5 equally weights DeepSWE v1.1 (113 tasks), Terminal-Bench v4 (66), and SWE-Atlas Q&A (124); the app's 113:89 coding reference mix is its own task-weighted policy, not the AA index. Retry provenance and Terminal-Bench version/resource pairing require precise interpretation.
12. Modeled API-equivalent quotas, entitlement-weighted fairness, per-account lanes, and bounded search limits do not prove native capacity, global fairness, or optimal allocation across all active projects.

Check the current source rather than accepting these findings blindly. Recommend implementable refinements to objectives, constraints, metrics, and reporting. Do not invent coefficients, thresholds, benchmark results, or confidence claims. Prefer a transparent Pareto or sensitivity analysis when preferences are not identified.

The primary AA sources currently used are:

- https://artificialanalysis.ai/methodology/coding-agents-benchmarking
- https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v4-3

AA Coding Agents v1.5 uses three equal-weight components. The app's task-weighted coding reference workload is separate. Agent-row v4 resources come from the agent evaluation means; model-row Terminal-Bench v2.1 scores and resources remain paired. Research weights are policy preferences, not success probabilities.
