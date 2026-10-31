use super::*;

pub(super) struct ControlKeySpendAuthority {
    pub(super) adapter: Arc<LocalControlAdapter>,
}
impl hiroute_domain::KeySpendAuthorityV1 for ControlKeySpendAuthority {
    fn reserve(
        &self,
        key: &str,
        generation: u64,
        estimate: Option<&hiroute_domain::KeySpendEstimateV1>,
    ) -> Result<Option<hiroute_domain::KeySpendReservationV1>, hiroute_domain::KeyBudgetGateErrorV1>
    {
        self.adapter
            .stores
            .lock()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "key_budget.lock"))?
            .control()
            .reserve_key_spend(key, generation, estimate)
    }
    fn mark_sent(&self, id: &str) -> PortResult<()> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "key_budget.lock"))?
            .control()
            .mark_key_spend_sent(id)
    }
    fn settle(&self, id: &str, value: &hiroute_domain::KeySpendSettlementV1) -> PortResult<()> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "key_budget.lock"))?
            .control()
            .settle_key_spend(id, value)
    }
}
