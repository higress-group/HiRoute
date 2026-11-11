use super::*;
use crate::model_feed::parse_feed;
use flate2::{Compression, write::GzEncoder};
use hiroute_domain::CanonicalDigest;
use std::{io::Write, net::TcpListener};

const FEED: &[u8] = include_bytes!("../../../../../assets/model-feed/payload.json");

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn exchange(
    status: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
    chunked: bool,
    etag: Option<&str>,
) -> (Result<FeedDownload, FeedError>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let status = status.to_owned();
    let headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.extend(byte);
            assert!(request.len() < 8192);
        }
        let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        if chunked {
            response.push_str("Transfer-Encoding: chunked\r\n");
        } else {
            response.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        let _ = socket.write_all(response.as_bytes());
        if chunked {
            for chunk in body.chunks(4096) {
                if write!(socket, "{:x}\r\n", chunk.len())
                    .and_then(|_| socket.write_all(chunk))
                    .and_then(|_| socket.write_all(b"\r\n"))
                    .is_err()
                {
                    break;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n");
        } else {
            let _ = socket.write_all(&body);
        }
        String::from_utf8(request).unwrap()
    });
    // Only this test can replace the fixed production URL and allow loopback HTTP.
    let client = client_builder()
        .https_only(false)
        .no_proxy()
        .build()
        .unwrap();
    let mut outbound = request(&client, "test-client", etag).build().unwrap();
    assert_eq!(outbound.url().as_str(), MODEL_FEED_URL);
    *outbound.url_mut() = reqwest::Url::parse(&url).unwrap();
    let response = client.execute(outbound).unwrap();
    let result = read_response(response);
    (result, server.join().unwrap())
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then(|| value.trim()))
}

fn content(result: Result<FeedDownload, FeedError>) -> (Vec<u8>, Option<String>) {
    match result.unwrap() {
        FeedDownload::Content { bytes, etag } => (bytes, etag),
        _ => panic!("expected a full publication"),
    }
}

fn failure(result: Result<FeedDownload, FeedError>) -> FeedFailureKind {
    match result {
        Err(error) => error.kind,
        Ok(_) => panic!("expected retrieval to fail"),
    }
}

#[test]
fn model_feed_gzip_negotiation_preserves_decoded_bytes_digest_and_etag() {
    for (encoding, chunked) in [("gzip", false), ("GZip", true)] {
        let (result, request) = exchange(
            "200 OK",
            &[("Content-Encoding", encoding), ("ETag", "\"next\"")],
            gzip(FEED),
            chunked,
            Some("\"held\""),
        );
        assert_eq!(header(&request, "accept-encoding"), Some("gzip"));
        assert_eq!(header(&request, "if-none-match"), Some("\"held\""));
        assert_eq!(
            header(&request, "user-agent"),
            Some("HiRoute/test-client (model-feed)")
        );
        let (bytes, etag) = content(result);
        assert_eq!(bytes, FEED);
        assert_eq!(etag.as_deref(), Some("\"next\""));
        let parsed = parse_feed(&bytes).unwrap();
        assert_eq!(parsed.payload_digest, CanonicalDigest::of_bytes(FEED));
        assert_eq!(parsed.payload, parse_feed(FEED).unwrap().payload);
    }
}

#[test]
fn model_feed_accepts_plain_responses_and_filters_unsafe_etags() {
    for headers in [vec![], vec![("Content-Encoding", "identity")]] {
        let (result, request) = exchange(
            "200 OK",
            &headers,
            FEED.to_vec(),
            false,
            Some("unsafe\netag"),
        );
        assert_eq!(header(&request, "if-none-match"), None);
        let (bytes, etag) = content(result);
        assert_eq!(bytes, FEED);
        assert_eq!(etag, None);
    }
}

#[test]
fn model_feed_conditional_missing_and_redirect_responses_keep_their_meaning() {
    let (result, _) = exchange(
        "304 Not Modified",
        &[("Content-Encoding", "gzip")],
        vec![],
        false,
        Some("\"held\""),
    );
    assert!(matches!(result, Ok(FeedDownload::NotModified)));
    let (result, _) = exchange("404 Not Found", &[], vec![], false, None);
    assert!(matches!(result, Ok(FeedDownload::NotPublished)));
    let (result, _) = exchange(
        "302 Found",
        &[("Location", "http://127.0.0.1:9/unfollowed")],
        vec![],
        false,
        None,
    );
    assert_eq!(failure(result), FeedFailureKind::Network);
}

