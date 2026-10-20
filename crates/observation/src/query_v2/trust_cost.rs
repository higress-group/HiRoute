use super::*;
use hiroute_domain::*;
use rusqlite::{Connection, OpenFlags};

impl crate::LocalObservationStore {
    pub fn observed_trust_cost(
        &self,
        reader: &ObservationReaderContext,
        scope: &TrustCostScopeV1,
        reference: &ApiEquivalentPriceV1,
        prices: &[ApiEquivalentPriceV1],
        now: i64,
    ) -> Result<ApiEquivalentValueReportV1, ObservationV2Error> {
        reader.check(now, false, false)?;
        if reader.allowed_runs().is_some() {
            return Err(ObservationV2Error::Unauthorized);
        }
        if !identifier(&scope.plan_id)
            || !identifier(&scope.currency)
            || scope.branch_id.as_ref().is_some_and(|b| !identifier(b))
            || scope.revision == Some(0)
            || scope.from_ms < 0
            || scope.from_ms >= scope.to_ms
            || scope.to_ms > now.saturating_add(1)
            || prices.len() > 4096
        {
            return Err(ObservationV2Error::Invalid);
        }
        for price in prices.iter().chain(std::iter::once(reference)) {
            if price.quote.currency != scope.currency
                || price.quote.valuation_kind != PriceValuationKindV1::ApiEquivalent
                || price.quote.exact_target.workspace_id != *reader.workspace()
                || price.quote.verify_digest().is_err()
            {
                return Err(ObservationV2Error::Invalid);
            }
        }
        let _permit = self.query_permit()?;
        let mut db =
            Connection::open_with_flags(&self.activity_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let _deadline = QueryDeadline::start_for(&db, reader)?;
        db.pragma_update(None, "cache_size", -16_384)?;
        let tx = db.transaction()?;
        let visibility: u64 = tx.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'", [], |r| r.get(0))?;
        let retained = now
            .saturating_sub(crate::managed_text::RETENTION_MS)
            .saturating_add(1);
        let comparison = crate::analysis::trust_comparison(
            &tx, reader, scope, reference, prices, retained, now,
        )?;
        tx.commit()?;
        check_visibility(&db, visibility)?;
        reader.check(now, false, false)?;
        let mut report = super::api_equivalent::comparison_report(
            &ObservationValueQueryV2 {
                from_ms: scope.from_ms,
                to_ms: scope.to_ms,
                session_id: None,
                plan_id: Some(scope.plan_id.clone()),
                currency: Some(scope.currency.clone()),
            },
            reference,
            prices,
            now,
            retained,
            comparison,
        );
        report.disclosure.push_str(" Trial scope conservatively includes unresolved normal requests in the Plan/time window without assigning a revision or execution group. Excluded stage-identity gaps are included in the missing count and prevent full coverage and a full-scope difference.");
        Ok(report)
    }
}
