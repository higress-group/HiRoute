use super::*;
use hiroute_domain::{TrustTaskOutcomeV1, TrustTaskPopulationV1};

impl RuntimeStore {
    pub fn trust_task_population(
        &self,
        workspace: &WorkspaceId,
        plan_id: &str,
        revision: Option<u64>,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<TrustTaskPopulationV1> {
        let connection = self.connection.borrow();
        let mut statement = connection.prepare(
            "SELECT t.record_json,r.record_json FROM delegation_tasks t
             LEFT JOIN delegation_runs r ON r.workspace_id=t.workspace_id AND r.run_id=json_extract(t.record_json,'$.latest_run_id')
             WHERE t.workspace_id=?1 AND json_extract(t.record_json,'$.plan.plan_id')=?2
               AND (?3 IS NULL OR json_extract(t.record_json,'$.plan.plan_revision')=?3)
               AND json_extract(t.record_json,'$.created_at_ms')>=?4 AND json_extract(t.record_json,'$.created_at_ms')<?5
             ORDER BY t.task_id LIMIT 50001"
        ).map_err(storage)?;
        let rows = statement
            .query_map(
                params![workspace.as_str(), plan_id, revision, from_ms, to_ms],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(storage)?;
        let mut tasks = Vec::new();
        for row in rows {
            let (task, run) = row.map_err(storage)?;
            let task: DelegationTaskV1 = decode(task)?;
            let run: Option<DelegationRunV1> = run.map(decode).transpose()?;
            tasks.push(TrustTaskOutcomeV1 {
                task_id: task.task_id,
                executor: serde_json::to_value(task.plan.harness)
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?
                    .as_str()
                    .ok_or(DelegationErrorV1::StorageUnavailable)?
                    .into(),
                ordinal: run.as_ref().map_or(0, |r| r.ordinal),
                state: run.map_or(RunStateV1::Unknown, |r| r.progress.state),
            });
        }
        let over_limit = tasks.len() > 50_000;
        Ok(TrustTaskPopulationV1 { tasks, over_limit })
    }
}
