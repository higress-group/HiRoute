//! Safe management of CPA-owned subscription sign-ins. OAuth material travels only through
//! the protected input transport; authorization never enables a source or selects models.

use serde::{Deserialize, Serialize};

use crate::ComputeCandidateRefV2;

pub const MANAGE_SUBSCRIPTION_LOGIN_OPERATION_V1: &str = "ManageSubscriptionLogin";
pub const SUBSCRIPTION_LOGIN_RESULT_SCHEMA_V1: &str = "hiroute.subscription-login-result/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionLoginProviderV1 {
    Codex,
    Claude,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeSubscriptionLoginRequestV1 {
    List {
        provider: SubscriptionLoginProviderV1,
    },
    Start {
        provider: SubscriptionLoginProviderV1,
    },
    Status {
        login_ref: String,
    },
    Callback {
        login_ref: String,
        input_candidate: ComputeCandidateRefV2,
    },
    Cancel {
        login_ref: String,
    },
    Forget {
        login_ref: String,
    },
}

impl ComputeSubscriptionLoginRequestV1 {
    pub fn valid(&self) -> bool {
        let login_ref = match self {
            Self::List { .. } | Self::Start { .. } => return true,
            Self::Status { login_ref }
            | Self::Cancel { login_ref }
            | Self::Forget { login_ref } => login_ref,
            Self::Callback {
                login_ref,
                input_candidate,
            } => {
                if input_candidate.validate_shape().is_err() {
                    return false;
                }
                login_ref
            }
        };
        !login_ref.is_empty()
            && login_ref.len() <= 256
            && !login_ref
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionLoginStatusV1 {
    Pending,
    Authorized,
    Cancelled,
    Failed,
    Expired,
    Forgotten,
}

/// An account reference is an opaque local identity, never a credential-store path or token.
/// `authorization_url` is returned only by Start and must not be logged or persisted by clients.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionLoginSessionV1 {
    pub provider: SubscriptionLoginProviderV1,
    pub login_ref: String,
    pub status: SubscriptionLoginStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_input_candidate: Option<ComputeCandidateRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<ComputeCandidateRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

/// List returns zero or more sessions. Every other action returns exactly its addressed session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionLoginResultV1 {
    pub schema: String,
    pub sessions: Vec<ComputeSubscriptionLoginSessionV1>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn login_callback_accepts_only_a_protected_input_reference() {
        let mut request = json!({
            "action": "callback", "login_ref": "login/one",
            "input_candidate": {"candidate_ref":"candidate/callback/one","candidate_revision":1}
        });
        assert!(
            serde_json::from_value::<ComputeSubscriptionLoginRequestV1>(request.clone())
                .unwrap()
                .valid()
        );
        for forbidden in [
            "code",
            "callback_url",
            "refresh_token",
            "access_token",
            "auth_dir",
        ] {
            request[forbidden] = json!("must-not-cross-control");
            assert!(
                serde_json::from_value::<ComputeSubscriptionLoginRequestV1>(request.clone())
                    .is_err()
            );
            request.as_object_mut().unwrap().remove(forbidden);
        }
    }

    #[test]
    fn forgetting_is_bound_to_an_exact_login_and_never_an_account_selector() {
        assert!(
            serde_json::from_value::<ComputeSubscriptionLoginRequestV1>(json!({
                "action":"forget", "provider":"codex", "account_ref":"account/one"
            }))
            .is_err()
        );
        assert!(
            !ComputeSubscriptionLoginRequestV1::Forget {
                login_ref: " ".into()
            }
            .valid()
        );
        assert!(
            ComputeSubscriptionLoginRequestV1::Forget {
                login_ref: "login/one".into()
            }
            .valid()
        );
    }
}
