use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpError {
    #[error("cannot connect to E2E SUT: {0}")]
    Connect(std::io::Error),
    #[error("E2E HTTP I/O failed: {0}")]
    Io(std::io::Error),
    #[error("E2E HTTP exchange exceeded its deadline")]
    Timeout,
    #[error("E2E peer returned malformed HTTP: {0}")]
    Malformed(&'static str),
    #[error("E2E HTTP response exceeded the bounded capture limit")]
    TooLarge,
}

pub(crate) async fn post_json(
    addr: SocketAddr,
    path: &str,
    protocol_headers: &[(&str, &str)],
    body: &[u8],
    deadline: Duration,
) -> Result<HttpResponse, HttpError> {
    timeout(deadline, async {
        let mut stream = TcpStream::connect(addr).await.map_err(HttpError::Connect)?;
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in protocol_headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(HttpError::Io)?;
        stream.write_all(body).await.map_err(HttpError::Io)?;
        stream.flush().await.map_err(HttpError::Io)?;
        read_response(&mut stream).await
    })
    .await
    .map_err(|_| HttpError::Timeout)?
}

pub(crate) async fn get(
    addr: SocketAddr,
    path: &str,
    headers: &[(&str, &str)],
    deadline: Duration,
) -> Result<HttpResponse, HttpError> {
    timeout(deadline, async {
        let mut stream = TcpStream::connect(addr).await.map_err(HttpError::Connect)?;
        let mut request = format!(
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n"
        );
        for (name, value) in headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(HttpError::Io)?;
        stream.flush().await.map_err(HttpError::Io)?;
        read_response(&mut stream).await
    })
    .await
    .map_err(|_| HttpError::Timeout)?
}

async fn read_response(stream: &mut TcpStream) -> Result<HttpResponse, HttpError> {
    let mut wire = Vec::new();
    let mut scratch = [0_u8; 16 * 1024];
    let mut shape = None;
    loop {
        let count = stream.read(&mut scratch).await.map_err(HttpError::Io)?;
        if count == 0 {
            break;
        }
        wire.extend_from_slice(&scratch[..count]);
        if wire.len() > MAX_HTTP_BYTES {
            return Err(HttpError::TooLarge);
        }
        if shape.is_none() {
            shape = response_shape(&wire)?;
        }
        if shape
            .as_ref()
            .is_some_and(|shape| response_is_complete(&wire, shape))
        {
            break;
        }
    }
    parse_response(&wire)
}

#[derive(Clone, Debug)]
struct ResponseShape {
    body_offset: usize,
    content_length: Option<usize>,
    chunked: bool,
}

fn response_shape(wire: &[u8]) -> Result<Option<ResponseShape>, HttpError> {
    let Some(head_end) = find_bytes(wire, b"\r\n\r\n") else {
        return Ok(None);
    };
    let head = std::str::from_utf8(&wire[..head_end])
        .map_err(|_| HttpError::Malformed("non-UTF-8 response head"))?;
    let mut content_length = None;
    let mut chunked = false;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpError::Malformed("response header has no colon"));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse()
                    .map_err(|_| HttpError::Malformed("invalid content length"))?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    Ok(Some(ResponseShape {
        body_offset: head_end + 4,
        content_length,
        chunked,
    }))
}

fn response_is_complete(wire: &[u8], shape: &ResponseShape) -> bool {
    let body = &wire[shape.body_offset.min(wire.len())..];
    if shape.chunked {
        decode_chunked(body).is_ok_and(|decoded| decoded.is_some())
    } else if let Some(length) = shape.content_length {
        body.len() >= length
    } else {
        false
    }
}

fn parse_response(wire: &[u8]) -> Result<HttpResponse, HttpError> {
    let shape =
        response_shape(wire)?.ok_or(HttpError::Malformed("response ended before its head"))?;
    let head_end = shape.body_offset - 4;
    let head = std::str::from_utf8(&wire[..head_end])
        .map_err(|_| HttpError::Malformed("non-UTF-8 response head"))?;
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or(HttpError::Malformed("missing HTTP status"))?
        .parse()
        .map_err(|_| HttpError::Malformed("invalid HTTP status"))?;
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or(HttpError::Malformed("response header has no colon"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let wire_body = &wire[shape.body_offset..];
    let body = if shape.chunked {
        decode_chunked(wire_body)?.ok_or(HttpError::Malformed("incomplete chunked response"))?
    } else if let Some(length) = shape.content_length {
        if wire_body.len() < length {
            return Err(HttpError::Malformed("truncated response body"));
        }
        wire_body[..length].to_vec()
    } else {
        wire_body.to_vec()
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked(wire: &[u8]) -> Result<Option<Vec<u8>>, HttpError> {
    let (body, complete) = decode_chunked_prefix(wire)?;
    Ok(complete.then_some(body))
}

pub(crate) fn decode_chunked_prefix(wire: &[u8]) -> Result<(Vec<u8>, bool), HttpError> {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    loop {
        let Some(relative_end) = find_bytes(&wire[cursor..], b"\r\n") else {
            return Ok((decoded, false));
        };
        let size_end = cursor + relative_end;
        let size_text = std::str::from_utf8(&wire[cursor..size_end])
            .map_err(|_| HttpError::Malformed("non-UTF-8 chunk size"))?;
        let size =
            usize::from_str_radix(size_text.split(';').next().unwrap_or_default().trim(), 16)
                .map_err(|_| HttpError::Malformed("invalid chunk size"))?;
        cursor = size_end + 2;
        if size == 0 {
            // The zero-size line is not the entity terminator: trailers must
            // also end in an empty line, including when there are none.
            loop {
                let Some(end) = find_bytes(&wire[cursor..], b"\r\n") else {
                    return Ok((decoded, false));
                };
                if end == 0 {
                    return Ok((decoded, true));
                }
                if !wire[cursor..cursor + end].contains(&b':') {
                    return Err(HttpError::Malformed("invalid chunk trailer"));
                }
                cursor += end + 2;
            }
        }
        let data_end = cursor.checked_add(size).ok_or(HttpError::TooLarge)?;
        let chunk_end = data_end.checked_add(2).ok_or(HttpError::TooLarge)?;
        if wire.len() < chunk_end {
            decoded.extend_from_slice(&wire[cursor..wire.len().min(data_end)]);
            return Ok((decoded, false));
        }
        decoded.extend_from_slice(&wire[cursor..cursor + size]);
        cursor += size;
        if wire.get(cursor..cursor + 2) != Some(b"\r\n") {
            return Err(HttpError::Malformed("chunk is missing its terminator"));
        }
        cursor += 2;
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_chunked_sse_response() {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\ndata:\r\n3\r\n x\n\r\n0\r\n\r\n";
        let response = parse_response(wire).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"data: x\n");
    }

    #[test]
    fn incomplete_chunked_prefix_retains_bytes_without_completion() {
        for wire in [
            b"3\r\nabc\r\n".as_slice(),
            b"3\r\nabc\r\n0\r\n",
            b"3\r\nabc\r\n0\r\nX-Trace: present\r\n",
            b"3\r\nabc",
        ] {
            assert_eq!(
                decode_chunked_prefix(wire).unwrap(),
                (b"abc".to_vec(), false)
            );
            assert!(decode_chunked(wire).unwrap().is_none());
        }
        assert_eq!(
            decode_chunked_prefix(b"3\r\nabc\r\n0\r\nX-Trace: present\r\n\r\n").unwrap(),
            (b"abc".to_vec(), true)
        );
        assert!(decode_chunked_prefix(b"3\r\nabc\r\n0\r\ninvalid\r\n\r\n").is_err());
    }
}
