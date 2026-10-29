//! Optimistic local-read fence. It does not represent native/CPA/source authority.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub struct ControlReadStamp {
    store: String,
    routing_inputs: u64,
}

impl ControlStore {
    /// Both same-connection and external relevant writes advance this revision in
    /// their own transaction. Native read-grant registration cannot invalidate it.
    /// CPA and catalog authority must still be checked by the caller.
    pub fn read_stamp(&self) -> PortResult<ControlReadStamp> {
        let db = self.connection.borrow();
        crate::migrations::routing_read_v32::validate(&db)
            .map_err(|_| port(PortErrorCode::Corrupt, "control.read-stamp.coverage"))?;
        db.query_row(
                "SELECT store_uuid,revision FROM storage_meta JOIN routing_read_revision USING(singleton)
             WHERE singleton=1 AND length(store_uuid)>0",
                [],
                |r| {
                    Ok(ControlReadStamp {
                        store: r.get(0)?,
                        routing_inputs: r.get(1)?,
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
        // A rolled-back change does not invalidate a committed local snapshot.
        let other = Connection::open(&path).unwrap();
        other
            .execute_batch("BEGIN; UPDATE storage_meta SET store_uuid=store_uuid; ROLLBACK;")
            .unwrap();
        assert_eq!(local, store.read_stamp().unwrap());
        other
            .execute(
                "UPDATE storage_meta SET store_uuid=store_uuid WHERE singleton=1",
                [],
            )
            .unwrap();
        assert_ne!(local, store.read_stamp().unwrap());
        other
            .execute_batch("DROP TRIGGER routing_read_plan_drafts_update")
            .unwrap();
        assert!(
            store.read_stamp().is_err(),
            "missing producer coverage is never a stable snapshot"
        );
    }
}
