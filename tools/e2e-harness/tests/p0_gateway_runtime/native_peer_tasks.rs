//! Opaque peer tasks retain observed routing without changing execution wire.
use super::*;
use serde_json::{Value, json};

fn reply() -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body: RESPONSES_OK,
    }
}

#[test]
fn real_listener_native_peer_tasks_retain_observed_routing_and_original_wire() {
    for external in [false, true] {
        for content in [
            json!([{"type":"input_text","text":"complex child task distinct from its ancestor"}]),
            json!([{"type":"encrypted_content","encrypted_content":"opaque native child task"}]),
        ] {
            let economy = NativeProvider::start(vec![reply()]);
            let primary = NativeProvider::start(vec![reply()]);
            let classifier = NativeProvider::start(vec![ProviderReply::Complete {
                status: 200,
                error_kind: None,
                body: CLASSIFIER_SIMPLE,
            }]);
            let fixture = if external {
                RuntimeFixture::launch_rest_classified(&[&economy, &primary, &classifier], 1)
            } else {
                RuntimeFixture::launch_classified(&[&economy, &primary], 1)
            };
            let input = json!([
                {"role":"user","content":"hello"},
                {"type":"agent_message","author":"parent","recipient":"child","content":content}
            ]);
            let result = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &[("X-HiRoute-Token", TOKEN)],
                &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input})).unwrap(),
            );
            assert_eq!(
                result.status,
                200,
                "{}",
                String::from_utf8_lossy(&result.body)
            );
            assert_eq!(economy.calls(), 1, "usable observed input retains routing");
            assert_eq!(classifier.calls(), usize::from(external));
            assert_eq!(primary.calls(), 0);
            let requests = economy.requests();
            let raw = &requests[0];
            let offset = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let executed: Value = serde_json::from_slice(&raw[offset..]).unwrap();
            assert_eq!(executed["input"], input);
        }
    }
}
