//! Command presentation policy, separate from the registry and lifecycle data.
use super::CommandKind;

pub(super) fn command_effect(command_id: &str, kind: CommandKind) -> &'static str {
    if command_id == "compute.connection.prepare" {
        return "local_candidate_preparation";
    }
    if command_id == "agent.launch" {
        return "managed_local_process";
    }
    match kind {
        CommandKind::Query | CommandKind::Preview | CommandKind::Watch => "none",
        CommandKind::Apply => "high_impact_write",
        CommandKind::Action => "typed_action",
        CommandKind::ExternalProbe => "bounded_probe",
    }
}
pub(super) fn command_network(command_id: &str, kind: CommandKind) -> &'static str {
    if command_id == "compute.connection.login" {
        return "local_control_then_provider_oauth";
    }
    if command_id == "agent.launch" {
        return "local_control_then_numeric_loopback_gateway";
    }
    if matches!(
        command_id,
        "tasks.start" | "tasks.continue" | "worker.exec" | "worker.continue"
    ) {
        return "local_control_then_managed_worker_and_loopback_gateway";
    }
    match kind {
        CommandKind::ExternalProbe => "local_control_then_registered_gateway",
        _ => "local_control_only",
    }
}
pub(super) fn automation(command_id: &str) -> &'static str {
    if matches!(command_id, "sessions.list" | "sessions.show") {
        return SESSION_ANALYSIS_HELP;
    }
    if command_id == "value.show" {
        return VALUE_ANALYSIS_HELP;
    }
    if command_id == "decision.services.apply" {
        return "Read decision services options first; reuse an exact saved connection when suitable. Convenience flags prepare a Preview only. A new connection requires --id, --name, --provider and its required endpoint/model plus --candidate, or explicit --no-auth for custom only. --candidate refers to an existing protected registration; never pass a credential in flags/JSON/chat. The user can run protected-input register --candidate REF --prompt --format bearer|raw in their own terminal. System One uses Authorization and bearer; custom supports --header NAME or --no-auth. Updates by --id preserve omitted fields; changing endpoint/provider/header requires explicit new credentials or custom --no-auth. --timeout-ms defaults to 10000 for a new connection. --apply-file creates a new private exact Apply request, never overwrites or applies; submit it with the same decision services apply verb. Keep its Operation lookup and identical request for uncertain delivery. Typed Preview/Apply remain alternatives; --apply-file accepts Preview input only. No save proves connectivity, native loading or decision quality.";
    }
    if command_id == "decision.services.test" {
        return "An explicit potentially billed probe. --service ID@REVISION resolves an exact current listed connection; historical pinned versions remain available through the typed request. Inspect data.outcome and failure_code even when the command itself succeeded. Passing establishes this synthetic connection test, not semantic decision quality, execution model availability or native loading. Never silently switch a selected service to local_rules after failure.";
    }
    if matches!(
        command_id,
        "routing.preview"
            | "compute.connection.preview"
            | "agents.connect.preview"
            | "agents.restore.preview"
    ) {
        return "Convenience flags and JSON input are alternatives. --apply-file creates a new private exact Apply request, never overwrites, and performs no Apply. Review effects, then submit that file with the matching Apply command. After uncertain submission, use submission.operation_lookup with operations find or resend the identical file. Core supported variants are compute save, Plan content v2 and Agent settings v2. Routing recovery: retain a successful private routing show response before changes; use routing preview --plan ID --restore-from BEFORE_JSON --apply-file NEW_FILE, review and routing apply --request-file NEW_FILE. This restores complete effective configuration with the current head revision, exclusively from the same Plan/workspace; do not mix with edit flags or typed request inputs. Compare all desired fields after recovery; revisions advance and native facet restoration is separate. Routing: repeat economy/primary flags to preserve order; select --decision local_rules or --decision-service ID@REVISION explicitly for a new smart Plan. Native choices: Hermes requires --protocol; Codex requires --native-models preserve_available|hiroute_only and --native-default preserve_native|plan; Claude requires explicit --preset opus|sonnet|haiku; Qoder/Pi/DSH require --protocol and add entries without claiming a native default. Missing choices remain unresolved. --facet may repeat to restore selected facets; other facets stay untouched. Desktop resident-service evidence must come from Desktop, never a fabricated CLI declaration.";
    }

    if command_id == "agents.connect.status" {
        return "Use --effective --context ID --plan ID for separate saved, native-loading, selection and exact request evidence. Unknown loading stays unknown. This read-only query never restarts an Agent or calls a model; recorded outcomes apply only to the identified request/session.";
    }
    if command_id == "compute.connection.login" {
        return "Keep the returned login_ref for status, cancel and forget. After uncertain delivery, list/status the existing session before starting again or resubmitting a one-use callback. Never persist the authorization URL or put callback material in argv, environment, logs or ordinary request JSON.";
    }
    if command_id == "compute.connection.prepare" {
        return "Reuse the opaque discovery reference and revision exactly. Prepare privately rereads native facts and returns selectable model refs. Save all currently selectable refs by default or the explicit requested subset through Preview/Apply; never omit selection or treat all as a wildcard.";
    }
    if command_id == "compute.scan" {
        return "No input retains released V1 rows. Request hiroute.compute-scan-request/v2 explicitly for connection points, with the selected native_target for Hermes. No scan result is a saved source or account entitlement.";
    }
    if command_id == "worker.executors" {
        return "Ready permits a managed launch attempt, not a promise of execution or authorization. Optional capabilities remain unknown until the actual ACP handshake; start, load and cancel report real outcomes.";
    }
    if command_id == "worker.dependencies.discover" {
        return "Automation must use --output json. For the chosen Harness, select only found candidates and copy that Harness's selection_revisions revision into expected_selection_revision; discovery never installs or selects anything.";
    }
    if command_id == "worker.dependencies.select" {
        return "Preserve the submitted JSON document. If delivery may have succeeded but no response was received, replay that same document unchanged; its deterministic request identity returns the existing Operation instead of applying twice. Do not substitute paths or a newer revision until the server reports a conflict and discovery is run again.";
    }
    if matches!(
        command_id,
        "tasks.start" | "tasks.continue" | "worker.exec" | "worker.continue"
    ) {
        return "The command durably admits a bounded Worker run; that Worker may make model requests only through its exact run-bound loopback Gateway route.";
    }
    if command_id.starts_with("worker.") {
        "Use stable submission keys and task/run locators. A bounded wait timing out is pending, never cancellation; query status or wait again, and read result only when available."
    } else if command_id == "agent.launch" {
        "Treat the exact -- delimiter as the child argv boundary. Before-spawn failures are typed machine envelopes; after spawn, consume the child streams and exit status directly."
    } else {
        "Use stable command_id, status, typed data, warnings[].code, and next_actions[].command_id; never parse display text."
    }
}
pub(super) fn output(command_id: &str) -> &'static str {
    if command_id == "worker.executors" {
        "Default text renders overall and scoped states for all six supported Harnesses. --output json emits the unchanged hiroute.machine-envelope/v2 object; --output quiet prints their stable wire names."
    } else if matches!(
        command_id,
        "worker.dependencies.discover" | "worker.dependencies.select"
    ) {
        "Default text renders selected tuples and discovered candidates but omits the CAS revisions, so automation must use --output json. --output quiet prints only selected Harness wire names (codex_cli, claude_code, qoder_cli, pi, deepseek_harness, or opencode), one per line, and is empty when nothing is selected."
    } else if command_id == "worker.list" {
        "Default text renders the instance task/state/title-or-ID/canonical-cwd page plus its next cursor. --output json emits the unchanged hiroute.machine-envelope/v2 object; --output quiet prints task IDs. A page length is never a total count."
    } else if command_id.starts_with("worker.") {
        "Default text renders the response for people. --output json emits the unchanged hiroute.machine-envelope/v2 object; --output quiet prints only a completed result and sends pending locators to stderr."
    } else if command_id == "agent.launch" {
        "On successful spawn, stdout and stderr are the launched Agent streams; the non-secret launch descriptor is not printed. Failures before spawn emit one hiroute.machine-envelope/v2 JSON object on stdout."
    } else {
        "One hiroute.machine-envelope/v2 JSON object on stdout; diagnostics belong on stderr."
    }
}
pub(super) fn exit_status(command_id: &str) -> &'static str {
    if command_id.starts_with("worker.") {
        "0 accepted/query/pending; final exec/continue failure is nonzero; 2 usage; 3 conflict/busy; 4 denied; 5 not found; 6 unavailable; 7 action required; 8 needs attention; 1 execution failure. A result query may exit 0 while honestly reporting a failed run."
    } else if command_id == "agent.launch" {
        "Returns the launched Agent exit code, or 128 plus its terminating Unix signal. Before spawn: 2 usage; 3 auth/routing conflict; 5 connection not found; 6 unavailable. Launcher lifecycle failures return 6."
    } else {
        "0 success/accepted; 2 usage; 3 conflict/blocker; 4 capability denied; 5 not found; 6 unavailable; 7 action required; 8 needs attention."
    }
}
pub(super) fn effects(command_id: &str, kind: CommandKind) -> &'static str {
    if command_id == "compute.connection.login" {
        return "Manages one independent provider login session and CPA-owned credentials. Login does not enable routing; use the existing subscription check and Preview/Apply save flow after choosing models. Forget targets exactly one local login.";
    }
    if matches!(command_id, "worker.exec" | "worker.continue") {
        return "Durably admits one bounded Worker run after current Plan eligibility, exact Plan, canonical cwd, idempotency, and resource checks.";
    }
    if command_id == "worker.dependencies.select" {
        return "Application-owned journaled operation after exact spec, digest, selection-revision CAS, and idempotency checks on owner-only Local Control.";
    }
    if command_id == "agents.check" {
        return "Configuration, native-authentication, and collaboration are bounded same-UID local checks. Live creates bounded connectivity-probe runtime state and may update real credential cooldown after authentic 401/429 responses; probe traffic is excluded from Session and Value.";
    }
    if command_id == "decision.services.test" {
        return "Sends one fixed synthetic routing decision to the explicitly configured service; may consume provider quota. It reads no real session history and does not publish or edit a route.";
    }
    if command_id == "compute.connection.test" {
        return "Runs one explicit bounded source check. It keeps protected input in daemon memory and does not persist source state until the separate Preview/Apply flow.";
    }
    if matches!(
        command_id,
        "compute.connection.apply"
            | "routing.apply"
            | "decision.services.apply"
            | "agents.connect.apply"
            | "agents.restore.apply"
    ) {
        return "Same-UID Application-owned journaled operation after exact spec, digest, revision, and idempotency checks.";
    }
    if command_id == "compute.connection.prepare" {
        return "Registers a private ephemeral candidate and protected credential reference after exact source reread. Does not save a source, mutate native files or call a model.";
    }
    if command_id == "agent.launch" {
        return "Creates one owner-only temporary launch overlay, directly spawns the exact managed Agent executable without a shell, and removes the overlay after child exit; no persistent user configuration is changed.";
    }
    match kind {
        CommandKind::Query | CommandKind::Watch => "Read-only; no product side effect.",
        CommandKind::Preview => {
            "Stateless Preview: no ID, TTL, Operation, Secret, file, database, publication, or network write."
        }
        CommandKind::Apply => {
            "Application-owned journaled operation after exact spec, digest, revision, capability, and idempotency checks."
        }
        CommandKind::Action => {
            "Application-owned typed operation; side effects are recorded by the Operation and external ledgers."
        }
        CommandKind::ExternalProbe => {
            "One explicit bounded probe through a registered connector; no inference request or fallback probe."
        }
    }
}
pub(super) fn network_and_model_use(command_id: &str, kind: CommandKind) -> &'static str {
    if command_id == "compute.connection.login" {
        return "May open the provider authorization flow and exchange or refresh independent OAuth credentials through CPA. No model inference is performed and native-client refresh tokens are never imported.";
    }
    if matches!(command_id, "worker.exec" | "worker.continue") {
        return "The selected Worker may send model traffic only through its run-bound numeric-loopback Gateway route; native tools follow the run-scoped permission policy.";
    }
    if command_id == "agents.check" {
        return "Local checks make no provider call and require no second authorization channel. Live quick sends one fixed inference request, tool sends a fixed inference request with a built-in no-side-effect tool, and conformance may send a bounded protocol matrix; every live suite may consume real provider quota and requires explicit consent plus a protected probe grant.";
    }
    if command_id == "decision.services.test" {
        return "Sends one fixed synthetic routing decision to the explicitly configured service; may consume provider quota. It reads no real session history and does not publish or edit a route.";
    }
    if command_id == "compute.connection.test" {
        return "Contacts only the explicitly selected, typed endpoint. Inventory checks make no inference call; selecting an inference model may send one bounded model request and consume provider quota.";
    }
    if command_id == "agent.launch" {
        return "Local Control supplies a non-secret descriptor. The launched Claude process may send caller-requested model traffic only through the numeric-loopback HiRoute Gateway.";
    }
    match kind {
        CommandKind::Preview => "No model use and no non-fixture network access.",
        CommandKind::ExternalProbe => {
            "Uses only the selected registered endpoint; never sends a model inference request."
        }
        CommandKind::Query | CommandKind::Watch => {
            "No model use; any external dependency is declared by the typed Application operation."
        }
        CommandKind::Apply | CommandKind::Action => {
            "No model inference; external access, when required, is limited to registered typed ports."
        }
    }
}
pub(super) fn examples(command_id: &str, joined_path: &str) -> Vec<String> {
    if command_id == "compute.connection.test" {
        return vec![
            example(command_id, joined_path),
            "hiroute compute connection test --request-file registered-request.json --output json".into(),
            "hiroute schema show --command-id compute.connection.test --include-schema --output json".into(),
        ];
    }
    if command_id == "decision.services.apply" {
        return vec![
            "hiroute decision services apply --request-file preview-request.json --output json  # Preview: schema_version and spec".into(),
            "hiroute decision services apply --request-file apply-request.json --output json  # Apply: add exact accept_digest, expected_revisions and a stable idempotency_key".into(),
            "hiroute schema show --command-id decision.services.apply --include-schema  # Complete wrappers; substitute current IDs and exact Preview values".into(),
        ];
    }
    vec![example(command_id, joined_path)]
}
pub(super) fn example(command_id: &str, joined_path: &str) -> String {
    match command_id {
        "compute.connection.test" => "hiroute compute connection test --request-stdin --output json < native-request.json".into(),
        "compute.connection.login" => "printf '%s' '{\"action\":\"start\",\"provider\":\"claude\"}' | hiroute compute connection login --request-stdin --output json".to_owned(),
        "worker.executors" => "hiroute worker executors --output json".to_owned(),
        "worker.dependencies.discover" => {
            "hiroute worker dependencies discover --harness codex_cli --output json".to_owned()
        }
        "worker.dependencies.select" => "printf '%s\\n' '{\"harness\":\"codex_cli\",\"adapter_path\":\"/ABSOLUTE/PATH/TO/codex-acp\",\"cli_path\":\"/ABSOLUTE/PATH/TO/codex\",\"node_path\":\"/ABSOLUTE/PATH/TO/node\",\"expected_selection_revision\":0}' | hiroute worker dependencies select --request-stdin --output json".to_owned(),
        "worker.plans" => "hiroute worker plans".to_owned(),
        "worker.list" => "hiroute worker list --limit 50".to_owned(),
        "worker.exec" => {
            "hiroute worker exec --plan worker --cwd /workspace -- 'inspect the repository'".to_owned()
        }
        "worker.status" => {
            "hiroute worker status --run run/one".to_owned()
        }
        "worker.wait" => {
            "hiroute worker wait --run run/one".to_owned()
        }
        "worker.result" => {
            "hiroute worker result --run run/one".to_owned()
        }
        "worker.read" => "hiroute worker read --run run/one".to_owned(),
        "worker.continue" => "hiroute worker continue --task task/one --expected-latest-run run/one -- 'continue the task'".to_owned(),
        "worker.cancel" => {
            "hiroute worker cancel --run run/one".to_owned()
        }
        "schema.show" => {
            "hiroute schema show --command-id schema.list --non-interactive --output json"
                .to_owned()
        }
        "agent.launch" => {
            "hiroute agent launch --agent claude-code --context agent-context/claude/example -- --print 'hello'".to_owned()
        }
        "compute.connection.options" => {
            "hiroute compute connection options --non-interactive --output json".to_owned()
        }
        "compute.list" => "hiroute compute list --non-interactive --output json".to_owned(),
        "compute.show" => {
            "hiroute compute show source/example --non-interactive --output json".to_owned()
        }
        "decision.services.list" => "hiroute decision services list --output json".to_owned(),
        "decision.services.apply" | "decision.services.test" => format!("hiroute {joined_path} --request-stdin --output json < request.json"),
        "routing.options" => "printf '%s' '{}' | hiroute routing options --request-stdin --output json".into(),
        "routing.preview" | "agents.connect.preview" | "agents.restore.preview" => format!("hiroute {joined_path} --request-file preview-request.json --output json"),
        "routing.apply" | "compute.connection.apply" | "agents.connect.apply" | "agents.restore.apply" => format!("hiroute {joined_path} --request-file apply-request.json --output json"),
        "routing.list" => "hiroute routing list --non-interactive --output json".to_owned(),
        "routing.show" => {
            "hiroute routing show plan/example --non-interactive --output json".to_owned()
        }
        "agents.scan" => "hiroute agents scan --non-interactive --output json".to_owned(),
        "sessions.list" => "hiroute sessions list --non-interactive --output json".to_owned(),
        "sessions.show" => {
            "hiroute sessions show session/example --non-interactive --output json".to_owned()
        }
        "sessions.receipt" => {
            "hiroute sessions receipt receipt/example --non-interactive --output json".to_owned()
        }
        "sessions.status" => {
            "hiroute sessions status --non-interactive --output json".to_owned()
        }
        _ => format!("hiroute {joined_path} --non-interactive --output json"),
    }
}

