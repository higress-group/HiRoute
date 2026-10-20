//! Fixed-origin, bounded retrieval. No caller can supply a URL, credential or proxy header.

use super::{FeedError, FeedFailureKind, MAX_MODEL_FEED_BYTES, MODEL_FEED_URL};
use std::{io::Read, time::Duration};

pub enum FeedDownload {
    Content {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
    NotModified,
    NotPublished,
}

pub fn retrieve_feed(client_version: &str, etag: Option<&str>) -> Result<FeedDownload, FeedError> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .https_only(true)
        .build()
        .map_err(|_| network())?;
    let mut request = client.get(MODEL_FEED_URL).header(
        reqwest::header::USER_AGENT,
        format!("HiRoute/{client_version} (model-feed)"),
    );
    if let Some(etag) = etag.filter(|s| s.len() <= 1024 && !s.chars().any(char::is_control)) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().map_err(|_| network())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(FeedDownload::NotModified);
    }
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(FeedDownload::NotPublished);
    }
    if !response.status().is_success() {
        return Err(network());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_MODEL_FEED_BYTES as u64)
    {
        return Err(FeedError::new(
            FeedFailureKind::Size,
            "feed response exceeds bound",
        ));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.len() <= 1024 && !s.chars().any(char::is_control))
        .map(str::to_owned);
    let mut bytes = Vec::new();
    response
        .take(MAX_MODEL_FEED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| network())?;
    if bytes.len() > MAX_MODEL_FEED_BYTES {
        return Err(FeedError::new(
            FeedFailureKind::Size,
            "feed response exceeds bound",
        ));
    }
    Ok(FeedDownload::Content { bytes, etag })
}

fn network() -> FeedError {
    FeedError::new(FeedFailureKind::Network, "model feed retrieval failed")
}
