use super::*;
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}
fn number() -> Value {
    json!({"type":"integer","minimum":0})
}
fn text() -> Value {
    json!({"type":"string"})
}
fn nullable(value: Value) -> Value {
    json!({"anyOf":[value,{"type":"null"}]})
}
fn array(value: Value) -> Value {
    json!({"type":"array","items":value})
}
fn schema(name: &str, mut value: Value) -> Value {
    value["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    value["$id"] = json!(format!("hiroute://contracts/cli/{name}.v1.schema.json"));
    value
}
fn execution_ref() -> Value {
    object(
        json!({"workspace_id":text(),"plan_id":text(),"content_revision":number(),"content_digest":text()}),
        &[
            "workspace_id",
            "plan_id",
            "content_revision",
            "content_digest",
        ],
    )
}
pub(super) fn files() -> Vec<(&'static str, String)> {
    let status = object(
        json!({
            "recovery_available":{"type":"boolean"},"retrieval_enabled":{"type":"boolean"},"held":nullable(object(json!({"sequence":number(),"issued_at":text(),"retrieved_at":number()}),&["sequence","issued_at","retrieved_at"])),
            "highest_accepted_sequence":nullable(number()),"last_attempt":nullable(object(json!({"at":number(),"result":{"enum":["ok","not_modified","already_current","not_published","network","size","schema","validation","older_sequence"]},"reason":nullable(text())}),&["at","result","reason"])),
            "verification":{"oneOf":[object(json!({"kind":{"enum":["validated","none"]}}),&["kind"]),object(json!({"kind":{"const":"discarded"},"reason":text()}),&["kind","reason"])]},
            "counts":object(json!({"applied":number(),"ignored_embedded":number(),"newer_client":number(),"unresolved":number()}),&["applied","ignored_embedded","newer_client","unresolved"]),
            "next_automatic_attempt_at":nullable(number()),"disclosure":text(),"disclosure_acknowledged":{"type":"boolean"}
        }),
        &[
            "recovery_available",
            "retrieval_enabled",
            "held",
            "highest_accepted_sequence",
            "last_attempt",
            "verification",
            "counts",
            "next_automatic_attempt_at",
            "disclosure",
            "disclosure_acknowledged",
        ],
    );
    let mut settings = object(
        json!({"retrieval_enabled":{"type":["boolean","null"]},"acknowledge_disclosure":{"type":["boolean","null"]}}),
        &[],
    );
    settings["minProperties"] = json!(1);
    let remove = object(
        json!({"accepted_digest":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"},"also_disable_retrieval":{"type":"boolean"}}),
        &["accepted_digest", "also_disable_retrieval"],
    );
    let removal = object(
        json!({"sources":array(object(json!({"source_id":text(),"display_name":text(),"model_configuration_ids":array(text()),"published_revisions":array(execution_ref())}),&["source_id","display_name","model_configuration_ids","published_revisions"])),"published_revisions":array(execution_ref()),"retained_sequence":nullable(number()),"retrieval_enabled":{"type":"boolean"},"digest":text()}),
        &[
            "sources",
            "published_revisions",
            "retained_sequence",
            "retrieval_enabled",
            "digest",
        ],
    );
    let localized = object(json!({"en":text(),"zh":text()}), &["en", "zh"]);
    let announcement = object(
        json!({"id":text(),"kind":{"enum":["new_model","model_update"]},"model_configuration_id":text(),"headline":localized,"note":localized,"links":array(text()),"included_on":text()}),
        &[
            "id",
            "kind",
            "model_configuration_id",
            "headline",
            "note",
            "links",
            "included_on",
        ],
    );
    let origin = json!({"oneOf":[object(json!({"kind":{"const":"embedded"},"client_version":text()}),&["kind","client_version"]),object(json!({"kind":{"const":"feed"},"sequence":number(),"included_on":text()}),&["kind","sequence","included_on"]),object(json!({"kind":{"const":"published"}}),&["kind"])]});
    let availability = json!({"oneOf":[object(json!({"kind":{"enum":["executable","needs_connection"]}}),&["kind"]),object(json!({"kind":{"const":"needs_newer_client"},"version":text()}),&["kind","version"]),object(json!({"kind":{"const":"used_by_published_plans"},"count":number()}),&["kind","count"])]});
    let reasoning = json!({"oneOf":[object(json!({"kind":{"enum":["toggle","budget"]}}),&["kind"]),object(json!({"kind":{"const":"profiles"},"profiles":array(text())}),&["kind","profiles"])]});
    let facts = object(
        json!({"publisher_id":text(),"context_tokens":number(),"tool":{"type":"boolean"},"vision":{"type":"boolean"},"reasoning":nullable(reasoning)}),
        &[
            "publisher_id",
            "context_tokens",
            "tool",
            "vision",
            "reasoning",
        ],
    );
    let models = object(
        json!({"model_configuration_id":text(),"display_name":text(),"origin":origin,"identity_stability":nullable(text()),"availability":availability,"binding_ids":array(text()),"facts":nullable(facts)}),
        &[
            "model_configuration_id",
            "display_name",
            "origin",
            "identity_stability",
            "availability",
            "binding_ids",
            "facts",
        ],
    );
    vec![
        (
            "model-feed-status.v1.schema.json",
            pretty_json(&schema("model-feed-status", status)),
        ),
        (
            "model-feed-settings.v1.schema.json",
            pretty_json(&schema("model-feed-settings", settings)),
        ),
        (
            "model-feed-remove.v1.schema.json",
            pretty_json(&schema("model-feed-remove", remove)),
        ),
        (
            "model-feed-removal-preview.v1.schema.json",
            pretty_json(&schema("model-feed-removal-preview", removal)),
        ),
        (
            "model-availability.v1.schema.json",
            pretty_json(&schema(
                "model-availability",
                object(
                    json!({"featured":array(announcement),"models":array(models)}),
                    &["featured", "models"],
                ),
            )),
        ),
    ]
}
