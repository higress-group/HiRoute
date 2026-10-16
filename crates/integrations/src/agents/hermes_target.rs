//! Validated effective-profile targets, shared by settings and passive source discovery.
use super::hermes_yaml::{HermesDocument, HermesNativeError, hermes_error};
use hiroute_domain::{
    AgentConnectionEffectRoleV1, AgentConnectionTransactionSubjectV1, AgentNativeTargetV1,
    CanonicalDigest,
};
use std::path::{Path, PathBuf};

pub fn hermes_context_id(root: &str) -> String {
    format!(
        "agent-context/hermes/{}",
        CanonicalDigest::of_bytes(root.as_bytes()).as_str()
    )
}

pub fn canonical_hermes_target(
    target: &AgentNativeTargetV1,
) -> Result<AgentNativeTargetV1, HermesNativeError> {
    target
        .validate()
        .map_err(|_| hermes_error("absolute profile root"))?;
    let input = Path::new(target.config_root());
    let root = input
        .canonicalize()
        .map_err(|_| hermes_error("selected profile root unavailable"))?;
    if !root.is_dir() {
        return Err(hermes_error("profile directory required"));
    }
    let config_root = root
        .to_str()
        .ok_or_else(|| hermes_error("profile path encoding"))?
        .to_owned();
    let target = AgentNativeTargetV1::Hermes { config_root };
    target
        .validate()
        .map_err(|_| hermes_error("canonical profile root"))?;
    Ok(target)
}

pub fn hermes_subject(
    root: &str,
) -> Result<AgentConnectionTransactionSubjectV1, HermesNativeError> {
    let digest = CanonicalDigest::of_bytes(root.as_bytes());
    AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent_hermes_default",
        format!("hermes-main-{}", &digest.as_str()[7..]),
        super::HERMES_INTEGRATION_PROFILE_REF_V1,
    )
    .map_err(|_| hermes_error("profile identity"))
}

/// The caller uses a freshly validated request target or an accepted Operation's sealed target.
/// This computes bindings only; it does not create/read files or confer write authority.
pub fn hermes_external_targets(root: &str) -> Result<Vec<(String, PathBuf)>, HermesNativeError> {
    let subject = hermes_subject(root)?;
    [
        (
            AgentConnectionEffectRoleV1::ManagedConfiguration,
            "config.yaml",
        ),
        (
            AgentConnectionEffectRoleV1::RoutingSkill,
            "skills/hiroute-management/SKILL.md",
        ),
        (
            AgentConnectionEffectRoleV1::CollaborationSkill,
            "skills/hiroute-collaboration/SKILL.md",
        ),
        (
            AgentConnectionEffectRoleV1::HermesPluginModule,
            "plugins/hiroute/__init__.py",
        ),
        (
            AgentConnectionEffectRoleV1::HermesPluginManifest,
            "plugins/hiroute/plugin.yaml",
        ),
    ]
    .into_iter()
    .map(|(role, path)| {
        Ok((
            role.settings_target_for(&subject)
                .map_err(|_| hermes_error("target identity"))?,
            Path::new(root).join(path),
        ))
    })
    .collect()
}

pub fn hermes_configuration_digest(root: &str) -> Result<CanonicalDigest, HermesNativeError> {
    let path = Path::new(root).join("config.yaml");
    let observed = super::filesystem_config::read_validated_config_bytes(&path)
        .map_err(|_| hermes_error("configuration access"))?;
    let bytes = observed
        .as_ref()
        .map_or(&[][..], |(bytes, _)| bytes.as_slice());
    HermesDocument::parse(bytes)?;
    Ok(CanonicalDigest::of_bytes(bytes))
}
