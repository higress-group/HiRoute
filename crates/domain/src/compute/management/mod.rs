//! Durable user-managed compute-source facts.
//!
//! This aggregate is the management truth for registered, manually configured, and connector
//! owned sources. It contains only opaque credential references and keyed fingerprints. Secret
//! bytes, input slots, filesystem locators, and OAuth capabilities have no representation here.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CanonicalDigest, ChangeSpecV1, CredentialRefV1, GatewayAuthenticationSemanticsV1,
    GatewayHeaderSemanticsV1, MaterializationState, NativeReasoningCapabilityV1, PortResult,
    RevisionSetV1, UpstreamProtocol, WorkspaceId,
};

pub const COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2: &str = "hiroute.compute-management-source/v3";
pub const COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V2: &str = "hiroute.compute-management-mutation/v2";
pub const COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2: &str = "hiroute.compute-management-change/v2";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeManagementFactBasisV2 {
    RegisteredCatalog,
    RuntimeFallback,
    Observed,
    UserDeclared,
    ConnectorVerified,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementFactValueV2<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    pub basis: ComputeManagementFactBasisV2,
}

impl<T> ComputeManagementFactValueV2<T> {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        match (&self.value, self.basis) {
            (None, ComputeManagementFactBasisV2::Unknown)
            | (
                Some(_),
                ComputeManagementFactBasisV2::RegisteredCatalog
                | ComputeManagementFactBasisV2::RuntimeFallback
                | ComputeManagementFactBasisV2::Observed
                | ComputeManagementFactBasisV2::UserDeclared
                | ComputeManagementFactBasisV2::ConnectorVerified,
            ) => Ok(()),
            _ => Err(ComputeManagementErrorV2::InvalidCapability),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeManagementMembershipV2 {
    Catalog,
    Observed,
    UserDeclared,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedCapabilitiesV2 {
    pub tool: ComputeManagementFactValueV2<bool>,
    pub vision: ComputeManagementFactValueV2<bool>,
    pub streaming: ComputeManagementFactValueV2<bool>,
    pub context_tokens: ComputeManagementFactValueV2<u64>,
    pub max_output_tokens: ComputeManagementFactValueV2<u64>,
    pub native_reasoning: ComputeManagementFactValueV2<NativeReasoningCapabilityV1>,
}

impl ComputeManagedCapabilitiesV2 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        self.tool.validate()?;
        self.vision.validate()?;
        self.streaming.validate()?;
        self.context_tokens.validate()?;
        self.max_output_tokens.validate()?;
        self.native_reasoning.validate()?;
        if self
            .native_reasoning
            .value
            .as_ref()
            .is_some_and(|value| value.validate().is_err())
            || self.context_tokens.value == Some(0)
            || self.max_output_tokens.value == Some(0)
        {
            return Err(ComputeManagementErrorV2::InvalidCapability);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedModelV2 {
    pub model_ref: String,
    pub binding_id: String,
    pub revision: u64,
    pub upstream_model_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_configuration_id: Option<String>,
    pub membership: ComputeManagementMembershipV2,
    /// Whether the latest verified connector/catalog facts still authorize this saved member for
    /// execution. Membership and binding identity are retained when this becomes false.
    pub execution_eligible: bool,
    pub capabilities: ComputeManagedCapabilitiesV2,
    pub capability_evidence_digest: CanonicalDigest,
}

impl ComputeManagedModelV2 {
    fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        for value in [
            &self.model_ref,
            &self.binding_id,
            &self.upstream_model_id,
            &self.display_name,
        ] {
            validate_text(value)?;
        }
        if self.revision == 0
            || self
                .catalog_configuration_id
                .as_ref()
                .is_some_and(|value| validate_text(value).is_err())
            || !valid_digest(&self.capability_evidence_digest)
        {
            return Err(ComputeManagementErrorV2::InvalidModel);
        }
        self.capabilities.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeManagementProvenanceV2 {
    Registered {
        connection_option_id: String,
        registry_version: String,
        catalog_digest: CanonicalDigest,
    },
    UserConfigured {
        configuration_revision: u64,
        evidence_digest: CanonicalDigest,
    },
    ConnectorOwned {
        connector_id: String,
        account_ref: String,
    },
}

impl ComputeManagementProvenanceV2 {
    pub const fn is_native(&self) -> bool {
        matches!(self, Self::Registered { .. } | Self::UserConfigured { .. })
    }

    pub const fn is_connector_owned(&self) -> bool {
        matches!(self, Self::ConnectorOwned { .. })
    }

    fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        match self {
            Self::Registered {
                connection_option_id,
                registry_version,
                catalog_digest,
            } => {
                validate_identifier(connection_option_id)?;
                validate_identifier(registry_version)?;
                if !valid_digest(catalog_digest) {
                    return Err(ComputeManagementErrorV2::InvalidProvenance);
                }
            }
            Self::UserConfigured {
                configuration_revision,
                evidence_digest,
            } => {
                if *configuration_revision == 0 || !valid_digest(evidence_digest) {
                    return Err(ComputeManagementErrorV2::InvalidProvenance);
                }
            }
            Self::ConnectorOwned {
                connector_id,
                account_ref,
            } => {
                validate_identifier(connector_id)?;
                validate_identifier(account_ref)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementTargetV2 {
    pub scheme: String,
    pub authority: String,
    pub port: u16,
    pub request_path: String,
    pub upstream_protocol: UpstreamProtocol,
    pub protocol_profile_id: String,
    pub protocol_profile_revision: u64,
}

impl ComputeManagementTargetV2 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        let https = self.scheme.eq_ignore_ascii_case("https");
        let loopback_http = self.scheme.eq_ignore_ascii_case("http")
            && self
                .authority
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if (!https && !loopback_http)
            || self.port == 0
            || !valid_authority(&self.authority)
            || !valid_request_path(&self.request_path)
            || validate_identifier(&self.protocol_profile_id).is_err()
            || self.protocol_profile_revision == 0
        {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        Ok(())
    }

    /// Opaque destination identity used by `CredentialRefV1`; it is deliberately not a URL.
    pub fn credential_destination(&self) -> Result<String, ComputeManagementErrorV2> {
        self.validate()?;
        let digest = CanonicalDigest::of(&(
            "hiroute.compute-management-target/v2",
            self.scheme.to_ascii_lowercase(),
            self.authority.to_ascii_lowercase(),
            self.port,
            &self.request_path,
            self.upstream_protocol,
            &self.protocol_profile_id,
            self.protocol_profile_revision,
        ))
        .map_err(|_| ComputeManagementErrorV2::InvalidTarget)?;
        Ok(format!("compute-target/{}", digest_suffix(&digest, 32)))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagedCredentialV2 {
    pub key_id: String,
    pub credential: CredentialRefV1,
    pub fingerprint: CanonicalDigest,
    pub ordinal: u32,
    pub enabled: bool,
}

impl ComputeManagedCredentialV2 {
    fn validate_for(
        &self,
        source_id: &str,
        destinations: &BTreeSet<String>,
    ) -> Result<(), ComputeManagementErrorV2> {
        validate_identifier(&self.key_id)?;
        if self.key_id != self.credential.credential_id()
            || self.credential.owner_scope() != format!("source/{source_id}")
            || self.credential.subject() != "hirouted"
            || self.credential.purpose() != "provider-auth"
            || self.credential.allowed_destinations() != destinations
            || self.credential.generation() == 0
            || !valid_digest(&self.fingerprint)
        {
            return Err(ComputeManagementErrorV2::InvalidCredential);
        }
        Ok(())
    }
}

/// A second or third exact protocol endpoint for a Native source. The existing source target is
/// its first endpoint; CPA sources never have these records.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeNativeEndpointV3 {
    pub target: ComputeManagementTargetV2,
    pub authentication: GatewayAuthenticationSemanticsV1,
    pub recheck: Option<ComputeNativeRecheckDescriptorV2>,
}

impl ComputeNativeEndpointV3 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        self.target.validate()?;
        validate_authentication(&self.authentication)?;
        if self
            .recheck
            .as_ref()
            .is_some_and(|value| value.validate().is_err())
        {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementValidationV2 {
    pub approval_operation_id: String,
    pub validation_ref: String,
    pub validation_revision: u64,
}

impl ComputeManagementValidationV2 {
    fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        validate_identifier(&self.approval_operation_id)?;
        validate_identifier(&self.validation_ref)?;
        if self.validation_revision == 0 {
            return Err(ComputeManagementErrorV2::InvalidValidation);
        }
        Ok(())
    }
}

/// Non-secret native probe context retained only in the durable source aggregate. Public
/// management projections deliberately omit it; a saved-source recheck consumes it server-side.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeNativeRecheckDescriptorV2 {
    /// Display only; does not confer provider, capability, billing or credential authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_template_id: Option<String>,
    pub inventory_path: Option<String>,
    pub protocol_header_semantics: GatewayHeaderSemanticsV1,
}

impl ComputeNativeRecheckDescriptorV2 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        if self
            .display_template_id
            .as_ref()
            .is_some_and(|id| validate_identifier(id).is_err())
        {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        let required_names = self
            .protocol_header_semantics
            .required_headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<BTreeSet<_>>();
        let forbidden_names = self
            .protocol_header_semantics
            .forbidden_forward_headers
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if self
            .inventory_path
            .as_ref()
            .is_some_and(|path| !valid_request_path(path))
            || self.protocol_header_semantics.content_type != "application/json"
            || self.protocol_header_semantics.required_headers.len() > 32
            || self
                .protocol_header_semantics
                .forbidden_forward_headers
                .len()
                > 32
            || self
                .protocol_header_semantics
                .required_headers
                .iter()
                .any(|(name, value)| {
                    !valid_header_name(name)
                        || !safe_recheck_header(name)
                        || !valid_header_value(value)
                })
            || self
                .protocol_header_semantics
                .forbidden_forward_headers
                .iter()
                .any(|name| {
                    !valid_header_name(name) || name.bytes().any(|byte| byte.is_ascii_uppercase())
                })
            || required_names.len() != self.protocol_header_semantics.required_headers.len()
            || forbidden_names.len()
                != self
                    .protocol_header_semantics
                    .forbidden_forward_headers
                    .len()
            || required_names
                .iter()
                .any(|name| forbidden_names.contains(name))
        {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementSourceV2 {
    pub schema: String,
    pub source_id: String,
    pub revision: u64,
    pub lineage_digest: CanonicalDigest,
    pub display_name: String,
    pub provenance: ComputeManagementProvenanceV2,
    pub target: ComputeManagementTargetV2,
    pub authentication: GatewayAuthenticationSemanticsV1,
    pub state: MaterializationState,
    pub models: Vec<ComputeManagedModelV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_recheck: Option<ComputeNativeRecheckDescriptorV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_native_endpoints: Vec<ComputeNativeEndpointV3>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<ComputeManagedCredentialV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ComputeManagementValidationV2>,
    pub last_candidate_ref: String,
    pub last_candidate_revision: u64,
}

impl ComputeManagementSourceV2 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        if self.schema != COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2
            || self.revision == 0
            || self.last_candidate_revision == 0
            || !valid_digest(&self.lineage_digest)
        {
            return Err(ComputeManagementErrorV2::UnsupportedSchema);
        }
        validate_identifier(&self.source_id)?;
        validate_identifier(&self.last_candidate_ref)?;
        validate_text(&self.display_name)?;
        self.provenance.validate()?;
        self.target.validate()?;
        validate_authentication(&self.authentication)?;
        if self.provenance.is_connector_owned() && !self.additional_native_endpoints.is_empty() {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        let mut protocols = BTreeSet::from([self.target.upstream_protocol]);
        for endpoint in &self.additional_native_endpoints {
            endpoint.validate()?;
            if !protocols.insert(endpoint.target.upstream_protocol) {
                return Err(ComputeManagementErrorV2::DuplicateIdentity);
            }
            if (self.authentication == GatewayAuthenticationSemanticsV1::None)
                != (endpoint.authentication == GatewayAuthenticationSemanticsV1::None)
            {
                return Err(ComputeManagementErrorV2::InvalidCredential);
            }
        }
        if self
            .native_recheck
            .as_ref()
            .is_some_and(|descriptor| descriptor.validate().is_err())
            || self.provenance.is_connector_owned() && self.native_recheck.is_some()
        {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }

        if self.models.is_empty() {
            return Err(ComputeManagementErrorV2::InvalidModel);
        }
        let mut model_refs = BTreeSet::new();
        let mut binding_ids = BTreeSet::new();
        for model in &self.models {
            model.validate()?;
            if !model_refs.insert(&model.model_ref) || !binding_ids.insert(&model.binding_id) {
                return Err(ComputeManagementErrorV2::DuplicateIdentity);
            }
        }

        let destinations = self.native_destinations()?;
        let mut key_ids = BTreeSet::new();
        let mut fingerprints = BTreeSet::new();
        for (index, credential) in self.credentials.iter().enumerate() {
            credential.validate_for(&self.source_id, &destinations)?;
            if credential.ordinal as usize != index
                || !key_ids.insert(&credential.key_id)
                || !fingerprints.insert(credential.fingerprint.as_str())
            {
                return Err(ComputeManagementErrorV2::DuplicateIdentity);
            }
        }

        let native = self.provenance.is_native();
        let connector_owned = self.provenance.is_connector_owned();
        let requires_native_key = native
            && (self.authentication != GatewayAuthenticationSemanticsV1::None
                || self.additional_native_endpoints.iter().any(|endpoint| {
                    endpoint.authentication != GatewayAuthenticationSemanticsV1::None
                }));
        if connector_owned && !self.credentials.is_empty()
            || !requires_native_key && native && !self.credentials.is_empty()
            || native != self.validation.is_none()
            || self
                .validation
                .as_ref()
                .is_some_and(|validation| validation.validate().is_err())
        {
            return Err(ComputeManagementErrorV2::InvalidCredential);
        }

        let has_enabled_key = self.credentials.iter().any(|credential| credential.enabled);
        let valid_state = match self.state {
            MaterializationState::Ready => {
                connector_owned
                    || matches!(self.authentication, GatewayAuthenticationSemanticsV1::None)
                    || requires_native_key && has_enabled_key
            }
            MaterializationState::NeedsCredential => {
                requires_native_key && self.credentials.is_empty()
            }
            MaterializationState::NeedsAuthorization => connector_owned,
            MaterializationState::Disabled => true,
        };
        if !valid_state {
            return Err(ComputeManagementErrorV2::InvalidState);
        }
        Ok(())
    }

    pub fn native_destinations(&self) -> Result<BTreeSet<String>, ComputeManagementErrorV2> {
        let mut destinations = BTreeSet::from([self.target.credential_destination()?]);
        for endpoint in &self.additional_native_endpoints {
            destinations.insert(endpoint.target.credential_destination()?);
        }
        Ok(destinations)
    }

    pub fn digest(&self) -> Result<CanonicalDigest, ComputeManagementErrorV2> {
        self.validate()?;
        CanonicalDigest::of(self).map_err(|_| ComputeManagementErrorV2::InvalidDigest)
    }

    pub fn enabled_credentials(&self) -> impl Iterator<Item = &ComputeManagedCredentialV2> {
        self.credentials
            .iter()
            .filter(|credential| credential.enabled)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementMutationV2 {
    schema: String,
    transaction: String,
    source_id: String,
    expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_digest: Option<CanonicalDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected: Option<ComputeManagementSourceV2>,
    desired_revision: u64,
    desired_digest: CanonicalDigest,
    change_spec_digest: CanonicalDigest,
    desired: ComputeManagementSourceV2,
}

impl ComputeManagementMutationV2 {
    pub fn from_planner(
        spec: &ChangeSpecV1,
        current: Option<&ComputeManagementSourceV2>,
        desired: ComputeManagementSourceV2,
    ) -> Result<Self, ComputeManagementErrorV2> {
        let mutation = Self {
            schema: COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V2.to_owned(),
            transaction: "compare_and_swap".to_owned(),
            source_id: desired.source_id.clone(),
            expected_revision: current.map_or(0, |source| source.revision),
            expected_digest: current.map(ComputeManagementSourceV2::digest).transpose()?,
            expected: current.cloned(),
            desired_revision: desired.revision,
            desired_digest: desired.digest()?,
            change_spec_digest: CanonicalDigest::of(spec)
                .map_err(|_| ComputeManagementErrorV2::InvalidDigest)?,
            desired,
        };
        mutation.validate_shape(spec)?;
        mutation.validate_against(current)?;
        Ok(mutation)
    }

    pub fn validate_shape(&self, spec: &ChangeSpecV1) -> Result<(), ComputeManagementErrorV2> {
        self.desired.validate()?;
        if self.schema != COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V2
            || self.transaction != "compare_and_swap"
            || self.source_id != self.desired.source_id
            || self.desired_revision != self.desired.revision
            || self.desired_revision != self.expected_revision.checked_add(1).unwrap_or(0)
            || self.desired_digest != self.desired.digest()?
            || self.change_spec_digest
                != CanonicalDigest::of(spec).map_err(|_| ComputeManagementErrorV2::InvalidDigest)?
            || spec.command_id != "compute.connection.apply"
            || spec.resource_id.as_deref() != Some(self.source_id.as_str())
            || spec
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2)
            || (self.expected_revision == 0) != self.expected.is_none()
            || (self.expected_revision == 0) != self.expected_digest.is_none()
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        if let Some(expected) = &self.expected
            && (expected.source_id != self.source_id
                || expected.revision != self.expected_revision
                || expected.digest()? != *self.expected_digest.as_ref().unwrap())
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        current: Option<&ComputeManagementSourceV2>,
    ) -> Result<(), ComputeManagementErrorV2> {
        let revision = current.map_or(0, |source| source.revision);
        let digest = current.map(ComputeManagementSourceV2::digest).transpose()?;
        if revision != self.expected_revision
            || digest.as_ref() != self.expected_digest.as_ref()
            || current != self.expected.as_ref()
        {
            return Err(ComputeManagementErrorV2::RevisionConflict);
        }
        Ok(())
    }

    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    pub fn expected(&self) -> Option<&ComputeManagementSourceV2> {
        self.expected.as_ref()
    }

    pub fn desired(&self) -> &ComputeManagementSourceV2 {
        &self.desired
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeManagementStoredSnapshotV2 {
    pub revisions: RevisionSetV1,
    pub sources: Vec<ComputeManagementSourceV2>,
}

/// Narrow safe-record persistence port. All writes still enter through the existing Operation
/// control effect; this trait intentionally offers reads only.
pub trait ComputeManagementRepositoryPort {
    fn compute_management_source(
        &self,
        source_id: &str,
    ) -> PortResult<Option<ComputeManagementSourceV2>>;

    fn compute_management_source_by_lineage(
        &self,
        lineage_digest: &CanonicalDigest,
    ) -> PortResult<Option<ComputeManagementSourceV2>>;

    fn compute_management_snapshot(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<ComputeManagementStoredSnapshotV2>;
}

pub fn validate_authentication(
    authentication: &GatewayAuthenticationSemanticsV1,
) -> Result<(), ComputeManagementErrorV2> {
    if let GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } = authentication
        && (!valid_header_name(header) || reserved_header(header))
    {
        return Err(ComputeManagementErrorV2::InvalidAuthentication);
    }
    Ok(())
}

pub fn digest_suffix(digest: &CanonicalDigest, length: usize) -> &str {
    let value = digest.as_str().strip_prefix("sha256:").unwrap_or_default();
    &value[..length.min(value.len())]
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn reserved_header(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "authorization"
            | "connection"
            | "content-length"
            | "content-type"
            | "host"
            | "proxy-authorization"
            | "transfer-encoding"
    )
}

fn valid_authority(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.contains('@')
        && !value.contains('/')
        && !value.contains('?')
        && !value.contains('#')
        && !value.chars().any(char::is_whitespace)
}

fn valid_request_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 2048
        && !value.contains('\r')
        && !value.contains('\n')
        && !value.contains('?')
        && !value.contains('#')
}

fn valid_header_value(value: &str) -> bool {
    !value.trim().is_empty()
        && value.trim() == value
        && value.len() <= 2_048
        && !value.contains(['\r', '\n', '\0'])
}

fn safe_recheck_header(value: &str) -> bool {
    matches!(
        value,
        "anthropic-version" | "anthropic-beta" | "openai-beta"
    )
}

fn validate_identifier(value: &str) -> Result<(), ComputeManagementErrorV2> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b':' | b'-')
        });
    valid
        .then_some(())
        .ok_or(ComputeManagementErrorV2::InvalidIdentifier)
}

fn validate_text(value: &str) -> Result<(), ComputeManagementErrorV2> {
    (!value.trim().is_empty()
        && value.len() <= 512
        && !value.contains('\r')
        && !value.contains('\n'))
    .then_some(())
    .ok_or(ComputeManagementErrorV2::InvalidIdentifier)
}

fn valid_digest(value: &CanonicalDigest) -> bool {
    CanonicalDigest::parse(value.as_str().to_owned()).is_ok()
        && value != &CanonicalDigest::of_bytes(&[])
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ComputeManagementErrorV2 {
    #[error("compute management schema is unsupported")]
    UnsupportedSchema,
    #[error("compute management identifier is invalid")]
    InvalidIdentifier,
    #[error("compute management provenance is invalid")]
    InvalidProvenance,
    #[error("compute management target is invalid")]
    InvalidTarget,
    #[error("compute management authentication is invalid")]
    InvalidAuthentication,
    #[error("compute management model is invalid")]
    InvalidModel,
    #[error("compute management capability fact is invalid")]
    InvalidCapability,
    #[error("compute management credential is invalid")]
    InvalidCredential,
    #[error("compute management validation is invalid")]
    InvalidValidation,
    #[error("compute management state is inconsistent")]
    InvalidState,
    #[error("compute management identities are duplicated")]
    DuplicateIdentity,
    #[error("compute management digest is invalid")]
    InvalidDigest,
    #[error("compute management mutation is inconsistent")]
    InvalidMutation,
    #[error("compute management revision changed")]
    RevisionConflict,
}

#[cfg(test)]
mod tests;
