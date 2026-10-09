mod v2;

use std::sync::Arc;

use hiroute_application_api::{
    CanonicalDigest, PrincipalKind, ProtectedClientGrantV2, SessionListPageV1,
    SessionListRequestV1, SessionLookupV1, ValuePeriodV1, ValueRequestV1, ValueScopeViewV1,
};
use hiroute_domain::{
    ContentMode, ObservationPrincipalV1, ObservationQueryError, ObservationQueryPort, ReceiptId,
    SessionId, SessionListQueryV1, ValueQueryV1, WorkspaceId,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::control::{
    ApplicationClockPort, ControlReadError, ControlStatePort, ValuePlanScopeV1, ValueScopePort,
};

use super::ObservationQueryService;

const DEFAULT_SESSION_LIMIT: usize = 50;
const MAX_SESSION_LIMIT: usize = 200;
const DAY_MILLIS: i64 = 86_400_000;

pub(crate) struct ObservationControl {
    service: ObservationQueryService,
    state: Arc<dyn ControlStatePort>,
    clock: Arc<dyn ApplicationClockPort>,
    value_scope: Arc<dyn ValueScopePort>,
    workspace_id: WorkspaceId,
}

impl ObservationControl {
    pub(crate) fn new(
        port: Arc<dyn ObservationQueryPort>,
        state: Arc<dyn ControlStatePort>,
        clock: Arc<dyn ApplicationClockPort>,
        value_scope: Arc<dyn ValueScopePort>,
    ) -> Self {
        Self {
            service: ObservationQueryService::new(port),
            state,
            clock,
            value_scope,
            workspace_id: WorkspaceId::default(),
        }
    }

    pub(crate) fn list(
        &self,
        _ambient: PrincipalKind,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        if payload.get("schema").and_then(Value::as_str) == Some("hiroute.observation.query/v2") {
            return self.read_v2(grant, payload, "ListSessions");
        }

        let request: SessionListRequestV1 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let principal = self.local_or_protected_principal(
            grant,
            if request.query.is_some() {
                "SearchSessionContent"
            } else {
                "ListSessions"
            },
            &request,
            request.query.is_none(),
        )?;
        let cursor = request.cursor.as_deref().map(decode_cursor).transpose()?;
        let to_ms = match request.to_ms.or_else(|| cursor.as_ref().map(|c| c.to_ms)) {
            Some(to_ms) => to_ms,
            None => self.clock.now_ms().map_err(map_control)?,
        };
        let from_ms = request
            .from_ms
            .or_else(|| cursor.as_ref().map(|c| c.from_ms))
            .unwrap_or(to_ms.saturating_sub(7 * DAY_MILLIS));
        let limit = usize::from(request.limit.unwrap_or(DEFAULT_SESSION_LIMIT as u16));
        if from_ms >= to_ms || limit == 0 || limit > MAX_SESSION_LIMIT {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let query = SessionListQueryV1 {
            from_ms: Some(from_ms),
            to_ms: Some(to_ms),
            agent_id: request.agent_id,
            query: request.query,
            model_switch: request.model_switch,
            include_unlinked: request.include_unlinked,
        };
        let binding = CanonicalDigest::of(&json!({ "query": query, "limit": limit }))
            .map_err(|_| ObservationQueryError::InvalidQuery)?;
        let offset = match cursor {
            Some(cursor) if cursor.binding == binding.as_str().trim_start_matches("sha256:") => {
                cursor.offset
            }
            Some(_) => return Err(ObservationQueryError::InvalidQuery),
            None => 0,
        };
        let mut sessions = self.service.list_sessions(&principal, &query)?.sessions;
        if offset > sessions.len() {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let remaining = sessions.len() - offset;
        let taken = remaining.min(limit);
        let next_offset = offset + taken;
        sessions = sessions.into_iter().skip(offset).take(taken).collect();
        let page = SessionListPageV1 {
            sessions,
            next_cursor: (next_offset < offset + remaining)
                .then(|| encode_cursor(next_offset, from_ms, to_ms, &binding)),
        };
        serde_json::to_value(page).map_err(|_| ObservationQueryError::Corrupt)
    }

    pub(crate) fn plan_quality(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        self.read_v2(grant, payload, "GetPlanQualitySamples")
    }

    pub(crate) fn show(
        &self,
        _ambient: PrincipalKind,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        if payload.get("schema").and_then(Value::as_str) == Some("hiroute.observation.query/v2") {
            return self.read_v2(grant, payload, "GetSession");
        }

        let lookup: SessionLookupV1 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let session_id =
            SessionId::parse(&lookup.id).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let content = ContentMode::from(lookup.content.unwrap_or_default());
        let principal = if content == ContentMode::None {
            self.local_or_protected_principal(grant, "GetSession", &lookup, true)?
        } else {
            self.protected_principal(grant, "ReadSessionContent", &lookup)?
        };
        serde_json::to_value(self.service.get_session(&principal, &session_id, content)?)
            .map_err(|_| ObservationQueryError::Corrupt)
    }

    pub(crate) fn receipt(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        if payload.get("schema").and_then(Value::as_str) == Some("hiroute.observation.query/v2") {
            return self.read_v2(grant, payload, "GetRoutingReceipt");
        }

        let lookup: SessionLookupV1 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        // This is an exact-ID, same-UID Local Control read. It does not accept a search scope or
        // broaden the requested receipt; an explicitly supplied protected grant is still fully
        // validated rather than ignored.
        let principal =
            self.local_or_protected_principal(grant, "ReadRoutingReceiptContent", &lookup, true)?;
        let receipt =
            ReceiptId::parse(lookup.id).map_err(|_| ObservationQueryError::InvalidQuery)?;
        serde_json::to_value(self.service.get_receipt(&principal, &receipt)?)
            .map_err(|_| ObservationQueryError::Corrupt)
    }

    pub(crate) fn status(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
    ) -> Result<Value, ObservationQueryError> {
        let principal =
            self.local_or_protected_principal(grant, "GetObservationStatus", &json!({}), true)?;
        serde_json::to_value(self.service.get_status(&principal)?)
            .map_err(|_| ObservationQueryError::Corrupt)
    }

    pub(crate) fn value(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        payload: Value,
    ) -> Result<Value, ObservationQueryError> {
        if payload.get("schema").and_then(Value::as_str) == Some("hiroute.observation.query/v2") {
            return self.read_v2(grant, payload, "GetValue");
        }

        // Recovery-only hiroute.value-query/v1 -> hiroute.value-view/v1 for
        // existing frozen ledgers. Current clients emit observation v2 intents.
        let request: ValueRequestV1 =
            serde_json::from_value(payload).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let principal = self.local_or_protected_principal(grant, "GetValue", &request, true)?;
        if request.from_ms.is_some() && request.period.is_some() {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let to_ms = match request.to_ms {
            Some(to_ms) => to_ms,
            None => self.clock.now_ms().map_err(map_control)?,
        };
        let from_ms = match request.from_ms {
            Some(from_ms) => from_ms,
            None => match request.period.unwrap_or(ValuePeriodV1::SevenDays) {
                ValuePeriodV1::Today => {
                    self.clock.local_day_start_ms(to_ms).map_err(map_control)?
                }
                ValuePeriodV1::SevenDays => to_ms.saturating_sub(7 * DAY_MILLIS),
                ValuePeriodV1::ThirtyDays => to_ms.saturating_sub(30 * DAY_MILLIS),
            },
        };
        if from_ms >= to_ms || request.currency.is_empty() {
            return Err(ObservationQueryError::InvalidQuery);
        }
        let scope = ValuePlanScopeV1 {
            from_ms,
            to_ms,
            currency: request.currency.clone(),
            session_id: request.session_id.clone(),
        };
        let explicit_plan = request.agent_plan_id.clone();
        let mut plan_ids = match explicit_plan.clone() {
            Some(plan_id) => vec![plan_id],
            None => self
                .value_scope
                .value_plan_ids(&self.workspace_id, &scope)
                .map_err(map_control)?,
        };
        if plan_ids.len() > 200 {
            return Err(ObservationQueryError::InvalidQuery);
        }
        plan_ids.sort();
        plan_ids.dedup();
        let mut plans = Vec::with_capacity(plan_ids.len());
        let mut encoded_bytes = 0;
        let started = std::time::Instant::now();
        for agent_plan_id in plan_ids {
            if started.elapsed() >= std::time::Duration::from_millis(500) {
                return Err(ObservationQueryError::Unavailable);
            }
            plans.push(self.service.get_value(
                &principal,
                &ValueQueryV1 {
                    agent_plan_id,
                    from_ms,
                    to_ms,
                    group_by: request.group_by,
                    currency: request.currency.clone(),
                    session_id: request.session_id.clone(),
                },
            )?);
            encoded_bytes +=
                serde_json::to_vec(plans.last().ok_or(ObservationQueryError::Corrupt)?)
                    .map_err(|_| ObservationQueryError::Corrupt)?
                    .len();
            if encoded_bytes > 1024 * 1024 - 4096
                || started.elapsed() >= std::time::Duration::from_millis(500)
            {
                return Err(ObservationQueryError::Unavailable);
            }
        }
        serde_json::to_value(ValueScopeViewV1 {
            from_ms,
            to_ms,
            currency: request.currency,
            group_by: request.group_by,
            agent_plan_id: explicit_plan,
            plans,
        })
        .map_err(|_| ObservationQueryError::Corrupt)
    }

    fn protected_principal<T: Serialize>(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        operation_kind: &str,
        scope: &T,
    ) -> Result<ObservationPrincipalV1, ObservationQueryError> {
        let grant = grant.ok_or(ObservationQueryError::Unauthorized)?;
        if grant.principal_kind.is_collaboration() {
            return Err(ObservationQueryError::Unauthorized);
        }
        let digest = CanonicalDigest::of(scope).map_err(|_| ObservationQueryError::InvalidQuery)?;
        let revisions = self
            .state
            .snapshot(&self.workspace_id)
            .map_err(map_control)?
            .revisions;
        self.state
            .validate_protected_capability(
                &grant.capability,
                &self.workspace_id,
                grant.principal_kind,
                operation_kind,
                &digest,
                &revisions,
            )
            .map_err(map_control)?;
        Ok(ObservationPrincipalV1::local_user(
            self.workspace_id.clone(),
        ))
    }

    fn local_or_protected_principal<T: Serialize>(
        &self,
        grant: Option<&ProtectedClientGrantV2>,
        operation_kind: &str,
        scope: &T,
        allow_same_uid: bool,
    ) -> Result<ObservationPrincipalV1, ObservationQueryError> {
        if grant.is_none() && allow_same_uid {
            return Ok(ObservationPrincipalV1::local_user(
                self.workspace_id.clone(),
            ));
        }
        self.protected_principal(grant, operation_kind, scope)
    }
}

fn encode_cursor(offset: usize, from_ms: i64, to_ms: i64, binding: &CanonicalDigest) -> String {
    format!(
        "lc2:{offset}:{from_ms}:{to_ms}:{}",
        binding.as_str().trim_start_matches("sha256:")
    )
}

struct SessionCursor<'a> {
    offset: usize,
    from_ms: i64,
    to_ms: i64,
    binding: &'a str,
}

fn decode_cursor(cursor: &str) -> Result<SessionCursor<'_>, ObservationQueryError> {
    let mut fields = cursor.split(':');
    let (Some("lc2"), Some(offset), Some(from_ms), Some(to_ms), Some(binding), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(ObservationQueryError::InvalidQuery);
    };
    Ok(SessionCursor {
        offset: offset
            .parse()
            .map_err(|_| ObservationQueryError::InvalidQuery)?,
        from_ms: from_ms
            .parse()
            .map_err(|_| ObservationQueryError::InvalidQuery)?,
        to_ms: to_ms
            .parse()
            .map_err(|_| ObservationQueryError::InvalidQuery)?,
        binding,
    })
}

fn map_control(error: ControlReadError) -> ObservationQueryError {
    match error {
        ControlReadError::Denied => ObservationQueryError::Unauthorized,
        ControlReadError::NotFound => ObservationQueryError::NotFound,
        ControlReadError::Corrupt => ObservationQueryError::Corrupt,
        ControlReadError::Unavailable | ControlReadError::SnapshotChanged => {
            ObservationQueryError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, Ordering};

    use hiroute_application_api::{
        PrincipalKind, ProtectedClientGrantV2, SessionContentModeV1, SessionLookupV1,
    };
    use hiroute_domain::{
        AgentPlanId, ContentAccess, ContentCompleteness, CorrelationProvenance, FactsCompleteness,
        ObservationCompletenessScope, ObservationStatusV1, OperationId, OperationV1,
        RetentionPolicyV1, RoutingReceiptV1, SessionDetailV1, SessionListV1, SessionSummaryV1,
        ValueGroupByV1, ValueViewV1,
    };

    use super::*;
    use crate::control::ControlStateSnapshotV1;

    struct FixedPorts {
        now_ms: AtomicI64,
        queries: Mutex<Vec<SessionListQueryV1>>,
        content_modes: Mutex<Vec<ContentMode>>,
        plans: Vec<AgentPlanId>,
    }

    impl FixedPorts {
        fn new(now_ms: i64, plans: &[&str]) -> Self {
            Self {
                now_ms: AtomicI64::new(now_ms),
                queries: Mutex::new(Vec::new()),
                content_modes: Mutex::new(Vec::new()),
                plans: plans
                    .iter()
                    .map(|plan| AgentPlanId::parse(*plan).unwrap())
                    .collect(),
            }
        }
    }

    impl ControlStatePort for FixedPorts {
        fn snapshot(
            &self,
            _workspace_id: &WorkspaceId,
        ) -> Result<ControlStateSnapshotV1, ControlReadError> {
            Ok(ControlStateSnapshotV1 {
                revisions: hiroute_application_api::RevisionSetV1 {
                    target: 9,
                    dependencies: Default::default(),
                },
                desired_state: None,
                recoverable_operations: Vec::new(),
            })
        }

        fn operation(
            &self,
            _operation_id: &OperationId,
        ) -> Result<Option<OperationV1>, ControlReadError> {
            Ok(None)
        }

        fn validate_protected_capability(
            &self,
            raw_capability: &str,
            _workspace_id: &WorkspaceId,
            principal: PrincipalKind,
            operation_kind: &str,
            _accepted_digest: &CanonicalDigest,
            _expected_revisions: &hiroute_application_api::RevisionSetV1,
        ) -> Result<(), ControlReadError> {
            if principal == PrincipalKind::InteractiveUser
                && ((raw_capability == "receipt-grant"
                    && operation_kind == "ReadRoutingReceiptContent")
                    || (raw_capability == "session-grant"
                        && operation_kind == "ReadSessionContent"))
            {
                return Ok(());
            }
            if raw_capability == "launcher-grant"
                && principal == PrincipalKind::InteractiveUser
                && matches!(
                    operation_kind,
                    "ReadSessionContent"
                        | "SearchSessionContent"
                        | "ListSessions"
                        | "GetSession"
                        | "GetObservationStatus"
                        | "GetValue"
                )
            {
                Ok(())
            } else {
                Err(ControlReadError::Denied)
            }
        }
    }

    impl ApplicationClockPort for FixedPorts {
        fn local_day_start_ms(&self, at_ms: i64) -> Result<i64, ControlReadError> {
            // UTC+08 test timezone: 08:00 UTC belongs to the day starting 16:00 UTC.
            Ok((at_ms + 8 * 3_600_000).div_euclid(DAY_MILLIS) * DAY_MILLIS - 8 * 3_600_000)
        }
        fn now_ms(&self) -> Result<i64, ControlReadError> {
            Ok(self.now_ms.load(Ordering::Relaxed))
        }
    }

    impl ValueScopePort for FixedPorts {
        fn value_plan_ids(
            &self,
            _workspace_id: &WorkspaceId,
            _scope: &ValuePlanScopeV1,
        ) -> Result<Vec<AgentPlanId>, ControlReadError> {
            Ok(self.plans.clone())
        }
    }

    impl ObservationQueryPort for FixedPorts {
        fn list_sessions(
            &self,
            _workspace_id: &WorkspaceId,
            query: &SessionListQueryV1,
        ) -> Result<SessionListV1, ObservationQueryError> {
            self.queries.lock().unwrap().push(query.clone());
            Ok(SessionListV1 {
                sessions: (0..3).map(summary).collect(),
            })
        }

        fn get_session(
            &self,
            _workspace_id: &WorkspaceId,
            session_id: &SessionId,
            content_mode: ContentMode,
        ) -> Result<SessionDetailV1, ObservationQueryError> {
            self.content_modes.lock().unwrap().push(content_mode);
            Ok(SessionDetailV1 {
                summary: summary_for(session_id.clone(), 0),
                turns: Vec::new(),
                content_access: ContentAccess::Authorized,
            })
        }

        fn get_receipt(
            &self,
            _workspace_id: &WorkspaceId,
            _receipt_id: &ReceiptId,
        ) -> Result<RoutingReceiptV1, ObservationQueryError> {
            Err(ObservationQueryError::NotFound)
        }

        fn get_value(
            &self,
            _workspace_id: &WorkspaceId,
            query: &ValueQueryV1,
        ) -> Result<ValueViewV1, ObservationQueryError> {
            Ok(empty_value(query))
        }

        fn get_status(
            &self,
            _workspace_id: &WorkspaceId,
        ) -> Result<ObservationStatusV1, ObservationQueryError> {
            Ok(ObservationStatusV1 {
                retention: RetentionPolicyV1::default(),
                store_revision: 0,
                facts_completeness: FactsCompleteness::Complete,
                content_completeness: ContentCompleteness::Unknown,
                completeness_scope: ObservationCompletenessScope::GatewayVisible,
                gaps: Vec::new(),
                gaps_truncated: false,
                activity_bytes: 0,
                content_bytes: 0,
                production_exporter: "not_installed".to_owned(),
            })
        }
    }

    fn grant() -> ProtectedClientGrantV2 {
        ProtectedClientGrantV2 {
            principal_kind: PrincipalKind::InteractiveUser,
            capability: "launcher-grant".into(),
        }
    }

    #[test]
    fn same_uid_peer_reads_facts_but_not_search_or_content_without_a_grant() {
        let ports = Arc::new(FixedPorts::new(10 * DAY_MILLIS, &[]));
        let control = control(ports.clone());
        for kind in [
            PrincipalKind::Skill,
            PrincipalKind::InteractiveUser,
            PrincipalKind::Desktop,
        ] {
            assert!(control.list(kind, None, json!({})).is_ok());
            assert!(control.show(kind, None, json!({"id":"session"})).is_ok());
            assert_eq!(
                control.list(kind, None, json!({"query":"secret"})),
                Err(ObservationQueryError::Unauthorized)
            );
            assert_eq!(
                control.show(kind, None, json!({"id":"session","content":"messages"})),
                Err(ObservationQueryError::Unauthorized)
            );
        }
        assert_eq!(
            control.receipt(None, json!({"id":"receipt"})),
            Err(ObservationQueryError::NotFound)
        );
        assert!(control.status(None).is_ok());
        assert!(control.value(None, json!({})).is_ok());
        assert_eq!(ports.queries.lock().unwrap().len(), 3);
        assert_eq!(
            ports.content_modes.lock().unwrap().as_slice(),
            [ContentMode::None; 3]
        );
    }

    fn control(ports: Arc<FixedPorts>) -> ObservationControl {
        ObservationControl::new(ports.clone(), ports.clone(), ports.clone(), ports)
    }

    #[test]
    fn receipt_and_session_content_require_distinct_operation_grants() {
        let ports = Arc::new(FixedPorts::new(10 * DAY_MILLIS, &[]));
        let handler = control(ports.clone());
        let mut authority = grant();
        authority.capability = "session-grant".into();
        let lookup = json!({"id":"same-object-id", "content":"messages-and-tools"});
        assert_eq!(
            handler.receipt(Some(&authority), lookup.clone()),
            Err(ObservationQueryError::Unauthorized)
        );
        authority.capability = "receipt-grant".into();
        assert_eq!(
            handler.receipt(Some(&authority), lookup.clone()),
            Err(ObservationQueryError::NotFound)
        );
        assert_eq!(
            handler.show(PrincipalKind::InteractiveUser, Some(&authority), lookup),
            Err(ObservationQueryError::Unauthorized)
        );
        assert!(ports.content_modes.lock().unwrap().is_empty());
    }

    #[test]
    fn generated_content_spelling_reaches_handler_only_with_protected_grant() {
        let ports = Arc::new(FixedPorts::new(10 * DAY_MILLIS, &[]));
        let payload = serde_json::to_value(SessionLookupV1 {
            id: "session-content".to_owned(),
            content: Some(SessionContentModeV1::MessagesAndTools),
        })
        .unwrap();
        assert_eq!(
            control(ports.clone()).show(PrincipalKind::Skill, None, payload.clone()),
            Err(ObservationQueryError::Unauthorized)
        );
        control(ports.clone())
            .show(
                PrincipalKind::Skill,
                Some(&ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::InteractiveUser,
                    capability: "launcher-grant".to_owned(),
                }),
                payload,
            )
            .unwrap();
        assert_eq!(
            *ports.content_modes.lock().unwrap(),
            [ContentMode::MessagesAndTools]
        );
    }

    #[test]
    fn application_owns_seven_day_session_default_and_bound_cursor() {
        let ports = Arc::new(FixedPorts::new(10 * DAY_MILLIS, &[]));
        let first = control(ports.clone())
            .list(PrincipalKind::Skill, Some(&grant()), json!({"limit": 2}))
            .unwrap();
        assert_eq!(first["sessions"].as_array().unwrap().len(), 2);
        let cursor = first["next_cursor"].as_str().unwrap();
        ports.now_ms.fetch_add(DAY_MILLIS, Ordering::Relaxed);
        let second = control(ports.clone())
            .list(
                PrincipalKind::Skill,
                Some(&grant()),
                json!({"limit": 2, "cursor": cursor}),
            )
            .unwrap();
        assert_eq!(second["sessions"].as_array().unwrap().len(), 1);
        let query = ports.queries.lock().unwrap()[0].clone();
        assert_eq!(query.from_ms, Some(3 * DAY_MILLIS));
        assert_eq!(query.to_ms, Some(10 * DAY_MILLIS));
        let second_query = ports.queries.lock().unwrap()[1].clone();
        assert_eq!(query.from_ms, second_query.from_ms);
        assert_eq!(query.to_ms, second_query.to_ms);
        for changed in [
            json!({"limit": 1, "cursor": cursor}),
            json!({"limit": 2, "cursor": cursor, "include_unlinked": true}),
            json!({"limit": 2, "cursor": cursor, "to_ms": 11 * DAY_MILLIS}),
            json!({"limit": 2, "cursor": "lc1:2:obsolete"}),
        ] {
            assert!(matches!(
                control(ports.clone()).list(PrincipalKind::Skill, Some(&grant()), changed),
                Err(ObservationQueryError::InvalidQuery)
            ));
        }
        control(ports.clone())
            .list(PrincipalKind::Skill, Some(&grant()), json!({"limit": 2}))
            .unwrap();
        assert_eq!(
            ports.queries.lock().unwrap().last().unwrap().to_ms,
            Some(11 * DAY_MILLIS)
        );
    }

    #[test]
    fn today_uses_local_calendar_boundary_instead_of_rolling_day() {
        let ports = Arc::new(FixedPorts::new(
            10 * DAY_MILLIS + 8 * 3_600_000,
            &["plan/alpha"],
        ));
        let value = control(ports)
            .value(Some(&grant()), json!({"period":"today"}))
            .unwrap();
        assert_eq!(value["from_ms"], 10 * DAY_MILLIS - 8 * 3_600_000);
    }

    #[test]
    fn application_default_value_scope_includes_every_stored_plan() {
        let ports = Arc::new(FixedPorts::new(
            10 * DAY_MILLIS,
            &["plan/alpha", "plan/beta"],
        ));
        let value = control(ports).value(Some(&grant()), json!({})).unwrap();
        assert_eq!(value["from_ms"], 3 * DAY_MILLIS);
        assert_eq!(value["to_ms"], 10 * DAY_MILLIS);
        assert_eq!(value["plans"].as_array().unwrap().len(), 2);
        assert!(value.get("agent_plan_id").is_none());
    }

    fn summary(index: usize) -> SessionSummaryV1 {
        summary_for(SessionId::parse(format!("session-{index}")).unwrap(), index)
    }

    fn summary_for(session_id: SessionId, index: usize) -> SessionSummaryV1 {
        SessionSummaryV1 {
            session_id,
            agent_id: "agent".to_owned(),
            correlation_provenance: CorrelationProvenance::AgentSupplied,
            started_at_ms: index as i64,
            updated_at_ms: index as i64,
            retention_deadline_ms: DAY_MILLIS,
            turn_count: 0,
            request_count: 0,
            model_switch: None,
            facts_completeness: FactsCompleteness::Complete,
            content_completeness: ContentCompleteness::Unknown,
            completeness_scope: ObservationCompletenessScope::GatewayVisible,
            tombstone_reason: None,
        }
    }

    fn empty_value(query: &ValueQueryV1) -> ValueViewV1 {
        ValueViewV1 {
            value_calculation_basis: hiroute_domain::VALUE_CALCULATION_BASIS_V1.to_owned(),
            agent_plan_id: query.agent_plan_id.clone(),
            currency: query.currency.clone(),
            group_by: ValueGroupByV1::None,
            groups: Vec::new(),
            entries: Vec::new(),
            daily_aggregates: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            baseline_api_equivalent_cost_micros: None,
            chosen_api_equivalent_cost_micros: None,
            actual_incremental_cost_micros: None,
            routing_savings_micros: None,
            entitlement_savings_micros: None,
            estimated_total_savings_micros: None,
            price_version_refs: BTreeSet::new(),
            price_override_revision_refs: BTreeSet::new(),
            facts_completeness: FactsCompleteness::Unknown,
            detail_available: true,
        }
    }
}
