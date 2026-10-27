use super::{Result, require};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Case {
    pub id: &'static str,
    pub domain: &'static str,
    pub requirement: &'static str,
    pub scope: &'static str,
    pub owner: &'static str,
    pub steps: &'static [&'static str],
    pub expected_red: Option<&'static str>,
    pub unavailable: Option<&'static str>,
    #[serde(skip)]
    pub(super) execute: Option<fn(&mut super::run::Context<'_>) -> Result<()>>,
}

const CASES: &[Case] = &[
    Case {
        id: "gateway.responses.controlled",
        domain: "gateway",
        requirement: "SPEC-32001",
        scope: "production_gateway/controlled_upstream/protocol_client",
        owner: "gateway",
        steps: &["gateway_build", "gateway_run", "gateway_evidence"],
        expected_red: None,
        unavailable: None,
        execute: Some(super::gateway::execute),
    },
    Case {
        id: "control.status-discovery",
        domain: "control",
        requirement: "SPEC-139002",
        scope: "production_cli_released_commands/staged_local_control/synthetic_agent_discovery",
        owner: "TASK-142003",
        steps: &[
            "control_build",
            "control_storage_prepare",
            "control_ready",
            "system_status",
            "client_service_status",
            "operation_idempotency_lookup",
            "agents_scan",
            "agents_list",
            "control_cleanup",
        ],
        expected_red: None,
        unavailable: None,
        execute: Some(super::control::discover),
    },
    Case {
        id: "control.embedded-catalog",
        domain: "control",
        requirement: "SPEC-139001",
        scope: "production_cli_released_commands/embedded_catalog/storage_tampering_ignored",
        owner: "TASK-142003",
        steps: &[
            "control_build",
            "control_storage_prepare",
            "embedded_catalog_ready",
            "storage_catalog_ignored",
            "embedded_catalog_cleanup",
        ],
        expected_red: None,
        unavailable: None,
        execute: Some(super::control::embedded_catalog),
    },
    Case {
        id: "agent.codex.restore",
        domain: "agent",
        requirement: "SPEC-32005",
        scope: "real_codex/formal_configuration_restore",
        owner: "MVP-14",
        steps: &[],
        expected_red: None,
        unavailable: Some("formal_agent_adapter_unavailable"),
        execute: None,
    },
    Case {
        id: "agent.claude.restore",
        domain: "agent",
        requirement: "SPEC-32005",
        scope: "real_claude/formal_configuration_restore",
        owner: "MVP-14",
        steps: &[],
        expected_red: None,
        unavailable: Some("formal_agent_adapter_unavailable"),
        execute: None,
    },
    Case {
        id: "desktop.plan.rename",
        domain: "desktop",
        requirement: "SPEC-28007",
        scope: "real_tauri/client_core/plan_rename_and_return",
        owner: "MVP-02",
        steps: &[],
        expected_red: None,
        unavailable: Some("desktop_adapter_unavailable"),
        execute: None,
    },
    Case {
        id: "real-account.responses",
        domain: "real-account",
        requirement: "SPEC-32004",
        scope: "production_real_account",
        owner: "Gateway domain",
        steps: &[],
        expected_red: None,
        unavailable: Some("real_account_adapter_and_authorization_unavailable"),
        execute: None,
    },
    Case {
        id: "install.windows",
        domain: "install",
        requirement: "SPEC-32004",
        scope: "windows_installed_candidate",
        owner: "MVP-18",
        steps: &[],
        expected_red: None,
        unavailable: Some("installation_adapter_unavailable"),
        execute: None,
    },
];

pub fn catalog() -> &'static [Case] {
    CASES
}

/// Empty filters mean the three required quick cases. Explicit IDs are exact, not globs.
pub fn select(domains: &[String], ids: &[String]) -> Result<Vec<Case>> {
    require(
        domains.iter().collect::<BTreeSet<_>>().len() == domains.len(),
        "duplicate_domain",
    )?;
    require(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "duplicate_case",
    )?;
    for domain in domains {
        require(CASES.iter().any(|c| c.domain == domain), "unknown_domain")?;
    }
    for id in ids {
        require(CASES.iter().any(|c| c.id == id), "unknown_case")?;
    }
    if domains.is_empty() && ids.is_empty() {
        return Ok(CASES[..3].to_vec());
    }
    let chosen = CASES
        .iter()
        .filter(|c| {
            (domains.is_empty() || domains.iter().any(|d| d == c.domain))
                && (ids.is_empty() || ids.iter().any(|id| id == c.id))
        })
        .copied()
        .collect::<Vec<_>>();
    require(!chosen.is_empty(), "zero_cases")?;
    require(
        ids.iter().all(|id| chosen.iter().any(|c| c.id == id)),
        "requested_case_not_selected",
    )?;
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_never_silently_drops_requested_cases() {
        assert_eq!(select(&[], &[]).unwrap().len(), 3);
        assert_eq!(select(&["control".into()], &[]).unwrap().len(), 2);
        assert!(select(&[], &["control.stauts".into()]).is_err());
        assert!(select(&["gateway".into()], &["control.status-discovery".into()]).is_err());
        assert!(
            select(
                &[],
                &["agent.codex.restore".into(), "agent.codex.restore".into()]
            )
            .is_err()
        );
    }
}
