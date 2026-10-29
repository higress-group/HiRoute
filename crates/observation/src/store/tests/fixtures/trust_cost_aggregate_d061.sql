-- Frozen aggregate from d061487e94b752be043e42aef30434d531f899f0. Negative control; no purpose reads.
WITH candidates AS (
    SELECT r.workspace_id,r.request_id,r.session_id,s.updated_at_ms,t.segment_id,:idle AS idle_before,
      COALESCE(t.membership_conflict,0) AS conflict,q.plan_id AS stage_plan,
      q.plan_revision AS stage_revision,
      COALESCE(q.executed_branch_id,q.selected_branch_id) AS stage_branch,
      json_extract(q.branch_execution_json,'$.group') AS stage_group,
      a.request_id AS projected,
      EXISTS(SELECT 1 FROM observation_analysis_dirty_v1 d
        WHERE d.workspace_id=r.workspace_id AND d.request_id=r.request_id) AS dirty
    FROM logical_requests r
    JOIN sessions s ON s.workspace_id=r.workspace_id AND s.session_id=r.session_id
    JOIN valuation_requests_v2 v ON v.workspace_id=r.workspace_id AND v.request_id=r.request_id
    LEFT JOIN observation_analysis_turns_v1 t ON t.workspace_id=r.workspace_id AND t.request_id=r.request_id
    LEFT JOIN plan_quality_segments q ON q.workspace_id=r.workspace_id AND q.segment_id=t.segment_id AND q.session_id=r.session_id
    LEFT JOIN observation_analysis_requests_v1 a ON a.workspace_id=r.workspace_id AND a.request_id=r.request_id
      AND a.revision=(SELECT MAX(latest.revision) FROM observation_analysis_requests_v1 latest
        WHERE latest.workspace_id=r.workspace_id AND latest.request_id=r.request_id)
    WHERE r.workspace_id=:workspace AND v.plan_id=:plan AND r.started_at_ms>=:from AND r.started_at_ms<:to
      AND r.traffic_kind='normal' AND (s.tombstone_reason IS NULL OR s.content_completeness='deleted')
  ), scoped AS (
    SELECT *, COALESCE(conflict=0 AND stage_plan=:plan AND (
      (:revision IS NOT NULL AND stage_revision!=:revision)
      OR (:branch IS NOT NULL AND stage_branch!=:branch)
      OR (stage_group IN ('regular','primary') AND stage_group!=:group)),0) AS outside,
      COALESCE(conflict=0 AND stage_plan=:plan AND stage_revision IS NOT NULL
        AND (:revision IS NULL OR stage_revision=:revision)
        AND (:branch IS NULL OR stage_branch=:branch) AND stage_group=:group,0) AS matched
    FROM candidates
  ) SELECT COALESCE(MAX(dirty OR projected IS NULL
    OR (segment_id IS NULL AND updated_at_ms>idle_before)),0),
    COALESCE(SUM(NOT matched),0) FROM scoped WHERE NOT outside;
