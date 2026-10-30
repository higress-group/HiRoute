use super::*;

const SOURCE_CONTROL_SCHEMA: &str = "hiroute.compute-source-control/v1";
const PROJECTION_CONTROL_SCHEMA: &str = "hiroute.compute-projection-control/v1";

/// A Release-authorized, revision-bound mutation for the one `compute_sources` row owned by
/// `compute.connection.apply`. Private fields and the absence of `Deserialize` keep callers from
/// turning the Operation journal into a generic table-write surface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComputeSourceMutationV1 {
    schema: &'static str,
    transaction: &'static str,
    source_id: String,
    connection_option_id: String,
    explicit_materialization: bool,
    expected_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_source_digest: Option<CanonicalDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_identity_digest: Option<CanonicalDigest>,
    desired_revision: u64,
    desired_source_digest: CanonicalDigest,
    desired_identity_digest: CanonicalDigest,
    registry_version: String,
    registry_digest: CanonicalDigest,
    change_spec_digest: CanonicalDigest,
    desired: crate::ComputeSourceV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_projection: Option<crate::ComputeProjectionExpectationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    desired_projection_digest: Option<CanonicalDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    desired_projection: Option<crate::ComputeControlProjectionV1>,
    #[serde(default, skip_serializing_if = "is_false")]
    legacy_source_transition: bool,
    #[serde(skip)]
    durable_v7: bool,
}

impl ComputeSourceMutationV1 {
    fn from_current_registry(
        spec: &ChangeSpecV1,
        current: Option<&crate::ComputeSourceV1>,
        desired: crate::ComputeSourceV1,
        registry: &crate::ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    ) -> Result<Self, OperationValidationError> {
        registry
            .validate()
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        desired
            .validate(registry, explicit_materialization)
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        let expected_revision = current.map_or(0, |source| source.revision);
        let mutation = Self {
            schema: SOURCE_CONTROL_SCHEMA,
            transaction: "compare_and_swap",
            source_id: desired.source_id.clone(),
            connection_option_id: desired.connection_option_id.clone(),
            explicit_materialization,
            expected_revision,
            expected_source_digest: current.map(CanonicalDigest::of).transpose()?,
            expected_identity_digest: current.map(|source| source.identity_digest.clone()),
            desired_revision: desired.revision,
            desired_source_digest: CanonicalDigest::of(&desired)?,
            desired_identity_digest: desired.identity_digest.clone(),
            registry_version: registry.registry_version.clone(),
            registry_digest: CanonicalDigest::of(registry)?,
            change_spec_digest: CanonicalDigest::of(spec)?,
            desired,
            expected_projection: None,
            desired_projection_digest: None,
            desired_projection: None,
            legacy_source_transition: false,
            durable_v7: false,
        };
        mutation.validate_shape(spec)?;
        mutation.validate_against(current)?;
        Ok(mutation)
    }

    fn from_current_projection(
        spec: &ChangeSpecV1,
        current: Option<&crate::ComputeSourceV1>,
        prepared: crate::PreparedComputeProjectionV1,
        registry: &crate::ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    ) -> Result<Self, OperationValidationError> {
        prepared
            .validate()
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        registry
            .validate()
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        let desired = prepared.desired.source.clone();
        desired
            .validate(registry, explicit_materialization)
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        // Catalog provenance carries the digest of the exact client-bundled JSON bytes, while
        // `registry_digest` below binds the parsed registry value used by this validator. They
        // intentionally are not compared as though both were semantic hashes; Application's
        // current-catalog port has already re-derived and byte-compared the complete projection.
        if prepared.desired.catalog.product_release != registry.product_release
            || prepared.desired.catalog.connector_registry_version != registry.registry_version
            || !desired
                .identity
                .evidence_refs
                .contains(&prepared.desired.scanner.evidence_digest)
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let legacy_source_transition = prepared.expected.source_revision > 0
            && current.is_none_or(|source| source.source_id != desired.source_id);
        let expected_revision = if legacy_source_transition {
            prepared.expected.source_revision
        } else {
            current.map_or(0, |source| source.revision)
        };
        if prepared.expected.source_revision != expected_revision
            || (!legacy_source_transition
                && prepared.expected.source_digest
                    != current.map(CanonicalDigest::of).transpose()?)
            || (legacy_source_transition && prepared.expected.source_digest.is_none())
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let desired_projection_digest = prepared
            .desired
            .digest()
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        let mutation = Self {
            schema: PROJECTION_CONTROL_SCHEMA,
            transaction: "compare_and_swap",
            source_id: desired.source_id.clone(),
            connection_option_id: desired.connection_option_id.clone(),
            explicit_materialization,
            expected_revision,
            expected_source_digest: if legacy_source_transition {
                prepared.expected.source_digest.clone()
            } else {
                current.map(CanonicalDigest::of).transpose()?
            },
            expected_identity_digest: current.map(|source| source.identity_digest.clone()),
            desired_revision: desired.revision,
            desired_source_digest: CanonicalDigest::of(&desired)?,
            desired_identity_digest: desired.identity_digest.clone(),
            registry_version: registry.registry_version.clone(),
            registry_digest: CanonicalDigest::of(registry)?,
            change_spec_digest: CanonicalDigest::of(spec)?,
            desired,
            expected_projection: Some(prepared.expected),
            desired_projection_digest: Some(desired_projection_digest),
            desired_projection: Some(prepared.desired),
            legacy_source_transition,
            durable_v7: false,
        };
        mutation.validate_shape(spec)?;
        mutation.validate_against(current)?;
        Ok(mutation)
    }

