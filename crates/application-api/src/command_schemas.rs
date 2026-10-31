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
        "compute.network.show" => (
            request!(NativeSourceNetworkQueryV1),
            response!(NativeSourceNetworkViewV1),
        ),
        "compute.network.preview" => (
            request!(NativeSourceNetworkPreviewV1),
            response!(NativeNetworkSavePreviewV1),
        ),
        "compute.network.apply" => (
            request!(ComputeConnectionApplyRequestV1),
            response!(ApplyResultV1),
        ),
        "compute.network.resolve" => (
            request!(NativeNetworkRoutePreviewRequestV1),
            response!(NativeNetworkRouteViewV1),
        ),
        "compute.connection.preview-network" => (
            request!(ComputeManagementChangeV4),
            response!(ComputeSavePreviewV2),
        ),
        "compute.defaults.get" => (
            request!(ComputeDefaultsQueryV1),
            response!(ComputeDefaultsResultV1),
        ),
        "compute.defaults.preview" => (
            request!(ComputeSavePreviewRequestV3),
            response!(ComputeSavePreviewV2),
        ),
        "models.metadata.list" => (
            request!(ListModelMetadataV1),
            response!(ModelMetadataPageV1),
        ),
        "models.metadata.get" => (
            request!(GetModelMetadataV1),
            response!(ModelMetadataDetailV1),
        ),
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
            analysis_request(
                &[
                    "value",
                    "home_value",
                    "value_report",
                    "api_equivalent_value",
                    "analysis",
                ],
                &["cost_ranking", "savings_summary"],
            ),
            response!(
                hiroute_domain::ObservationValueSummaryV2,
                hiroute_domain::ObservationValueReportV2,
                hiroute_domain::ApiEquivalentValueReportV1,
                hiroute_domain::ObservationAnalysisPageV1
            ),
        ),
        "sessions.list" => (
            alternatives(vec![
                schema::<SessionListRequestV1>(false),
                analysis_request(
                    &["status", "sessions", "search", "session_search", "analysis"],
                    &["conversations", "switches"],
                ),
            ]),
            response!(
                SessionListPageV1,
                ObservationMaintenanceStatusV2,
                ObservationSessionPageV2,
                ObservationSearchPageV2,
                ObservationSessionSearchPageV1,
                hiroute_domain::ObservationAnalysisPageV1
            ),
        ),
        "sessions.show" => (
            alternatives(vec![
                schema::<SessionLookupV1>(false),
                analysis_request(
                    &["timeline", "catalog", "ancestry", "content", "analysis"],
                    &["conversation", "routing"],
                ),
            ]),
            response!(
                hiroute_domain::SessionDetailV1,
                ObservationRequestPage,
                ObservationCatalogPageV2,
                ObservationAncestryV2,
                ObservationContentPageV2,
                hiroute_domain::ObservationAnalysisPageV1
            ),
        ),
        "sessions.receipt" => (
            alternatives(vec![
                schema::<SessionLookupV1>(false),
                observation_request(&["facts"]),
            ]),
            response!(hiroute_domain::RoutingReceiptV1, ObservationFactsPageV2),
        ),
        "sessions.timeline" => (
            observation_request(&["routing_timeline"]),
            response!(RoutingTimelinePageV1),
        ),
        "sessions.status" => (
            request!(ClientEmptyRequestV1),
            response!(hiroute_domain::ObservationStatusV1),
        ),
        "observation.storage.show" => (
            schema::<ClientEmptyRequestV1>(false),
            schema::<hiroute_domain::ObservationCapacityStatusV1>(true),
        ),
        "observation.storage.set" => (
            schema::<ObservationCapacityUpdateV1>(false),
            schema::<hiroute_domain::ObservationCapacityStatusV1>(true),
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
        "decision.sources.list" => (
            request!(ClientEmptyRequestV1),
            response!(DecisionConnectionSourcesV1),
        ),
        "decision.connections.list" => (
            request!(ClientEmptyRequestV1),
            response!(DecisionServicesV2),
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
        "compute.connection.test-network" => (
            request!(ComputeConnectionTestRequestV2),
            response!(ModelConnectionCheckViewV2),
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
        "decision.services.apply" | "decision.connections.apply" => (
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
        "decision.network.test" => (
            request!(DecisionServiceTestRequestV2),
            response!(DecisionServiceTestResultV3),
        ),
        "decision.connections.test" => (
            request!(DecisionServiceTestRequestV2),
            response!(DecisionServiceTestResultV2),
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
    } else if command_id == "decision.connections.apply" {
        specialize_change(
            &mut request,
            &schema::<hiroute_domain::DecisionServiceOperationInputV2>(false),
        );
        specialize_change(
            &mut response,
            &schema::<hiroute_domain::DecisionServiceOperationInputV2>(true),
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
        example_guidance: if command_id == "compute.connection.test" {
            "Templates show syntax, not a successful check. Save the native template as native-request.json or the registered template as registered-request.json. For a registered option, run hiroute compute connection options --output json, choose an option with registered_check_available=true, and replace the entire request.expected_catalog with data.catalog from that response. Register the credential with hiroute protected-input register --candidate <REF> --secret-fd <FD> --output json; after registered=true, use that REF for request.input_candidate.candidate_ref and the matching candidate_ref (request.candidate_ref for registered, request.draft.candidate_ref for native), with input_candidate.candidate_revision=1 for this protected-input protocol. Use your own lineage_ref/check_id and edit_revision for this check. Native API templates also require your actual endpoint, protocol profile and authentication semantics. Keep secret bytes in the separate protected-input channel. inference_model_id=null performs no model inference; a selected inference model can consume quota. Read the result's reachability, authentication, directory and inference fields: a successful command or selectable candidate does not establish provider verification. For a key-authenticated option with neither an inventory path nor a selected inference model, no remote probe runs; reachability=not_run, authentication=unknown, directory=not_run and inference=not_run leave connectivity and the API key unverified. The current bundled OpenRouter template follows this path. Catalog example digests/revisions are placeholders and cannot authorize a check."
        } else {
            "Examples show syntax. Resolve resource IDs from current queries. For Apply, copy the exact Preview spec/change, digest and expected revisions; keep one submission key. Example digests and revisions do not authorize writes."
        },
    })
}

fn examples(command_id: &str) -> Vec<Value> {
    match command_id {
        "compute.connection.test" => connection_test_examples(),
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
        "observation.storage.show" => vec![json!({})],
        "observation.storage.set" => vec![
            json!({"max_active_bytes":20_000_000_000u64}),
            json!({"max_active_bytes":null}),
        ],
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

fn connection_test_examples() -> Vec<Value> {
    let candidate = json!({
        "candidate_ref":"candidate/native/example", "candidate_revision":1
    });
    vec![
        json!({
            "kind":"native",
            "request":{
                "draft":{
                    "inference_model_id":null,
                    "candidate_ref":"candidate/native/example",
                    "lineage_ref":"lineage/native/example",
                    "display_name":"My API",
                    "existing_source_id":null,
                    "edit_revision":1,
                    "check_id":"check/native/example-1",
                    "base_url":"https://provider.example/v1",
                    "base_kind":"api_root",
                    "request_path_override":null,
                    "inventory_path_override":null,
                    "protocol":"responses",
                    "protocol_profile_id":"profile/custom/responses",
                    "protocol_profile_revision":1,
                    "authentication":{"kind":"bearer"},
                    "configuration_revision":1,
                    "models":[]
                },
                "input_candidate":candidate
            }
        }),
        json!({
            "kind":"registered",
            "request":{
                "connection_option_id":"openrouter.free.global.v1",
                "display_name":"OpenRouter Free",
                "expected_catalog":{
                    "product_release":"replace-from-current-options",
                    "catalog_binding_id":"replace-from-current-options",
                    "release_sequence":1,
                    "connector_registry_digest":format!("sha256:{}", "0".repeat(64)),
                    "model_data_digest":format!("sha256:{}", "0".repeat(64)),
                    "cross_reference_digest":format!("sha256:{}", "0".repeat(64))
                },
                "candidate_ref":"candidate/native/example",
                "lineage_ref":"lineage/native/example",
                "edit_revision":1,
                "check_id":"check/native/example-1",
                "input_candidate":candidate,
                "models":[],
                "inference_model_id":null
            }
        }),
    ]
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
fn analysis_request(views: &[&str], analysis_views: &[&str]) -> Value {
    let mut result = observation_request(views);
    if let Some(values) = result
        .pointer_mut("/$defs/AnalysisViewV1/enum")
        .and_then(Value::as_array_mut)
    {
        values.retain(|v| v.as_str().is_some_and(|s| analysis_views.contains(&s)));
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

#[cfg(test)]
mod timeline_tests {
    use super::*;

    fn object_with_property<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
        if value.get("properties").and_then(|p| p.get(name)).is_some() {
            return Some(value);
        }
        match value {
            Value::Object(object) => object
                .values()
                .find_map(|child| object_with_property(child, name)),
            Value::Array(values) => values
                .iter()
                .find_map(|child| object_with_property(child, name)),
            _ => None,
        }
    }

    #[test]
    fn timeline_machine_schema_describes_optional_presentation() {
        let schemas = command_schemas("sessions.timeline").unwrap();
        let query = object_with_property(&schemas.request, "include_presentation").unwrap();
        assert_eq!(
            query["properties"]["include_presentation"]["type"],
            "boolean"
        );
        assert!(
            !query["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "include_presentation")
        );
        let turn = object_with_property(&schemas.response, "presentation").unwrap();
        assert!(
            !turn["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "presentation")
        );
        assert!(object_with_property(&schemas.response, "request_models").is_some());
        assert!(object_with_property(&schemas.response, "execution_attribution").is_some());
    }
}
