use super::*;

pub(super) fn validate_draft(
    draft: &NativeModelConnectionDraftV1,
    credential: &NativeModelConnectionCredentialV1<'_>,
) -> Result<(), NativeModelConnectionErrorV1> {
    if draft.inference_model_id.as_ref().is_some_and(|id| {
        !bounded_text(id, 512)
            || draft.runtime_fallback_denied_model_ids.contains(id)
            || !draft
                .models
                .iter()
                .any(|model| &model.upstream_model_id == id)
    }) || draft
        .display_template_id
        .as_ref()
        .is_some_and(|id| !bounded_text(id, 256) || id.trim() != id)
        || !bounded_text(&draft.lineage_ref, 256)
        || draft
            .trusted_lineage_digest
            .as_ref()
            .is_some_and(|digest| digest == &CanonicalDigest::of_bytes(&[]))
        || draft.trusted_lineage_digest.is_some() && draft.existing_source_id.is_none()
        || !bounded_text(&draft.display_name, 256)
        || draft.base_url.len() > 2_048
        || draft.additional_native_endpoints.len() > 2
        || draft.additional_native_endpoints.iter().any(|endpoint| {
            endpoint.validate().is_err()
                || endpoint.target.upstream_protocol == draft.protocol
                || (endpoint.authentication == GatewayAuthenticationSemanticsV1::None)
                    != (draft.authentication == GatewayAuthenticationSemanticsV1::None)
        })
        || draft.edit_revision == 0
        || !bounded_text(&draft.check_id, 256)
        || !bounded_text(&draft.protocol_profile_id, 256)
        || draft.models.len() > DEFAULT_MODEL_CONNECTION_MODEL_LIMIT
        || draft.runtime_fallback_denied_model_ids.len() > 100_000
        || draft
            .runtime_fallback_denied_model_ids
            .iter()
            .any(|value| !bounded_text(value, 512) || value.trim() != value)
        || draft
            .existing_source_id
            .as_ref()
            .is_some_and(|value| !bounded_text(value, 256))
        || draft
            .request_path_override
            .as_ref()
            .is_some_and(|value| value.len() > 2_048)
        || draft
            .inventory_path_override
            .as_ref()
            .is_some_and(|value| value.len() > 2_048)
    {
        return Err(NativeModelConnectionErrorV1::InvalidDraft);
    }
    let qualification_valid = match (
        &draft.authentication,
        draft.qualification.free_access,
        draft.qualification.evidence_ref.as_deref(),
    ) {
        (GatewayAuthenticationSemanticsV1::None, Some(FreeAccess::Direct), Some(evidence)) => {
            bounded_text(evidence, 512)
        }
        (GatewayAuthenticationSemanticsV1::None, None, None) => true,
        (
            GatewayAuthenticationSemanticsV1::Bearer
            | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
            Some(FreeAccess::ApiKeyRequired),
            Some(evidence),
        ) => bounded_text(evidence, 512),
        (
            GatewayAuthenticationSemanticsV1::Bearer
            | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
            None,
            None,
        ) => true,
        _ => false,
    };
    let credential_kind_valid = matches!(
        (&draft.authentication, credential),
        (
            GatewayAuthenticationSemanticsV1::None,
            NativeModelConnectionCredentialV1::NotRequired
        ) | (
            GatewayAuthenticationSemanticsV1::Bearer
                | GatewayAuthenticationSemanticsV1::ApiKeyHeader { .. },
            NativeModelConnectionCredentialV1::PendingInput
                | NativeModelConnectionCredentialV1::Protected { .. }
                | NativeModelConnectionCredentialV1::Saved { .. }
        )
    );
    let credential_valid = credential_kind_valid && credential_shape_valid(credential);
    let provenance_valid = match &draft.provenance {
        NativeConnectionProvenanceInputV1::Registered {
            connection_option_id,
            registry_version,
            catalog_digest,
        } => {
            bounded_text(connection_option_id, 256)
                && bounded_text(registry_version, 256)
                && catalog_digest != &CanonicalDigest::of_bytes(&[])
        }
        NativeConnectionProvenanceInputV1::UserConfigured {
            configuration_revision,
        } => *configuration_revision > 0,
    };
    let mut model_ids = std::collections::BTreeSet::new();
    let models_valid = draft.models.iter().all(|model| {
        bounded_text(&model.upstream_model_id, 512)
            && model.upstream_model_id.trim() == model.upstream_model_id
            && bounded_text(&model.display_name, 256)
            && model
                .catalog_configuration_id
                .as_ref()
                .is_none_or(|value| bounded_text(value, 256))
            && model_ids.insert(model.upstream_model_id.as_str())
            && capability_shape_valid(&model.capabilities)
    });
    if qualification_valid && credential_valid && provenance_valid && models_valid {
        Ok(())
    } else {
        Err(NativeModelConnectionErrorV1::InvalidDraft)
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes
}

fn credential_shape_valid(credential: &NativeModelConnectionCredentialV1<'_>) -> bool {
    match credential {
        NativeModelConnectionCredentialV1::NotRequired
        | NativeModelConnectionCredentialV1::PendingInput => true,
        NativeModelConnectionCredentialV1::Protected {
            descriptor,
            input_slot,
            ..
        } => bounded_text(input_slot, 256) && protected_descriptor_shape_valid(descriptor),
        NativeModelConnectionCredentialV1::Saved {
            credential_id,
            expected_generation,
            ..
        } => bounded_text(credential_id, 256) && *expected_generation > 0,
    }
}

fn protected_descriptor_shape_valid(descriptor: &ProtectedInputSourceDescriptorV1) -> bool {
    match descriptor {
        ProtectedInputSourceDescriptorV1::ManualInput => true,
        ProtectedInputSourceDescriptorV1::DiscoveredConfig {
            scanner_id,
            scanner_version,
            source_ref,
            field_selector,
            observed_revision,
        } => {
            bounded_text(scanner_id, 128)
                && bounded_text(scanner_version, 128)
                && bounded_text(source_ref, 512)
                && bounded_text(field_selector, 512)
                && *observed_revision > 0
        }
    }
}

fn capability_shape_valid(capabilities: &NativeModelCapabilityDeclarationV1) -> bool {
    fact_shape_valid(&capabilities.tool)
        && fact_shape_valid(&capabilities.vision)
        && fact_shape_valid(&capabilities.streaming)
        && fact_shape_valid(&capabilities.context_tokens)
        && fact_shape_valid(&capabilities.max_output_tokens)
        && fact_shape_valid(&capabilities.native_reasoning)
        && capabilities
            .native_reasoning
            .value
            .as_ref()
            .is_none_or(|value| value.validate().is_ok())
}

fn fact_shape_valid<T>(fact: &NativeCandidateFactValueV1<T>) -> bool {
    matches!(
        (&fact.value, fact.basis),
        (None, NativeCandidateFactBasisV1::Unknown)
            | (
                Some(_),
                NativeCandidateFactBasisV1::RegisteredCatalog
                    | NativeCandidateFactBasisV1::RuntimeFallback
                    | NativeCandidateFactBasisV1::Observed
                    | NativeCandidateFactBasisV1::UserDeclared
            )
    )
}
