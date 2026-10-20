//! Product-owned grant/Agent provenance. This grants no request authority and never
//! derives an Agent from client-supplied conversation identifiers or headers.
use super::LocalObservationStore;
use crate::writer::ObservationStoreError;
use hiroute_domain::{ExecutionFactEnvelopeV1, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observation_agent_grants_v1 (
    workspace_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    grant_generation INTEGER NOT NULL CHECK(grant_generation > 0),
    connection_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    PRIMARY KEY(workspace_id, grant_id, grant_generation)
);
CREATE TABLE IF NOT EXISTS observation_session_agents_v1 (
    workspace_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    conflicted INTEGER NOT NULL DEFAULT 0 CHECK(conflicted IN (0,1)),
    PRIMARY KEY(workspace_id, session_id),
    FOREIGN KEY(workspace_id,session_id) REFERENCES sessions(workspace_id,session_id) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS observation_session_agent_erasure_v1
AFTER UPDATE OF agent_id,correlation,tombstone_reason ON sessions
WHEN NEW.agent_id='' AND NEW.correlation='unproven' AND NEW.tombstone_reason IS NOT NULL
BEGIN
    DELETE FROM observation_session_agents_v1
    WHERE workspace_id=NEW.workspace_id AND session_id=NEW.session_id;
END;
"#;

impl LocalObservationStore {
    /// Register only after the composition owner verifies the exact Agent operation
    /// and its grant against the publication. Kept across rotation and restoration;
    /// registration deliberately does not backfill existing sessions.
    pub fn register_agent_grant(
        &self,
        workspace: &WorkspaceId,
        grant_id: &str,
        generation: u64,
        connection_id: &str,
        agent_id: &str,
    ) -> Result<(), ObservationStoreError> {
        if generation == 0
            || generation > i64::MAX as u64
            || [grant_id, connection_id, agent_id]
                .iter()
                .any(|s| s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
        {
            return Err(ObservationStoreError::Corrupt);
        }
        let mut db = self.connection.lock();
        let tx = db
            .transaction()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        tx.execute("INSERT OR IGNORE INTO observation_agent_grants_v1(workspace_id,grant_id,grant_generation,connection_id,agent_id) VALUES(?1,?2,?3,?4,?5)", params![workspace.as_str(),grant_id,generation,connection_id,agent_id]).map_err(|_| ObservationStoreError::ActivityUnavailable)?;
        let identity: (String,String) = tx.query_row("SELECT connection_id,agent_id FROM observation_agent_grants_v1 WHERE workspace_id=?1 AND grant_id=?2 AND grant_generation=?3",params![workspace.as_str(),grant_id,generation],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_| ObservationStoreError::Corrupt)?;
        if identity != (connection_id.into(), agent_id.into()) {
            return Err(ObservationStoreError::Corrupt);
        }
        tx.commit()
            .map_err(|_| ObservationStoreError::ActivityUnavailable)
    }
}

pub(crate) fn project(db: &Connection, event: &ExecutionFactEnvelopeV1) -> rusqlite::Result<()> {
    let workspace = event.correlation.workspace_id.as_str();
    let session = event.correlation.conversation_id.as_str();
    let agent: Option<String> = db.query_row("SELECT agent_id FROM observation_agent_grants_v1 WHERE workspace_id=?1 AND grant_id=?2 AND grant_generation=?3",params![workspace,event.trust.grant_id,event.trust.grant_generation],|r|r.get(0)).optional()?;
    let Some(agent) = agent else {
        return Ok(());
    };
    db.execute("INSERT INTO observation_session_agents_v1(workspace_id,session_id,agent_id) VALUES(?1,?2,?3)
        ON CONFLICT(workspace_id,session_id) DO UPDATE SET conflicted=MAX(conflicted,agent_id!=excluded.agent_id)",params![workspace,session,agent])?;
    db.execute("UPDATE sessions SET agent_id=(SELECT CASE WHEN conflicted=0 THEN agent_id ELSE '' END FROM observation_session_agents_v1 WHERE workspace_id=?1 AND session_id=?2) WHERE workspace_id=?1 AND session_id=?2",params![workspace,session])?;
    Ok(())
}
