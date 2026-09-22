//! Historical resident login-item intent, retained for owned-item cleanup on restore.
//!
//! New settings saves do not inspect or register a macOS login item. Older settings
//! Operations may have journaled a host-executed registration; the last restore observes
//! its removal and the host compensates that removal if the apply never commits.
use hiroute_application_api::{AgentLoginItemDeclarationV2, AgentLoginItemStatusV2};
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    CanonicalDigest, ExternalEffectIntentV1, OperationValidationError, OwnedEffectKind,
    OwnedEffectV1, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

const SCHEMA: &str = "hiroute.settings-login-item/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsLoginItemPayload {
    schema: String,
    pub context_id: String,
    pub before: AgentLoginItemStatusV2,
    pub after: AgentLoginItemStatusV2,
    pub created: bool,
}

pub fn is_settings_login_item(intent: &ExternalEffectIntentV1) -> bool {
    intent.effect_id() == "agent-connection-login-item"
}

/// Seals the host's login-item observation into the settings plan. An establishment must
/// prove an active item and a removal must prove the host left it not active; ownership for
/// removals is proven by journal facts on the backend, never by the declaration itself.
pub fn settings_login_item_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    declaration: &AgentLoginItemDeclarationV2,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings
        || !(declaration.establishes_resident_service() || declaration.removes_resident_service())
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = SettingsLoginItemPayload {
        schema: SCHEMA.into(),
        context_id: context_id.into(),
        before: declaration.before,
        after: declaration.after,
        created: declaration.created,
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::LoginItem,
        None,
        &payload,
        0o600,
    )?;
    decode_settings_login_item(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn decode_settings_login_item(
    intent: &ExternalEffectIntentV1,
) -> PortResult<SettingsLoginItemPayload> {
    let invalid = || PortError::new(PortErrorCode::InvalidData, "login.item.intent");
    let value = intent.desired();
    if intent.effect_id() != "agent-connection-login-item"
        || intent.kind() != OwnedEffectKind::LoginItem
        || intent.before_fingerprint().is_some()
        || intent.desired_mode() != 0o600
        || value["transaction"] != "settings"
        || value["role"] != "login_item"
    {
        return Err(invalid());
    }
    let payload: SettingsLoginItemPayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    let declaration = AgentLoginItemDeclarationV2 {
        before: payload.before,
        after: payload.after,
        created: payload.created,
    };
    if payload.schema != SCHEMA
        || !super::settings::identity(&payload.context_id)
        || !(declaration.establishes_resident_service() || declaration.removes_resident_service())
    {
        return Err(invalid());
    }
    Ok(payload)
}

/// The durable owned-effect record for the host-executed login item. The before/after
/// fingerprints bind the observed statuses; compensation metadata exists exactly when this
/// operation changed the item, so rollback never removes a pre-existing user login item and a
/// failed removal re-registers the item this feature owns.
pub fn settings_login_item_effect(intent: &ExternalEffectIntentV1) -> PortResult<OwnedEffectV1> {
    let payload = decode_settings_login_item(intent)?;
    let before = CanonicalDigest::of_bytes(payload.before.as_str().as_bytes());
    let after = CanonicalDigest::of_bytes(payload.after.as_str().as_bytes());
    let compensation = if payload.created {
        json!({"revert": "unregister", "owner": "desktop-host"})
    } else if payload.removes_resident_service() {
        json!({"revert": "register", "owner": "desktop-host"})
    } else {
        json!({})
    };
    Ok(OwnedEffectV1 {
        effect_id: intent.effect_id().to_owned(),
        kind: OwnedEffectKind::LoginItem,
        target: intent.target().to_owned(),
        before_fingerprint: Some(before),
        after_fingerprint: Some(after),
        compensation: compensation.into(),
    })
}

impl SettingsLoginItemPayload {
    pub fn created(&self) -> bool {
        self.created
    }

    fn removes_resident_service(&self) -> bool {
        AgentLoginItemDeclarationV2 {
            before: self.before,
            after: self.after,
            created: self.created,
        }
        .removes_resident_service()
    }
}