    fn from_durable(
        spec: &ChangeSpecV1,
        durable: DurableComputeSourceMutationV1,
    ) -> Result<Self, OperationValidationError> {
        let schema = match durable.schema.as_str() {
            SOURCE_CONTROL_SCHEMA => SOURCE_CONTROL_SCHEMA,
            PROJECTION_CONTROL_SCHEMA => PROJECTION_CONTROL_SCHEMA,
            _ => return Err(OperationValidationError::UnregisteredEffectPlan),
        };
        let mutation = Self {
            schema,
            transaction: "compare_and_swap",
            source_id: durable.source_id,
            connection_option_id: durable.connection_option_id,
            explicit_materialization: durable.explicit_materialization,
            expected_revision: durable.expected_revision,
            expected_source_digest: durable.expected_source_digest,
            expected_identity_digest: durable.expected_identity_digest,
            desired_revision: durable.desired_revision,
            desired_source_digest: durable.desired_source_digest,
            desired_identity_digest: durable.desired_identity_digest,
            registry_version: durable.registry_version,
            registry_digest: durable.registry_digest,
            change_spec_digest: durable.change_spec_digest,
            desired: durable.desired,
            expected_projection: durable.expected_projection,
            desired_projection_digest: durable.desired_projection_digest,
            desired_projection: durable.desired_projection,
            legacy_source_transition: durable.legacy_source_transition,
            durable_v7: false,
        };
        if mutation.validate_shape(spec).is_err() {
            let mut legacy = mutation;
            legacy.durable_v7 = true;
            legacy.validate_v7_shape(spec)?;
            return Ok(legacy);
        }
        Ok(mutation)
    }

    fn validate_shape(&self, spec: &ChangeSpecV1) -> Result<(), OperationValidationError> {
        self.desired
            .validate_shape()
            .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        validate_scope_identifier(&self.registry_version)?;
        let expected_digests_present = self.expected_revision > 0;
        let desired_revision = self
            .expected_revision
            .checked_add(1)
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        let projection_shape = match (
            self.schema,
            self.expected_projection.as_ref(),
            self.desired_projection_digest.as_ref(),
            self.desired_projection.as_ref(),
        ) {
            (SOURCE_CONTROL_SCHEMA, None, None, None) => true,
            (
                PROJECTION_CONTROL_SCHEMA,
                Some(expected),
                Some(projection_digest),
                Some(projection),
            ) => {
                projection.validate_shape().is_ok()
                    && expected.validate_for(projection).is_ok()
                    && projection.source == self.desired
                    && projection_digest == &CanonicalDigest::of(projection)?
                    && expected.source_revision == self.expected_revision
                    && expected.source_digest == self.expected_source_digest
                    && projection.catalog.connector_registry_version == self.registry_version
            }
            _ => false,
        };
        if !projection_shape
            || self.transaction != "compare_and_swap"
            || self.source_id != self.desired.source_id
            || self.connection_option_id != self.desired.connection_option_id
            || self.desired_revision != self.desired.revision
            || self.desired_revision != desired_revision
            || self.desired_source_digest != CanonicalDigest::of(&self.desired)?
            || self.desired_identity_digest != self.desired.identity_digest
            || self.change_spec_digest != CanonicalDigest::of(spec)?
            || expected_digests_present != self.expected_source_digest.is_some()
            || (!self.legacy_source_transition
                && expected_digests_present != self.expected_identity_digest.is_some())
            || (self.legacy_source_transition
                && (self.schema != PROJECTION_CONTROL_SCHEMA || self.expected_revision == 0))
            || self.durable_v7
            || CanonicalDigest::parse(self.registry_digest.as_str().to_owned()).is_err()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        validate_connection_spec(spec, self)
    }

