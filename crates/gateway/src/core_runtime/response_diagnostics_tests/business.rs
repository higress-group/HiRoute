//! Formal delivery and exact usage assertions for listener scenarios.
use super::*;

pub(super) fn assert_business(facts: &[Value], failed: bool, after_body: bool) {
    assert_business_with_usage(facts, failed, after_body, true);
}

pub(super) fn assert_business_with_usage(
    facts: &[Value],
    failed: bool,
    after_body: bool,
    canonical_usage: bool,
) {
    let named = |name| {
        facts
            .iter()
            .filter(|r| r.pointer("/fact/kind").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        named("attempt_started").len(),
        1,
        "only one actual attempt may run: bounded prebody or committed afterbody"
    );
    let finished = named("attempt_finished");
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["fact"]["ordinal"], 1);
    assert_eq!(
        finished[0]["fact"]["outcome"],
        if after_body {
            "postcommit_transport_failed"
        } else if failed {
            "rejected"
        } else {
            "accepted"
        }
    );
    if failed && !after_body {
        // Relay eligibility remains Continue; the separately frozen one-attempt
        // budget ends the request without promoting another candidate.
        assert_eq!(finished[0]["fact"]["disposition"], "continue");
        assert_eq!(finished[0]["fact"]["retryable"], true);
        assert_eq!(
            finished[0]["fact"]["commits"]["downstream_semantic"],
            "clear"
        );
    }
    if after_body {
        let semantic_started = finished[0]["fact"]["commits"]["downstream_semantic"] != "clear";
        assert_eq!(
            finished[0]["fact"]["termination_reason"],
            if semantic_started {
                "stream_started_no_retry"
            } else {
                "provider_encoder_failure"
            }
        );
        assert_eq!(
            finished[0]["fact"]["stream_outcome"],
            if semantic_started {
                "stream_started_no_retry"
            } else {
                "aborted_before_semantic_commit"
            }
        );
    }
    assert_eq!(
        finished[0]["fact"]["commits"]["upstream_request"],
        "write_confirmed"
    );
    assert_eq!(
        finished[0]["fact"]["commits"]["downstream_headers"],
        if failed && !after_body {
            "clear"
        } else {
            "write_confirmed"
        }
    );
    let requests = named("request_finished");
    assert_eq!(finished[0]["fact"]["cleanup_outcome"], "completed");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["fact"]["outcome"],
        if failed {
            if finished[0]["fact"]["commits"]["downstream_semantic"] != "clear" {
                "postcommit_transport_failed"
            } else {
                "failed"
            }
        } else {
            "accepted"
        }
    );
    assert_eq!(requests[0]["fact"]["attempts_started"], 1);
    assert_eq!(requests[0]["fact"]["attempts_finished"], 1);
    let semantic_commits = named("semantic_commit").len();
    assert_eq!(semantic_commits, if failed && !after_body { 0 } else { 1 });
    if failed {
        // Failed entities report no usage; never synthesize zero tokens.
        assert!(named("usage_and_cache").is_empty());
    } else {
        assert_usage_sources(facts, 1, canonical_usage);
    }
}

pub(super) fn assert_success_usage(facts: &[Value], ordinal: u64) {
    assert_usage_sources(facts, ordinal, true);
}

fn assert_usage_sources(facts: &[Value], ordinal: u64, canonical_usage: bool) {
    let usage: Vec<_> = facts
        .iter()
        .filter(|r| r["fact"]["kind"] == "usage_and_cache")
        .collect();
    let count = |source: &str| {
        usage
            .iter()
            .filter(|r| r["fact"]["source"] == source)
            .count()
    };
    assert_eq!(
        count("accepted_canonical_model_event"),
        usize::from(canonical_usage)
    );
    // A bounded optional decoder can lose the huge canonical frame. Main
    // execution still delivers it and retains exact provider usage; never
    // report that usage as an accepted canonical observation.
    if canonical_usage {
        assert!(count("provider_completion") <= 1);
    } else {
        assert_eq!(count("provider_completion"), 1);
    }
    let attempt = facts
        .iter()
        .find(|r| r["fact"]["kind"] == "attempt_started" && r["fact"]["ordinal"] == ordinal)
        .unwrap();
    assert!(attempt["attempt_id"].is_string());
    for record in usage {
        assert_eq!(record["fact"]["ordinal"], ordinal);
        assert_eq!(record["attempt_id"], attempt["attempt_id"]);
        assert_eq!(record["correlation"], attempt["correlation"]);
        assert!(matches!(
            record["fact"]["source"].as_str(),
            Some("provider_completion" | "accepted_canonical_model_event")
        ));
        assert_eq!(record["fact"]["input_tokens"], 11);
        assert_eq!(record["fact"]["output_tokens"], 7);
    }
}

fn wait_replay_clean(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !root.exists() || fs::read_dir(root).unwrap().next().is_none() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Replay resources survived request cleanup"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
