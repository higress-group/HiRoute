use std::path::Path;

use rusqlite::Connection;

use super::{
    DatabaseKind, LATEST_SCHEMA_VERSION, NOOP_V9, convergence_v15, migrate, migration_sql,
};

#[derive(Clone, Copy)]
enum FrozenV14Owner {
    Collaboration,
    ComputeManagement,
}

fn frozen_v14_database(kind: DatabaseKind, owner: FrozenV14Owner) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for version in 1..=13 {
        connection
            .execute_batch(migration_sql(kind, version).unwrap())
            .unwrap();
        connection
            .pragma_update(None, "user_version", version)
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO schema_migrations(
                    version, applied_at, binary_version
                 ) VALUES (?1, 1, 'frozen-v14')",
                [version],
            )
            .unwrap();
    }
    let branch_sql = match (kind, owner) {
        (DatabaseKind::Secrets, FrozenV14Owner::Collaboration) => {
            convergence_v15::SECRETS_V14_COLLABORATION_PREPARATION
        }
        (DatabaseKind::Control, FrozenV14Owner::ComputeManagement) => {
            convergence_v15::CONTROL_COMPUTE_MANAGEMENT
        }
        _ => NOOP_V9,
    };
    connection.execute_batch(branch_sql).unwrap();
    connection.pragma_update(None, "user_version", 14).unwrap();
    connection
        .execute(
            "INSERT OR REPLACE INTO schema_migrations(
                version, applied_at, binary_version
             ) VALUES (14, 1, 'frozen-v14')",
            [],
        )
        .unwrap();
    connection
}

#[test]
fn both_frozen_v14_shapes_upgrade_without_losing_their_applied_history() {
    for owner in [
        FrozenV14Owner::Collaboration,
        FrozenV14Owner::ComputeManagement,
    ] {
        for kind in [
            DatabaseKind::Control,
            DatabaseKind::Runtime,
            DatabaseKind::Secrets,
        ] {
            let mut connection = frozen_v14_database(kind, owner);
            migrate(
                &mut connection,
                Path::new("unused.db"),
                kind,
                Path::new("unused-backups"),
                None,
                None,
                Some(LATEST_SCHEMA_VERSION),
            )
            .unwrap();

            let version: u32 = connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, LATEST_SCHEMA_VERSION);
            let frozen_history: u32 = connection
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations
                     WHERE version = 14 AND binary_version = 'frozen-v14'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(frozen_history, 1);

            if kind == DatabaseKind::Control {
                for table in ["compute_management_sources", "compute_management_effects"] {
                    let present: u32 = connection
                        .query_row(
                            "SELECT COUNT(*) FROM sqlite_schema
                             WHERE type = 'table' AND name = ?1",
                            [table],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_eq!(present, 1);
                }
            }
            if kind == DatabaseKind::Secrets {
                let prepared_grant_columns: u32 = connection
                    .query_row(
                        "SELECT COUNT(*)
                         FROM pragma_table_info('agent_collaboration_credentials')
                         WHERE name = 'prepared_grant_json'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(prepared_grant_columns, 1);
            }
        }
    }
}
