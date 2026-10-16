//! Installed, offline contracts for the management journey, derived from the wire DTOs.
//! These definitions describe syntax; current options and Preview remain authoritative for
//! dynamic IDs, permissions, capabilities and business validation.
use crate::*;
use hiroute_domain::ObservationMaintenanceStatusV2;
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(schemars::JsonSchema, Serialize)]
pub struct CommandSchemasV1 {
    pub request: Value,
    pub response: Value,
    pub examples: Vec<Value>,
    pub example_guidance: &'static str,
}

fn schema<T: JsonSchema>(serialize: bool) -> Value {
    let settings = SchemaSettings::draft2020_12();
    let settings = if serialize {
        settings.for_serialize()
    } else {
        settings.for_deserialize()
    };
    let mut value = serde_json::to_value(settings.into_generator().into_root_schema_for::<T>())
        .expect("generated schema serializes");
    if !serialize {
        strict_objects(&mut value);
    }
    value
}

fn strict_objects(value: &mut Value) {
    match value {
        Value::Object(object) => {
            // CLI member validation is stricter than persisted serde readers. Maps retain their
            // typed additionalProperties; tagged unions retain their variant definitions.
            if object.contains_key("properties") && !object.contains_key("additionalProperties") {
                object.insert("additionalProperties".into(), json!(false));
            }
            for child in object.values_mut() {
                strict_objects(child);
            }
        }
        Value::Array(values) => {
            for value in values {
                strict_objects(value);
            }
        }
        _ => {}
    }
}

// Each root keeps its own definitions, so union alternatives cannot accidentally resolve a
// same-named definition from another wrapper. No reference ever leaves this document.
fn alternatives(values: Vec<Value>) -> Value {
    let mut defs = serde_json::Map::new();
    let mut refs = Vec::new();
    for (index, mut value) in values.into_iter().enumerate() {
        let name = format!("variant_{index}");
        rebase(&mut value, &format!("#/$defs/{name}"));
        value
            .as_object_mut()
            .expect("schema object")
            .remove("$schema");
        defs.insert(name.clone(), value);
        refs.push(json!({"$ref":format!("#/$defs/{name}")}));
    }
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "anyOf":refs, "$defs":defs})
}
fn rebase(value: &mut Value, prefix: &str) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get_mut("$ref")
                && let Some(suffix) = reference.strip_prefix('#')
            {
                *reference = format!("{prefix}{suffix}");
            }
            for child in object.values_mut() {
                rebase(child, prefix);
            }
        }
        Value::Array(values) => {
            for value in values {
                rebase(value, prefix);
            }
        }
        _ => {}
    }
}

macro_rules! request { ($($ty:ty),+ $(,)?) => { alternatives(vec![$(schema::<$ty>(false)),+]) }; }
macro_rules! response { ($($ty:ty),+ $(,)?) => { alternatives(vec![$(schema::<MachineEnvelopeV2<$ty>>(true)),+, schema::<MachineEnvelopeV2<CliInputDiagnosticDataV1>>(true), schema::<MachineEnvelopeV2<CliRequiredChoicesV1>>(true)]) }; }

