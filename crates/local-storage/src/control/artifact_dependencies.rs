//! Exact file dependencies of current receipts and unresolved operations.
use super::*;
use artifact_heads::invalid;

pub(crate) struct Dependency {
    pub operation: OperationId,
    pub intent: ExternalEffectIntentV1,
    pub receipt: Option<OwnedEffectV1>,
    pub required: bool,
}
type Dependencies = BTreeMap<(String, String), Dependency>;

pub(super) fn native_payload(intent: &ExternalEffectIntentV1) -> bool {
    matches!(
        intent.desired()["payload"]["schema"].as_str(),
        Some(
            "hiroute.settings-codex-model-file/v1"
                | "hiroute.settings-claude-model-file/v1"
                | "hiroute.settings-hermes-model-file/v1"
                | "hiroute.settings-qoder-model-file/v1"
                | "hiroute.settings-pi-model-file/v1"
                | "hiroute.settings-dsh-model-file/v1"
                | "hiroute.settings-opencode-model-file/v1"
                | "hiroute.settings-native-plugin-file/v1"
                | "hiroute.settings-hermes-plugin-file/v1"
        )
    )
}

pub(super) fn add(
    output: &mut Dependencies,
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
    required: bool,
) -> PortResult<()> {
    let receipt = artifact_heads::receipt(operation, intent)?.cloned();
    if required && receipt.is_none() {
        return Err(invalid());
    }
    let key = (
        operation.operation_id.to_string(),
        intent.effect_id().into(),
    );
    if let Some(existing) = output.get_mut(&key) {
        if existing.intent != *intent || existing.receipt != receipt {
            return Err(invalid());
        }
        existing.required |= required;
    } else {
        output.insert(
            key,
            Dependency {
                operation: operation.operation_id.clone(),
                intent: intent.clone(),
                receipt,
                required,
            },
        );
    }
    Ok(())
}

pub(super) fn add_predecessor(
    control: &ControlStore,
    output: &mut Dependencies,
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
) -> PortResult<()> {
    let payload = &intent.desired()["payload"];
    let mut expected_digest = None;
    let (id, effect_id) = if native_payload(intent) {
        let action = &payload["change"];
        let Some(id) = action["previous_operation"]
            .as_str()
            .or_else(|| action["original_operation"].as_str())
        else {
            return Ok(());
        };
        (
            OperationId::parse(id).map_err(|_| invalid())?,
            intent.effect_id().to_owned(),
        )
    } else if payload["schema"] == "hiroute.settings-skill-file/v1"
        && !payload["before"]["file_effect"].is_null()
    {
        let reference: hiroute_domain::SkillFileEffectRef =
            serde_json::from_value(payload["before"]["file_effect"].clone())
                .map_err(|_| invalid())?;
        reference.validate().map_err(|_| invalid())?;
        if reference.target != intent.target() {
            return Err(invalid());
        }
        expected_digest = Some(reference.intent_digest);
        (reference.operation_id, reference.effect_id)
    } else {
        return Ok(());
    };
    if id == operation.operation_id {
        return Err(invalid());
    }
    let previous = control.load_operation(&id)?.ok_or_else(invalid)?;
    if previous.workspace_id != operation.workspace_id
        || previous.state != OperationState::Succeeded
        || !journal::is_current_artifact_step(&control.connection.borrow(), &previous)?
    {
        return Err(invalid());
    }
    let original = artifact_heads::file_intents(&previous)
        .find(|i| i.effect_id() == effect_id && i.target() == intent.target())
        .ok_or_else(invalid)?;
    if expected_digest
        .is_some_and(|d| CanonicalDigest::of(original.desired()).ok().as_ref() != Some(&d))
    {
        return Err(invalid());
    }
    // The renderer consumes this direct record. A settled reconfiguration copied its
    // original user baseline into its own record; following older links would pin history.
    add(output, &previous, original, true)
}

