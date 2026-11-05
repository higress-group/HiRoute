use super::*;

#[test]
fn slow_tls_uses_model_connect_budget_and_still_obeys_the_request_deadline() {
    // Each case launches the actual candidate process. Only the loopback peer delays TLS.
    for (budget, overall, delay, success) in [
        (None, 8_000, 2_500, true),
        (Some(200), 8_000, 600, false),
        (Some(4_000), 8_000, 2_500, true),
        (Some(4_000), 200, 600, false),
    ] {
        let upstream = NativeProvider::start(vec![ProviderReply::DelayedHandshake {
            duration: Duration::from_millis(delay),
            status: 200,
            body: RESPONSES_OK,
        }]);
        let fixture =
            RuntimeFixture::launch_with_model_connect_timeout(&[&upstream], budget, overall);
        let started = Instant::now();
        let response = fixture.request();
        assert_eq!(
            response.status == 200,
            success,
            "budget={budget:?} overall={overall} status={} body={:?}",
            response.status,
            response.body
        );
        if success {
            assert!(started.elapsed() >= Duration::from_secs(2));
            assert_eq!(upstream.requests().len(), 1);
        } else {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "connection/request budget was not enforced"
            );
        }
    }
}
