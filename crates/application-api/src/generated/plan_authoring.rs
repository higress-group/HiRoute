//! JSON schemas for the typed editor input; no arbitrary patch or authority fields.
use super::*;
fn object(required: &[&str], properties: Value) -> Value {
    json!({"type":"object","additionalProperties":false,"required":required,"properties":properties})
}
fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/$defs/{name}")})
}
fn definitions() -> Value {
    let selection = object(
        &["binding_id"],
        json!({"binding_id":{"type":"string","minLength":1},"reasoning":{"oneOf":[
            object(&["kind","profile"],json!({"kind":{"const":"profile"},"profile":{"type":"string","minLength":1}})),
            object(&["kind","enabled"],json!({"kind":{"const":"toggle"},"enabled":{"type":"boolean"}})),
            object(&["kind","tokens"],json!({"kind":{"const":"budget"},"tokens":{"type":"integer","minimum":1,"maximum":4294967295u64}}))
        ]}}),
    );
    let candidates = json!({"type":"array","maxItems":128,"items":reference("selection")});
    let requirements = object(
        &[],
        json!({"tool":{"type":"boolean"},"vision":{"type":"boolean"},"streaming":{"type":"boolean"},
        "minimum_context_tokens":{"type":"integer","minimum":0,"maximum":10000000},"minimum_output_tokens":{"type":"integer","minimum":0,"maximum":10000000}}),
    );
    let limits = object(
        &[
            "maximum_attempts",
            "request_timeout_ms",
            "attempt_timeout_ms",
        ],
        json!({
        "context_window_tokens":{"type":"integer","minimum":1,"maximum":9223372036854775807u64},
        "maximum_attempts":{"type":"integer","minimum":1,"maximum":64},"request_timeout_ms":{"type":"integer","minimum":1000,"maximum":3600000},"attempt_timeout_ms":{"type":"integer","minimum":1000,"maximum":3600000}}),
    );
    let auth = object(
        &["name", "value_secret_ref"],
        json!({"name":{"type":"string","minLength":1,"maxLength":128},"value_secret_ref":{"type":"string","minLength":1,"maxLength":256}}),
    );
    let endpoint = json!({"type":"string","maxLength":2048});
    let timeout = json!({"type":"integer","minimum":1,"maximum":3600000});
    let service = object(
        &["id", "revision", "name", "connection"],
        json!({
            "id":{"type":"string","maxLength":128},"revision":{"type":"integer","minimum":1},"name":{"type":"string","maxLength":128},
            "connection":{"oneOf":[
                object(&["kind","provider","model","endpoint","timeout_ms","auth_header"],json!({"kind":{"const":"system_one"},"provider":{"type":"string","maxLength":128},"model":{"type":"string","maxLength":256},"endpoint":endpoint,"timeout_ms":timeout,"auth_header":auth})),
                object(&["kind","endpoint","timeout_ms"],json!({"kind":{"const":"custom"},"endpoint":endpoint,"timeout_ms":timeout,"auth_header":{"oneOf":[{"type":"null"},auth]}}))
            ]}
        }),
    );
    let classifier = json!({"oneOf":[
        object(&["kind"], json!({"kind":{"const":"local_rules"}})),
        object(&["kind","service"],json!({"kind":{"const":"decision_service"},"service":service}))
    ]});
    let prompt = json!({"type":"string"});
    let threshold = json!({"type":"integer","minimum":0,"maximum":1000});
    let judgment = object(
        &["degree", "competence"],
        json!({
            "degree":object(&["simple_threshold_millis","instructions","simple","complex"],json!({"simple_threshold_millis":threshold,"instructions":prompt,"simple":prompt,"complex":prompt})),
            "competence":object(&["floor_millis","instructions","criteria"],json!({"floor_millis":threshold,"instructions":prompt,"criteria":{"type":"array","minItems":3,"maxItems":3,"items":prompt}}))
        }),
    );
    let branch = object(
        &[
            "id",
            "name",
            "condition",
            "candidates",
            "primary_candidates",
        ],
        json!({
            "id":{"type":"string","minLength":1,"maxLength":128,"not":{"const":"smart_saving"}},"name":{"type":"string","maxLength":128},"condition":prompt,
            "candidates":reference("candidates"),"primary_candidates":reference("candidates"),"judgment":{"oneOf":[{"type":"null"},reference("judgment")]}
        }),
    );
    let routing = object(
        &[
            "classifier",
            "branches",
            "default_branch_id",
            "judgment",
            "reselect_on_user_message",
        ],
        json!({
            "classifier":classifier,"branches":{"type":"array","maxItems":16,"items":branch},"default_branch_id":{"type":"string","maxLength":128},"judgment":reference("judgment"),"reselect_on_user_message":{"type":"boolean"}
        }),
    );
    let smart = object(
        &[
            "economy",
            "primary",
            "judgment",
            "reselect_on_user_message",
            "classifier",
            "complex_keywords",
        ],
        json!({"economy":reference("candidates"),"primary":reference("candidates"),"judgment":reference("judgment"),"reselect_on_user_message":{"type":"boolean"},"classifier":classifier,"complex_keywords":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":64}}}),
    );
    let free = object(
        &["candidates", "primary", "primary_fallback"],
        json!({"candidates":reference("candidates"),"primary":reference("candidates"),"primary_fallback":{"type":"boolean"}}),
    );
    let editor = object(
        &[
            "schema",
            "display_name",
            "purpose",
            "mode",
            "candidates",
            "smart",
            "free",
            "delegation_enabled",
            "requirements",
            "limits",
        ],
        json!({
            "schema":{"const":"hiroute.plan-editor/v2"},"display_name":{"type":"string","maxLength":128},"purpose":{"type":"string","maxLength":512},
            "custom_alias":{"type":"string","maxLength":64},"mode":{"enum":["fixed_model","smart_saving","custom_branches","free_first"]},"branch_routing":{"oneOf":[{"type":"null"},routing]},
            "candidates":reference("candidates"),"smart":smart,"free":free,"delegation_enabled":{"type":"boolean"},"requirements":requirements,"limits":limits,
            "work":object(&["harness","protocol"],json!({"harness":{"enum":["codex_cli","claude_code","qoder_cli","pi","deepseek_harness"]},"protocol":{"enum":["responses","messages"]}}))
        }),
    );
    let draft = object(
        &["schema", "workspace_id", "draft_id", "revision", "editor"],
        json!({
        "schema":{"const":"hiroute.plan-draft/v1"},"workspace_id":{"type":"string","minLength":1},"draft_id":{"type":"string","minLength":1},"revision":{"type":"integer","minimum":1},"plan_id":{"type":"string","minLength":1},"base_head_revision":{"type":"integer","minimum":1},"editor":reference("editor")}),
    );
    let content = object(
        &["schema", "target", "editor"],
        json!({"schema":{"const":"hiroute.plan-content-change/v2"},"target":{"oneOf":[
        object(&["intent","creation_key"],json!({"intent":{"const":"create"},"creation_key":{"type":"string","minLength":1,"maxLength":128}})),
        object(&["intent","plan_id","expected_head_revision"],json!({"intent":{"const":"update"},"plan_id":{"type":"string","minLength":1},"expected_head_revision":{"type":"integer","minimum":1}}))]},
        "editor":reference("editor"),"consumed_draft":{"oneOf":[{"type":"null"},object(&["draft_id","revision"],json!({"draft_id":{"type":"string","minLength":1},"revision":{"type":"integer","minimum":1}}))]}}),
    );
    let lifecycle = object(
        &["schema", "plan_id", "expected_head_revision", "status"],
        json!({"schema":{"const":"hiroute.plan-lifecycle-change/v1"},"plan_id":{"type":"string","minLength":1},"expected_head_revision":{"type":"integer","minimum":1},"status":{"enum":["enabled","disabled","deleted"]}}),
    );
    let draft_change = object(
        &["schema", "workspace_id", "draft_id", "action"],
        json!({"schema":{"const":"hiroute.plan-draft-change/v1"},"workspace_id":{"type":"string","minLength":1},"draft_id":{"type":"string","minLength":1},"expected_revision":{"type":["integer","null"],"minimum":1},"action":{"oneOf":[object(&["kind","draft"],json!({"kind":{"const":"save"},"draft":reference("draft")})),object(&["kind"],json!({"kind":{"const":"discard"}}))]}}),
    );
    let catalog_query = object(
        &[],
        json!({"limit":{"type":"integer","minimum":1,"maximum":128,"default":32},
        "cursor":{"oneOf":[{"type":"null"},object(&["snapshot_digest","offset"],json!({"snapshot_digest":{"type":"string","minLength":1},"offset":{"type":"integer","minimum":1}}))]}}),
    );
    json!({"judgment":judgment,"catalog_query":catalog_query,"selection":selection,"candidates":candidates,"editor":editor,"draft":draft,"content_change":content,"lifecycle_change":lifecycle,"draft_change":draft_change})
}
pub(super) fn files() -> Vec<(&'static str, String)> {
    [
        ("agent-plan-catalog-query.v2.schema.json", "catalog_query"),
        ("plan-editor.v2.schema.json", "editor"),
        ("plan-content-change.v2.schema.json", "content_change"),
        ("plan-lifecycle-change.v1.schema.json", "lifecycle_change"),
        ("plan-draft-change.v1.schema.json", "draft_change"),
    ].into_iter().map(|(path, name)| (path, pretty_json(&json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema", "$id":format!("hiroute://contracts/cli/{path}"),
        "$ref":format!("#/$defs/{name}"), "$defs": definitions(),
    })))).collect()
}
