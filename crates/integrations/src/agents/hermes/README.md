# Hermes main-model integration

The shared Agent settings transaction owns admission, grants, Operations and
recovery. Hermes leaves own native configuration and resources:

- [Target resolution](../hermes_target.rs) binds one explicit canonical effective
  profile directory to its context and accepted Operation. Daemon environment does
  not select the profile.
- [Configuration](../hermes_config.rs) owns the default main model/provider and
  context controls, one compact per-Plan provider and the plugin's enabled settings.
  Native names use `hiroute-main-` plus a short alias digest; full profile/context
  identity stays in the existing grant/effect/restore bindings. Legacy long names
  remain readable/restorable and an explicit owned reconfiguration can adopt the
  compact form. Foreign destinations or remaining references block replacement.
  Conditional restore preserves unrelated configuration and refuses changed owned
  fields. Native source providers, credentials, channels and history remain native.
- [Plugin effects](../hermes_plugin_effects.rs) install the bundled dependency-free
  module/manifest in the same native tail. Identical borrowed files retain their
  ownership; foreign content is a conflict. Installed files do not prove loading
  in an already-running Hermes process.
- [Support inspection](../hermes_support.rs) reads the selected launcher's native
  plugin/config/context interfaces and protocol SDK. Responses requires OpenAI
  Responses with extra headers; Chat Completions independently requires OpenAI
  Chat Completions with extra headers; Messages requires Anthropic Messages with extra
  headers. Optional reasoning replay does not gate admission. Hermes v0.21.7 or
  newer is recommended because 0.19.1 omits reasoning content on the tested local
  Chat tool continuation; compatible old installations still connect.