impl ManagedArtifactStore {
    pub(crate) fn validate_dependencies(
        &self,
        dependencies: &[Dependency],
    ) -> Result<(), LocalStorageError> {
        use hiroute_domain::NativeAgentArtifactPort;
        for dependency in dependencies {
            let intent = &dependency.intent;
            let receipt = dependency.receipt.as_ref();
            let marker = self.load_marker(&dependency.operation, intent.effect_id())?;
            if let Some(marker) = marker {
                if marker.kind != intent.kind()
                    || marker.target != intent.target()
                    || marker.before_digest.as_ref() != intent.before_fingerprint()
                    || marker.after_mode != intent.desired_mode()
                    || (!marker.rendered && marker.sensitive != intent.sensitive())
                    || (marker.rendered
                        && marker.intent_digest.as_ref()
                            != Some(
                                &CanonicalDigest::of(intent.desired())
                                    .map_err(|_| LocalStorageError::Locked)?,
                            ))
                    || (dependency.required && (!marker.activated || marker.compensated))
                {
                    return Err(LocalStorageError::Locked);
                }
                if !marker.rendered {
                    let desired = serde_json::to_vec(intent.desired())
                        .map_err(|_| LocalStorageError::Locked)?;
                    if artifact_fingerprint(&desired, intent.desired_mode())? != marker.after_digest
                    {
                        return Err(LocalStorageError::Locked);
                    }
                }
                self.validated_backup(&marker)
                    .map_err(|_| LocalStorageError::Locked)?;
                if receipt.is_some_and(|r| r != &Self::effect(&marker)) {
                    return Err(LocalStorageError::Locked);
                }
            } else if (dependency.required || receipt.is_some())
                && !receipt.is_some_and(|r| {
                    matches!(
                        r.compensation["schema"].as_str(),
                        Some(
                            "hiroute.native-file-reference/v1"
                                | "hiroute.native-restoration-ack/v1"
                        )
                    )
                })
            {
                return Err(LocalStorageError::Locked);
            }
            // A protected record may be written before staging. Authenticate it if present,
            // even for an interrupted operation without a marker/receipt yet.
            let catalog =
                intent.desired()["payload"]["schema"] == "hiroute.codex-catalog-artifact/v1";
            if native_payload(intent) || catalog {
                let record = self
                    .load_native_restore(&dependency.operation, intent)
                    .map_err(|_| LocalStorageError::Locked)?;
                let markerless = receipt.filter(|r| {
                    matches!(
                        r.compensation["schema"].as_str(),
                        Some(
                            "hiroute.native-file-reference/v1"
                                | "hiroute.native-restoration-ack/v1"
                        )
                    )
                });
                let required_record = (catalog
                    || intent.desired()["payload"]["change"]["action"] == "configure")
                    && (dependency.required || receipt.is_some());
                if (required_record || markerless.is_some()) && record.is_none() {
                    return Err(LocalStorageError::Locked);
                }
                if catalog {
                    let expected: CanonicalDigest = serde_json::from_value(
                        intent.desired()["payload"]["content_digest"].clone(),
                    )
                    .map_err(|_| LocalStorageError::Locked)?;
                    if record
                        .as_ref()
                        .is_some_and(|bytes| CanonicalDigest::of_bytes(bytes) != expected)
                    {
                        return Err(LocalStorageError::Locked);
                    }
                    // This immutable, HiRoute-owned file is a current model dependency.
                    // Borrowed native configuration may change independently; it is not
                    // subjected to this whole-file equality requirement.
                    if dependency.required
                        && self
                            .read_native_target(intent.target())
                            .map_err(|_| LocalStorageError::Locked)?
                            .as_ref()
                            .is_none_or(|bytes| CanonicalDigest::of_bytes(bytes) != expected)
                    {
                        return Err(LocalStorageError::Locked);
                    }
                }
                if let Some(receipt) = markerless {
                    let value: Value =
                        serde_json::from_slice(record.as_ref().ok_or(LocalStorageError::Locked)?)
                            .map_err(|_| LocalStorageError::Locked)?;
                    if value["schema"] != receipt.compensation["schema"]
                        || value["fingerprint"]
                            != serde_json::to_value(&receipt.after_fingerprint)
                                .map_err(|_| LocalStorageError::Locked)?
                    {
                        return Err(LocalStorageError::Locked);
                    }
                }
            }
        }
        Ok(())
    }
}
