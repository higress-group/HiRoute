use hiroute_domain::{
    CanonicalDigest, PRODUCT_CONTRACT_REVISION, PROPOSAL_MAP_REVISION, SchemaVersion,
};
use serde::Serialize;

use crate::protocol::{
    command_arguments, command_idempotency, command_model_call, command_preview_apply_pair,
    command_request_schema, command_response_schema, command_stdin_channels, command_usage,
    staged_coverage,
};

const COMMAND_MANIFEST_SCHEMA: SchemaVersion = SchemaVersion::new(1, 0);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandScope {
    P0,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandLifecycle {
    Released,
    Planned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Query,
    Preview,
    Apply,
    Action,
    Watch,
    ExternalProbe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    Executable,
    Planned,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlannedScenarioV1 {
    pub scenario_id: String,
    pub polarity: String,
    pub state: CoverageState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HelpContractV1 {
    pub purpose: String,
    pub usage: String,
    pub arguments_and_options: String,
    pub effects: String,
    pub network_and_model_use: String,
    pub automation: String,
    pub output: String,
    pub exit_status: String,
    pub examples: Vec<String>,
    pub see_also: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommandDescriptorV1 {
    pub command_id: String,
    pub path: Vec<String>,
    pub operation_id: String,
    pub trace_ids: Vec<String>,
    pub scope: CommandScope,
    pub lifecycle: CommandLifecycle,
    pub kind: CommandKind,
    pub request_schema: String,
    pub response_schema: String,
    pub effect: String,
    pub network: String,
    pub model_call: String,
    pub stdin_channels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_apply_pair: Option<String>,
    pub idempotency: String,
    pub help: HelpContractV1,
    pub positive: PlannedScenarioV1,
    pub negative: PlannedScenarioV1,
}

impl CommandDescriptorV1 {
    pub fn help_document(&self) -> String {
        let examples = self
            .help
            .examples
            .iter()
            .map(|example| format!("  {example}"))
            .collect::<Vec<_>>()
            .join("\n");
        let see_also = self
            .help
            .see_also
            .iter()
            .map(|item| format!("  {item}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Purpose\n  {}\n\nUsage\n  {}\n\nArguments/Options\n  {}\n\nEffects\n  {}\n\nNetwork and model use\n  {}\n\nAutomation\n  {}\n\nOutput\n  {}\n\nExit status\n  {}\n\nExamples\n{}\n\nSee also\n{}\n",
            self.help.purpose,
            self.help.usage,
            self.help.arguments_and_options,
            self.help.effects,
            self.help.network_and_model_use,
            self.help.automation,
            self.help.output,
            self.help.exit_status,
            examples,
            see_also,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommandManifestV1 {
    pub schema_version: SchemaVersion,
    pub product_contract_revision: String,
    pub proposal_map_revision: String,
    pub descriptor_digest: CanonicalDigest,
    pub visibility: String,
    pub commands: Vec<CommandDescriptorV1>,
}

#[derive(Clone, Copy)]
struct StaticCommandDescriptor {
    command_id: &'static str,
    path: &'static [&'static str],
    operation_id: &'static str,
    trace_ids: &'static [&'static str],
    lifecycle: CommandLifecycle,
    kind: CommandKind,
    purpose: &'static str,
}

impl StaticCommandDescriptor {
    fn to_owned(self) -> CommandDescriptorV1 {
        let path = self
            .path
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>();
        let joined_path = self.path.join(" ");
        let lifecycle_state = match self.lifecycle {
            CommandLifecycle::Released => CoverageState::Executable,
            CommandLifecycle::Planned => CoverageState::Planned,
        };
        CommandDescriptorV1 {
            command_id: self.command_id.to_owned(),
            path,
            operation_id: self.operation_id.to_owned(),
            trace_ids: self.trace_ids.iter().map(|id| (*id).to_owned()).collect(),
            scope: CommandScope::P0,
            lifecycle: self.lifecycle,
            kind: self.kind,
            request_schema: command_request_schema(self.command_id).to_owned(),
            response_schema: command_response_schema(self.command_id).to_owned(),
            effect: command_effect(self.command_id, self.kind).to_owned(),
            network: command_network(self.command_id, self.kind).to_owned(),
            model_call: command_model_call(self.command_id).to_owned(),
            stdin_channels: command_stdin_channels(self.command_id)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            preview_apply_pair: command_preview_apply_pair(self.command_id).map(str::to_owned),
            idempotency: command_idempotency(self.command_id).to_owned(),
            help: HelpContractV1 {
                purpose: self.purpose.to_owned(),
                usage: command_usage(self.command_id, &joined_path),
                arguments_and_options: command_arguments(self.command_id),
                effects: effects(self.command_id, self.kind).to_owned(),
                network_and_model_use: network_and_model_use(self.command_id, self.kind).to_owned(),
                automation: automation(self.command_id).to_owned(),
                output: output(self.command_id).to_owned(),
                exit_status: exit_status(self.command_id).to_owned(),
                examples: examples(self.command_id, &joined_path),
                see_also: vec![format!(
                    "hiroute schema show --command-id {}",
                    self.command_id
                )],
            },
            positive: PlannedScenarioV1 {
                scenario_id: format!("p0.{}.positive", self.command_id),
                polarity: "positive".to_owned(),
                state: staged_coverage(self.command_id, true).unwrap_or(lifecycle_state),
            },
            negative: PlannedScenarioV1 {
                scenario_id: format!("p0.{}.negative", self.command_id),
                polarity: "negative".to_owned(),
                state: staged_coverage(self.command_id, false).unwrap_or(lifecycle_state),
            },
        }
    }
}
mod help;
use help::*;

macro_rules! command {
    ($id:literal, [$($path:literal),+], $operation:literal, [$($trace:literal),*], $lifecycle:ident, $kind:ident, $purpose:literal) => {
        StaticCommandDescriptor {
            command_id: $id,
            path: &[$($path),+],
            operation_id: $operation,
            trace_ids: &[$($trace),*],
            lifecycle: CommandLifecycle::$lifecycle,
            kind: CommandKind::$kind,
            purpose: $purpose,
        }
    };
}

// Design #24 plus its managed-Claude and public Worker amendments freeze the P0 leaf commands.
// Only commands with executable positive and negative scenarios may be marked Released and
// projected into manifest.v1.json.
const COMMANDS: &[StaticCommandDescriptor] = &[
    command!(
        "console.open",
        ["console", "open"],
        "CreateConsoleLaunch",
        [],
        Released,
        Action,
        "Create a single-use owner console launch link."
    ),
    command!(
        "console.token.create",
        ["console", "token", "create"],
        "CreateConsoleToken",
        [],
        Released,
        Action,
        "Create a named owner console access token, shown once."
    ),
    command!(
        "console.token.list",
        ["console", "token", "list"],
        "ListConsoleTokens",
        [],
        Released,
        Query,
        "List console token metadata without credential secrets."
    ),
    command!(
        "console.token.revoke",
        ["console", "token", "revoke"],
        "RevokeConsoleToken",
        [],
        Released,
        Action,
        "Revoke a console access token and its sessions."
    ),
    command!(
        "schema.list",
        ["schema", "list"],
        "ListSchemas",
        [],
        Released,
        Query,
        "List the exact released CLI contract without exposing planned or reserved operations."
    ),
    command!(
        "schema.show",
        ["schema", "show"],
        "ShowSchema",
        [],
        Released,
        Query,
        "Show one exact released command descriptor and its machine contract."
    ),
    command!(
        "system.client-status",
        ["system", "client-status"],
        "GetClientServiceStatus",
        [],
        Planned,
        Query,
        "Read actual local service composition and publication availability."
    ),
    command!(
        "operations.find",
        ["operations", "find"],
        "FindOperationByIdempotency",
        [],
        Released,
        Query,
        "Find the original Operation in one exact idempotency scope without submitting a write."
    ),
    command!(
        "system.status",
        ["system", "status"],
        "GetSystemStatus",
        ["CLI-014", "APP-014"],
        Released,
        Query,
        "Read daemon, release, capability, and product readiness status."
    ),
    command!(
        "system.doctor",
        ["system", "doctor"],
        "DiagnoseSystem",
        [],
        Planned,
        Query,
        "Run read-only typed diagnostics without repairing desired state."
    ),
    command!(
        "settings.show",
        ["settings", "show"],
        "GetSettings",
        [],
        Planned,
        Query,
        "Read effective personal workspace settings."
    ),
    command!(
        "settings.preview",
        ["settings", "preview"],
        "PreviewSettingsChange",
        [],
        Planned,
        Preview,
        "Preview an exact settings change and all resulting effects."
    ),
    command!(
        "settings.apply",
        ["settings", "apply"],
        "ApplySettingsChange",
        [],
        Planned,
        Apply,
        "Apply a previously reproduced exact settings change."
    ),
    command!(
        "setup.preview",
        ["setup", "preview"],
        "PreviewSetup",
        ["CLI-001", "APP-001"],
        Planned,
        Preview,
        "Preview the complete headless personal setup through Application."
    ),
    command!(
        "setup.apply",
        ["setup", "apply"],
        "ApplySetup",
        ["CLI-002", "APP-002"],
        Planned,
        Apply,
        "Apply the exact setup spec as a recoverable Operation."
    ),
    command!(
        "setup.status",
        ["setup", "status"],
        "GetSetupStatus",
        ["CLI-003", "APP-003"],
        Planned,
        Query,
        "Read setup convergence and recovery state."
    ),
    command!(
        "agents.scan",
        ["agents", "scan"],
        "ScanAgents",
        [],
        Released,
        Query,
        "Discover exact supported Agent installations and profiles."
    ),
    command!(
        "agents.list",
        ["agents", "list"],
        "ListAgents",
        [],
        Released,
        Query,
        "List discovered Agent installations and connection state."
    ),
    command!(
        "agents.previous-routing.preview",
        ["agents", "previous-routing", "preview"],
        "PreviewAgentPreviousRouting",
        ["CLI-068", "APP-068"],
        Released,
        Query,
        "Recover selected previous mappings as an ordinary Agent settings draft."
    ),
    command!(
        "agents.show",
        ["agents", "show"],
        "GetAgent",
        [],
        Planned,
        Query,
        "Show one exact Agent profile and managed-field ownership."
    ),
    command!(
        "agents.connect.preview",
        ["agents", "connect", "preview"],
        "PreviewAgentConnectionChange",
        ["CLI-054", "APP-054"],
        Released,
        Preview,
        "Preview Agent grant, catalog, Skill, overlay, and owned configuration changes."
    ),
    command!(
        "agents.connect.apply",
        ["agents", "connect", "apply"],
        "ApplyAgentConnectionChange",
        ["CLI-055", "APP-055"],
        Released,
        Apply,
        "Apply an exact AgentConnection change through the recoverable Application saga."
    ),
    command!(
        "agents.connect.status",
        ["agents", "connect", "status"],
        "GetAgentConnectionStatus",
        ["CLI-056", "APP-056"],
        Released,
        Query,
        "Read AgentConnection and managed artifact convergence."
    ),
    command!(
        "agents.restore.preview",
        ["agents", "restore", "preview"],
        "PreviewAgentConnectionRestore",
        ["CLI-009", "APP-009"],
        Released,
        Preview,
        "Preview field-owned Agent configuration restoration without overwriting concurrent edits."
    ),
    command!(
        "agents.restore.apply",
        ["agents", "restore", "apply"],
        "ApplyAgentConnectionRestore",
        ["CLI-019", "APP-019"],
        Released,
        Apply,
        "Restore only still-owned Agent fields from an exact restore point."
    ),
    command!(
        "agents.check",
        ["agents", "check"],
        "CheckAgentConnection",
        [],
        Released,
        ExternalProbe,
        "Drive the registered Agent profile through the formal product entrypoint."
    ),
    command!(
        "agent.launch",
        ["agent", "launch"],
        "GetManagedAgentLaunchDescriptor",
        ["CLI-057", "APP-057"],
        Released,
        Action,
        "Launch the managed Claude Code connection through the published three-slot snapshot."
    ),
    command!(
        "compute.scan",
        ["compute", "scan"],
        "ScanCompute",
        [],
        Released,
        Query,
        "Discover registered subscription, Native API, and free compute sources."
    ),
    command!(
        "compute.list",
        ["compute", "list"],
        "ListCompute",
        [],
        Released,
        Query,
        "List compute sources, bindings, inventory, and redacted readiness."
    ),
    command!(
        "compute.show",
        ["compute", "show"],
        "GetCompute",
        ["CLI-010", "APP-010"],
        Released,
        Query,
        "Show exact source identity, binding, offering, and cooldown state."
    ),
    command!(
        "compute.refresh",
        ["compute", "refresh"],
        "RefreshCompute",
        ["CLI-037", "APP-037"],
        Planned,
        Action,
        "Refresh inventory only through its registered connector contract."
    ),
    command!(
        "compute.connection.prepare",
        ["compute", "connection", "prepare"],
        "PrepareDiscoveredModelConnection",
        [],
        Released,
        Query,
        "Prepare the current opaque native connection-point reference without an inference call."
    ),
    command!(
        "compute.connection.options",
        ["compute", "connection", "options"],
        "ListConnectionOptions",
        ["CLI-048", "APP-048"],
        Released,
        Query,
        "List trusted Release Connector Registry connection options."
    ),
    command!(
        "compute.connection.preview",
        ["compute", "connection", "preview"],
        "PreviewComputeConnectionChange",
        [],
        Released,
        Preview,
        "Preview a connection identity and registered endpoint change without accepting a Secret."
    ),
    command!(
        "compute.connection.apply",
        ["compute", "connection", "apply"],
        "ApplyComputeConnectionChange",
        [],
        Released,
        Apply,
        "Apply an exact registered compute connection change."
    ),
    command!(
        "compute.connection.authorize",
        ["compute", "connection", "authorize"],
        "AuthorizeComputeConnection",
        ["CLI-011", "APP-011"],
        Released,
        Action,
        "Start or resume typed authorization for a registered connection option."
    ),
    command!(
        "compute.connection.login",
        ["compute", "connection", "login"],
        "ManageSubscriptionLogin",
        [],
        Released,
        Action,
        "Manage an independent Codex or Claude subscription sign-in without enabling routing."
    ),
    command!(
        "compute.connection.test",
        ["compute", "connection", "test"],
        "TestComputeConnection",
        [],
        Released,
        ExternalProbe,
        "Run one explicit bounded connection probe, with inference only when requested."
    ),
    command!(
        "compute.connection.test-network",
        ["compute", "connection", "test-network"],
        "TestComputeConnectionV2",
        ["CLI-080", "APP-080"],
        Released,
        ExternalProbe,
        "Check an explicit Native draft or saved source with admitted route evidence."
    ),
    command!(
        "compute.credential.list",
        ["compute", "credential", "list"],
        "ListCredentials",
        ["CLI-049", "APP-049"],
        Planned,
        Query,
        "List only CredentialRef, fingerprint, generation, and redacted runtime state."
    ),
    command!(
        "compute.credential.add",
        ["compute", "credential", "add"],
        "ApplyCredentialAdd",
        ["CLI-050", "CLI-051", "APP-050", "APP-051"],
        Planned,
        Apply,
        "Add a credential from a protected input channel without plaintext argv."
    ),
    command!(
        "compute.credential.replace",
        ["compute", "credential", "replace"],
        "ApplyCredentialReplace",
        ["CLI-050", "CLI-051", "APP-050", "APP-051"],
        Planned,
        Apply,
        "Replace an exact credential generation from protected input."
    ),
    command!(
        "compute.credential.rotate",
        ["compute", "credential", "rotate"],
        "ApplyCredentialRotate",
        ["CLI-050", "CLI-051", "APP-050", "APP-051"],
        Planned,
        Apply,
        "Rotate an exact credential while preserving reference atomicity."
    ),
    command!(
        "compute.credential.remove",
        ["compute", "credential", "remove"],
        "ApplyCredentialRemove",
        ["CLI-050", "CLI-051", "APP-050", "APP-051"],
        Planned,
        Apply,
        "Remove an exact credential only after dependency and revision checks."
    ),
    command!(
        "compute.key-pool.preview",
        ["compute", "key-pool", "preview"],
        "PreviewKeyPoolChange",
        [],
        Planned,
        Preview,
        "Preview ordering and enablement for one homogeneous credential pool."
    ),
    command!(
        "compute.key-pool.apply",
        ["compute", "key-pool", "apply"],
        "ApplyKeyPoolChange",
        [],
        Planned,
        Apply,
        "Apply an exact homogeneous credential-pool change."
    ),
    command!(
        "routing.options",
        ["routing", "options"],
        "GetPlanEditorOptions",
        [],
        Released,
        Query,
        "Read connected candidates and exact-native plan rating suggestions."
    ),
    command!(
        "worker.executors",
        ["worker", "executors"],
        "WorkerExecutorAvailability",
        [],
        Released,
        Query,
        "Read configured Codex, Claude Code, Qoder, Pi, and DeepSeek Harness launch availability without requiring a Plan or Agent allowlist."
    ),
    command!(
        "worker.dependencies.discover",
        ["worker", "dependencies", "discover"],
        "WorkerDependenciesDiscover",
        [],
        Released,
        Query,
        "Discover and validate local Worker CLI, adapter, and Node candidates."
    ),
    command!(
        "worker.dependencies.select",
        ["worker", "dependencies", "select"],
        "SelectWorkerDependencies",
        [],
        Released,
        Apply,
        "Select one validated Worker dependency tuple through same-UID local management."
    ),
    command!(
        "worker.plans",
        ["worker", "plans"],
        "WorkerPlans",
        [],
        Released,
        Query,
        "List every published, delegation-enabled Worker Plan in this daemon instance."
    ),
    command!(
        "worker.list",
        ["worker", "list"],
        "WorkerList",
        [],
        Released,
        Query,
        "Page durable Worker tasks in this daemon instance without filtering accepted history by current Plan policy."
    ),
    command!(
        "worker.exec",
        ["worker", "exec"],
        "WorkerExec",
        [],
        Released,
        Apply,
        "Submit one local Worker task with an explicit cwd and run-scoped permission policy."
    ),
    command!(
        "worker.status",
        ["worker", "status"],
        "WorkerStatus",
        [],
        Released,
        Query,
        "Read one Worker task/run or locate an exact exec/continue submission key."
    ),
    command!(
        "worker.wait",
        ["worker", "wait"],
        "WorkerWait",
        [],
        Released,
        Watch,
        "Wait up to thirty seconds for a Worker run revision without cancelling it."
    ),
    command!(
        "worker.result",
        ["worker", "result"],
        "WorkerResult",
        [],
        Released,
        Query,
        "Read the persisted result and honest completion state for one Worker run."
    ),
    command!(
        "worker.read",
        ["worker", "read"],
        "WorkerRead",
        [],
        Released,
        Query,
        "Read one bounded best-effort public progress window for a Worker run."
    ),
    command!(
        "worker.continue",
        ["worker", "continue"],
        "WorkerContinue",
        [],
        Released,
        Apply,
        "Continue one exact task/session with a run-scoped permission policy."
    ),
    command!(
        "worker.cancel",
        ["worker", "cancel"],
        "WorkerCancel",
        [],
        Released,
        Action,
        "Persist cancellation for one exact Worker run and wake only its owned scope."
    ),
    command!(
        "worker.cleanup.confirm",
        ["worker", "cleanup", "confirm"],
        "WorkerConfirmResidual",
        [],
        Released,
        Action,
        "Confirm user-handled residual resources for one exact terminal run without changing its result."
    ),
    command!(
        "tasks.list",
        ["tasks", "list"],
        "ListDelegations",
        [],
        Planned,
        Query,
        "List visible delegated tasks in durable admission order."
    ),
    command!(
        "tasks.show",
        ["tasks", "show"],
        "GetDelegation",
        [],
        Planned,
        Query,
        "Read one visible task/run or find an original Start/Continue submission key."
    ),
    command!(
        "tasks.start",
        ["tasks", "start"],
        "StartDelegation",
        [],
        Planned,
        Apply,
        "Start one authorized Worker run; retry only with the original idempotency key."
    ),
    command!(
        "tasks.wait",
        ["tasks", "wait"],
        "WaitDelegation",
        [],
        Planned,
        Watch,
        "Wait boundedly for a run revision without cancelling it."
    ),
    command!(
        "tasks.result",
        ["tasks", "result"],
        "ReadDelegationResult",
        [],
        Planned,
        Query,
        "Read an authorized run's state and persisted result; waiting does not cancel."
    ),
    command!(
        "tasks.cancel",
        ["tasks", "cancel"],
        "CancelDelegation",
        [],
        Planned,
        Action,
        "Persist cancellation for one exact run and wake its owned Worker scope."
    ),
    command!(
        "tasks.continue",
        ["tasks", "continue"],
        "ContinueDelegation",
        [],
        Planned,
        Apply,
        "Continue one exact task through its verified native session with a new run token."
    ),
    command!(
        "work-plans.list",
        ["work-plans", "list"],
        "ListWorkPlans",
        [],
        Planned,
        Query,
        "List currently authorized published Worker plans using protected collaboration identity."
    ),
    command!(
        "routing.list",
        ["routing", "list"],
        "ListAgentPlanCatalog",
        ["CLI-053", "APP-053"],
        Released,
        Query,
        "List AgentPlans, optionally filtered by an AgentConnection grant."
    ),
    command!(
        "routing.show",
        ["routing", "show"],
        "GetAgentPlanStatus",
        ["CLI-006", "APP-006"],
        Released,
        Query,
        "Show one immutable AgentPlan revision and materialized routing contract."
    ),
    command!(
        "routing.preview",
        ["routing", "preview"],
        "PreviewAgentPlanChange",
        ["CLI-004", "APP-004"],
        Released,
        Preview,
        "Preview a complete materialized AgentPlan change."
    ),
    command!(
        "routing.apply",
        ["routing", "apply"],
        "ApplyAgentPlanChange",
        ["CLI-005", "APP-005"],
        Released,
        Apply,
        "Apply an exact AgentPlan change and closed publication revision."
    ),
    command!(
        "decision.services.list",
        ["decision", "services", "list"],
        "ListDecisionServices",
        [],
        Released,
        Query,
        "List saved decision services and their latest immutable connection versions."
    ),
    command!(
        "decision.services.apply",
        ["decision", "services", "apply"],
        "ApplyDecisionService",
        [],
        Released,
        Apply,
        "Save a decision service version or delete an unreferenced service with revision checking."
    ),
    command!(
        "decision.sources.list",
        ["decision", "sources", "list"],
        "ListDecisionConnectionSourcesV1",
        [],
        Released,
        Query,
        "List safe source summaries available to decision connections."
    ),
    command!(
        "decision.connections.list",
        ["decision", "connections", "list"],
        "ListDecisionServicesV2",
        [],
        Released,
        Query,
        "List versioned decision connections with current source state."
    ),
    command!(
        "decision.connections.apply",
        ["decision", "connections", "apply"],
        "ApplyDecisionServiceV2",
        [],
        Released,
        Apply,
        "Save a versioned decision connection with exact source and credential revisions."
    ),
    command!(
        "decision.connections.test",
        ["decision", "connections", "test"],
        "TestDecisionServiceV2",
        [],
        Released,
        ExternalProbe,
        "Test an exact saved decision connection revision with a synthetic request."
    ),
    command!(
        "decision.network.test",
        ["decision", "network", "test"],
        "TestDecisionServiceV3",
        ["CLI-079", "APP-079"],
        Released,
        ExternalProbe,
        "Test an exact saved decision revision with admitted network evidence."
    ),
    command!(
        "decision.services.test",
        ["decision", "services", "test"],
        "TestClassifierDecision",
        [],
        Released,
        ExternalProbe,
        "Run one explicit synthetic decision through a configured decision service."
    ),
    command!(
        "routing.classifier.secret.apply",
        ["routing", "classifier", "secret", "apply"],
        "ApplyClassifierHeaderSecret",
        [],
        Planned,
        Apply,
        "Create or replace one classifier HTTP-header Secret from protected input."
    ),
    command!(
        "models.list",
        ["models", "list"],
        "ListModels",
        ["CLI-038", "APP-038"],
        Planned,
        Query,
        "List effective model configurations joined to registered source inventory."
    ),
    command!(
        "models.feed.status",
        ["models", "feed", "status"],
        "GetModelFeedStatus",
        ["CLI-059", "APP-059"],
        Released,
        Query,
        "Show the verified model feed, retrieval setting and last attempt."
    ),
    command!(
        "models.feed.refresh",
        ["models", "feed", "refresh"],
        "RefreshModelFeed",
        ["CLI-060", "APP-060"],
        Released,
        Apply,
        "Retrieve and verify the model feed now."
    ),
    command!(
        "models.feed.settings.apply",
        ["models", "feed", "settings", "apply"],
        "ApplyModelFeedSettings",
        ["CLI-061", "APP-061"],
        Released,
        Apply,
        "Change service-owned model feed retrieval and disclosure settings."
    ),
    command!(
        "models.feed.remove.preview",
        ["models", "feed", "remove", "preview"],
        "PreviewModelFeedRemoval",
        ["CLI-062", "APP-062"],
        Released,
        Query,
        "Preview saved sources affected by removing the held model feed."
    ),
    command!(
        "models.feed.remove.apply",
        ["models", "feed", "remove", "apply"],
        "ApplyModelFeedRemoval",
        ["CLI-063", "APP-063"],
        Released,
        Apply,
        "Remove the held feed while retaining its highest accepted sequence."
    ),
    command!(
        "models.availability",
        ["models", "availability"],
        "GetModelAvailability",
        ["CLI-064", "APP-064"],
        Released,
        Query,
        "Show model origins, announcements and current local availability."
    ),
    command!(
        "compute.network.show",
        ["compute", "network", "show"],
        "GetNativeSourceNetworkV1",
        ["CLI-074", "APP-074"],
        Released,
        Query,
        "Read a Native source policy, application state and affected consumers."
    ),
    command!(
        "compute.network.preview",
        ["compute", "network", "preview"],
        "PreviewNativeSourceNetworkV1",
        ["CLI-075", "APP-075"],
        Released,
        Preview,
        "Preview a revision-bound Native source network policy edit."
    ),
    command!(
        "compute.network.apply",
        ["compute", "network", "apply"],
        "ApplyNativeSourceNetworkV1",
        ["CLI-076", "APP-076"],
        Released,
        Apply,
        "Apply an exact sealed Native source network preview through its existing Operation."
    ),
    command!(
        "compute.network.resolve",
        ["compute", "network", "resolve"],
        "PreviewNativeNetworkRouteV1",
        ["CLI-077", "APP-077"],
        Released,
        Query,
        "Interpret a Native route without DNS or provider requests."
    ),
    command!(
        "compute.connection.preview-network",
        ["compute", "connection", "preview-network"],
        "PreviewComputeSaveV4",
        ["CLI-078", "APP-078"],
        Released,
        Preview,
        "Preview a combined source and network policy save."
    ),
    command!(
        "compute.defaults.get",
        ["compute", "defaults", "get"],
        "GetComputeDefaults",
        ["CLI-073", "APP-073"],
        Released,
        Query,
        "Read saved intent, effective capabilities and the exact official default locator."
    ),
    command!(
        "compute.defaults.preview",
        ["compute", "defaults", "preview"],
        "PreviewComputeSaveV3",
        ["CLI-072", "APP-072"],
        Released,
        Preview,
        "Preview a source save with explicit official-default or preserved-value intent."
    ),
    command!(
        "models.metadata.list",
        ["models", "metadata", "list"],
        "ListModelMetadata",
        ["CLI-070", "APP-070"],
        Released,
        Query,
        "Read a bounded page of complete official model metadata, including descriptive-only records."
    ),
    command!(
        "models.metadata.get",
        ["models", "metadata", "get"],
        "GetModelMetadata",
        ["CLI-071", "APP-071"],
        Released,
        Query,
        "Read an exact scoped metadata record from the selected snapshot."
    ),
    command!(
        "models.show",
        ["models", "show"],
        "ShowModel",
        ["CLI-026", "APP-026"],
        Released,
        Query,
        "Show exact model, capability, reasoning, rating, and source facts."
    ),
    command!(
        "models.free",
        ["models", "free"],
        "ListFreeModels",
        ["CLI-036", "APP-036"],
        Planned,
        Query,
        "List the offline Release free catalog and binding readiness."
    ),
    command!(
        "prices.effective",
        ["prices", "effective"],
        "GetEffectivePrices",
        ["CLI-040", "APP-040"],
        Released,
        Query,
        "Read effective current-catalog price facts and local override revisions."
    ),
    command!(
        "prices.override.preview",
        ["prices", "override", "preview"],
        "PreviewPriceOverrideChange",
        ["CLI-041", "APP-041"],
        Released,
        Preview,
        "Preview a typed local price override change."
    ),
    command!(
        "prices.override.apply",
        ["prices", "override", "apply"],
        "ApplyPriceOverrideChange",
        ["CLI-042", "APP-042"],
        Released,
        Apply,
        "Apply an exact local price override revision."
    ),
    command!(
        "operations.get",
        ["operations", "get"],
        "GetOperation",
        ["CLI-012", "APP-012"],
        Released,
        Query,
        "Read one durable Operation and its terminal or recovery state."
    ),
    command!(
        "operations.watch",
        ["operations", "watch"],
        "WatchOperations",
        ["CLI-013", "APP-013"],
        Planned,
        Watch,
        "Watch monotonic typed Operation updates."
    ),
    command!(
        "operations.cancel",
        ["operations", "cancel"],
        "CancelOperation",
        ["CLI-015", "APP-015"],
        Planned,
        Action,
        "Request cancellation only at a declared recoverable boundary."
    ),
    command!(
        "sessions.list",
        ["sessions", "list"],
        "ListSessions",
        ["CLI-007", "APP-007"],
        Released,
        Query,
        "Search Session and Turn facts without returning content by default."
    ),
    command!(
        "sessions.show",
        ["sessions", "show"],
        "GetSession",
        ["CLI-008", "APP-008"],
        Released,
        Query,
        "Show one Session timeline with content only under an explicit authorized mode."
    ),
    command!(
        "sessions.receipt",
        ["sessions", "receipt"],
        "GetRoutingReceipt",
        ["CLI-021", "APP-021"],
        Released,
        Query,
        "Read an immutable structured RoutingReceipt without content or recomputation."
    ),
    command!(
        "sessions.export",
        ["sessions", "export"],
        "ExportSessions",
        ["CLI-016", "APP-016"],
        Planned,
        Query,
        "Export authorized local observations with content disabled by default."
    ),
    command!(
        "sessions.status",
        ["sessions", "status"],
        "GetObservationStatus",
        ["CLI-022", "APP-022"],
        Released,
        Query,
        "Read fact and content completeness independently."
    ),
    command!(
        "sessions.delete.preview",
        ["sessions", "delete", "preview"],
        "PreviewSessionDeletion",
        ["CLI-017", "APP-017"],
        Planned,
        Preview,
        "Preview exact observation deletion scope and retention effects."
    ),
    command!(
        "sessions.delete.apply",
        ["sessions", "delete", "apply"],
        "ApplySessionDeletion",
        ["CLI-018", "APP-018"],
        Planned,
        Apply,
        "Apply an exact observation deletion without inventing new evidence."
    ),
    command!(
        "value.show",
        ["value", "show"],
        "GetValue",
        ["CLI-039", "APP-039"],
        Released,
        Query,
        "Read recorded token, cache and valuation totals with coverage."
    ),
    command!(
        "sessions.timeline",
        ["sessions", "timeline"],
        "GetRoutingTimeline",
        ["CLI-069", "APP-069"],
        Released,
        Query,
        "Read agent turns with selection, historical protection, score, requests and costs; --follow streams changed turns."
    ),
    command!(
        "observation.plan-trust",
        ["observation", "plan-trust"],
        "GetPlanTrust",
        ["CLI-065", "APP-065"],
        Released,
        Query,
        "Read complete-scoring coverage and competence for a Plan revision."
    ),
    command!(
        "observation.plan-trust.replay",
        ["observation", "plan-trust", "replay"],
        "GetPlanTrustReplay",
        ["CLI-066", "APP-066"],
        Released,
        Query,
        "Replay exact thresholds over a complete retained population; estimates are not predictions."
    ),
    command!(
        "observation.plan-trust.suggestions",
        ["observation", "plan-trust", "suggestions"],
        "GetPlanTrustSuggestions",
        ["CLI-067", "APP-067"],
        Released,
        Query,
        "Read optional evidence-based trust suggestions without changing a Plan."
    ),
    command!(
        "observation.storage.show",
        ["observation", "storage", "show"],
        "GetObservationCapacity",
        ["CLI-059", "APP-059"],
        Released,
        Query,
        "Read the local observation capacity policy and measured reclamation status."
    ),
    command!(
        "observation.storage.set",
        ["observation", "storage", "set"],
        "SetObservationCapacity",
        ["CLI-060", "APP-060"],
        Released,
        Action,
        "Save a positive byte threshold or disable early observation expiry."
    ),
    command!(
        "observation.plan-quality.samples",
        ["observation", "plan-quality", "samples"],
        "GetPlanQualitySamples",
        ["CLI-058", "APP-058"],
        Released,
        Query,
        "List the latest competence assessment for each persisted execution segment."
    ),
];

pub fn planned_commands() -> Vec<CommandDescriptorV1> {
    COMMANDS
        .iter()
        .copied()
        .map(StaticCommandDescriptor::to_owned)
        .collect()
}

pub fn released_commands() -> Vec<CommandDescriptorV1> {
    COMMANDS
        .iter()
        .copied()
        .filter(|descriptor| descriptor.lifecycle == CommandLifecycle::Released)
        .map(StaticCommandDescriptor::to_owned)
        .collect()
}

/// Staged P0 commands whose real handlers may be exercised by control-shell verification while
/// their public lifecycle remains Planned until final `hirouted --role=all` composition.
pub fn staged_control_commands() -> Vec<CommandDescriptorV1> {
    const IDS: &[&str] = &[
        "work-plans.list",
        "tasks.list",
        "tasks.show",
        "tasks.start",
        "tasks.wait",
        "tasks.result",
        "tasks.cancel",
        "tasks.continue",
        "system.client-status",
        "operations.find",
        "routing.list",
        "routing.show",
        "routing.options",
        "decision.services.test",
        "routing.classifier.secret.apply",
        "system.status",
        "setup.preview",
        "setup.apply",
        "setup.status",
        "operations.get",
        "operations.watch",
        "operations.cancel",
        "agents.scan",
        "agents.list",
        "agents.check",
        "sessions.list",
        "sessions.show",
        "sessions.receipt",
        "sessions.status",
        "value.show",
        "observation.plan-quality.samples",
    ];
    COMMANDS
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.lifecycle == CommandLifecycle::Planned
                && IDS.contains(&descriptor.command_id)
        })
        .map(StaticCommandDescriptor::to_owned)
        .collect()
}

pub fn command_by_id(command_id: &str) -> Option<CommandDescriptorV1> {
    COMMANDS
        .iter()
        .copied()
        .find(|descriptor| descriptor.command_id == command_id)
        .map(StaticCommandDescriptor::to_owned)
}

pub fn command_by_operation(operation_id: &str) -> Option<CommandDescriptorV1> {
    COMMANDS
        .iter()
        .copied()
        .find(|descriptor| descriptor.operation_id == operation_id)
        .map(StaticCommandDescriptor::to_owned)
}

pub fn descriptor_digest() -> CanonicalDigest {
    CanonicalDigest::of(&planned_commands()).expect("the static command registry is serializable")
}

pub fn planned_manifest() -> CommandManifestV1 {
    CommandManifestV1 {
        schema_version: COMMAND_MANIFEST_SCHEMA,
        product_contract_revision: PRODUCT_CONTRACT_REVISION.to_owned(),
        proposal_map_revision: PROPOSAL_MAP_REVISION.to_owned(),
        descriptor_digest: descriptor_digest(),
        visibility: "planned".to_owned(),
        commands: planned_commands(),
    }
}

pub fn release_manifest() -> CommandManifestV1 {
    let commands = released_commands();
    debug_assert!(commands.iter().all(|descriptor| {
        descriptor.positive.state == CoverageState::Executable
            && descriptor.negative.state == CoverageState::Executable
    }));
    CommandManifestV1 {
        schema_version: COMMAND_MANIFEST_SCHEMA,
        product_contract_revision: PRODUCT_CONTRACT_REVISION.to_owned(),
        proposal_map_revision: PROPOSAL_MAP_REVISION.to_owned(),
        descriptor_digest: descriptor_digest(),
        visibility: "release".to_owned(),
        commands,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ReservedOperationV1 {
    pub operation_id: &'static str,
    pub phase: &'static str,
}

const RESERVED_OPERATIONS: &[ReservedOperationV1] = &[
    ReservedOperationV1 {
        operation_id: "PreviewLocalModelChange",
        phase: "future",
    },
    ReservedOperationV1 {
        operation_id: "ApplyLocalModelChange",
        phase: "future",
    },
    ReservedOperationV1 {
        operation_id: "GetLocalModelStatus",
        phase: "future",
    },
    ReservedOperationV1 {
        operation_id: "ExplainRouteDecision",
        phase: "post_mvp",
    },
    ReservedOperationV1 {
        operation_id: "ListCalibrationCandidates",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "GetCalibrationContext",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "SubmitCalibration",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "PreviewPersonalRatingSync",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "ApplyPersonalRatingSync",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "ListRatingServers",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "RefreshRatingSnapshot",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "GetRatingServerStatus",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "PreviewRatingServerRegistration",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "ApplyRatingServerRegistration",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "StartHiRouteAccountLogin",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "GetHiRouteAccountStatus",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "RevokeHiRouteAccountSession",
        phase: "p1",
    },
    ReservedOperationV1 {
        operation_id: "ListExecutableAgentExecutionProfiles",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "SubmitACPDelegation",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "PreviewCoordinatorSetup",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "ApplyCoordinatorSetup",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "GetCoordinatorSetupStatus",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "RunRootTaskCalibration",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "GetRootTaskCalibrationRun",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "RevealCalibrationDraft",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "ListScenarioEvaluations",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "BeginCoordinatorTaskEpisode",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "CompleteCoordinatorTaskEpisode",
        phase: "p2",
    },
    ReservedOperationV1 {
        operation_id: "GetCoordinatorTaskEpisode",
        phase: "p2",
    },
];

pub fn reserved_operation(operation_id: &str) -> Option<ReservedOperationV1> {
    RESERVED_OPERATIONS
        .iter()
        .copied()
        .find(|operation| operation.operation_id == operation_id)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn reserved_operations_do_not_enter_either_cli_manifest() {
        let ids = planned_commands()
            .into_iter()
            .map(|command| command.operation_id)
            .collect::<BTreeSet<_>>();
        for operation in RESERVED_OPERATIONS {
            assert!(!ids.contains(operation.operation_id));
        }
    }
}