pub fn command_schemas(command_id: &str) -> Option<CommandSchemasV1> {
    let (mut request, mut response) = match command_id {
        "prices.effective" => (
            request!(GetEffectivePricesV2),
            response!(EffectivePricesResultV2),
        ),
        "prices.override.preview" => (
            request!(PreviewPriceOverrideChangeV2),
            response!(PriceOverridePreviewV2),
        ),
        "prices.override.apply" => (
            request!(ApplyPriceOverrideChangeV2),
            response!(PriceApplyOperationV2),
        ),
        "value.show" => (
            observation_request(&[
                "value",
                "home_value",
                "value_report",
                "api_equivalent_value",
            ]),
            response!(
                hiroute_domain::ObservationValueSummaryV2,
                hiroute_domain::ObservationValueReportV2,
                hiroute_domain::ApiEquivalentValueReportV1
            ),
        ),
        "sessions.list" => (
            alternatives(vec![
                schema::<SessionListRequestV1>(false),
                observation_request(&["status", "sessions", "search", "session_search"]),
            ]),
            response!(
                SessionListPageV1,
                ObservationMaintenanceStatusV2,
                ObservationSessionPageV2,
                ObservationSearchPageV2,
                ObservationSessionSearchPageV1
            ),
        ),
        "sessions.show" => (
            alternatives(vec![
                schema::<SessionLookupV1>(false),
                observation_request(&["timeline", "catalog", "ancestry", "content"]),
            ]),
            response!(
                hiroute_domain::SessionDetailV1,
                ObservationRequestPage,
                ObservationCatalogPageV2,
                ObservationAncestryV2,
                ObservationContentPageV2
            ),
        ),
        "sessions.receipt" => (
            alternatives(vec![
                schema::<SessionLookupV1>(false),
                observation_request(&["facts"]),
            ]),
            response!(hiroute_domain::RoutingReceiptV1, ObservationFactsPageV2),
        ),
        "sessions.status" => (
            request!(ClientEmptyRequestV1),
            response!(hiroute_domain::ObservationStatusV1),
        ),
        "observation.plan-quality.samples" => (
            observation_request(&["plan_quality"]),
            response!(PlanQualitySamplesPage),
        ),
        "agents.check" => (
            request!(AgentCheckRequestV1),
            response!(AgentCheckResultV1, AgentCollaborationCheckFailureV1),
        ),
        "decision.services.list" => (
            request!(ClientEmptyRequestV1),
            response!(DecisionServicesListV1),
        ),
        "agents.scan" | "agents.list" => (
            request!(AgentScanRequestV1),
            response!(AgentDiscoveryListV1),
        ),
        "system.client-status" => (
            request!(ClientEmptyRequestV1),
            response!(ClientServiceStatusV1),
        ),
        "compute.scan" => (
            request!(ComputeScanRequestV1, ComputeScanRequestV2),
            response!(ComputeScanResultV1, ComputeScanResultV2),
        ),
        "compute.list" | "compute.show" => (
            request!(ComputeManagementQueryV2),
            response!(ComputeManagementSnapshotV2),
        ),
        "compute.connection.options" => (
            request!(ComputeConnectionOptionsRequestV1),
            response!(ComputeConnectionOptionsResultV1),
        ),
        "compute.connection.prepare" => (
            request!(PrepareDiscoveredModelConnectionRequestV1),
            response!(ComputeCandidateViewV2),
        ),
        "compute.connection.authorize" => (
            request!(ComputeConnectionAuthorizationRequestV1),
            response!(ComputeSubscriptionCheckResultV2),
        ),
        "compute.connection.preview" => (
            request!(
                ComputeConnectionPreviewRequestV1,
                ComputeSavePreviewRequestV2,
                ComputeSubscriptionCheckPreviewRequestV2
            ),
            response!(
                ComputeConnectionPreviewResultV1,
                ComputeSavePreviewV2,
                ComputeSubscriptionCheckPreviewV2,
                CliPreparedPreviewV1<ComputeConnectionPreviewResultV1>,
                CliPreparedPreviewV1<ComputeSavePreviewV2>
            ),
        ),
        "compute.connection.apply" => (
            request!(ComputeConnectionApplyRequestV1),
            response!(ApplyResultV1),
        ),
        "compute.connection.login" => (
            request!(ComputeSubscriptionLoginRequestV1),
            response!(ComputeSubscriptionLoginResultV1),
        ),
        "compute.connection.test" => (
            request!(ComputeConnectionTestRequestV1),
            response!(ModelConnectionCheckViewV1, ComputeCandidateViewV2),
        ),
        "routing.options" => (
            request!(PlanEditorOptionsRequestV1),
            response!(PlanEditorOptionsV1),
        ),
        "routing.list" => (
            request!(AgentPlanCatalogQueryV2),
            response!(AgentPlanCatalogViewV2),
        ),
        "routing.show" => (request!(AgentPlanLookupV1), response!(AgentPlanStatusV2)),
        "routing.preview" => (
            request!(
                PlanContentPreviewRequestV2,
                PlanDraftPreviewRequestV1,
                PlanLifecyclePreviewRequestV1,
                PublicationCheckpointPreviewRequestV1
            ),
            response!(
                PlanContentPreviewV2,
                PlanDraftPreviewV1,
                PlanLifecyclePreviewV1,
                PublicationCheckpointPreviewV1,
                CliPreparedPreviewV1<PlanContentPreviewV2>
            ),
        ),
        "routing.apply" => (
            request!(
                PlanContentApplyRequestV2,
                PlanDraftApplyRequestV1,
                PlanLifecycleApplyRequestV1,
                PublicationCheckpointApplyRequestV1
            ),
            response!(ApplyResultV1),
        ),
        "agents.connect.preview" => (
            request!(
                AgentConnectionPreviewRequestV1,
                AgentSettingsPreviewRequestV2
            ),
            response!(
                AgentConnectionPreviewResultV1,
                AgentSettingsPreviewV2,
                CliPreparedPreviewV1<AgentSettingsPreviewV2>
            ),
        ),
        "agents.restore.preview" => (
            request!(AgentSettingsPreviewRequestV2),
            response!(
                AgentSettingsPreviewV2,
                CliPreparedPreviewV1<AgentSettingsPreviewV2>
            ),
        ),
        "agents.connect.apply" => (
            request!(
                AgentConnectionApplyRequestV1,
                AgentSettingsApplyV2,
                AgentSettingsRetryV1
            ),
            response!(ApplyResultV1),
        ),
        "agents.restore.apply" => (request!(AgentSettingsApplyV2), response!(ApplyResultV1)),
        "agents.connect.status" => (
            request!(AgentConnectionStatusRequestV1, AgentSettingsStatusRequestV2),
            response!(AgentConnectionStatusV1, AgentSettingsStatusV2),
        ),
        "operations.get" => (
            request!(OperationLookupV1),
            response!(hiroute_domain::OperationV1),
        ),
        "operations.find" => (
            request!(OperationIdempotencyLookupV1),
            response!(OperationIdempotencyResultV1),
        ),
        "decision.services.apply" => (
            request!(PreviewRequestV1, ApplyRequestV1),
            response!(
                PreviewResultV1,
                ApplyResultV1,
                CliPreparedPreviewV1<PreviewResultV1>
            ),
        ),
        "decision.services.test" => (
            request!(ClassifierDecisionTestRequestV1),
            response!(ClassifierDecisionTestResultV1),
        ),
        _ => return None,
    };
    if command_id == "decision.services.apply" {
        specialize_change(
            &mut request,
            &schema::<hiroute_domain::DecisionServiceChangeV1>(false),
        );
        specialize_change(
            &mut response,
            &schema::<hiroute_domain::DecisionServiceChangeV1>(true),
        );
        remove_property(&mut request, "apply_capability");
    } else if command_id == "compute.connection.apply" {
        specialize_change(
            &mut request,
            &request!(
                ComputeConnectionPlannerPayloadV1,
                ComputeManagementChangeV2,
                ComputeSubscriptionCheckChangeV2
            ),
        );
    }
    Some(CommandSchemasV1 {
        request,
        response,
        examples: examples(command_id),
        example_guidance: "Examples show syntax. Resolve resource IDs from current queries. For Apply, copy the exact Preview spec/change, digest and expected revisions; keep one submission key. Example digests and revisions do not authorize writes.",
    })
}

