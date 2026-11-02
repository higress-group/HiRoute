//! Upgrade the keyed reference projection without rewriting its authoritative owners.
pub(super) const CONTROL: &str = r#"
DROP TRIGGER decision_version_references_insert;
DROP TRIGGER decision_version_references_update;
DROP TRIGGER decision_draft_references_insert;
DROP TRIGGER decision_draft_references_update;
CREATE TRIGGER decision_version_references_insert AFTER INSERT ON plan_versions BEGIN
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'version', NEW.plan_id, NEW.content_revision
    FROM json_tree(NEW.version_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2', 'decision_service_v3')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_version_references_update AFTER UPDATE OF workspace_id, plan_id, content_revision, version_json ON plan_versions BEGIN
    DELETE FROM decision_plan_references WHERE workspace_id=OLD.workspace_id AND owner_kind='version' AND owner_id=OLD.plan_id AND owner_revision=OLD.content_revision;
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'version', NEW.plan_id, NEW.content_revision
    FROM json_tree(NEW.version_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2', 'decision_service_v3')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_draft_references_insert AFTER INSERT ON plan_drafts BEGIN
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'draft', NEW.draft_id, 0
    FROM json_tree(NEW.draft_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2', 'decision_service_v3')
      AND json_type(node.value, '$.service.id')='text';
END;
CREATE TRIGGER decision_draft_references_update AFTER UPDATE OF workspace_id, draft_id, draft_json ON plan_drafts BEGIN
    DELETE FROM decision_plan_references WHERE workspace_id=OLD.workspace_id AND owner_kind='draft' AND owner_id=OLD.draft_id AND owner_revision=0;
    INSERT OR IGNORE INTO decision_plan_references
    SELECT NEW.workspace_id, json_extract(node.value, '$.service.id'), 'draft', NEW.draft_id, 0
    FROM json_tree(NEW.draft_json) node
    WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2', 'decision_service_v3')
      AND json_type(node.value, '$.service.id')='text';
END;
INSERT OR IGNORE INTO decision_plan_references
SELECT v.workspace_id, json_extract(node.value, '$.service.id'), 'version', v.plan_id, v.content_revision
FROM plan_versions v, json_tree(v.version_json) node
WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2', 'decision_service_v3')
  AND json_type(node.value, '$.service.id')='text';
INSERT OR IGNORE INTO decision_plan_references
SELECT d.workspace_id, json_extract(node.value, '$.service.id'), 'draft', d.draft_id, 0
FROM plan_drafts d, json_tree(d.draft_json) node
WHERE node.type='object' AND json_extract(node.value, '$.kind') IN ('decision_service', 'decision_service_v2', 'decision_service_v3')
  AND json_type(node.value, '$.service.id')='text';
"#;

#[cfg(test)]
mod tests;
