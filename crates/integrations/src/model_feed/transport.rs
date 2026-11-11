//! Fixed-origin, bounded retrieval. No caller can supply a URL, credential or proxy header.

use super::{FeedError, FeedFailureKind, MAX_MODEL_FEED_BYTES, MODEL_FEED_URL};
use flate2::read::MultiGzDecoder;
use std::{io::Read, time::Duration};

// Bound gzip headers and empty members as well as decoded JSON.
const MAX_ENCODED_MODEL_FEED_BYTES: u64 = 32 * 1024 * 1024;

pub enum FeedDownload {
    Content {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
    NotModified,
    NotPublished,
}

pub fn retrieve_feed(client_version: &str, etag: Option<&str>) -> Result<FeedDownload, FeedError> {
    let client = client_builder().build().map_err(|_| network())?;
    let response = request(&client, client_version, etag)
        .send()
        .map_err(|_| network())?;
    read_response(response)
}

fn client_builder() -> reqwest::blocking::ClientBuilder {
    // Decode only this feed, without enabling reqwest compression features for other clients.
    reqwest::blocking::Client::builder()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .https_only(true)
}

fn request(
    client: &reqwest::blocking::Client,
    client_version: &str,
    etag: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    let mut request = client
        .get(MODEL_FEED_URL)
        .header(reqwest::header::ACCEPT_ENCODING, "gzip")
        .header(
            reqwest::header::USER_AGENT,
            format!("HiRoute/{client_version} (model-feed)"),
        );
    if let Some(etag) = etag.filter(|s| s.len() <= 1024 && !s.chars().any(char::is_control)) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    request
}

fn read_response(response: reqwest::blocking::Response) -> Result<FeedDownload, FeedError> {
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(FeedDownload::NotModified);
    }
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(FeedDownload::NotPublished);
    }
    if !response.status().is_success() {
        return Err(network());
    }
    let mut encodings = response
        .headers()
        .get_all(reqwest::header::CONTENT_ENCODING)
        .iter();
    let encoding = encodings
        .next()
        .map(|value| value.to_str().map(str::trim))
        .transpose()
        .map_err(|_| unsupported_encoding())?;
    if encodings.next().is_some() {
        return Err(unsupported_encoding());
    }
    let gzip = match encoding {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("identity") => false,
        Some(value) if value.eq_ignore_ascii_case("gzip") => true,
        _ => return Err(unsupported_encoding()),
    };
    let encoded_limit = if gzip {
        MAX_ENCODED_MODEL_FEED_BYTES
    } else {
        MAX_MODEL_FEED_BYTES as u64
    };
    if response.content_length().is_some_and(|n| n > encoded_limit) {
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
    let bytes = if gzip {
        // All gzip members contribute to the decoded limit and exact publication identity.
        read_gzip(response)?
    } else {
        read_body(response)?
    };
    Ok(FeedDownload::Content { bytes, etag })
}

fn read_gzip(reader: impl Read) -> Result<Vec<u8>, FeedError> {
    let mut encoded = reader.take(MAX_ENCODED_MODEL_FEED_BYTES + 1);
    let decoded = read_body(MultiGzDecoder::new(&mut encoded));
    if encoded.limit() == 0 {
        return Err(FeedError::new(
            FeedFailureKind::Size,
            "encoded feed response exceeds bound",
        ));
    }
    decoded
}

fn read_body(reader: impl Read) -> Result<Vec<u8>, FeedError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_MODEL_FEED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| network())?;
    if bytes.len() > MAX_MODEL_FEED_BYTES {
        return Err(FeedError::new(
            FeedFailureKind::Size,
            "feed response exceeds bound",
        ));
    }
    Ok(bytes)
}

fn unsupported_encoding() -> FeedError {
    FeedError::new(FeedFailureKind::Schema, "unsupported feed content encoding")
}

fn network() -> FeedError {
    FeedError::new(FeedFailureKind::Network, "model feed retrieval failed")
}

#[cfg(test)]
mod tests;
