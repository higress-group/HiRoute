//! Immutable connection versions, committed with the existing terminal Operation transaction.
use super::ControlStore;
use hiroute_domain::{
    DecisionServiceChangeV1, DecisionServiceV1, OperationState, OperationV1, PortError,
    PortErrorCode, PortResult, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "decision_service.invalid")
}
fn storage() -> PortError {
    PortError::new(PortErrorCode::Unavailable, "decision_service.storage")
}

impl ControlStore {
    pub fn decision_service(
        &self,
        workspace: &WorkspaceId,
        id: &str,
        revision: u64,
    ) -> PortResult<Option<DecisionServiceV1>> {
        let connection = self.connection.borrow();
        let encoded: Option<String> = connection.query_row(
            "SELECT service_json FROM decision_services WHERE workspace_id=?1 AND service_id=?2 AND revision=?3",
            params![workspace.as_str(), id, revision],
            |row| row.get(0),
        ).optional().map_err(|_| storage())?;
        encoded
            .map(|encoded| {
                let value: DecisionServiceV1 =
                    serde_json::from_str(&encoded).map_err(|_| invalid())?;
                if !value.validate() || value.id != id || value.revision != revision {
                    return Err(invalid());
                }
                Ok(value)
            })
            .transpose()
    }

    pub fn decision_services(&self, workspace: &WorkspaceId) -> PortResult<Vec<DecisionServiceV1>> {
        let connection = self.connection.borrow();
        let mut statement = connection.prepare("SELECT service_json FROM decision_services d WHERE workspace_id=?1 AND revision=(SELECT MAX(revision) FROM decision_services WHERE workspace_id=d.workspace_id AND service_id=d.service_id) ORDER BY service_id").map_err(|_| storage())?;
        let rows = statement
            .query_map([workspace.as_str()], |row| row.get::<_, String>(0))
            .map_err(|_| storage())?;
        rows.map(|row| {
            let value: DecisionServiceV1 =
                serde_json::from_str(&row.map_err(|_| storage())?).map_err(|_| invalid())?;
            if !value.validate() {
                return Err(invalid());
            }
            Ok(value)
        })
        .collect()
    }
}

pub(super) fn stage(
    connection: &Connection,
    workspace: &WorkspaceId,
    desired: &Value,
) -> PortResult<()> {
    let Some(value) = desired.get("decision_service_change") else {
        return Ok(());
    };
    let change: DecisionServiceChangeV1 =
        serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    check(connection, workspace, &change)
}

