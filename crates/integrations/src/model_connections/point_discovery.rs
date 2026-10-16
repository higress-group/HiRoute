//! Closed mixed native/catalog facts supplied by the trusted point preparation owner.
use super::*;

impl<P, T> NativeModelConnectionServiceV1<P, T>
where
    P: ComputeCandidatePort,
    T: ModelDirectoryTransportV1,
{
    /// No public payload can claim RegisteredCatalog. The daemon qualifies an exact product and
    /// protocol first, then fills only unknown native facts and appends eligible catalog models.
    pub fn prepare_native_point_discovery(
        &self,
        draft: NativeModelConnectionDraftV1,
        credential: NativeModelConnectionCredentialV1<'_>,
        evidence: CanonicalDigest,
    ) -> Result<ComputeCandidateViewV2, NativeModelConnectionErrorV1> {
        if !matches!(
            draft.provenance,
            NativeConnectionProvenanceInputV1::UserConfigured { .. }
        ) || draft.models.iter().any(|model| {
            let bases = [
                model.capabilities.tool.basis,
                model.capabilities.vision.basis,
                model.capabilities.streaming.basis,
                model.capabilities.context_tokens.basis,
                model.capabilities.max_output_tokens.basis,
                model.capabilities.native_reasoning.basis,
            ];
            !matches!(
                model.membership,
                ComputeModelMembershipV2::Observed | ComputeModelMembershipV2::Catalog
            ) || bases.iter().any(|basis| {
                !matches!(
                    basis,
                    NativeCandidateFactBasisV1::Observed
                        | NativeCandidateFactBasisV1::Unknown
                        | NativeCandidateFactBasisV1::RegisteredCatalog
                )
            }) || (bases.contains(&NativeCandidateFactBasisV1::RegisteredCatalog)
                && model.catalog_configuration_id.is_none())
                || (model.membership == ComputeModelMembershipV2::Catalog
                    && !registered_catalog_model(model))
        }) {
            return Err(NativeModelConnectionErrorV1::InvalidDraft);
        }
        self.prepare_static_discovery(draft, credential, evidence)
    }
}