The upstream replay fix first ships in stable v0.21.7
(`336b11584ce30ba19610d93c33e460c31356d030`, NousResearch/hermes-agent#136358).
Source distributions may report `0.0.0`; inspect actual plugin/configuration and
SDK interfaces, not a numeric minimum. Preserve the existing dependency digest
inputs for accepted-operation continuity, without requiring replay source markers.
PM-managed installations are inspected through their literal launcher and its
owning tool store's committed environment record. Only the selected dependency
generation supplies SDK facts; missing, malformed or out-of-install selections
remain unavailable. Inspection does not execute the launcher, import native code
or search older environments for a passing SDK.
HiRoute does not upgrade Hermes or alter native reasoning. Scan, management
preparation and owned restore retain their action-specific capabilities.
Installed-source inspection reuses the shared executable resolver (including
normal user/system bin locations). When no command is found, it can inspect the
bundled installation's `~/.hermes-bundle/venv/bin/hermes` console script. A selected
or broken primary command is never replaced merely to find passing capabilities.
The launcher is not executed; source and SDK facts remain bound to its literal
interpreter/package paths.
- [Passive sources](../hermes_sources.rs) read the selected profile's static
  providers/default and literal profile environment. Helpers, rotating custom
  credential pools and unresolved composition remain unavailable. Unrelated native
  pools do not replace a selected custom provider's key. Managed HiRoute providers
  cannot become recursive upstream sources.
  The bare `model.provider: custom` form also supports an explicit Chat protocol,
  `model.base_url`, and a static inline key or profile `key_env`. Its native inline
  key precedence is preserved. URL-dependent protocol fallbacks and a profile
  `CUSTOM_BASE_URL` override remain unresolved rather than guessing a connection.

The installed management Skill is independent of the model connection. Prepare it
with `management_skill: configure` and `model: keep`; restore either facet without
restoring the other. Collaboration is a separate profile-local `hiroute-collaboration`
Skill using the existing explicit/default trigger modes and public Worker CLI.
Both Skills can be prepared in one settings request or either order, with independent
restore references and shared/borrowed ownership. Skill-only operations do not
inspect unused model configuration, require a Plan/Worker or prepare the model
plugin. Native Skill and shell interfaces are checked independently of middleware
and protocol SDKs. Hermes initiates tasks but is not a Worker executor.

Select `chat_completions`, `responses` or `messages` explicitly for the managed main model. Native SDK support and upstream request expressibility are separate checks. In particular, native Responses encrypted reasoning/summary controls cannot be dropped when the upstream only accepts Chat Completions; use native Chat for a qualified Chat source. Chat uses the existing OpenAI SDK and does not require the optional Anthropic SDK. Existing saved transports are not changed automatically.

Normal connection Apply prepares the provider and adapter together. Reload native
plugins or start a new native process when required. A channel/session `/model`
choice can retain a different model; use native selection controls in that channel
to adopt the managed default. Static channel selections are reported without
exposing channel IDs; unobserved live session selection remains unknown. Auxiliary
model slots stay under native control.

The managed provider's API key contains the same scoped local model grant as its
`X-HiRoute-Token` header. Hermes auxiliary calls bypass request middleware: Chat
uses the standard bearer channel, and Messages can use one `x-api-key` containing
the same issued grant when both higher-priority headers are absent. Gateway still
verifies the current grant, protocol, alias and revocation without inferring input
origin; see the [request-routing contract](../../../../../issue-spec/specs/request-routing/spec.md).
Keep this credential in that provider, never global model headers that can reach
another auxiliary provider. Protected restore records retain their exact prior
provider values, including older placeholder keys.

The native auxiliary API does not currently expose its task name through the
main-request middleware. Without a native session or purpose marker, its request
remains independently authorized and classifiable, with request-scoped observation
and no main-conversation history association. It is not labelled as a known
auxiliary merely because middleware metadata is absent: the same absence can
occur when a main-request adapter fails. This preserves main routing availability
and excludes unassociated summaries from later main-stage evaluation, at the cost
of an extra classification and unavailable auxiliary-to-parent attribution.
Do not install global auxiliary-purpose headers or infer purpose from summary text.

The plugin matches the exact saved provider definition, alias, native address and
transport; it carries the token and the actual native session ID through existing
SDK headers. It preserves unrelated request parameters and headers. Invalid or
missing IDs do not remove authorization. Gateway treats identity as Hermes only
for a digest-bound managed consumer, then scopes it to that connection; equal
content never supplies missing Hermes identity.
Current public whole-turn hooks cannot establish human provenance: Hermes 0.20
feeds native async delegation notifications into the same input queue as real
input. Gateway retains observed classification, actual execution history and
cross-turn evaluation with explicit uncertainty, including for older adapters'
freshness flags. Source uncertainty does not permanently pin requests to Primary.

The current middleware emits optional routing-context v2 with the native session,
whole-turn ID and per-call API request ID when available, plus `observed_batch` input
semantics. It does not register an input callback or emit `X-Hermes-Observed-Input`.
The public callback sees clean input before native memory, persistence, voice and
reminder composition. A unique match can therefore identify an old `continue`
instead of the decorated current event. Retained v1 text hints are diagnostic
compatibility inputs only; they cannot select an earlier User or demote later Users.
All transports retain the complete latest wire User envelope, including every
content part, with explicit source uncertainty. A trailing reminder can reduce
intent precision; the adapter does not reconstruct a clean original prompt.

Optional lineage failure preserves the already prepared purpose/session/turn/request
context and authorization. Older SDKs without `register_hook` still use ordinary
request middleware. API request IDs correlate calls; they do not deduplicate
execution or identify a logical task. Child sessions stay independent, and child
output consumed by a parent is external context rather than parent execution.
Gateway restart discards capture, so older missing model/tool output remains
unavailable. Complete new A followed by B can recover stage evaluation and
protection in the same conversation. Async notifications still lack a reliable
origin-turn relationship; successful routing does not establish accurate scoring.

Optional compression ancestry comes from the selected profile's read-only native
SessionDB interfaces. At most 32 ancestor rows are read; the adapter validates
compression closure and the unique live child, rejecting cycles and ordinary forks.
Hermes 0.20's unbounded lineage enumerator is never invoked. Four profile workers,
64 pending sessions per worker, 128 cached proofs per worker and the bounded request
wait limit the optional work. Missing, locked, replaced or unsupported evidence
leaves a continuation gap; it never blocks the model call. Valid lineage supplies
only candidate continuity: actual-history inheritance still requires the Gateway's
captured exact boundary. Observation preserves each physical session and receipt.

Source import is separate from model connection. The shared V2 scan/prepare flow
groups equal native scope/provider/address/protocol/authentication branches, then
offers native declarations and exact qualified catalog models. CLI guidance and
Desktop initially select all currently selectable models; Apply always contains
an explicit nonempty model-reference array. A refresh never expands saved models.

See [the Agent contract](../../../../../issue-spec/specs/agent-connections/spec.md),
[discovery contract](../../../../../issue-spec/specs/model-discovery/spec.md),
[source tests](../hermes_sources_tests.rs), and the
[public CLI journey](../../../../daemon/tests/support/connection_point_product.py).