    /// Accepts only the exact pre-v8 projection encoding. Raw values remain unchanged so the
    /// ChangeSpec and deterministic step digests are still the bytes admitted by v7.
    fn validate_v7_shape(&self, spec: &ChangeSpecV1) -> Result<(), OperationValidationError> {
        if self.schema != PROJECTION_CONTROL_SCHEMA
            || self.transaction != "compare_and_swap"
            || self.legacy_source_transition
            || self.desired_revision
                != self
                    .expected_revision
                    .checked_add(1)
                    .ok_or(OperationValidationError::UnregisteredEffectPlan)?
            || self.source_id != self.desired.source_id
            || self.connection_option_id != self.desired.connection_option_id
            || self.desired_source_digest != CanonicalDigest::of(&self.desired)?
            || self.desired_identity_digest != self.desired.identity_digest
            || self.change_spec_digest != CanonicalDigest::of(spec)?
            || (self.expected_revision > 0) != self.expected_source_digest.is_some()
            || (self.expected_revision > 0) != self.expected_identity_digest.is_some()
            || CanonicalDigest::parse(self.registry_digest.as_str().to_owned()).is_err()
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        validate_scope_identifier(&self.registry_version)?;
        let expected = self
            .expected_projection
            .as_ref()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        let raw_projection = self
            .desired_projection
            .as_ref()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        if raw_projection.source != self.desired
            || expected.source_revision != self.expected_revision
            || expected.source_digest != self.expected_source_digest
            || self.desired_projection_digest.as_ref()
                != Some(&CanonicalDigest::of(raw_projection)?)
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        let normalized = normalize_v7_projection(raw_projection)?;
        crate::PreparedComputeProjectionV1 {
            expected: expected.clone(),
            desired: normalized,
        }
        .validate()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
        validate_connection_spec(spec, self)
    }