fn examples(command_id: &str) -> Vec<Value> {
    match command_id {
        "decision.services.apply" => {
            let change = json!({"id":"service/example", "expected_revision":0, "service":{
                "id":"service/example","revision":1,"name":"Example decision service",
                "connection":{"kind":"custom","endpoint":"https://decisions.example/v1/decisions","timeout_ms":10000,"auth_header":null}
            }});
            let spec = json!({"schema_version":hiroute_domain::CHANGE_SPEC_SCHEMA_V1, "command_id":"decision.services.apply", "resource_id":"service/example", "desired_state":change});
            vec![
                json!({"schema_version":hiroute_domain::CHANGE_SPEC_SCHEMA_V1,"spec":spec}),
                json!({"schema_version":hiroute_domain::CHANGE_SPEC_SCHEMA_V1,"spec":spec,"accept_digest":format!("sha256:{}", "0".repeat(64)),"expected_revisions":{"target":0},"idempotency_key":"replace-with-one-stable-key"}),
            ]
        }
        "routing.options" => vec![json!({}), json!({"authoring":{}})],
        "compute.scan" => vec![json!({}), json!({"schema":COMPUTE_SCAN_REQUEST_SCHEMA_V2})],
        "compute.connection.options"
        | "compute.list"
        | "routing.list"
        | "agents.scan"
        | "agents.list"
        | "system.client-status" => vec![json!({})],
        _ => vec![],
    }
}

fn observation_request(views: &[&str]) -> Value {
    let mut result = schema::<ObservationReadRequestV2>(false);
    if let Some(variants) = result
        .pointer_mut("/$defs/ObservationReadIntentV2/oneOf")
        .and_then(Value::as_array_mut)
    {
        variants.retain(|variant| {
            variant
                .pointer("/properties/view/const")
                .and_then(Value::as_str)
                .is_some_and(|view| views.contains(&view))
        });
    }
    result
}
fn remove_property(value: &mut Value, name: &str) {
    match value {
        Value::Object(object) => {
            if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
                properties.remove(name);
            }
            for child in object.values_mut() {
                remove_property(child, name);
            }
        }
        Value::Array(values) => {
            for value in values {
                remove_property(value, name);
            }
        }
        _ => {}
    }
}
fn specialize_change(value: &mut Value, desired: &Value) {
    fn walk(value: &mut Value, desired: &Value, pointer: &str) {
        match value {
            Value::Object(object) => {
                if let Some(properties) =
                    object.get_mut("properties").and_then(Value::as_object_mut)
                    && properties.contains_key("desired_state")
                    && properties.contains_key("command_id")
                {
                    let mut schema = desired.clone();
                    rebase(&mut schema, &format!("#{pointer}/properties/desired_state"));
                    properties.insert("desired_state".into(), schema);
                    return;
                }
                for (key, child) in object.iter_mut() {
                    walk(child, desired, &format!("{pointer}/{key}"));
                }
            }
            Value::Array(values) => {
                for (i, child) in values.iter_mut().enumerate() {
                    walk(child, desired, &format!("{pointer}/{i}"));
                }
            }
            _ => {}
        }
    }
    walk(value, desired, "");
}
