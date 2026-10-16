use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::GatewayAuthenticationSemanticsV1;

use super::CONNECTOR_REGISTRY_SCHEMA_V1;
use super::common::*;

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorRuntimeKind {
    BuiltinNative,
    CpaBridge,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionOrigin {
    NativeApi,
    AgentSubscription,
    FreeCatalog,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryStrategyKind {
    RemoteModels,
    RichCatalog,
    BundledCatalog,
    Union,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationKind {
    None,
    ProviderApiKey,
    ConnectorOwnedOpaque,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamProtocol {
    Responses,
    ChatCompletions,
    Messages,
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum BillingClass {
    Free,
    Subscription,
    Paid,
    Unknown,
}

impl BillingClass {
    pub const fn is_runnable(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    pub const fn requires_explicit_materialization(self) -> bool {
        matches!(self, Self::Subscription | Self::Paid)
    }
}

#[derive(
    schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FreeAccess {
    Direct,
    ApiKeyRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorDescriptorV1 {
    pub connector_id: String,
    pub revision: u64,
    pub runtime_kind: ConnectorRuntimeKind,
    pub implementation_ref: String,
    pub implementation_revision: u64,
    pub accepted_origins: BTreeSet<ConnectionOrigin>,
    pub authentication: AuthenticationKind,
    pub required_secret_slots: Vec<String>,
    pub endpoint_profile_refs: Vec<String>,
    pub catalog_adapter_ref: String,
    pub catalog_adapter_revision: u64,
    pub error_classifier_ref: String,
    pub error_classifier_revision: u64,
    pub usage_decoder_ref: String,
    pub usage_decoder_revision: u64,
    pub cache_policy_ref: String,
    pub cache_policy_revision: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolEndpointV1 {
    pub protocol_endpoint_id: String,
    pub protocol: UpstreamProtocol,
    pub base_url: String,
    pub request_path: String,
    pub adapter_ref: String,
    pub adapter_revision: u64,
    pub stable_preference: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_path: Option<String>,
    /// Exact secret rendering required by this protocol endpoint. Legacy registries omit it
    /// and may only use an explicit legacy compatibility path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication_semantics: Option<GatewayAuthenticationSemanticsV1>,
    /// Catalog-bound static protocol headers. Secret-bearing headers are represented only by
    /// `authentication_semantics`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_headers: Vec<(String, String)>,
}

impl ProtocolEndpointV1 {
    /// Authorizes a destination by exact bytes. URL normalization is intentionally not used as an
    /// equivalence relation: case, punycode/IDN spelling, trailing dots, ports, path/query changes,
    /// and redirect targets must all be independently registered.
    pub fn authorize_exact_destination(&self, candidate: &str) -> bool {
        candidate == format!("{}{}", self.base_url, self.request_path)
    }

    pub fn inventory_destination(&self) -> Option<String> {
        self.inventory_path
            .as_ref()
            .map(|path| format!("{}{}", self.base_url, path))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointProfileV1 {
    pub endpoint_profile_id: String,
    pub revision: u64,
    pub connector_id: String,
    pub connector_revision: u64,
    pub provider_platform_id: String,
    pub service_offering_id: String,
    pub entitlement_id: String,
    pub usage_scope: String,
    pub region_id: String,
    pub logical_endpoint_group: String,
    pub protocol_endpoints: Vec<ProtocolEndpointV1>,
    pub inventory_strategy: InventoryStrategyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_protocol_endpoint_id: Option<String>,
    pub verification_evidence: String,
    pub last_verified_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionOptionV1 {
    pub connection_option_id: String,
    pub display_name: String,
    pub origin: ConnectionOrigin,
    pub connector_id: String,
    pub connector_revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub billing_class: BillingClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_offer_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_verification_evidence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorRegistryBundleV1 {
    pub schema: String,
    pub registry_version: String,
    pub product_release: String,
    pub connectors: Vec<ConnectorDescriptorV1>,
    pub endpoint_profiles: Vec<EndpointProfileV1>,
    pub connection_options: Vec<ConnectionOptionV1>,
}

impl ConnectorRegistryBundleV1 {
    pub fn validate(&self) -> Result<(), ComputeContractError> {
        if self.schema != CONNECTOR_REGISTRY_SCHEMA_V1 {
            return Err(ComputeContractError::UnsupportedSchema);
        }
        validate_identifier(&self.registry_version)?;
        validate_identifier(&self.product_release)?;
        ensure_unique(
            self.connectors
                .iter()
                .map(|value| value.connector_id.as_str()),
        )?;
        ensure_unique(
            self.endpoint_profiles
                .iter()
                .map(|value| value.endpoint_profile_id.as_str()),
        )?;
        ensure_unique(
            self.connection_options
                .iter()
                .map(|value| value.connection_option_id.as_str()),
        )?;
        for connector in &self.connectors {
            validate_identifier(&connector.connector_id)?;
            validate_identifier(&connector.implementation_ref)?;
            for reference in [
                &connector.catalog_adapter_ref,
                &connector.error_classifier_ref,
                &connector.usage_decoder_ref,
                &connector.cache_policy_ref,
            ] {
                validate_identifier(reference)?;
            }
            if connector.revision == 0
                || connector.implementation_revision == 0
                || connector.catalog_adapter_revision == 0
                || connector.error_classifier_revision == 0
                || connector.usage_decoder_revision == 0
                || connector.cache_policy_revision == 0
                || connector.accepted_origins.is_empty()
                || connector.endpoint_profile_refs.is_empty()
            {
                return Err(ComputeContractError::InvalidRegistry);
            }
            ensure_unique(connector.required_secret_slots.iter().map(String::as_str))?;
            for slot in &connector.required_secret_slots {
                validate_identifier(slot)?;
            }
            match connector.authentication {
                AuthenticationKind::None if !connector.required_secret_slots.is_empty() => {
                    return Err(ComputeContractError::InvalidRegistry);
                }
                AuthenticationKind::ProviderApiKey
                    if connector.required_secret_slots.is_empty() =>
                {
                    return Err(ComputeContractError::InvalidRegistry);
                }
                AuthenticationKind::ConnectorOwnedOpaque
                    if !connector.required_secret_slots.is_empty() =>
                {
                    return Err(ComputeContractError::InvalidRegistry);
                }
                _ => {}
            }
            ensure_unique(connector.endpoint_profile_refs.iter().map(String::as_str))?;
            for endpoint_ref in &connector.endpoint_profile_refs {
                validate_identifier(endpoint_ref)?;
                if self.endpoint_profile(endpoint_ref).is_none_or(|profile| {
                    profile.connector_id != connector.connector_id
                        || profile.connector_revision != connector.revision
                }) {
                    return Err(ComputeContractError::CrossReference);
                }
            }
            if connector.runtime_kind == ConnectorRuntimeKind::CpaBridge
                && (connector.authentication != AuthenticationKind::ConnectorOwnedOpaque
                    || connector.accepted_origins
                        != BTreeSet::from([ConnectionOrigin::AgentSubscription]))
            {
                return Err(ComputeContractError::InvalidRegistry);
            }
            if connector.runtime_kind == ConnectorRuntimeKind::BuiltinNative
                && connector.authentication == AuthenticationKind::ConnectorOwnedOpaque
            {
                return Err(ComputeContractError::InvalidRegistry);
            }
        }
        for profile in &self.endpoint_profiles {
            validate_identifier(&profile.endpoint_profile_id)?;
            validate_identifier(&profile.connector_id)?;
            validate_identifier(&profile.provider_platform_id)?;
            validate_identifier(&profile.service_offering_id)?;
            validate_identifier(&profile.entitlement_id)?;
            validate_identifier(&profile.usage_scope)?;
            validate_identifier(&profile.region_id)?;
            validate_identifier(&profile.logical_endpoint_group)?;
            validate_evidence(&profile.verification_evidence)?;
            let connector = self
                .connector(&profile.connector_id)
                .ok_or(ComputeContractError::CrossReference)?;
            if profile.revision == 0
                || profile.connector_revision != connector.revision
                || !connector
                    .endpoint_profile_refs
                    .contains(&profile.endpoint_profile_id)
                || profile.protocol_endpoints.is_empty()
                || profile.last_verified_at <= 0
            {
                return Err(ComputeContractError::InvalidRegistry);
            }
            ensure_unique(
                profile
                    .protocol_endpoints
                    .iter()
                    .map(|value| value.protocol_endpoint_id.as_str()),
            )?;
            let mut stable_preferences = BTreeSet::new();
            for endpoint in &profile.protocol_endpoints {
                validate_protocol_endpoint(endpoint)?;
                match (
                    &connector.authentication,
                    &endpoint.authentication_semantics,
                ) {
                    (AuthenticationKind::None, Some(GatewayAuthenticationSemanticsV1::None))
                    | (
                        AuthenticationKind::ProviderApiKey,
                        Some(
                            GatewayAuthenticationSemanticsV1::Bearer
                            | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
                        ),
                    )
                    | (_, None) => {}
                    _ => return Err(ComputeContractError::InvalidRegistry),
                }
                if !stable_preferences.insert(endpoint.stable_preference) {
                    return Err(ComputeContractError::InvalidRegistry);
                }
            }
            match profile.inventory_strategy {
                InventoryStrategyKind::BundledCatalog => {
                    if profile.inventory_protocol_endpoint_id.is_some() {
                        return Err(ComputeContractError::InvalidRegistry);
                    }
                }
                InventoryStrategyKind::RemoteModels
                | InventoryStrategyKind::RichCatalog
                | InventoryStrategyKind::Union => {
                    let Some(endpoint_id) = &profile.inventory_protocol_endpoint_id else {
                        return Err(ComputeContractError::InvalidRegistry);
                    };
                    let endpoint = profile
                        .protocol_endpoints
                        .iter()
                        .find(|endpoint| &endpoint.protocol_endpoint_id == endpoint_id)
                        .ok_or(ComputeContractError::CrossReference)?;
                    if endpoint.inventory_path.is_none() {
                        return Err(ComputeContractError::InvalidRegistry);
                    }
                }
            }
        }
        for option in &self.connection_options {
            validate_identifier(&option.connection_option_id)?;
            if !valid_display_name(&option.display_name) {
                return Err(ComputeContractError::InvalidRegistry);
            }
            let connector = self
                .connector(&option.connector_id)
                .ok_or(ComputeContractError::CrossReference)?;
            let profile = self
                .endpoint_profile(&option.endpoint_profile_id)
                .ok_or(ComputeContractError::CrossReference)?;
            if connector.revision != option.connector_revision
                || profile.revision != option.endpoint_profile_revision
                || !connector
                    .endpoint_profile_refs
                    .contains(&option.endpoint_profile_id)
                || profile.connector_id != option.connector_id
                || profile.connector_revision != option.connector_revision
                || !connector.accepted_origins.contains(&option.origin)
                || !option.billing_class.is_runnable()
            {
                return Err(ComputeContractError::CrossReference);
            }
            match option.billing_class {
                BillingClass::Free if option.origin == ConnectionOrigin::FreeCatalog => {
                    let reference = option
                        .free_offer_ref
                        .as_deref()
                        .ok_or(ComputeContractError::InvalidRegistry)?;
                    validate_identifier(reference)?;
                    match (
                        connector.authentication,
                        option.direct_verification_evidence.as_deref(),
                    ) {
                        (AuthenticationKind::None, Some(evidence)) => {
                            validate_evidence(evidence)?;
                        }
                        (AuthenticationKind::ProviderApiKey, None) => {}
                        _ => return Err(ComputeContractError::InvalidRegistry),
                    }
                }
                BillingClass::Free => return Err(ComputeContractError::InvalidRegistry),
                _ if option.origin == ConnectionOrigin::FreeCatalog
                    || option.free_offer_ref.is_some()
                    || option.direct_verification_evidence.is_some() =>
                {
                    return Err(ComputeContractError::InvalidRegistry);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn connector(&self, connector_id: &str) -> Option<&ConnectorDescriptorV1> {
        self.connectors
            .iter()
            .find(|value| value.connector_id == connector_id)
    }

    pub fn endpoint_profile(&self, profile_id: &str) -> Option<&EndpointProfileV1> {
        self.endpoint_profiles
            .iter()
            .find(|value| value.endpoint_profile_id == profile_id)
    }

    pub fn connection_option(&self, option_id: &str) -> Option<&ConnectionOptionV1> {
        self.connection_options
            .iter()
            .find(|value| value.connection_option_id == option_id)
    }

    pub fn resolve_option(
        &self,
        option_id: &str,
    ) -> Result<ResolvedConnectionOptionV1, ComputeContractError> {
        self.resolve_validated_option(option_id)
            .map(|value| value.0)
    }

    pub fn resolve_validated_option(
        &self,
        option_id: &str,
    ) -> Result<ValidatedConnectionOptionV1, ComputeContractError> {
        self.validate()?;
        self.resolve_validated_option_inner(option_id)
    }

    /// Validates once and seals immutable exact options for a trusted catalog's lifetime.
    pub fn validated_options(
        &self,
    ) -> Result<std::collections::BTreeMap<String, ValidatedConnectionOptionV1>, ComputeContractError>
    {
        self.validate()?;
        self.connection_options
            .iter()
            .map(|option| {
                self.resolve_validated_option_inner(&option.connection_option_id)
                    .map(|value| (option.connection_option_id.clone(), value))
            })
            .collect()
    }

    fn resolve_validated_option_inner(
        &self,
        option_id: &str,
    ) -> Result<ValidatedConnectionOptionV1, ComputeContractError> {
        // The option ID is the sole user-selected authority. There is no fallback by hostname,
        // provider label, URL, or protocol.
        validate_identifier(option_id)?;
        let option = self
            .connection_option(option_id)
            .ok_or(ComputeContractError::UnknownConnectionOption)?;
        let connector = self
            .connector(&option.connector_id)
            .ok_or(ComputeContractError::CrossReference)?;
        let endpoint_profile = self
            .endpoint_profile(&option.endpoint_profile_id)
            .ok_or(ComputeContractError::CrossReference)?;
        Ok(ValidatedConnectionOptionV1(ResolvedConnectionOptionV1 {
            option: option.clone(),
            connector: connector.clone(),
            endpoint_profile: endpoint_profile.clone(),
        }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConnectionOptionV1 {
    pub option: ConnectionOptionV1,
    pub connector: ConnectorDescriptorV1,
    pub endpoint_profile: EndpointProfileV1,
}

/// Constructible only through full registry validation; no mutable access or deserialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedConnectionOptionV1(ResolvedConnectionOptionV1);

impl std::ops::Deref for ValidatedConnectionOptionV1 {
    type Target = ResolvedConnectionOptionV1;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn validate_protocol_endpoint(endpoint: &ProtocolEndpointV1) -> Result<(), ComputeContractError> {
    validate_identifier(&endpoint.protocol_endpoint_id)?;
    validate_identifier(&endpoint.adapter_ref)?;
    let host = endpoint
        .base_url
        .strip_prefix("https://")
        .unwrap_or_default();
    let valid_host = !host.is_empty()
        && host.len() <= 253
        && !host.ends_with('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        });
    if endpoint.adapter_revision == 0
        || !endpoint.base_url.starts_with("https://")
        || !valid_host
        || endpoint.base_url.ends_with('/')
        || endpoint.base_url.contains('@')
        || endpoint.base_url.contains('?')
        || endpoint.base_url.contains('#')
        || endpoint.base_url[8..].contains('/')
        || endpoint.base_url[8..].contains(':')
        || endpoint.base_url[8..].contains("..")
        || endpoint
            .base_url
            .bytes()
            .any(|byte| byte.is_ascii_uppercase() || !byte.is_ascii())
        || !valid_path(&endpoint.request_path)
        || endpoint
            .inventory_path
            .as_deref()
            .is_some_and(|path| !valid_path(path))
    {
        return Err(ComputeContractError::InvalidEndpoint);
    }
    let required_names = endpoint
        .required_headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    let authentication_header = match &endpoint.authentication_semantics {
        Some(GatewayAuthenticationSemanticsV1::Bearer) => Some("authorization"),
        Some(GatewayAuthenticationSemanticsV1::ApiKeyHeader { header }) => Some(header.as_str()),
        Some(GatewayAuthenticationSemanticsV1::None) | None => None,
    };
    if endpoint.required_headers.len() > 16
        || required_names.len() != endpoint.required_headers.len()
        || endpoint.required_headers.iter().any(|(name, value)| {
            !valid_header_name(name)
                || !safe_static_protocol_header(name)
                || authentication_header == Some(name.as_str())
                || value.is_empty()
                || value.len() > 256
                || value.trim() != value
                || value.chars().any(char::is_control)
        })
    {
        return Err(ComputeContractError::InvalidEndpoint);
    }
    if let Some(GatewayAuthenticationSemanticsV1::ApiKeyHeader { header }) =
        &endpoint.authentication_semantics
        && (!valid_header_name(header) || !safe_authentication_header(header))
    {
        return Err(ComputeContractError::InvalidEndpoint);
    }
    Ok(())
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn safe_authentication_header(value: &str) -> bool {
    !matches!(
        value,
        "authorization"
            | "connection"
            | "content-length"
            | "cookie"
            | "host"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn safe_static_protocol_header(value: &str) -> bool {
    matches!(
        value,
        "anthropic-version" | "anthropic-beta" | "openai-beta"
    )
}

fn valid_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('?')
        && !value.contains('#')
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~')
        })
}
