//! Command presentation policy, separate from the registry and lifecycle data.
use super::CommandKind;

pub(super) fn command_effect(command_id: &str, kind: CommandKind) -> &'static str {
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
        "Default text renders overall and scoped states for all five supported Harnesses. --output json emits the unchanged hiroute.machine-envelope/v2 object; --output quiet prints their stable wire names."
    } else if matches!(
        command_id,
        "worker.dependencies.discover" | "worker.dependencies.select"
    ) {
        "Default text renders selected tuples and discovered candidates but omits the CAS revisions, so automation must use --output json. --output quiet prints only selected Harness wire names (codex_cli, claude_code, qoder_cli, pi, or deepseek_harness), one per line, and is empty when nothing is selected."
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
pub(super) fn example(command_id: &str, joined_path: &str) -> String {
    match command_id {
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