#[test]
fn model_feed_limits_decoded_gzip_and_plain_bodies_at_the_same_boundary() {
    for compressed in [false, true] {
        for excess in [0, 1] {
            let mut bytes = FEED.to_vec();
            bytes.resize(MAX_MODEL_FEED_BYTES + excess, b' ');
            let headers = if compressed {
                vec![("Content-Encoding", "gzip")]
            } else {
                vec![]
            };
            let body = if compressed { gzip(&bytes) } else { bytes };
            if compressed {
                assert!(body.len() < MAX_MODEL_FEED_BYTES);
            }
            let (result, _) = exchange("200 OK", &headers, body, compressed, None);
            if excess == 0 {
                let (bytes, _) = content(result);
                assert_eq!(bytes.len(), MAX_MODEL_FEED_BYTES);
                assert!(bytes.starts_with(FEED));
            } else {
                assert_eq!(failure(result), FeedFailureKind::Size);
            }
        }
    }
}

#[test]
fn model_feed_decodes_all_gzip_members_and_rejects_trailing_garbage() {
    let split = FEED.len() / 2;
    let mut body = gzip(&FEED[..split]);
    body.extend(gzip(&FEED[split..]));
    let headers = [("Content-Encoding", "gzip")];
    let (result, _) = exchange("200 OK", &headers, body, true, None);
    assert_eq!(content(result).0, FEED);

    let mut body = gzip(FEED);
    body.extend(gzip(&vec![b' '; MAX_MODEL_FEED_BYTES + 1 - FEED.len()]));
    let (result, _) = exchange("200 OK", &headers, body, false, None);
    assert_eq!(failure(result), FeedFailureKind::Size);

    let mut body = gzip(FEED);
    body.extend(b"not a gzip member");
    let (result, _) = exchange("200 OK", &headers, body, false, None);
    assert_eq!(failure(result), FeedFailureKind::Network);
}

#[test]
fn model_feed_bounds_encoded_gzip_headers_before_decoded_output_exists() {
    // Valid empty members can carry large headers without producing any decoded bytes.
    let encoder = flate2::GzBuilder::new()
        .extra(vec![0; u16::MAX as usize])
        .write(Vec::new(), Compression::fast());
    let member = encoder.finish().unwrap();
    let body = member.repeat(MAX_ENCODED_MODEL_FEED_BYTES as usize / member.len() + 1);
    assert!(body.len() as u64 > MAX_ENCODED_MODEL_FEED_BYTES);
    assert_eq!(
        read_gzip(body.as_slice()).unwrap_err().kind,
        FeedFailureKind::Size
    );
}

#[test]
fn model_feed_rejects_corrupt_gzip_and_unsupported_content_codings() {
    let encoded = gzip(FEED);
    let mut bad_crc = encoded.clone();
    let crc = bad_crc.len() - 8;
    bad_crc[crc] ^= 1;
    for body in [
        b"not gzip".to_vec(),
        encoded[..encoded.len() / 2].to_vec(),
        bad_crc,
    ] {
        let (result, _) = exchange("200 OK", &[("Content-Encoding", "gzip")], body, false, None);
        assert_eq!(failure(result), FeedFailureKind::Network);
    }
    for headers in [
        vec![("Content-Encoding", "br")],
        vec![("Content-Encoding", "gzip, gzip")],
        vec![("Content-Encoding", "gzip"), ("Content-Encoding", "gzip")],
    ] {
        let (result, _) = exchange("200 OK", &headers, gzip(FEED), false, None);
        assert_eq!(failure(result), FeedFailureKind::Schema);
    }
}

#[test]
#[ignore = "requires access to the production hiroute.ai feed"]
fn model_feed_live_https_gzip_smoke() {
    let client = client_builder().build().unwrap();
    let response = request(&client, env!("CARGO_PKG_VERSION"), None)
        .send()
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers()[reqwest::header::CONTENT_ENCODING],
        "gzip"
    );
    let (bytes, _) = content(read_response(response));
    let parsed = parse_feed(&bytes).unwrap();
    assert!(!parsed.payload.data.models.is_empty());
    assert!(
        !parsed
            .payload
            .data
            .metadata_catalog
            .model_records
            .is_empty()
    );
    println!(
        "gzip feed: decoded_bytes={} data_revision={} models={} metadata_records={}",
        bytes.len(),
        parsed.payload.data_revision,
        parsed.payload.data.models.len(),
        parsed.payload.data.metadata_catalog.model_records.len()
    );
}
