//! Fail-closed checks at the real rollback receipt left before its terminal write.
use super::*;
use hiroute_domain::ComputeManagementSourceV2;

fn snapshot(stores: &LocalStorageSet) -> Vec<Vec<String>> {
    stores.control().with_connection(|connection| {
        [
            "SELECT source_json,owner_operation_id,CAST(revision AS TEXT) FROM compute_management_sources ORDER BY source_id",
            "SELECT desired_json,desired_digest,owner_operation_id,CAST(target_revision AS TEXT) FROM workspace_state ORDER BY workspace_id",
            "SELECT CAST(revision AS TEXT) FROM workspace_revision_heads ORDER BY workspace_id",
        ].into_iter().flat_map(|sql| {
            let mut statement = connection.prepare(sql).unwrap();
            let columns = statement.column_count();
            statement.query_map([], |row| {
                (0..columns).map(|index| row.get::<_, String>(index)).collect()
            }).unwrap().collect::<Result<Vec<Vec<String>>, _>>().unwrap()
        }).collect()
    })
}

pub(super) fn assert_compensation_owner_and_receipt_guards(
    stores: &LocalStorageSet,
    operation_id: &hiroute_domain::OperationId,
    before: &ComputeManagementSourceV2,
    restored: &ComputeManagementSourceV2,
) {
    let operation = stores
        .control()
        .load_operation(operation_id)
        .unwrap()
        .unwrap();
    let references: Vec<_> = restored
        .credentials
        .iter()
        .map(|key| key.credential.clone())
        .collect();
    let healthy = snapshot(stores);
    let before_owner: String = stores.control().with_connection(|connection| {
        connection.query_row("SELECT before_owner_operation_id FROM compute_management_effects WHERE operation_id=?1", [operation_id.as_str()], |row| row.get(0)).unwrap()
    });
    assert_ne!(before_owner, operation_id.as_str());
    let restore = || {
        stores.control().with_connection(|connection| {
        connection.execute("UPDATE compute_management_sources SET source_json=?1,owner_operation_id=?2,revision=?3", rusqlite::params![healthy[0][0], healthy[0][1], healthy[0][2]]).unwrap();
        connection.execute("UPDATE workspace_state SET desired_json=?1,desired_digest=?2,owner_operation_id=?3,target_revision=?4", rusqlite::params![healthy[1][0], healthy[1][1], healthy[1][2], healthy[1][3]]).unwrap();
        connection.execute("UPDATE workspace_revision_heads SET revision=?1", rusqlite::params![healthy[2][0]]).unwrap();
    })
    };

    // Exact original source bytes alone cannot authorize repair after its owner changed.
    stores.control().with_connection(|connection| {
        connection.execute("UPDATE compute_management_sources SET source_json=?1,revision=?2,owner_operation_id=?3", rusqlite::params![super::super::encode(before).unwrap(), before.revision, operation_id.as_str()]).unwrap();
    });
    let foreign = snapshot(stores);
    let error = stores
        .control()
        .reconcile_compute_compensation(&operation, &references)
        .unwrap_err();
    assert_eq!(
        error,
        PortError::new(PortErrorCode::Conflict, "compute.compensation.cas")
    );
    assert_eq!(snapshot(stores), foreign);
    restore();

    for field in ["owner", "digest", "head"] {
        stores.control().with_connection(|connection| match field {
            "owner" => connection
                .execute(
                    "UPDATE workspace_state SET owner_operation_id=?1",
                    [&before_owner],
                )
                .unwrap(),
            "digest" => connection
                .execute(
                    "UPDATE workspace_state SET desired_digest='corrupt-receipt'",
                    [],
                )
                .unwrap(),
            _ => connection
                .execute(
                    "UPDATE workspace_revision_heads SET revision=revision+1",
                    [],
                )
                .unwrap(),
        });
        let corrupted = snapshot(stores);
        let error = stores
            .control()
            .reconcile_compute_compensation(&operation, &references)
            .unwrap_err();
        assert_eq!(
            error,
            PortError::new(PortErrorCode::Conflict, "compute.compensation.receipt")
        );
        assert_eq!(snapshot(stores), corrupted);
        restore();
    }
    // Restoring the fixture allows the unchanged receipt to replay without another revision.
    stores
        .control()
        .reconcile_compute_compensation(&operation, &references)
        .unwrap();
    assert_eq!(snapshot(stores), healthy);
}