const SESSION_ANALYSIS_HELP: &str = r#"Same-OS-user Local Control reads retained message/tool content and searches without a separate GUI ticket. Supplied capability FDs remain validated; Gateway keys grant no management authority.

Analysis journey (opt in; legacy views/cursors keep their meanings):
  hiroute sessions list --view conversations --period 7d --limit 50
  hiroute sessions list --view switches --day 2026-10-16
  hiroute sessions show SESSION --view routing --analysis-scope TOKEN
  hiroute sessions show SESSION --view conversation --request REQUEST --analysis-scope TOKEN

Use --period today|yesterday|7d|30d, --day YYYY-MM-DD, or --from-ms N --to-ms N; choose one mode. Civil days follow this service host's local day (actual offsets are returned); absolute bounds are UTC milliseconds, half-open. Default is 7d. Analysis scopes return requested/effective bounds, expiry and relation/numeric/price watermarks. Retention may clip their start so used evidence stays retained for the token lifetime. No stable interval and missing native evidence are explicit gaps, not empty-success claims.

Retain analysis_scope for ranking/switch/content drill-down; follow next_cursor with the same view/filters/limit and no new time selection. Follow-ups can narrow --session/--request, never widen the original Plan/Agent/query scope. New requests or unrelated lineage do not change a pinned range. Expired scope/cursor, withdrawal/full deletion or changed authority require a fresh initial query; do not silently compare totals across refreshed scopes. A page can be shorter than limit; its length is not the total.

