//! Deliver actual listener facts through the production adapter and durable
//! observation query, so correct routing cannot hide an incorrect user view.
use hiroute_domain::{ObservationReaderContext, RoutingTimelinePageV1, RoutingTimelineQueryV1};
use hiroute_observation::{
    DigestAuthority, LocalObservationStore,
    writer::{IngestOutcome, ObservationCommitPort},
};
use serde_json::Value;

pub(super) fn timeline(facts: &[Value]) -> RoutingTimelinePageV1 {
    let directory = tempfile::tempdir().unwrap();
    let events = facts
        .iter()
        .map(|fact| {
            hiroute_integrations::gateway::project_execution_payload(
                &serde_json::to_vec(fact).unwrap(),
            )
            .expect("the actual Gateway envelope must pass the production adapter")
        })
        .collect::<Vec<_>>();
    {
        let store =
            LocalObservationStore::open(directory.path(), DigestAuthority::new([71; 32])).unwrap();
        for event in &events {
            assert!(
                matches!(
                    store.ingest_fact(event, &[]).unwrap(),
                    IngestOutcome::Ack(_)
                ),
                "actual fact rejected: {:?}",
                event.fact
            );
        }
    }
    // Also verify that admission is durable, rather than a query-local hint.
    let store =
        LocalObservationStore::open(directory.path(), DigestAuthority::new([71; 32])).unwrap();
    let reader = ObservationReaderContext::local_user(
        events[0].correlation.workspace_id.clone(),
        "listener-acceptance".into(),
        1,
        i64::MAX,
        false,
        false,
    )
    .unwrap();
    let now = events
        .iter()
        .map(|e| e.occurred_at_ms().unwrap())
        .max()
        .unwrap()
        + 1;
    store
        .observed_routing_timeline(
            &reader,
            &RoutingTimelineQueryV1 {
                include_presentation: false,
                session_id: events[0].correlation.conversation_id.to_string(),
                after_watermark: None,
                limit: 50,
            },
            now,
        )
        .unwrap()
}
