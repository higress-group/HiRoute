//! Optimistic local-read fence. It does not represent native/CPA/source authority.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub struct ControlReadStamp {
    store: String,
    external_commits: u64,
    connection_changes: u64,
}

impl ControlStore {
    /// Unlike a workspace revision alone, this observes all local configuration writes,
    /// including writes made on this connection and commits made by another connection.
    /// Callers must still validate their independently owned external dependencies.
    pub fn read_stamp(&self) -> PortResult<ControlReadStamp> {
        self.connection
            .borrow()
            .query_row(
                "SELECT store_uuid,(SELECT data_version FROM pragma_data_version),total_changes()
             FROM storage_meta WHERE singleton=1 AND length(store_uuid)>0",
                [],
                |r| {
                    Ok(ControlReadStamp {
                        store: r.get(0)?,
                        external_commits: r.get(1)?,
                        connection_changes: r.get(2)?,
                    })
                },
            )
            .map_err(|_| port(PortErrorCode::Corrupt, "control.read-stamp.identity"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_same_connection_and_external_commits_invalidate_a_local_snapshot() {
        let root = crate::test_tempdir().unwrap();
        let path = root.path().join("control.db");
        let store = ControlStore::open(
            &crate::test_storage_authority(),
            &path,
            root.path().join("backups"),
        )
        .unwrap();
        let before = store.read_stamp().unwrap();
        assert_eq!(before, store.read_stamp().unwrap());
        store
            .connection
            .borrow()
            .execute(
                "UPDATE storage_meta SET store_uuid=store_uuid WHERE singleton=1",
                [],
            )
            .unwrap();
        let local = store.read_stamp().unwrap();
        assert_ne!(before, local);
        let other = Connection::open(&path).unwrap();
        other
            .execute(
                "UPDATE storage_meta SET store_uuid=store_uuid WHERE singleton=1",
                [],
            )
            .unwrap();
        assert_ne!(local, store.read_stamp().unwrap());
    }
}