    pub fn validate_against(
        &self,
        current: Option<&crate::ComputeSourceV1>,
    ) -> Result<(), OperationValidationError> {
        if self.durable_v7 {
            let legacy_current = current
                .map(|source| {
                    let mut source = source.clone();
                    source.identity_digest = CanonicalDigest::of(&source.identity)?;
                    Ok::<_, OperationValidationError>(source)
                })
                .transpose()?;
            if current.map_or(0, |source| source.revision) != self.expected_revision
                || legacy_current
                    .as_ref()
                    .map(CanonicalDigest::of)
                    .transpose()?
                    .as_ref()
                    != self.expected_source_digest.as_ref()
                || legacy_current
                    .as_ref()
                    .map(|source| &source.identity_digest)
                    != self.expected_identity_digest.as_ref()
            {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            return Ok(());
        }
        if self.legacy_source_transition {
            if self.expected_revision == 0 {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            if let Some(current) = current
                && (current.revision != self.expected_revision
                    || CanonicalDigest::of(current)?
                        != *self
                            .expected_source_digest
                            .as_ref()
                            .ok_or(OperationValidationError::UnregisteredEffectPlan)?
                    || self
                        .expected_identity_digest
                        .as_ref()
                        .is_some_and(|digest| digest != &current.identity_digest)
                    || current.source_id == self.source_id
                    || current.connection_option_id != self.connection_option_id)
            {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            return Ok(());
        }
        let current_revision = current.map_or(0, |source| source.revision);
        let current_digest = current.map(CanonicalDigest::of).transpose()?;
        let current_identity = current.map(|source| &source.identity_digest);
        if current_revision != self.expected_revision
            || current_digest.as_ref() != self.expected_source_digest.as_ref()
            || current_identity != self.expected_identity_digest.as_ref()
            || current.is_some_and(|source| {
                source.source_id != self.source_id
                    || source.connection_option_id != self.connection_option_id
            })
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        Ok(())
    }

    pub fn desired(&self) -> &crate::ComputeSourceV1 {
        &self.desired
    }

    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    pub fn expected_source_digest(&self) -> Option<&CanonicalDigest> {
        self.expected_source_digest.as_ref()
    }

    pub fn expected_projection(&self) -> Option<&crate::ComputeProjectionExpectationV1> {
        self.expected_projection.as_ref()
    }

    pub fn desired_projection(&self) -> Option<&crate::ComputeControlProjectionV1> {
        self.desired_projection.as_ref()
    }
}

fn normalize_v7_projection(
    raw: &crate::ComputeControlProjectionV1,
) -> Result<crate::ComputeControlProjectionV1, OperationValidationError> {
    let legacy_identity_digest = CanonicalDigest::of(&raw.source.identity)?;
    if raw.source.identity_digest != legacy_identity_digest
        || raw.binding.source_identity_digest != legacy_identity_digest
        || raw.binding.source_id != raw.source.source_id
        || raw.binding.source_revision != raw.source.revision
        || raw.inventory.source_id != raw.source.source_id
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if let Some(pool) = &raw.credential_pool_identity
        && (pool.source_identity_digest != legacy_identity_digest
            || pool.binding_digest != CanonicalDigest::of(&raw.binding)?)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let mut normalized = raw.clone();
    let semantic_digest = normalized
        .source
        .identity
        .digest()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    normalized.source.identity_digest = semantic_digest.clone();
    normalized.binding.source_identity_digest = semantic_digest.clone();
    if let Some(pool) = &mut normalized.credential_pool_identity {
        pool.source_identity_digest = semantic_digest;
        pool.binding_digest = CanonicalDigest::of(&normalized.binding)?;
    }
    normalized
        .validate_shape()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(normalized)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableComputeSourceMutationV1 {
    schema: String,
    transaction: String,
    source_id: String,
    connection_option_id: String,
    explicit_materialization: bool,
    expected_revision: u64,
    #[serde(default)]
    expected_source_digest: Option<CanonicalDigest>,
    #[serde(default)]
    expected_identity_digest: Option<CanonicalDigest>,
    desired_revision: u64,
    desired_source_digest: CanonicalDigest,
    desired_identity_digest: CanonicalDigest,
    registry_version: String,
    registry_digest: CanonicalDigest,
    change_spec_digest: CanonicalDigest,
    desired: crate::ComputeSourceV1,
    #[serde(default)]
    expected_projection: Option<crate::ComputeProjectionExpectationV1>,
    #[serde(default)]
    desired_projection_digest: Option<CanonicalDigest>,
    #[serde(default)]
    desired_projection: Option<crate::ComputeControlProjectionV1>,
    #[serde(default)]
    legacy_source_transition: bool,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

pub(super) fn decode_source_mutation(
    spec: &ChangeSpecV1,
    control: &Value,
) -> Result<ComputeSourceMutationV1, OperationValidationError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct MutationEnvelope {
        compute_source_mutation: DurableComputeSourceMutationV1,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ProjectionEnvelope {
        compute_projection_plan: ProjectionPlannerControlV1,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ProjectionPlannerControlV1 {
        #[serde(default)]
        current: Option<crate::ComputeSourceV1>,
        prepared: crate::PreparedComputeProjectionV1,
        registry: crate::ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    }

    if let Ok(envelope) = serde_json::from_value::<MutationEnvelope>(control.clone()) {
        if !matches!(
            envelope.compute_source_mutation.schema.as_str(),
            SOURCE_CONTROL_SCHEMA | PROJECTION_CONTROL_SCHEMA
        ) || envelope.compute_source_mutation.transaction != "compare_and_swap"
        {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
        return ComputeSourceMutationV1::from_durable(spec, envelope.compute_source_mutation);
    }
    let envelope: ProjectionEnvelope = serde_json::from_value(control.clone())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    ComputeSourceMutationV1::from_current_projection(
        spec,
        envelope.compute_projection_plan.current.as_ref(),
        envelope.compute_projection_plan.prepared,
        &envelope.compute_projection_plan.registry,
        envelope.compute_projection_plan.explicit_materialization,
    )
}

fn validate_connection_spec(
    spec: &ChangeSpecV1,
    mutation: &ComputeSourceMutationV1,
) -> Result<(), OperationValidationError> {
    let input = object(spec)?;
    if input.keys().any(|key| {
        !matches!(
            key.as_str(),
            "connection_option_id"
                | "source_id"
                | "explicit_materialization"
                | "expected_source_revision"
                | "projection"
        )
    }) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if spec.command_id != "compute.connection.apply"
        || required_identifier(input, "connection_option_id")? != mutation.connection_option_id
        || required_identifier(input, "source_id")? != mutation.source_id
        || input
            .get("explicit_materialization")
            .and_then(Value::as_bool)
            != Some(mutation.explicit_materialization)
        || required_u64(input, "expected_source_revision")? != mutation.expected_revision
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    match (
        input.get("projection"),
        mutation.expected_projection.as_ref(),
        mutation.desired_projection.as_ref(),
    ) {
        (None, None, None) => {}
        (Some(value), Some(expected), Some(desired)) => {
            let prepared: crate::PreparedComputeProjectionV1 =
                serde_json::from_value(value.clone())
                    .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
            if &prepared.expected != expected || &prepared.desired != desired {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
        }
        _ => return Err(OperationValidationError::UnregisteredEffectPlan),
    }
    Ok(())
}

impl TransactionPlanV1 {
    pub fn from_compute_source_planner(
        spec: ChangeSpecV1,
        current: Option<&crate::ComputeSourceV1>,
        desired: crate::ComputeSourceV1,
        registry: &crate::ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    ) -> Result<Self, OperationValidationError> {
        let mutation = ComputeSourceMutationV1::from_current_registry(
            &spec,
            current,
            desired,
            registry,
            explicit_materialization,
        )?;
        let control = json!({"compute_source_mutation": &mutation});
        validate_connection_plan(&spec, &control, &[], &[], &[])?;
        Ok(Self {
            spec,
            control,
            credential_pool: None,
            worker_dependency_selection: None,
            secrets: Vec::new(),
            agent_access_grants: Vec::new(),
            runtime: Vec::new(),
            external: Vec::new(),
        })
    }

    pub fn from_compute_projection_planner(
        spec: ChangeSpecV1,
        current: Option<&crate::ComputeSourceV1>,
        prepared: crate::PreparedComputeProjectionV1,
        registry: &crate::ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    ) -> Result<Self, OperationValidationError> {
        let mutation = ComputeSourceMutationV1::from_current_projection(
            &spec,
            current,
            prepared,
            registry,
            explicit_materialization,
        )?;
        let control = json!({"compute_source_mutation": &mutation});
        validate_connection_plan(&spec, &control, &[], &[], &[])?;
        Ok(Self {
            spec,
            control,
            credential_pool: None,
            worker_dependency_selection: None,
            secrets: Vec::new(),
            agent_access_grants: Vec::new(),
            runtime: Vec::new(),
            external: Vec::new(),
        })
    }
}

pub(super) fn validate_connection_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if matches!(
        spec.desired_state.get("schema").and_then(Value::as_str),
        Some(
            crate::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2
                | crate::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3
                | crate::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4
        )
    ) {
        return super::compute_management::validate_management_plan(
            spec, control, secrets, runtime, external,
        );
    }
    if !secrets.is_empty() || !runtime.is_empty() || !external.is_empty() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    decode_source_mutation(spec, control)?;
    Ok(())
}

pub(super) fn validate_credential_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    pool: Option<&CredentialPoolMutationV1>,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    let remove = spec.command_id == "compute.credential.remove";
    if !runtime.is_empty()
        || !external.is_empty()
        || (!remove && secrets.len() != 1)
        || (remove && secrets.len() > 1)
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let pool = pool.ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    pool.validate_shape()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if control != &json!({"credential_pool_mutation": pool}) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let input = object(spec)?;
    if input.keys().any(|key| {
        !matches!(
            key.as_str(),
            "connection_option_id"
                | "source_id"
                | "pool_id"
                | "binding_id"
                | "binding_revision"
                | "offer_ref"
                | "offer_revision"
                | "model_configuration_id"
                | "credential_id"
                | "input_slot"
                | "secret_source"
                | "expected_generation"
                | "expected_pool_revision"
        )
    }) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let option = required_identifier(input, "connection_option_id")?;
    let source = required_identifier(input, "source_id")?;
    let pool_id = required_identifier(input, "pool_id")?;
    let binding_id = required_identifier(input, "binding_id")?;
    let binding_revision = required_u64(input, "binding_revision")?;
    let offer_ref = required_identifier(input, "offer_ref")?;
    let offer_revision = required_u64(input, "offer_revision")?;
    let model_configuration_id = required_identifier(input, "model_configuration_id")?;
    let credential_id = required_identifier(input, "credential_id")?;
    let expected_generation = required_u64(input, "expected_generation")?;
    let expected_pool_revision = required_u64(input, "expected_pool_revision")?;
    let input_slot = input.get("input_slot").and_then(Value::as_str);
    let secret_source = input.get("secret_source");
    if remove != input_slot.is_none()
        || remove != secret_source.is_none()
        || secret_source.is_some_and(|value| validate_secret_source(value).is_err())
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }

    let secret = secrets.first();
    let expected_pool_kind = match spec.command_id.as_str() {
        "compute.credential.add" => CredentialPoolMutationKind::Add,
        "compute.credential.replace" => CredentialPoolMutationKind::Replace,
        "compute.credential.remove" => CredentialPoolMutationKind::Remove,
        _ => return Err(OperationValidationError::UnregisteredEffectPlan),
    };
    let desired = pool.desired();
    let pool_credential = desired
        .credentials
        .iter()
        .find(|entry| entry.credential.credential_id() == credential_id);
    let pool_transition_matches_secret = if remove {
        pool_credential.is_none() && secret.is_none_or(|mutation| mutation.fingerprint().is_none())
    } else {
        pool_credential.is_some_and(|entry| {
            secret.is_some_and(|mutation| {
                mutation.fingerprint().is_some_and(|fingerprint| {
                    entry.fingerprint == *fingerprint
                        && entry.credential.generation() == expected_generation.saturating_add(1)
                })
            })
        })
    };
    let secret_matches = secret.is_none_or(|mutation| {
        let reference = mutation.credential();
        mutation.kind()
            == if remove {
                SecretMutationKind::Delete
            } else {
                SecretMutationKind::Upsert
            }
            && mutation.expected_generation() == expected_generation
            && mutation.input_slot() == input_slot
            && (remove || mutation.fingerprint().is_some())
            && reference.credential_id() == credential_id
            && reference.owner_scope() == format!("source/{source}")
            && reference.subject() == "hirouted"
            && reference.purpose() == "provider-auth"
            && reference.allowed_destinations()
                == &BTreeSet::from([format!("connection-option/{option}")])
            && reference.generation() == expected_generation
    });
    if !secret_matches
        || pool.kind() != expected_pool_kind
        || pool.credential_id() != Some(credential_id)
        || pool.expected_revision() != expected_pool_revision
        || desired.pool_id != pool_id
        || desired.binding_id != binding_id
        || desired.binding_revision != binding_revision
        || desired.source_id != source
        || desired.connection_option_id != option
        || desired.offer_ref != offer_ref
        || desired.offer_revision != offer_revision
        || desired.model_configuration_id != model_configuration_id
        || !pool_transition_matches_secret
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

pub(super) fn validate_control_only_plan(
    spec: &ChangeSpecV1,
    control: &Value,
    pool: Option<&CredentialPoolMutationV1>,
    secrets: &[SecretMutationV1],
    runtime: &[RuntimeMutationV1],
    external: &[ExternalEffectIntentV1],
) -> Result<(), OperationValidationError> {
    if !secrets.is_empty() || !runtime.is_empty() || !external.is_empty() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let input = object(spec)?;
    match spec.command_id.as_str() {
        "compute.key-pool.apply" => validate_reorder(input, control, pool),
        "prices.override.apply" => validate_price_override(input, control, pool),
        _ => Err(OperationValidationError::UnregisteredEffectPlan),
    }
}

fn validate_reorder(
    input: &serde_json::Map<String, Value>,
    control: &Value,
    pool: Option<&CredentialPoolMutationV1>,
) -> Result<(), OperationValidationError> {
    if input.keys().any(|key| {
        !matches!(
            key.as_str(),
            "source_id" | "pool_id" | "ordered_credential_ids" | "expected_pool_revision"
        )
    }) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let source = required_identifier(input, "source_id")?;
    let pool_id = required_identifier(input, "pool_id")?;
    let expected = required_u64(input, "expected_pool_revision")?;
    let ids = input
        .get("ordered_credential_ids")
        .and_then(Value::as_array)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    if ids.is_empty() {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        let id = id
            .as_str()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
        validate_scope_identifier(id)?;
        if !unique.insert(id) {
            return Err(OperationValidationError::UnregisteredEffectPlan);
        }
    }
    let pool = pool.ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    pool.validate_shape()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let desired_ids = pool
        .desired()
        .credentials
        .iter()
        .map(|entry| entry.credential.credential_id())
        .collect::<Vec<_>>();
    let input_ids = ids.iter().filter_map(Value::as_str).collect::<Vec<_>>();
    if pool.kind() != CredentialPoolMutationKind::Reorder
        || pool.expected_revision() != expected
        || pool.desired().pool_id != pool_id
        || pool.desired().source_id != source
        || desired_ids != input_ids
        || control != &json!({"credential_pool_mutation": pool})
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

fn validate_price_override(
    input: &serde_json::Map<String, Value>,
    control: &Value,
    pool: Option<&CredentialPoolMutationV1>,
) -> Result<(), OperationValidationError> {
    if pool.is_some()
        || input.keys().any(|key| {
            !matches!(
                key.as_str(),
                "override_id"
                    | "offer_ref"
                    | "model_configuration_id"
                    | "currency"
                    | "operation"
                    | "target_rule_id"
                    | "input_value"
                    | "output_value"
                    | "expected_override_revision"
            )
        })
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    for key in ["override_id", "offer_ref", "model_configuration_id"] {
        required_identifier(input, key)?;
    }
    let currency = required_identifier(input, "currency")?;
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let operation = required_identifier(input, "operation")?;
    let target_rule = input.get("target_rule_id").and_then(Value::as_str);
    let input_value = input.get("input_value").and_then(Value::as_u64);
    let output_value = input.get("output_value").and_then(Value::as_u64);
    let valid_values = match operation {
        "replace" => target_rule.is_none() && input_value.is_some() && output_value.is_some(),
        "multiply_parts_per_million" => {
            target_rule.is_none()
                && input_value.is_some_and(|factor| factor > 0)
                && output_value.is_none()
        }
        "disable_catalog_rule" => {
            target_rule.is_some_and(|rule| validate_scope_identifier(rule).is_ok())
                && input_value.is_none()
                && output_value.is_none()
        }
        _ => false,
    };
    if !valid_values
        || input
            .get("expected_override_revision")
            .and_then(Value::as_u64)
            .is_none()
    {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    if control != &json!({"price_override_change": Value::Object(input.clone())}) {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    Ok(())
}

fn object(
    spec: &ChangeSpecV1,
) -> Result<&serde_json::Map<String, Value>, OperationValidationError> {
    spec.desired_state
        .as_object()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)
}

fn required_identifier<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, OperationValidationError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    validate_scope_identifier(value)?;
    Ok(value)
}

fn required_u64(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<u64, OperationValidationError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(OperationValidationError::UnregisteredEffectPlan)
}

fn validate_secret_source(value: &Value) -> Result<(), OperationValidationError> {
    let object = value
        .as_object()
        .ok_or(OperationValidationError::UnregisteredEffectPlan)?;
    match object.get("kind").and_then(Value::as_str) {
        Some("manual_input") if object.len() == 1 => Ok(()),
        Some("discovered_config")
            if object.len() == 6
                && object.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "kind"
                            | "scanner_id"
                            | "scanner_version"
                            | "source_ref"
                            | "field_selector"
                            | "observed_revision"
                    )
                }) =>
        {
            for key in [
                "scanner_id",
                "scanner_version",
                "source_ref",
                "field_selector",
            ] {
                required_identifier(object, key)?;
            }
            if required_u64(object, "observed_revision")? == 0 {
                return Err(OperationValidationError::UnregisteredEffectPlan);
            }
            Ok(())
        }
        _ => Err(OperationValidationError::UnregisteredEffectPlan),
    }
}
