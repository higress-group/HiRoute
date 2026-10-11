use super::*;

fn object(required: &[&str], properties: Value) -> Value {
    json!({"type":"object","additionalProperties":false,"required":required,"properties":properties})
}

fn reference() -> Value {
    json!({"type":"string","minLength":1,"maxLength":256,"pattern":"^[^\\s\\x00-\\x1f\\x7f]+$"})
}

fn candidate() -> Value {
    object(
        &["candidate_ref", "candidate_revision"],
        json!({
            "candidate_ref":reference(),"candidate_revision":{"type":"integer","minimum":1}
        }),
    )
}

pub(super) fn request() -> Value {
    let provider = json!({"enum":["codex","claude"]});
    json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":"hiroute://contracts/cli/subscription-login-request.v1.schema.json",
        "oneOf":[
            object(&["action","provider"],json!({"action":{"enum":["list","start"]},"provider":provider})),
            object(&["action","login_ref"],json!({"action":{"enum":["status","cancel","forget"]},"login_ref":reference()})),
            object(&["action","login_ref","input_candidate"],json!({"action":{"const":"callback"},"login_ref":reference(),"input_candidate":candidate()}))
        ]
    })
}

pub(super) fn response() -> Value {
    let mut schema = object(
        &["schema", "sessions"],
        json!({
            "schema":{"const":crate::SUBSCRIPTION_LOGIN_RESULT_SCHEMA_V1},
            "sessions":{"type":"array","items":object(&["provider","login_ref","status"],json!({
                "provider":{"enum":["codex","claude"]},
                "login_ref":reference(),
                "status":{"enum":["pending","authorized","cancelled","failed","expired","forgotten"]},
                "authorization_url":{"type":"string","maxLength":8192,"description":"Start only; browser navigation, never client logging or persistence."},
                "callback_input_candidate":candidate(),
                "account_ref":reference(),
                "candidate":candidate(),
                "reason_code":{"type":"string","maxLength":128,"pattern":"^[a-z0-9_]+$"}
            }))}
        }),
    );
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$id"] = json!("hiroute://contracts/cli/subscription-login-result.v1.schema.json");
    schema
}
