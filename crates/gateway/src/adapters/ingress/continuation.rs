use crate::server::core_runtime::model_ir::{ExactProviderPathV1, ModelIrError};

/// Only opaque provider state needs trusted, source-specific ownership.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IngressRequestBindings {
    pub provider_state_owner: Option<ExactProviderPathV1>,
}

pub(super) fn validate_bindings(bindings: &IngressRequestBindings) -> Result<(), ModelIrError> {
    if bindings
        .provider_state_owner
        .as_ref()
        .is_some_and(|owner| !owner.is_complete())
    {
        return Err(ModelIrError::ProviderStateOwnershipRequired);
    }
    Ok(())
}