--query searches retained content in the conversations list. Set it when creating a search scope; an existing scope rejects a new or different keyword. Start a separate search scope and retain the original comparison scope. Repeating its original keyword is allowed. Pending/unavailable index is marked partial. --model-switch is invalid for new views; its legacy any|only|exclude behavior is unchanged. Recorded selection/competence stages explain routing; stage boundaries and credential relay are not user-turn model changes. Compare actual accepted endpoints and explicit reasoning settings. continuation_boundary marks a verified native compression boundary, including mid-turn compression; turn_context_reset/unknown/pending never establish adjacent user turns. Missing proof leaves physical sessions separate. Member previews include exact counts and partial markers. Request content is bounded; reference_only/partial entries supply content_id/request_id for typed V2 content pagination, never expose provider private state. Use sessions receipt RECEIPT for immutable routing facts and observation plan-quality samples for stage details. Missing/deleted/expired capture does not prove wrong routing.

Legacy sessions list --include-unlinked --query TEXT and sessions show SESSION --content messages-and-tools remain available. Typed V2 accepts sessions/search/session_search, timeline/catalog/ancestry/content, and family-specific analysis requests. Read schema show --command-id sessions.show --include-schema for complete contracts. Observation errors opt in to timeout, reader_busy, index_unavailable or storage_unavailable diagnostics; retry the same scope where still valid. No read performs a classifier call or changes routing."#;

