use hiroute_domain::{
    GatewayPublicationV1, PlanDraftV1, PlanReferenceProtectionV1, PlanVersionV1, PortError,
    PortErrorCode, PortResult, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension};

#[cfg(test)]
thread_local! {
    pub(super) static PUBLICATION_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

const CURRENT: &str = "EXISTS(SELECT 1 FROM plan_heads h WHERE h.workspace_id=v.workspace_id AND h.plan_id=v.plan_id AND json_extract(h.head_json,'$.status') != 'deleted' AND json_extract(h.head_json,'$.reference.content_revision')=v.content_revision)";
const HELD: &str = "EXISTS(SELECT 1 FROM plan_version_holds h WHERE h.workspace_id=v.workspace_id AND h.plan_id=v.plan_id AND h.content_revision=v.content_revision)";
const INVALID_HEAD: &str = "EXISTS(SELECT 1 FROM plan_heads h WHERE h.workspace_id=v.workspace_id AND h.plan_id=v.plan_id AND (json_extract(h.head_json,'$.status') IS NULL OR json_extract(h.head_json,'$.status') NOT IN ('enabled','disabled','deleted') OR json_type(h.head_json,'$.reference.content_revision') IS NOT 'integer' OR json_extract(h.head_json,'$.reference.plan_id') IS NOT h.plan_id OR json_extract(h.head_json,'$.reference.workspace_id') IS NOT h.workspace_id))";

fn unavailable(_: rusqlite::Error) -> PortError {
    PortError::new(
        PortErrorCode::Unavailable,
        "compute.management.references.read",
    )
}

fn corrupt() -> PortError {
    PortError::new(
        PortErrorCode::Corrupt,
        "compute.management.references.corrupt",
    )
}

pub(super) struct ActivePlanReferences<'a> {
    connection: &'a Connection,
    workspace: &'a WorkspaceId,
    pub publications: Vec<(PlanReferenceProtectionV1, GatewayPublicationV1)>,
}

impl<'a> ActivePlanReferences<'a> {
    pub fn read(connection: &'a Connection, workspace: &'a WorkspaceId) -> PortResult<Self> {
        let has_versions: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM plan_versions WHERE workspace_id=?1)",
                [workspace.as_str()],
                |row| row.get(0),
            )
            .map_err(unavailable)?;
        let recovery: Option<bool> = connection
            .query_row(
                "SELECT ready FROM plan_version_recovery WHERE workspace_id=?1",
                [workspace.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(unavailable)?;
        if recovery == Some(false) || (has_versions && recovery != Some(true)) {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "compute.management.references.recovery_required",
            ));
        }
        let mut statement = connection.prepare(
            "SELECT state,publication_bytes FROM gateway_publications WHERE workspace_id=?1 AND state IN ('active','prepared','lkg')",
        ).map_err(unavailable)?;
        let rows = statement
            .query_map([workspace.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(unavailable)?;
        let mut publications = Vec::new();
        for row in rows {
            let (state, bytes) = row.map_err(unavailable)?;
            #[cfg(test)]
            PUBLICATION_DECODES.set(PUBLICATION_DECODES.get() + 1);
            let publication =
                GatewayPublicationV1::decode_persisted(&bytes).map_err(|_| corrupt())?;
            if publication.workspace_id != *workspace {
                return Err(corrupt());
            }
            let protection = match state.as_str() {
                "active" => PlanReferenceProtectionV1::ActivePublication,
                "prepared" => PlanReferenceProtectionV1::PreparedPublication,
                "lkg" => PlanReferenceProtectionV1::LkgPublication,
                _ => return Err(corrupt()),
            };
            publications.push((protection, publication));
        }
        Ok(Self {
            connection,
            workspace,
            publications,
        })
    }

    pub fn versions(
        &self,
        service: Option<&str>,
        mut visit: impl FnMut(PlanVersionV1, Vec<PlanReferenceProtectionV1>) -> PortResult<()>,
    ) -> PortResult<()> {
        let source = if service.is_some() {
            "decision_plan_references r LEFT JOIN plan_versions v ON v.workspace_id=r.workspace_id AND v.plan_id=r.owner_id AND v.content_revision=r.owner_revision WHERE r.workspace_id=?1 AND r.service_id=?2 AND r.owner_kind='version'"
        } else {
            "plan_versions v WHERE v.workspace_id=?1"
        };
        let query = format!(
            "SELECT version_json,state,current_head,held,invalid_head FROM (SELECT v.version_json,v.state,{CURRENT} AS current_head,{HELD} AS held,{INVALID_HEAD} AS invalid_head FROM {source}) WHERE state='prepared' OR current_head OR held OR invalid_head OR state IS NULL OR state NOT IN ('prepared','published')"
        );
        let mut statement = self.connection.prepare(&query).map_err(unavailable)?;
        let mut rows = if let Some(service) = service {
            statement
                .query([self.workspace.as_str(), service])
                .map_err(unavailable)?
        } else {
            statement
                .query([self.workspace.as_str()])
                .map_err(unavailable)?
        };
        while let Some(row) = rows.next().map_err(unavailable)? {
            if row.get::<_, bool>(4).map_err(unavailable)? {
                return Err(corrupt());
            }
            let json: String = row.get(0).map_err(unavailable)?;
            let state: String = row.get(1).map_err(unavailable)?;
            let current: bool = row.get(2).map_err(unavailable)?;
            let held: bool = row.get(3).map_err(unavailable)?;
            let version: PlanVersionV1 = serde_json::from_str(&json).map_err(|_| corrupt())?;
            version.validate().map_err(|_| corrupt())?;
            if version.reference.workspace_id != *self.workspace
                || !matches!(state.as_str(), "prepared" | "published")
            {
                return Err(corrupt());
            }
            let mut reasons = Vec::new();
            if current {
                reasons.push(PlanReferenceProtectionV1::CurrentHead);
            }
            if state == "prepared" {
                reasons.push(PlanReferenceProtectionV1::PreparedVersion);
            }
            if held {
                reasons.push(PlanReferenceProtectionV1::UnresolvedHold);
            }
            visit(version, reasons)?;
        }
        Ok(())
    }

    pub fn drafts(
        &self,
        service: Option<&str>,
        mut visit: impl FnMut(PlanDraftV1) -> PortResult<()>,
    ) -> PortResult<()> {
        let query = if service.is_some() {
            "SELECT d.draft_json FROM decision_plan_references r LEFT JOIN plan_drafts d ON d.workspace_id=r.workspace_id AND d.draft_id=r.owner_id WHERE r.workspace_id=?1 AND r.service_id=?2 AND r.owner_kind='draft' AND r.owner_revision=0"
        } else {
            "SELECT draft_json FROM plan_drafts WHERE workspace_id=?1"
        };
        let mut statement = self.connection.prepare(query).map_err(unavailable)?;
        let mut rows = if let Some(service) = service {
            statement
                .query([self.workspace.as_str(), service])
                .map_err(unavailable)?
        } else {
            statement
                .query([self.workspace.as_str()])
                .map_err(unavailable)?
        };
        while let Some(row) = rows.next().map_err(unavailable)? {
            let json: String = row.get(0).map_err(unavailable)?;
            let draft: PlanDraftV1 = serde_json::from_str(&json).map_err(|_| corrupt())?;
            draft.validate().map_err(|_| corrupt())?;
            if draft.workspace_id != *self.workspace {
                return Err(corrupt());
            }
            visit(draft)?;
        }
        Ok(())
    }
}
