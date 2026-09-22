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
    let classifier = json!({"oneOf":[
        object(&["kind"], json!({"kind":{"const":"local_rules"}})),
        object(
            &["kind","endpoint","timeout_ms"],
            json!({
                "kind":{"const":"rest"},
                "endpoint":{"type":"string","minLength":1,"maxLength":2048},
                "timeout_ms":{"type":"integer","minimum":1,"maximum":3600000},
                "auth_header":object(
                    &["name","value_secret_ref"],
                    json!({
                        "name":{"type":"string","minLength":1,"maxLength":128},
                        "value_secret_ref":{"type":"string","minLength":1,"maxLength":256}
                    })
                )
            })
        )
    ]});
    let smart = object(
        &[
            "economy",
            "primary",
            "primary_fallback",
            "classifier",
            "complex_keywords",
        ],
        json!({"economy":reference("candidates"),"primary":reference("candidates"),"primary_fallback":{"type":"boolean"},"classifier":classifier,"complex_keywords":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":64}}}),
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
            "custom_alias":{"type":"string","maxLength":64},"mode":{"enum":["fixed_model","smart_saving","free_first"]},
            "candidates":reference("candidates"),"smart":smart,"free":free,"delegation_enabled":{"type":"boolean"},"requirements":requirements,"limits":limits,
            "work":object(&["harness","protocol"],json!({"harness":{"enum":["codex_cli","claude_code"]},"protocol":{"enum":["responses","messages"]}}))
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
    json!({"catalog_query":catalog_query,"selection":selection,"candidates":candidates,"editor":editor,"draft":draft,"content_change":content,"lifecycle_change":lifecycle,"draft_change":draft_change})
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