fn check(
    connection: &Connection,
    workspace: &WorkspaceId,
    change: &DecisionServiceChangeV1,
) -> PortResult<()> {
    change.validate().map_err(|_| invalid())?;
    let actual = connection
        .query_row(
            "SELECT MAX(revision) FROM decision_services WHERE workspace_id=?1 AND service_id=?2",
            params![workspace.as_str(), change.id],
            |row| row.get::<_, Option<u64>>(0),
        )
        .map_err(|_| storage())?
        .unwrap_or(0);
    if actual != change.expected_revision {
        return Err(PortError::new(
            PortErrorCode::Conflict,
            "decision_service.revision_conflict",
        ));
    }
    if change.service.is_none() {
        for sql in [
            "SELECT version_json FROM plan_versions WHERE workspace_id=?1",
            "SELECT draft_json FROM plan_drafts WHERE workspace_id=?1",
        ] {
            let mut statement = connection.prepare(sql).map_err(|_| storage())?;
            let rows = statement
                .query_map([workspace.as_str()], |row| row.get::<_, String>(0))
                .map_err(|_| storage())?;
            for row in rows {
                let value: Value =
                    serde_json::from_str(&row.map_err(|_| storage())?).map_err(|_| invalid())?;
                if references(&value, &change.id) {
                    return Err(PortError::new(
                        PortErrorCode::Conflict,
                        "decision_service.in_use",
                    ));
                }
            }
        }
    } else if change.input_slot.is_some() {
        let service = change.service.as_ref().ok_or_else(invalid)?;
        let reference = &service
            .connection
            .transport()
            .2
            .ok_or_else(invalid)?
            .value_secret_ref;
        // Credentials are immutable per version; replacing a key must not mutate published plans.
        let mut statement = connection
            .prepare("SELECT service_json FROM decision_services WHERE workspace_id=?1")
            .map_err(|_| storage())?;
        let rows = statement
            .query_map([workspace.as_str()], |row| row.get::<_, String>(0))
            .map_err(|_| storage())?;
        for row in rows {
            let old: DecisionServiceV1 =
                serde_json::from_str(&row.map_err(|_| storage())?).map_err(|_| invalid())?;
            if old
                .connection
                .transport()
                .2
                .is_some_and(|header| &header.value_secret_ref == reference)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn references(value: &Value, id: &str) -> bool {
    match value {
        Value::Object(map) => {
            (map.get("kind").and_then(Value::as_str) == Some("decision_service")
                && map
                    .get("service")
                    .and_then(|s| s.get("id"))
                    .and_then(Value::as_str)
                    == Some(id))
                || map.values().any(|v| references(v, id))
        }
        Value::Array(values) => values.iter().any(|v| references(v, id)),
        _ => false,
    }
}

pub(super) fn finish(connection: &Connection, operation: &OperationV1) -> PortResult<()> {
    if operation.state != OperationState::Succeeded {
        return Ok(());
    }
    let staged: Option<String> = connection.query_row("SELECT staged_json FROM control_effects WHERE operation_id=?1 AND activated=1 AND compensated=0", [operation.operation_id.as_str()], |row| row.get(0)).optional().map_err(|_| storage())?;
    let Some(staged) = staged else {
        return Ok(());
    };
    let value: Value = serde_json::from_str(&staged).map_err(|_| invalid())?;
    let Some(change) = value
        .get("value")
        .and_then(|v| v.get("decision_service_change"))
    else {
        return Ok(());
    };
    let change: DecisionServiceChangeV1 =
        serde_json::from_value(change.clone()).map_err(|_| invalid())?;
    // Repeated terminal recovery observes the already-committed operation and must be idempotent.
    let done: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM decision_service_operations WHERE operation_id=?1)",
            [operation.operation_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage())?;
    if done {
        return Ok(());
    }
    check(connection, &operation.workspace_id, &change)?;
    if let Some(service) = &change.service {
        let encoded = serde_json::to_string(service).map_err(|_| invalid())?;
        connection.execute("INSERT INTO decision_services(workspace_id,service_id,revision,service_json) VALUES(?1,?2,?3,?4)", params![operation.workspace_id.as_str(),service.id,service.revision,encoded]).map_err(|_| storage())?;
    } else {
        connection
            .execute(
                "DELETE FROM decision_services WHERE workspace_id=?1 AND service_id=?2",
                params![operation.workspace_id.as_str(), change.id],
            )
            .map_err(|_| storage())?;
    }
    connection
        .execute(
            "INSERT INTO decision_service_operations(operation_id) VALUES(?1)",
            [operation.operation_id.as_str()],
        )
        .map_err(|_| storage())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{ClassifierAuthHeaderV1, DecisionConnectionV1};
    use serde_json::json;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE decision_services(workspace_id TEXT,service_id TEXT,revision INTEGER,service_json TEXT); CREATE TABLE plan_versions(workspace_id TEXT,version_json TEXT); CREATE TABLE plan_drafts(workspace_id TEXT,draft_json TEXT);").unwrap();
        connection
    }
    fn service() -> DecisionServiceV1 {
        DecisionServiceV1 {
            id: "decision-main".into(),
            revision: 1,
            name: "百炼".into(),
            connection: DecisionConnectionV1::SystemOne {
                provider: "bailian-token-plan".into(),
                model: "decision-model-preview".into(),
                endpoint: "https://example.test/systemone".into(),
                timeout_ms: 1000,
                auth_header: ClassifierAuthHeaderV1 {
                    name: "Authorization".into(),
                    value_secret_ref: "decision/main/v1".into(),
                },
            },
        }
    }
    #[test]
    fn exact_decision_revision_survives_a_newer_saved_connection() {
        let root = crate::test_tempdir().unwrap();
        let storage = crate::LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let control = storage.control();
        let workspace = WorkspaceId::default();
        let first = service();
        let mut latest = first.clone();
        latest.revision = 2;
        latest.name = "New connection".into();
        for value in [&first, &latest] {
            control.connection.borrow().execute(
                "INSERT INTO decision_services(workspace_id,service_id,revision,service_json) VALUES(?1,?2,?3,?4)",
                params![workspace.as_str(), value.id, value.revision, serde_json::to_string(value).unwrap()],
            ).unwrap();
        }
        assert_eq!(
            control.decision_services(&workspace).unwrap(),
            vec![latest.clone()]
        );
        assert_eq!(
            control.decision_service(&workspace, &first.id, 1).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            control.decision_service(&workspace, &first.id, 2).unwrap(),
            Some(latest)
        );
        assert_eq!(
            control.decision_service(&workspace, &first.id, 3).unwrap(),
            None
        );
        assert_eq!(
            control
                .decision_service(&workspace, "not-saved", 1)
                .unwrap(),
            None
        );
    }
    #[test]
    fn decision_service_edits_require_cas_and_never_overwrite_published_credentials() {
        let connection = connection();
        let workspace = WorkspaceId::default();
        let first = service();
        connection
            .execute(
                "INSERT INTO decision_services VALUES(?1,?2,1,?3)",
                params![
                    workspace.as_str(),
                    first.id,
                    serde_json::to_string(&first).unwrap()
                ],
            )
            .unwrap();
        let mut next = first.clone();
        next.revision = 2;
        let mut change = DecisionServiceChangeV1 {
            id: first.id,
            expected_revision: 1,
            service: Some(next),
            input_slot: None,
        };
        check(&connection, &workspace, &change).unwrap();
        change.input_slot = Some("replacement-input".into());
        assert!(check(&connection, &workspace, &change).is_err());
        if let DecisionConnectionV1::SystemOne { auth_header, .. } =
            &mut change.service.as_mut().unwrap().connection
        {
            auth_header.value_secret_ref = "decision/main/v2".into();
        }
        check(&connection, &workspace, &change).unwrap();
        change.expected_revision = 0;
        assert!(check(&connection, &workspace, &change).is_err());
    }
    #[test]
    fn decision_service_deletion_checks_drafts_and_frozen_plan_versions() {
        for table in ["plan_drafts", "plan_versions"] {
            let connection = connection();
            let workspace = WorkspaceId::default();
            let service = service();
            connection
                .execute(
                    "INSERT INTO decision_services VALUES(?1,?2,1,?3)",
                    params![
                        workspace.as_str(),
                        service.id,
                        serde_json::to_string(&service).unwrap()
                    ],
                )
                .unwrap();
            let change = DecisionServiceChangeV1 {
                id: service.id.clone(),
                expected_revision: 1,
                service: None,
                input_slot: None,
            };
            check(&connection, &workspace, &change).unwrap();
            let frozen = json!({"strategy":{"routing":{"classifier":{"kind":"decision_service","service":service}}}});
            connection
                .execute(
                    &format!("INSERT INTO {table} VALUES(?1,?2)"),
                    params![workspace.as_str(), frozen.to_string()],
                )
                .unwrap();
            assert!(check(&connection, &workspace, &change).is_err());
            connection
                .execute(&format!("DELETE FROM {table}"), [])
                .unwrap();
            check(&connection, &workspace, &change).unwrap();
        }
    }
}
