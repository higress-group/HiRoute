use super::*;
use hiroute_application_api::{
    ComputeKeyBudgetSaveRequestV1, ComputeKeyBudgetsQueryV1, ComputeKeyBudgetsResultV1,
};

impl LocalControlAdapter {
    pub(super) fn preview_key_budgets(
        &self,
        mut request: ComputeKeyBudgetSaveRequestV1,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError> {
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        self.validate_subscription_save_change(&request.change.change)?;
        self.validate_native_network_save(&request.change.change, request.network_edit.as_ref())?;
        self.bind_saved_key_inputs(&request.change.change)?;
        self.establish_save_defaults(&mut request.change, &catalog)?;
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let preview = ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .preview_key_budgets(request)
        .map_err(map_planning)?;
        self.validate_feed_save_plan(preview.plan(), &catalog)?;
        if self
            .current_catalog()
            .is_none_or(|c| c.generation != catalog.generation)
        {
            return Err(ComputeManagementControlError::Conflict);
        }
        Ok(preview.result)
    }

    pub(super) fn read_key_budgets(
        &self,
        query: ComputeKeyBudgetsQueryV1,
    ) -> Result<ComputeKeyBudgetsResultV1, ComputeManagementControlError> {
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let control = stores.control();
        let bindings = if let Some(source) = &query.source_id {
            if source.is_empty() || source.len() > 256 || query.cursor.is_some() {
                return Err(ComputeManagementControlError::Invalid);
            }
            control
                .key_budget_bindings(source)
                .map_err(|_| ComputeManagementControlError::Unavailable)?
        } else {
            Default::default()
        };
        let mut ids = if query.source_id.is_some() {
            bindings
                .values()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            control
                .key_budget_available_ids(query.cursor.as_deref(), 100)
                .map_err(|_| ComputeManagementControlError::Unavailable)?
        };
        if ids.len() > 256 {
            return Err(ComputeManagementControlError::Invalid);
        }
        let next_cursor = (query.source_id.is_none() && ids.len() == 100)
            .then(|| ids.last().cloned())
            .flatten();
        ids.dedup();
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .as_millis() as i64;
        let budgets = control
            .key_budget_views(&ids, at)
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        Ok(ComputeKeyBudgetsResultV1 {
            source_id: query.source_id,
            bindings,
            budgets,
            next_cursor,
        })
    }
}
