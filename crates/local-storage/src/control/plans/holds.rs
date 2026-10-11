use super::*;
use hiroute_domain::{VersionOwnerRefV1, VersionReservationV1};
use std::collections::BTreeSet;

impl ControlStore {
    pub fn require_plan_version_recovery_ready(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<(), PlanVersionError> {
        require_ready(&self.connection.borrow(), workspace)
    }
    /// Lifecycle deletion considers every revision, including old continuation holds and
    /// prepared content. An unfinished recovery cannot prove that no owner remains.
    pub fn plan_has_retained_versions(
        &self,
        workspace: &WorkspaceId,
        plan: &AgentPlanId,
    ) -> Result<bool, PlanVersionError> {
        let connection = self.connection.borrow();
        require_ready(&connection, workspace)?;
        connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM plan_version_holds WHERE workspace_id=?1 AND plan_id=?2)
             OR EXISTS(SELECT 1 FROM plan_versions WHERE workspace_id=?1 AND plan_id=?2 AND state='prepared')",
            params![workspace.as_str(), plan.as_str()], |row| row.get(0),
        ).map_err(storage)
    }

    /// Each daemon startup closes retention recovery before exposing its admission ports.
    pub fn begin_plan_version_recovery(&self) -> Result<(), PlanVersionError> {
        self.connection
            .borrow()
            .execute("UPDATE plan_version_recovery SET ready=0", [])
            .map_err(storage)?;
        Ok(())
    }

    /// Reservation acquisition is one short control transaction. The caller retains the shared
    /// admission Guard, closes this transaction, then commits runtime acceptance separately.
    pub fn acquire_exact_plan_version(
        &self,
        reservation: &VersionReservationV1,
    ) -> Result<PlanVersionV1, PlanVersionError> {
        reservation.owner.validate()?;
        reservation.reference.validate()?;
        if reservation.expires_at_unix <= 0 {
            return Err(PlanVersionError::Invalid);
        }
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        require_ready(&tx, &reservation.reference.workspace_id)?;
        let version = lookup_in(&tx, &reservation.reference)?;
        retain_in(&tx, reservation, false)?;
        tx.commit().map_err(storage)?;
        Ok(version)
    }

    pub fn renew_plan_version(
        &self,
        workspace: &WorkspaceId,
        owner: &VersionOwnerRefV1,
        expires_at_unix: i64,
    ) -> Result<(), PlanVersionError> {
        owner.validate()?;
        if expires_at_unix <= 0 {
            return Err(PlanVersionError::Invalid);
        }
        let changed = self.connection.borrow().execute("UPDATE plan_version_holds SET expires_at=max(expires_at,?3) WHERE workspace_id=?1 AND owner_key=?2", params![workspace.as_str(), encode(owner)?, expires_at_unix]).map_err(storage)?;
        if changed == 0 {
            Err(PlanVersionError::Unavailable)
        } else {
            Ok(())
        }
    }

    pub fn release_plan_version(
        &self,
        workspace: &WorkspaceId,
        owner: &VersionOwnerRefV1,
    ) -> Result<(), PlanVersionError> {
        owner.validate()?;
        self.connection
            .borrow()
            .execute(
                "DELETE FROM plan_version_holds WHERE workspace_id=?1 AND owner_key=?2",
                params![workspace.as_str(), encode(owner)?],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// `owners` is 20's COMPLETE durable owner set for this workspace after recovery, including
    /// explicitly continuable tasks. It is not a page or a caller-supplied task list. Verify every
    /// reference before changing anything; an unavailable version leaves the barrier closed.
    pub fn reconcile_plan_versions(
        &self,
        workspace: &WorkspaceId,
        owners: &[VersionReservationV1],
        now_unix: i64,
    ) -> Result<(), PlanVersionError> {
        let mut connection = self.connection.borrow_mut();
        connection.execute("INSERT INTO plan_version_recovery(workspace_id,ready) VALUES(?1,0) ON CONFLICT(workspace_id) DO UPDATE SET ready=0", [workspace.as_str()]).map_err(storage)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut keys = BTreeSet::new();
        for owner in owners {
            owner.owner.validate()?;
            if &owner.reference.workspace_id != workspace
                || owner.expires_at_unix <= 0
                || !keys.insert(encode(&owner.owner)?)
            {
                return Err(PlanVersionError::Invalid);
            }
            lookup_in(&tx, &owner.reference)?;
            retain_in(&tx, owner, true)?;
        }
        let old = {
            let mut stmt = tx.prepare("SELECT owner_key,expires_at,confirmed_owner FROM plan_version_holds WHERE workspace_id=?1").map_err(storage)?;
            stmt.query_map([workspace.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?
        };
        for (key, expires, confirmed) in old {
            if !keys.contains(&key) && (confirmed || expires <= now_unix) {
                tx.execute(
                    "DELETE FROM plan_version_holds WHERE workspace_id=?1 AND owner_key=?2",
                    params![workspace.as_str(), key],
                )
                .map_err(storage)?;
            }
        }
        tx.execute("INSERT INTO plan_version_recovery(workspace_id,ready) VALUES(?1,1) ON CONFLICT(workspace_id) DO UPDATE SET ready=1", [workspace.as_str()]).map_err(storage)?;
        tx.commit().map_err(storage)
    }

    pub fn plan_version_is_held(
        &self,
        reference: &PlanExecutionRef,
    ) -> Result<bool, PlanVersionError> {
        self.connection.borrow().query_row("SELECT EXISTS(SELECT 1 FROM plan_version_holds WHERE workspace_id=?1 AND plan_id=?2 AND content_revision=?3)", params![reference.workspace_id.as_str(), reference.plan_id.as_str(), reference.content_revision], |r| r.get(0)).map_err(storage)
    }
}

pub(super) fn require_ready(
    connection: &Connection,
    workspace: &WorkspaceId,
) -> Result<(), PlanVersionError> {
    let ready: Option<bool> = connection
        .query_row(
            "SELECT ready FROM plan_version_recovery WHERE workspace_id=?1",
            [workspace.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    if ready == Some(true) {
        Ok(())
    } else {
        Err(PlanVersionError::RecoveryRequired)
    }
}

fn retain_in(
    connection: &Connection,
    reservation: &VersionReservationV1,
    confirmed: bool,
) -> Result<(), PlanVersionError> {
    let key = encode(&reservation.owner)?;
    let reference = &reservation.reference;
    let old: Option<(String,u64,String)> = connection.query_row("SELECT plan_id,content_revision,content_digest FROM plan_version_holds WHERE workspace_id=?1 AND owner_key=?2", params![reference.workspace_id.as_str(),key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
    if old.as_ref().is_some_and(|(id, revision, digest)| {
        id != reference.plan_id.as_str()
            || revision != &reference.content_revision
            || digest != reference.content_digest.as_str()
    }) {
        return Err(PlanVersionError::Conflict);
    }
    connection.execute("INSERT INTO plan_version_holds(workspace_id,owner_key,plan_id,content_revision,content_digest,expires_at,confirmed_owner) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(workspace_id,owner_key) DO UPDATE SET expires_at=max(expires_at,excluded.expires_at),confirmed_owner=max(confirmed_owner,excluded.confirmed_owner)", params![reference.workspace_id.as_str(),key,reference.plan_id.as_str(),reference.content_revision,reference.content_digest.as_str(),reservation.expires_at_unix,confirmed]).map_err(storage)?;
    Ok(())
}
