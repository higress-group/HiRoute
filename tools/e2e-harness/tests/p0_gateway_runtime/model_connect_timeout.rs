use super::*;
use std::io::{Read, Write};
use std::net::TcpStream;

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
        if overall == 200 {
            // The logical deadline may close the downstream before an HTTP response exists.
            // Observe the actual socket rather than requiring a fabricated HTTP error head.
            let body = format!(r#"{{"model":"{MODEL}","input":"hello"}}"#);
            let mut client = TcpStream::connect(fixture.address).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(client, "POST /v1/responses HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", fixture.address, body.len()).unwrap();
            let mut bytes = Vec::new();
            if let Err(error) = client.read_to_end(&mut bytes) {
                assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
            }
            assert!(
                started.elapsed() >= Duration::from_millis(100),
                "request was rejected before its logical deadline"
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            assert!(bytes.is_empty() || bytes.starts_with(b"HTTP/1.1 504"));
            continue;
        }
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
