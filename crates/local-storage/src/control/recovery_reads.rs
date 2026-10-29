//! Recovery dependency reads keep the live writer's already decoded journal separate.
use super::*;

#[cfg(test)]
thread_local! { pub(super) static DECODE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

impl ControlStore {
    /// Exclude only an exact, committed caller journal, verified in the same
    /// snapshot as the complete recovery membership. Other unresolved operations
    /// retain full decoding. This is not an ID-based recovery exemption.
    pub fn other_recoverable_operations(
        &self,
        current: &OperationV1,
    ) -> PortResult<Vec<OperationV1>> {
        self.read_recoverable_operations(Some(current))
    }

    pub(super) fn read_recoverable_operations(
        &self,
        current: Option<&OperationV1>,
    ) -> PortResult<Vec<OperationV1>> {
        let db = self.connection.borrow();
        let snapshot = if db.is_autocommit() {
            Some(db.unchecked_transaction().map_err(|_| unavailable())?)
        } else {
            None
        };
        current_dependencies::validate_recovery_members(&db)?;
        if let Some(current) = current
            && !journal::is_current(&db, current)?
        {
            return Err(port(PortErrorCode::Conflict, "control.recovery.current"));
        }
        let mut statement = db.prepare(
            "SELECT operation_json FROM operation_recovery_members m JOIN operations USING(operation_id)
             WHERE (state NOT IN ('succeeded', 'rolled_back', 'needs_attention')
                OR (state = 'needs_attention' AND operation_id =
                    (SELECT operation_id FROM writer_claim WHERE singleton = 1)))
               AND (?1 IS NULL OR operation_id != ?1)
             ORDER BY created_at, operation_id",
        ).map_err(|_| unavailable())?;
        let rows = statement
            .query_map([current.map(|op| op.operation_id.as_str())], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| unavailable())?;
        let mut operations = Vec::new();
        for row in rows {
            let encoded = row.map_err(|_| port(PortErrorCode::Corrupt, "control.recovery.row"))?;
            #[cfg(test)]
            DECODE_COUNT.with(|count| count.set(count.get() + 1));
            operations.push(decode_stored_operation(
                &self.diagnostics.borrow(),
                &db,
                &encoded,
            )?);
        }
        drop(statement);
        if let Some(snapshot) = snapshot {
            snapshot.commit().map_err(|_| unavailable())?;
        }
        Ok(operations)
    }
}

fn unavailable() -> PortError {
    port(PortErrorCode::Unavailable, "control.recovery.read")
}
