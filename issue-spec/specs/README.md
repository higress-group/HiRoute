# HiRoute specifications

These specifications define HiRoute's current product and runtime contracts in
English. They are organized by user capability and durable ownership, rather than
implementation tasks or issue history. Each file states purpose, requirements,
representative scenarios, and local implementation references.

MUST and MUST NOT are required behavior; MAY describes an allowed choice. A source
reference locates the enforcing implementation or an executable scenario. It is not
a statement that an acceptance run passed. Platform, native-client, live-account,
and performance evidence belongs to revision-bound validation records.

## Product capabilities

| User capability | Canonical specification | Scope |
| --- | --- | --- |
| Install, open, configure, and update Desktop | [Getting started](getting-started/spec.md) | Service health, shell interaction, preferences, complete-package upgrade/recovery |
| Find supported local model configuration | [Model discovery](model-discovery/spec.md) | Passive scan, explicit approval, exact native precedence, stale-source checks |
| Connect and maintain model sources | [Model connections](model-connections/spec.md) | Catalog/custom endpoints, source identity, credentials, subscription modes, lifecycle |
| Inspect model facts, scores, and prices | [Model facts and price policy](model-catalog-price/spec.md) | Embedded/signed metadata, withdrawal boundaries, scores, source overrides, frozen valuation |
| Manage several Keys in one connection | [Credential pools](credential-pools/spec.md) | Ordered edits, current leases, isolated cooldown, protected input |
| Author and publish reusable routes | [Routing plans](routing-plans/spec.md) | Identity, four modes, drafts, content/lifecycle, client budgets, installed consumers |
| Keep a declared candidate order | [Fixed candidate order](ordered-routing/spec.md) | Fixed-model mode and permitted ContextHold preference |
| Prefer explicitly free sources | [Free-first routing](free-first-routing/spec.md) | Free qualification, suggestions, explicit primary fallback |
| Try models with Regular/Primary routing | [Difficulty routing](smart-saving-routing/spec.md) | Thresholds, trust levels, evidence gates and per-turn recovery |
| Connect a decision model or define categories | [Decision services](decision-services/spec.md) | Saved revisions, System One/custom v1, category/degree/competence policy |
| Execute a model request with bounded relay | [Request routing](request-routing/spec.md) | Grant/Fixed scope, native controls, expressibility, continuity, pre-body relay |
| Configure native coding Agents reversibly | [Agent connections](agent-connections/spec.md) | Independent model/Skill facets, provider differences, grant/restore ownership |
| Discover authorized models or delegate by purpose | [Agent plan discovery](agent-plan-discovery/spec.md) | Model directory versus instance Worker directory and collaboration guidance |
| Start, inspect, cancel, or continue delegated work | [Worker delegation](agent-delegation/spec.md) | Admission, frozen execution, Harness differences, capacity, progress, exact session |
| Read conversations and execution evidence | [Session history](session-history/spec.md) | Native correlation, live search, content visibility, receipts, quality stages |
| Inspect consumption and value | [Usage and value](usage-and-savings/spec.md) | Actual usage, currencies, cache ratio, API-equivalent estimates, retained aggregates |
| Delete local content or recover supported stores | [Local data](local-data-and-diagnostics/spec.md) | Retention, visibility/reclamation, coherent backups/migration, OS-access boundary |
| Configure and read safe runtime diagnostics | [Diagnostics](runtime-diagnostic-logging/spec.md) | Applied level, closed typed failures, safe correlation, bounded local logs |

## Runtime and shared boundaries

| Specification | Scope |
| --- | --- |
| [Recorded route identity](request-routing-history/spec.md) | Admitted historical route/name independent of current defaults and sensitive content |
| [Control-plane changes](control-plane-changes/spec.md) | Canonical intent, Preview/admission, idempotency, effect ownership, publication recovery |
| [Shared clients](shared-client-access/spec.md) | Client Core, local transport/trust, strict JSON input, stateless CLI, installation/ABI |
| [Local Worker platform](worker-platform/spec.md) | Explicit launch materials, process ownership, stop scope, native capability exceptions |
| [Gateway Core](gateway-core-foundation/spec.md) | Filter lifecycle, configuration leases, body/SSE ownership, writer gates, transport fences |
| [Gateway execution plane](p0-gateway-execution-plane/spec.md) | Published authority, protocol projection, Replay, response evidence, independent sinks |
| [Product smoke](product-smoke-verification/spec.md) | Selected real entry paths, result classes, evidence limits, isolation and recovery |

## Terminology and authority

- **Model source / connection**: a user-owned supplying context with its own endpoints,
  credentials, selected models, and stable identity. Equal model names do not merge sources.
- **AgentPlan / routing plan**: a reusable named model route with purpose, immutable
  content versions, and an optional delegation configuration. It does not belong to a main Agent.
- **Fixed route**: one exact source/account/model mapping inside a model grant;
  it is distinct from a fixed-model plan's ordered candidate list.
- **Harness / Worker**: the coding Agent chosen to execute explicit delegated work.
  Model requests alone never spawn one. Model access and collaboration are independent facets.
- **Logical request / Attempt**: one authenticated model request and each permitted
  upstream candidate execution within it. Key relay, model relay, and OAuth subrequests
  are distinct from a new request or Worker run.
- **Session / Agent turn / execution stage**: different recorded relations. A turn may
  contain several requests; a stage is an actual contiguous route execution with frozen
  selection/rubric. A request receipt does not prove the whole Agent task completed.
- **Operation**: the durable control-plane transaction and its exact intent, steps,
  effect ownership, and recovery. Client display state is not another Operation authority.

Keep one canonical description per boundary and link it from other capabilities.
Specifications are self-contained; code maps help locate owners, local guides supply
operator detail, and typed producers/OpenAPI define full field schemas. Do not
embed internal issue-server links, release approvals, private account details,
implementation task IDs, candidate logs, or handoff histories in public specifications.

When implementation changes, update affected behavior, failure, privacy, and
compatibility contracts together. Supported released sources retain their declared
meaning through the registered conversion boundary; a new current implementation
or latest release does not silently end support. Planned features belong to planning
artifacts until their production behavior exists. Consolidation must retain distinct
failure conditions even when several old examples become one requirement or table.
