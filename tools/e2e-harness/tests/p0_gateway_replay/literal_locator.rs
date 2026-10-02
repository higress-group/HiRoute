use super::*;

#[test]
fn real_hirouted_forwards_locator_shaped_client_text_unchanged() {
    let provider = NativeProvider::start(vec![success()]);
    let fixture = RuntimeFixture::launch_with_replay(&[&provider], 1, 64 * 1024, 16 * 1024);
    let mut literal = "__hiroute_content_ref_v2_1_0_0_0__".to_owned();
    let mut fixed_body = None;
    for _ in 0..8 {
        let body = request_document(literal.clone());
        let escaped_len = serde_json::to_vec(&String::from_utf8(body.clone()).unwrap())
            .unwrap()
            .len()
            - 2;
        let next = format!(
            "__hiroute_content_ref_v2_1_0_{}_{}__",
            body.len(),
            escaped_len
        );
        if next == literal {
            fixed_body = Some(body);
            break;
        }
        literal = next;
    }
    let body = fixed_body.expect("raw locator lengths converge");
    let response = fixture.request_body(&body);
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let projected: serde_json::Value = serde_json::from_slice(http_body(&requests[0])).unwrap();
    assert_eq!(projected["input"], literal);
    wait_replay_empty(&fixture.replay_root);
}