const VALUE_ANALYSIS_HELP: &str = r#"Ordinary value show keeps frozen historical valuation. New analysis paths:
  hiroute value show --view summary --period 7d
  hiroute value show --group-by session --order-by cost-desc --period 7d
  hiroute value show --group-by request --order-by cost-desc --analysis-scope TOKEN --session SESSION

Summary defaults to current-price API-equivalent estimates and an unambiguous selected Plan's primary binding. Ambiguous reference/currency or missing prices returns choices; resolve actual binding IDs from routing options, never invent rates or exchange rates. Ranking needs no reference. Choose --basis recorded|api-equivalent; --currency USD|CNY and --valuation-kind usage-estimate|api-equivalent select one recorded currency/kind. Do not add different kinds or mix recorded estimates with current-price recalculation. Same time selectors, retention-safe bounds, analysis_scope, next_cursor and expiry rules as sessions --view apply; drill-downs inherit the pinned relation/numeric/price generations. Live traffic/new settlement/current price edits do not silently change them.

The complete selected scope is aggregated before cost-desc pagination. Failed/retried attempts are counted once and rounded once per attempt, using exact source/model identity matching. Missing amounts are unknown, never zero/free. An incomplete ranking is highest-known-only. Savings shows signed paired subtotals, compared/missing/excluded coverage and decision overhead as unmeasured; a zero reference denominator has no percentage. Subscription/Coding Plan API-equivalent savings proves neither cash savings nor equal output/quality. Recorded receipts remain unchanged.

For missing rates read prices effective, use its exact Binding locator/edit_context with prices override preview/apply, and valuation_kind=api_equivalent. Keep complete recovery material and exact retry requests. Scope/P capture capacity or reader contention is retryable within the same 1 s query budget; pending reconciliation reports target/progress and stalled state, never an empty total. Expired/deleted/withdrawn evidence requires an explicitly refreshed initial scope. Use sessions --help for related content/routing drill-downs.

Legacy one-shot --basis api-equivalent --reference-binding BINDING --currency CNY with --routing PLAN or --session SESSION and an interval/period retains its original shape/time meaning and paired-coverage rules; it has no day grouping. New --view summary does not accept --group-by/--order-by. Read schema show --command-id value.show --include-schema for typed alternatives."#;
