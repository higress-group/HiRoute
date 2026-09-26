//! Bounded wire pages are tied to one complete catalog snapshot; edits invalidate old cursors.
use hiroute_application_api::*;
use hiroute_domain::CanonicalDigest;
pub(super) fn paginate(
    mut catalog: AgentPlanCatalogViewV2,
    query: AgentPlanCatalogQueryV2,
) -> Result<AgentPlanCatalogViewV2, ErrorCode> {
    if !(1..=128).contains(&query.limit) || catalog.next_cursor.is_some() {
        return Err(ErrorCode::InvalidArguments);
    }
    catalog
        .plans
        .sort_by(|a, b| a.agent_plan_id.as_str().cmp(b.agent_plan_id.as_str()));
    catalog.drafts.sort_by(|a, b| a.draft_id.cmp(&b.draft_id));
    let digest = CanonicalDigest::of(&catalog).map_err(|_| ErrorCode::Internal)?;
    let count = catalog
        .plans
        .len()
        .checked_add(catalog.drafts.len())
        .ok_or(ErrorCode::Internal)?;
    let offset = query.cursor.as_ref().map(|c| c.offset).unwrap_or(0);
    if query
        .cursor
        .as_ref()
        .is_some_and(|c| c.snapshot_digest != digest || c.offset == 0 || c.offset >= count)
    {
        return Err(ErrorCode::ChangePreviewStale);
    }
    let plan_count = catalog.plans.len();
    catalog.plans = catalog
        .plans
        .into_iter()
        .skip(offset)
        .take(query.limit)
        .collect();
    catalog.drafts = catalog
        .drafts
        .into_iter()
        .skip(offset.saturating_sub(plan_count))
        .take(query.limit - catalog.plans.len())
        .collect();
    let next = offset + catalog.plans.len() + catalog.drafts.len();
    catalog.next_cursor = (next < count).then_some(AgentPlanCatalogCursorV2 {
        snapshot_digest: digest,
        offset: next,
    });
    Ok(catalog)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_pages_are_bounded_and_edits_invalidate_existing_cursor() {
        let draft = |id: &str| {
            serde_json::from_value(serde_json::json!({
            "schema":"hiroute.plan-draft/v1","workspace_id":"personal/default","draft_id":id,"revision":1,
            "editor":{"schema":"hiroute.plan-editor/v2","display_name":"Draft","purpose":"","mode":"fixed_model","candidates":[],
            "smart":{"economy":[],"primary":[],"primary_fallback":false,"reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
            "free":{"candidates":[],"primary":[],"primary_fallback":false},"delegation_enabled":false,"requirements":{},
            "limits":{"maximum_attempts":1,"request_timeout_ms":1000,"attempt_timeout_ms":1000}}
        })).unwrap()
        };
        let mut catalog = AgentPlanCatalogViewV2 {
            schema: "hiroute.agent-plan-catalog/v2".into(),
            plans: vec![],
            drafts: vec![draft("draft/b"), draft("draft/a")],
            next_cursor: None,
        };
        let first = paginate(
            catalog.clone(),
            AgentPlanCatalogQueryV2 {
                limit: 1,
                cursor: None,
            },
        )
        .unwrap();
        assert_eq!(first.drafts[0].draft_id, "draft/a");
        let query = AgentPlanCatalogQueryV2 {
            limit: 1,
            cursor: first.next_cursor,
        };
        let second = paginate(catalog.clone(), query.clone()).unwrap();
        assert_eq!(second.drafts[0].draft_id, "draft/b");
        assert!(second.next_cursor.is_none());
        catalog.drafts[0].revision += 1;
        assert!(matches!(
            paginate(catalog, query),
            Err(ErrorCode::ChangePreviewStale)
        ));
    }
}
