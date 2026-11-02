//! Selected native behavior, not a release label, authorizes borrowed provider settings.
use hiroute_domain::delegation::DelegationErrorV1;
use std::path::Path;

pub(super) fn require_host_managed_provider(
    binary: &Path,
    search_path: &str,
) -> Result<(), DelegationErrorV1> {
    hiroute_integrations::ClaudeNativeIngressProbe::check_host_managed_provider(binary, search_path)
        .map_err(DelegationErrorV1::DependencyCheckFailed)
}
